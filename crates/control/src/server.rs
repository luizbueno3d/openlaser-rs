//! The loopback control server (std::net on 127.0.0.1 only).
//!
//! Limits (AUT A9/A10, B10, review B7/X1): three separate pools.
//! - Pending: accepted, not yet authenticated, at most `MAX_PENDING_AUTH`. When it is full the oldest pending socket
//!   that has SENT NOTHING is closed to admit the new one (the oldest of all only if every pending socket has spoken),
//!   so a flood of silent sockets can neither refuse a newcomer nor hold a slot an agent needs, nor push out a client
//!   whose auth frame has already arrived but whose thread the OS has not run yet (bytes are looked for at accept,
//!   before that thread exists). A client whose bytes had not arrived yet can still lose its pending slot to a fast
//!   enough churn: the promise is that a stop gets through within `STOP_DELIVERY_BUDGET` by retrying, as the bridge's
//!   `StopHandle` does. Each pending socket has `AUTH_DEADLINE` (one deadline, not per read) and `AUTH_MAX_BYTES` to
//!   authenticate.
//! - Full service: up to `MAX_CONNECTIONS` authenticated connections.
//! - Stop-only: the next `STOP_ONLY_RESERVE` authenticated connections, each for exactly one `machine.stop`
//!   exchange within `AUTH_DEADLINE`. Past both, the auth gets an id-null "connection limit reached" and a close.
//!
//! Only a token holder takes a full or stop-only slot. A request line stops being read at `MAX_REQUEST_BYTES + 1`, so
//! a silent, trickling or endless client cannot hold a slot or a thread. `machine.stop` and `ping` are answered on
//! the connection's own thread and never touch the UI queue.
//!
//! Closing a connection (eviction, `disconnect_all`, `shutdown`) sets its flag before shutting the socket down, and
//! every read waits in `CLOSE_POLL` steps that look at the flag: on Windows, shutting a socket down from another
//! thread does not wake a read already waiting on it, and bytes sent afterwards are still delivered to that read. A
//! frame read on a closed connection is not served, except `machine.stop`, which is never refused once read.
//!
//! Stop delivery is AT-LEAST-ONCE. A client that gets no reply (its connection was closed under it, the reply was
//! lost) resends the stop on a fresh connection, so one stop request can reach FastStop more than once, every time
//! for the same machine. That is safe because the latch is idempotent (a second trip only latches again), and the
//! report a client receives is the one FastStop produced for the delivery it answers: never a cached, merged or
//! made-up one.

use std::collections::VecDeque;
use std::io::{BufReader, Read};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Error, budgets, discovery, token, wire};

/// A keep-alive answered on the transport thread (`{"pong":true}`), so a client can keep its stop connection warm
/// without waiting on the UI. Not a control method (§4.5): like `hello` and `auth` it never reaches the UI queue.
pub const PING_METHOD: &str = "ping";

pub struct ControlRequest {
    pub conn: u64,
    pub client: String,
    pub request: wire::Request,
    pub reply: std::sync::mpsc::Sender<wire::Reply>,
    pub deadline: std::time::Instant,
    pub dispatch: Arc<Dispatch>,
}

/// QUEUED → DISPATCHED (UI thread starts it) or QUEUED → ABANDONED (deadline passed), settled once, so a
/// timeout says either "not run, safe to resend" or "may still apply".
#[derive(Debug, Default)]
pub struct Dispatch {
    state: AtomicU8,
}

impl Dispatch {
    pub fn try_dispatch(&self) -> bool {
        self.state.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub fn abandon(&self) -> bool {
        self.state.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
}

/// `machine.stop` is answered on the transport thread through this, before anything touches the UI queue. It may be
/// called more than once for one client stop (delivery is at-least-once, see the module docs), so it must be
/// idempotent and report each call truthfully.
pub type FastStop = Arc<dyn Fn(Option<&str>) -> serde_json::Value + Send + Sync>;

pub struct ServerConfig {
    /// 0 = free port.
    pub port: u16,
    pub token: token::Token,
    pub fast_stop: FastStop,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// One socket, shared by its reader, its writer and the handles that close it (one file descriptor per connection).
type Socket = Arc<TcpStream>;

/// How often a waiting read looks up to see whether its connection was closed (evicted, `disconnect_all`,
/// `shutdown`). Shutting the socket down wakes a blocked read on macOS and Linux; on Windows it does not, and this
/// bounds a closed connection's thread there (instead of the 30 s idle timeout).
const CLOSE_POLL: Duration = Duration::from_millis(100);

/// What the server holds to close a connection: its socket, and the flag its thread looks at.
#[derive(Clone)]
struct Closer {
    socket: Socket,
    closed: Arc<AtomicBool>,
}

impl Closer {
    /// The flag first, so the thread stops waiting within `CLOSE_POLL` wherever the shutdown does not wake it.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

/// A pending connection as the pools see it.
struct Pending {
    conn: u64,
    closer: Closer,
    /// Bytes have arrived on it (seen at accept, or read since): it is closed to make room only when no pending
    /// socket is left that has sent nothing.
    spoke: Arc<AtomicBool>,
}

/// An authenticated full-service connection, as `clients()` and `disconnect_all` see it.
struct Client {
    conn: u64,
    name: String,
    closer: Closer,
}

type Clients = Arc<Mutex<Vec<Client>>>;

/// Which pool a connection counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Pending,
    Full,
    StopOnly,
}

/// Why an authenticated connection gets no slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoSlot {
    /// It was closed to make room in the pending pool (or by `disconnect_all`) while it authenticated.
    Evicted,
    /// Every full and stop-only slot is held by an authenticated connection.
    Full,
}

#[derive(Default)]
struct Pools {
    /// Accepted, not yet authenticated, oldest first.
    pending: VecDeque<Pending>,
    full: usize,
    stop_only: usize,
}

impl Pools {
    /// Admits a freshly accepted socket, never refusing it: while the pool is full, the oldest pending socket that
    /// has sent nothing is closed, or, when every one of them has spoken, the oldest.
    fn add_pending(&mut self, pending: Pending) {
        while self.pending.len() >= budgets::MAX_PENDING_AUTH.max(1) {
            let index = self.pending.iter().position(|p| !p.spoke.load(Ordering::Acquire)).unwrap_or(0);
            match self.pending.remove(index) {
                Some(victim) => victim.closer.close(),
                None => break,
            }
        }
        self.pending.push_back(pending);
    }

    /// Moves a pending connection that has just authenticated into a full or stop-only slot.
    fn authenticate(&mut self, conn: u64) -> Result<Class, NoSlot> {
        let index = self.pending.iter().position(|p| p.conn == conn).ok_or(NoSlot::Evicted)?;
        let class = if self.full < budgets::MAX_CONNECTIONS {
            Class::Full
        } else if self.stop_only < budgets::STOP_ONLY_RESERVE {
            Class::StopOnly
        } else {
            return Err(NoSlot::Full);
        };
        self.pending.remove(index);
        match class {
            Class::Full => self.full = self.full.saturating_add(1),
            Class::StopOnly => self.stop_only = self.stop_only.saturating_add(1),
            Class::Pending => {}
        }
        Ok(class)
    }

    fn release(&mut self, conn: u64, class: Class) {
        match class {
            Class::Pending => self.pending.retain(|p| p.conn != conn),
            Class::Full => self.full = self.full.saturating_sub(1),
            Class::StopOnly => self.stop_only = self.stop_only.saturating_sub(1),
        }
    }

    fn live(&self) -> usize {
        self.pending.len().saturating_add(self.full).saturating_add(self.stop_only)
    }
}

type SharedPools = Arc<Mutex<Pools>>;

fn lock_pools(pools: &SharedPools) -> std::sync::MutexGuard<'_, Pools> {
    pools.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A connection's place in the pools, given back however its thread ends (a failed spawn or a panic included).
struct Slot {
    pools: SharedPools,
    conn: u64,
    class: Class,
}

impl Drop for Slot {
    fn drop(&mut self) {
        lock_pools(&self.pools).release(self.conn, self.class);
    }
}

/// True when the client has already sent something (typically its auth frame, written right after connecting),
/// looked at without consuming it. Leaves the socket non-blocking: the caller sets the mode it needs next.
fn has_spoken(stream: &TcpStream) -> bool {
    stream.set_nonblocking(true).is_ok() && matches!(stream.peek(&mut [0_u8; 1]), Ok(n) if n > 0)
}

/// std::net on 127.0.0.1 only; ≤ 16 connections (+ a stop-only reserve, + a pool of pending sockets of its own);
/// 30 s idle; first frame must be {"method":"auth","params":{"token","client":{"name","version"}}} (an optional
/// `hello` may come first, answered with the server's pid so a client can check the discovery file before it sends
/// the token).
pub struct ControlServer {
    port: u16,
    stop: Arc<AtomicBool>,
    clients: Clients,
    pools: SharedPools,
    thread: Option<JoinHandle<()>>,
}

/// Removes an authenticated connection from the client list however its thread ends (a panic included).
struct Registration {
    clients: Clients,
    conn: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner).retain(|client| client.conn != self.conn);
    }
}

impl ControlServer {
    pub fn start(cfg: ServerConfig) -> Result<(Self, std::sync::mpsc::Receiver<ControlRequest>), Error> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cfg.port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let clients: Clients = Arc::new(Mutex::new(Vec::new()));
        let pools: SharedPools = Arc::new(Mutex::new(Pools::default()));
        let thread_stop = Arc::clone(&stop);
        let thread_clients = Arc::clone(&clients);
        let thread_pools = Arc::clone(&pools);
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared { token: cfg.token, fast_stop: cfg.fast_stop, wake: cfg.wake });
        let thread = thread::Builder::new().name("openlaser-control".to_owned()).spawn(move || {
            let mut next_conn = 1_u64;
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        // Looked at here, before the connection's thread exists, so whether the pool spares it does
                        // not depend on how soon the OS runs that thread.
                        let spoke = Arc::new(AtomicBool::new(has_spoken(&stream)));
                        if stream.set_nonblocking(false).is_err() {
                            let _ = stream.shutdown(Shutdown::Both);
                            continue;
                        }
                        let socket: Socket = Arc::new(stream);
                        let id = next_conn;
                        next_conn = next_conn.saturating_add(1);
                        let closer = Closer { socket: Arc::clone(&socket), closed: Arc::new(AtomicBool::new(false)) };
                        // Never refused for capacity here: a silent pending socket (or the oldest) makes room instead.
                        lock_pools(&thread_pools).add_pending(Pending { conn: id, closer: closer.clone(), spoke: Arc::clone(&spoke) });
                        let slot = Slot { pools: Arc::clone(&thread_pools), conn: id, class: Class::Pending };
                        let context = ConnectionContext {
                            conn: id,
                            closed: closer.closed,
                            spoke,
                            shared: Arc::clone(&shared),
                            tx: tx.clone(),
                            clients: Arc::clone(&thread_clients),
                            stop: Arc::clone(&thread_stop),
                        };
                        let fallback = Arc::clone(&socket);
                        let spawned = thread::Builder::new().name(format!("openlaser-control-{id}")).spawn(move || {
                            serve_connection(&socket, slot, context);
                        });
                        // The closure (socket handle and slot) is dropped with the error; tell the client why.
                        if spawned.is_err() {
                            refuse(&fallback, "server busy: cannot start a connection thread; retry");
                        }
                    }
                    Ok((stream, _)) => {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                    Err(_) => thread::sleep(Duration::from_millis(25)),
                }
            }
        })?;
        Ok((Self { port, stop, clients, pools, thread: Some(thread) }, rx))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Authenticated clients, "name version" each.
    pub fn clients(&self) -> Vec<String> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|client| client.name.clone()).collect()
    }

    /// Every live socket, authenticated or not (diagnostics and tests).
    pub fn live_connections(&self) -> usize {
        lock_pools(&self.pools).live()
    }

    /// Sockets accepted but not yet authenticated (diagnostics and tests); never more than `MAX_PENDING_AUTH`.
    pub fn pending_connections(&self) -> usize {
        lock_pools(&self.pools).pending.len()
    }

    /// Closes every agent connection, and every socket that has not authenticated yet: their threads stop within
    /// `CLOSE_POLL` on every platform, and nothing read on them afterwards is served (a stop excepted).
    pub fn disconnect_all(&self) {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        for client in clients.iter() {
            client.closer.close();
        }
        clients.clear();
        drop(clients);
        // Out of the pool as well, so one whose auth frame was already read cannot take a slot.
        for pending in lock_pools(&self.pools).pending.drain(..) {
            pending.closer.close();
        }
    }

    pub fn shutdown(mut self) {
        self.disconnect_all();
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

struct Shared {
    token: token::Token,
    fast_stop: FastStop,
    wake: Arc<dyn Fn() + Send + Sync>,
}

struct ConnectionContext {
    conn: u64,
    closed: Arc<AtomicBool>,
    spoke: Arc<AtomicBool>,
    shared: Arc<Shared>,
    tx: std::sync::mpsc::Sender<ControlRequest>,
    clients: Clients,
    stop: Arc<AtomicBool>,
}

/// Writes one id-null error reply on a connection that will not be served, then closes it. Never blocks for long:
/// a fresh socket's send buffer is empty, and the write timeout bounds the rest.
fn refuse(stream: &TcpStream, message: &str) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let mut writer = stream;
    let _ = wire::write_reply(&mut writer, &error_reply(serde_json::Value::Null, message));
    let _ = stream.shutdown(Shutdown::Both);
}

fn error_reply(id: serde_json::Value, message: &str) -> wire::Reply {
    wire::Reply { id, ok: false, result: None, error: Some(message.to_owned()) }
}

/// Reads through one deadline for the whole pre-auth exchange (a per-read timeout would let a client trickle a
/// byte at a time forever); after a full-service authentication the deadline is cleared and the idle timeout
/// applies per read, while a stop-only connection keeps a deadline for its one exchange. Every wait is cut into
/// `CLOSE_POLL` steps that look at the connection's flag and the server's.
struct DeadlineStream {
    stream: Socket,
    deadline: Option<Instant>,
    /// Set when the server closes this connection (eviction, `disconnect_all`).
    closed: Arc<AtomicBool>,
    /// Set when the server shuts down.
    server_stop: Arc<AtomicBool>,
    /// Set on the first byte read: the pending pool spares a socket that has spoken.
    spoke: Arc<AtomicBool>,
}

impl DeadlineStream {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.server_stop.load(Ordering::Acquire)
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut stream: &TcpStream = &self.stream;
        let limit = self.deadline.unwrap_or_else(|| Instant::now() + budgets::IDLE_TIMEOUT);
        loop {
            // Before every wait: bytes that arrive on a closed connection are not read.
            if self.is_closed() {
                return Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "the server closed this connection"));
            }
            let remaining = limit.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let what = if self.deadline.is_some() { "authentication deadline passed" } else { "idle timeout" };
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, what));
            }
            self.stream.set_read_timeout(Some(remaining.min(CLOSE_POLL)))?;
            match stream.read(buf) {
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Ok(n) => {
                    if n > 0 {
                        self.spoke.store(true, Ordering::Release);
                    }
                    return Ok(n);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

enum Frame {
    Request(wire::Request),
    /// The line was read in full but is not a request: answer "bad JSON" and keep serving.
    Bad(String),
    /// Over the budget: answer, then close (the rest of the line is never read).
    TooLong(usize),
    Closed,
}

fn read_frame(reader: &mut impl std::io::BufRead, max: usize) -> Frame {
    match wire::read_line_bounded(reader, max) {
        Ok(Some(line)) => match serde_json::from_slice::<wire::Request>(&line) {
            Ok(request) => Frame::Request(request),
            Err(e) => lenient_stop(&line).map_or_else(|| Frame::Bad(e.to_string()), Frame::Request),
        },
        Ok(None) => Frame::Closed,
        Err(Error::LineTooLong { max }) => Frame::TooLong(max),
        Err(_) => Frame::Closed,
    }
}

/// A JSON object naming `machine.stop` that is not otherwise a valid request (no `id`, say) is still a stop: a stop
/// is never refused for the shape of its frame. Before authentication it is refused like any other method.
fn lenient_stop(line: &[u8]) -> Option<wire::Request> {
    let value: serde_json::Value = serde_json::from_slice(line).ok()?;
    (value.get("method")?.as_str()? == "machine.stop").then(|| wire::Request {
        id: value.get("id").cloned().unwrap_or(serde_json::Value::Null),
        method: "machine.stop".to_owned(),
        params: value.get("params").cloned().unwrap_or(serde_json::Value::Null),
    })
}

/// Printable, bounded text for the GUI's "Agent connected: …" indicator and logs.
fn sanitize(text: Option<&str>) -> String {
    let clean: String = text.unwrap_or("").chars().filter(|c| !c.is_control()).take(budgets::CLIENT_NAME_MAX_CHARS).collect();
    let clean = clean.trim();
    if clean.is_empty() { "unknown".to_owned() } else { clean.to_owned() }
}

fn serve_connection(socket: &Socket, mut slot: Slot, context: ConnectionContext) {
    let ConnectionContext { conn, closed, spoke, shared, tx, clients, stop } = context;
    let _ = socket.set_write_timeout(Some(budgets::IDLE_TIMEOUT));
    let mut stream: &TcpStream = socket;
    let deadline = Some(Instant::now() + budgets::AUTH_DEADLINE);
    let mut reader =
        BufReader::new(DeadlineStream { stream: Arc::clone(socket), deadline, closed: Arc::clone(&closed), server_stop: Arc::clone(&stop), spoke });

    // ---- pre-auth: an optional `hello`, then `auth`, all within AUTH_DEADLINE and AUTH_MAX_BYTES per frame.
    let mut said_hello = false;
    let (auth_id, client) = loop {
        let request = match read_frame(&mut reader, budgets::AUTH_MAX_BYTES) {
            Frame::Request(request) => request,
            Frame::Bad(_) => {
                let _ = wire::write_reply(&mut stream, &error_reply(serde_json::Value::Null, "authentication required"));
                return;
            }
            Frame::TooLong(max) => {
                let _ = wire::write_reply(&mut stream, &error_reply(serde_json::Value::Null, &format!("authentication frame exceeds {max} bytes")));
                return;
            }
            Frame::Closed => return,
        };
        if request.method == discovery::HELLO_METHOD && !said_hello {
            said_hello = true;
            let hello = serde_json::json!({"server":"openlaser","pid":std::process::id(),"protocol":1});
            if wire::write_reply(&mut stream, &wire::Reply { id: request.id, ok: true, result: Some(hello), error: None }).is_err() {
                return;
            }
            continue;
        }
        if request.method != "auth" {
            let _ = wire::write_reply(&mut stream, &error_reply(request.id, "authentication required"));
            return;
        }
        let token_value = request.params.get("token").and_then(serde_json::Value::as_str).unwrap_or("");
        if !shared.token.matches(token_value) {
            let _ = wire::write_reply(&mut stream, &error_reply(request.id, "authentication failed"));
            return;
        }
        let info = request.params.get("client");
        let name = sanitize(info.and_then(|v| v.get("name")).and_then(serde_json::Value::as_str));
        let version = sanitize(info.and_then(|v| v.get("version")).and_then(serde_json::Value::as_str));
        break (request.id, format!("{name} {version}"));
    };
    // Only now, with the token proven, does the connection take a full or stop-only slot.
    let class = match lock_pools(&slot.pools).authenticate(conn) {
        Ok(class) => class,
        // Closed to make room (or by disconnect_all) while it authenticated: nothing more to say on it.
        Err(NoSlot::Evicted) => return,
        Err(NoSlot::Full) => {
            let _ = wire::write_reply(&mut stream, &error_reply(serde_json::Value::Null, "connection limit reached"));
            return;
        }
    };
    slot.class = class;
    let stop_only = class == Class::StopOnly;
    // One exchange, and promptly, on a stop-only slot (it is not somewhere to wait); otherwise the idle timeout.
    reader.get_mut().deadline = stop_only.then(|| Instant::now() + budgets::AUTH_DEADLINE);
    let _registration = Registration { clients: Arc::clone(&clients), conn };
    if !stop_only {
        let closer = Closer { socket: Arc::clone(socket), closed: Arc::clone(&closed) };
        clients.lock().unwrap_or_else(PoisonError::into_inner).push(Client { conn, name: client.clone(), closer });
    }
    let authenticated = serde_json::json!({"authenticated":true, "stopOnly": stop_only});
    if wire::write_reply(&mut stream, &wire::Reply { id: auth_id, ok: true, result: Some(authenticated), error: None }).is_err() {
        return;
    }

    // ---- authenticated: one request at a time.
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let request = match read_frame(&mut reader, budgets::MAX_REQUEST_BYTES) {
            Frame::Request(request) => request,
            Frame::Bad(error) => {
                if wire::write_reply(&mut stream, &error_reply(serde_json::Value::Null, &format!("bad JSON: {error}"))).is_err() || stop_only {
                    break;
                }
                continue;
            }
            Frame::TooLong(max) => {
                let _ = wire::write_reply(&mut stream, &error_reply(serde_json::Value::Null, &format!("request exceeds {max} bytes; connection closed")));
                break;
            }
            Frame::Closed => break,
        };
        // Closed by the server while this frame arrived (Windows delivers bytes to a read that was already waiting
        // when the socket was shut down): nothing more is served here, except a stop, which is never refused once
        // read. Its reply cannot be delivered, so the client resends it elsewhere (at-least-once, module docs).
        let closing = reader.get_ref().is_closed();
        if request.method == "machine.stop" {
            // A machine that is not a string means every machine: a stop is never refused for its params.
            let machine = request.params.get("machine").and_then(serde_json::Value::as_str);
            let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (shared.fast_stop)(machine))) {
                Ok(value) => wire::Reply { id: request.id, ok: true, result: Some(value), error: None },
                Err(_) => error_reply(request.id, "internal error: machine.stop panicked; the stop may not have reached the machine: use the E-stop"),
            };
            if wire::write_reply(&mut stream, &reply).is_err() || stop_only || closing {
                break;
            }
            continue;
        }
        if closing {
            break;
        }
        if request.method == "auth" {
            if wire::write_reply(&mut stream, &error_reply(request.id, "already authenticated")).is_err() {
                break;
            }
            continue;
        }
        if stop_only {
            let _ = wire::write_reply(&mut stream, &error_reply(request.id, "connection limit reached"));
            break;
        }
        if request.method == PING_METHOD {
            let pong = wire::Reply { id: request.id, ok: true, result: Some(serde_json::json!({"pong":true})), error: None };
            if wire::write_reply(&mut stream, &pong).is_err() {
                break;
            }
            continue;
        }
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let request_id = request.id.clone();
        let dispatch = Arc::new(Dispatch::default());
        let control_request = ControlRequest {
            conn,
            client: client.clone(),
            request,
            reply: reply_tx,
            deadline: Instant::now() + budgets::APP_DEADLINE,
            dispatch: Arc::clone(&dispatch),
        };
        if tx.send(control_request).is_err() {
            let _ = wire::write_reply(&mut stream, &error_reply(request_id, "the app is shutting down; the request was not run"));
            break;
        }
        (shared.wake)();
        match reply_rx.recv_timeout(budgets::APP_DEADLINE) {
            Ok(reply) => {
                if wire::write_reply(&mut stream, &reply).is_err() {
                    break;
                }
            }
            Err(_) => {
                let safe = dispatch.abandon();
                let msg = if safe {
                    "timeout: not run (the app did not pick the request up in 60s); safe to resend"
                } else {
                    "timeout: no reply in 60s; the command may still be running and may still apply; inspect state before retrying"
                };
                let _ = wire::write_reply(&mut stream, &error_reply(request_id, msg));
                if !safe {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::AtomicUsize;

    fn start(fast_stop: FastStop) -> (ControlServer, std::sync::mpsc::Receiver<ControlRequest>, String) {
        let token = token::Token::generate().unwrap();
        let hex = token.to_hex();
        let (server, rx) = ControlServer::start(ServerConfig { port: 0, token, fast_stop, wake: Arc::new(|| {}) }).unwrap();
        (server, rx, hex)
    }

    fn send(stream: &mut TcpStream, id: u64, method: &str, params: serde_json::Value) {
        let mut line = serde_json::to_vec(&wire::Request { id: serde_json::json!(id), method: method.to_owned(), params }).unwrap();
        line.push(b'\n');
        stream.write_all(&line).unwrap();
        stream.flush().unwrap();
    }

    /// The next reply, or None when the connection is closed (or nothing comes within 5 s).
    fn recv(stream: &TcpStream) -> Option<wire::Reply> {
        // macOS refuses socket options (EINVAL) on a socket the peer has already reset: nothing more will come.
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let mut reader = BufReader::new(stream.try_clone().ok()?);
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&line).unwrap()),
        }
    }

    fn connect(port: u16, token: &str, client: &str) -> TcpStream {
        let (stream, reply) = try_connect(port, token, client);
        assert!(reply.ok, "{reply:?}");
        stream
    }

    /// Connects and sends the auth frame; returns the socket and the server's answer to it.
    fn try_connect(port: u16, token: &str, client: &str) -> (TcpStream, wire::Reply) {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        send(&mut stream, 0, "auth", serde_json::json!({"token":token,"client":{"name":client,"version":"test"}}));
        let reply = recv(&stream).expect("an answer to the auth frame");
        (stream, reply)
    }

    fn stop_only(reply: &wire::Reply) -> bool {
        reply.result.as_ref().and_then(|r| r.get("stopOnly")).and_then(serde_json::Value::as_bool).unwrap()
    }

    fn silent(port: u16) -> TcpStream {
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap()
    }

    fn wait_pending(server: &ControlServer, n: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        while server.pending_connections() != n && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(server.pending_connections(), n);
    }

    /// True when the peer closed `stream` (EOF or reset) within `within`, without sending anything.
    fn closed_within(stream: &TcpStream, within: Duration) -> bool {
        // macOS refuses socket options (EINVAL) on a socket the peer has already reset: that is a closed socket.
        if stream.set_read_timeout(Some(within)).is_err() {
            return true;
        }
        let mut byte = [0_u8; 1];
        let mut reader = stream;
        match reader.read(&mut byte) {
            Ok(0) => true,
            Ok(_) => false,
            Err(e) => !matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut),
        }
    }

    fn wait_live(server: &ControlServer, n: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        while server.live_connections() != n && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(server.live_connections(), n);
    }

    #[test]
    fn fast_stop_answers_without_ui_thread() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let (server, _receiver, token) = start(Arc::new(move |machine| {
            observed.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"latched":true,"machine":machine})
        }));
        let mut stream = connect(server.port(), &token, "stop-test");
        let started = Instant::now();
        send(&mut stream, 1, "machine.stop", serde_json::json!({}));
        assert!(recv(&stream).unwrap().ok);
        assert!(started.elapsed() < budgets::STOP_WAIT);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.shutdown();
    }

    #[test]
    fn stop_accepted_on_17th_connection() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut connections = Vec::new();
        for index in 0..budgets::MAX_CONNECTIONS {
            connections.push(connect(server.port(), &token, &format!("client-{index}")));
        }
        let mut last = connect(server.port(), &token, "stop-only");
        send(&mut last, 17, "machine.stop", serde_json::json!({}));
        assert!(recv(&last).unwrap().ok);
        // Exactly one exchange: the stop-only connection is then closed.
        assert!(recv(&last).is_none());
        let mut other = connect(server.port(), &token, "stop-only-2");
        send(&mut other, 18, "doc.inspect", serde_json::json!({}));
        assert_eq!(recv(&other).unwrap().error.as_deref(), Some("connection limit reached"));
        drop(connections);
        server.shutdown();
    }

    /// X1 (B7): sockets that never authenticate live in a pool of their own. When it is full the OLDEST is closed
    /// to admit the newcomer, so silent sockets can neither refuse a connection nor take an agent's or the stop
    /// reserve's slot: with the pool flooded, a client still gets full service, the 17th a stop, and only
    /// authenticated connections can exhaust the slots (then the auth gets an id-null "connection limit reached").
    #[test]
    fn unauthenticated_sockets_have_their_own_pool() {
        let stops = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&stops);
        let (server, receiver, token) = start(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"latched":true})
        }));
        let port = server.port();
        let extra = 8;
        let flood: Vec<TcpStream> = (0..budgets::MAX_PENDING_AUTH + extra).map(|_| silent(port)).collect();
        wait_pending(&server, budgets::MAX_PENDING_AUTH);
        // The oldest were closed to make room, at once (not at their AUTH_DEADLINE); the newest are still open.
        let started = Instant::now();
        for stream in flood.iter().take(extra) {
            assert!(closed_within(stream, Duration::from_secs(5)), "an evicted pending socket was left open");
        }
        assert!(started.elapsed() < budgets::AUTH_DEADLINE, "evictions waited for the auth deadline: {:?}", started.elapsed());
        assert!(!closed_within(flood.last().unwrap(), Duration::from_millis(50)), "the newest pending socket was closed");
        assert!(server.pending_connections() <= budgets::MAX_PENDING_AUTH);

        // With the pending pool full, a client still authenticates, with full service.
        let (mut first, reply) = try_connect(port, &token, "first");
        assert!(reply.ok && !stop_only(&reply), "{reply:?}");
        send(&mut first, 1, "doc.inspect", serde_json::json!({}));
        let queued = receiver.recv_timeout(Duration::from_secs(5)).expect("the request reached the UI queue");
        assert!(queued.dispatch.try_dispatch());
        queued.reply.send(wire::Reply { id: queued.request.id, ok: true, result: Some(serde_json::json!("ok")), error: None }).unwrap();
        assert!(recv(&first).unwrap().ok);

        // Every full-service slot goes to a token holder, however many silent sockets come and go meanwhile.
        let mut agents = vec![first];
        for index in 1..budgets::MAX_CONNECTIONS {
            let _churn = silent(port);
            let (stream, reply) = try_connect(port, &token, &format!("agent-{index}"));
            assert!(reply.ok && !stop_only(&reply), "agent {index}: {reply:?}");
            agents.push(stream);
        }
        assert_eq!(server.clients().len(), budgets::MAX_CONNECTIONS);
        let (mut stopper, reply) = try_connect(port, &token, "stopper");
        assert!(reply.ok && stop_only(&reply), "{reply:?}");
        send(&mut stopper, 2, "machine.stop", serde_json::json!({}));
        assert!(recv(&stopper).unwrap().ok);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(recv(&stopper).is_none(), "a stop-only connection serves exactly one exchange");

        // The reserve, held by authenticated (idle) stop-only connections: the next auth is refused, with id null.
        let reserve: Vec<TcpStream> = (0..budgets::STOP_ONLY_RESERVE)
            .map(|index| try_connect(port, &token, &format!("reserve-{index}")))
            .map(|(stream, reply)| {
                assert!(reply.ok && stop_only(&reply), "{reply:?}");
                stream
            })
            .collect();
        let (_refused, reply) = try_connect(port, &token, "one-too-many");
        assert_eq!((reply.id, reply.ok, reply.error.as_deref()), (serde_json::Value::Null, false, Some("connection limit reached")));
        // A freed reserve slot is a stop slot again.
        drop(reserve);
        wait_stop_only(&server, 0);
        let (mut late, reply) = try_connect(port, &token, "late-stop");
        assert!(reply.ok && stop_only(&reply), "{reply:?}");
        send(&mut late, 3, "machine.stop", serde_json::json!({}));
        assert!(recv(&late).unwrap().ok);
        assert_eq!(stops.load(Ordering::SeqCst), 2);
        drop((flood, agents));
        server.shutdown();
    }

    fn wait_stop_only(server: &ControlServer, n: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        while lock_pools(&server.pools).stop_only != n && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(lock_pools(&server.pools).stop_only, n);
    }

    /// X1 at the wire: 16 authenticated agents plus a flood of ≥ 40 silent sockets, reopened as they close. The
    /// promise is the one `StopHandle` relies on: a stop gets through within STOP_DELIVERY_BUDGET, retrying as it
    /// must (one connection attempt may lose its pending slot to the churn before its bytes arrive), round after
    /// round. Delivery is at-least-once, so each round asserts at least one FastStop call, not exactly one.
    #[test]
    fn stop_gets_through_a_silent_socket_flood() {
        let stops = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&stops);
        let (server, _receiver, token) = start(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"latched":true})
        }));
        let port = server.port();
        let agents: Vec<TcpStream> = (0..budgets::MAX_CONNECTIONS).map(|index| connect(port, &token, &format!("agent-{index}"))).collect();
        let flood = Flood::start(port, 48);
        flood.wait_saturated(&server);
        for round in 0..5_u64 {
            let before = stops.load(Ordering::SeqCst);
            let started = Instant::now();
            if let Err(why) = stop_with_retry(port, &token, round) {
                panic!("round {round}: {why}");
            }
            assert!(started.elapsed() < budgets::STOP_DELIVERY_BUDGET, "round {round}: {:?}", started.elapsed());
            assert!(stops.load(Ordering::SeqCst) > before, "round {round}: answered, but FastStop was not called");
        }
        assert!(flood.reopened() > 0, "the flood never churned");
        flood.finish();
        drop(agents);
        server.shutdown();
    }

    /// What `StopHandle` does, at the wire: connect, auth, `machine.stop`; anything short of a reply (no answer to
    /// the auth frame, a refusal, a closed socket) is retried after a short backoff until STOP_DELIVERY_BUDGET runs
    /// out. Returns the number of attempts it took.
    fn stop_with_retry(port: u16, token: &str, id: u64) -> Result<usize, String> {
        let deadline = Instant::now() + budgets::STOP_DELIVERY_BUDGET;
        let mut pause = Duration::from_millis(5);
        let mut attempts = 0_usize;
        let mut last = String::from("no attempt");
        while Instant::now() < deadline {
            attempts += 1;
            match stop_once(port, token, id, deadline) {
                Ok(()) => return Ok(attempts),
                Err(why) => last = why,
            }
            thread::sleep(pause.min(deadline.saturating_duration_since(Instant::now())));
            pause = (pause * 2).min(Duration::from_millis(50));
        }
        Err(format!("no stop got through within {:?} ({attempts} attempts; last: {last})", budgets::STOP_DELIVERY_BUDGET))
    }

    fn stop_once(port: u16, token: &str, id: u64, deadline: Instant) -> Result<(), String> {
        let wait = |stream: &TcpStream| {
            let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
            stream.set_read_timeout(Some(left.min(Duration::from_millis(500)))).map_err(|e| e.to_string())
        };
        let mut stream = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_millis(250)).map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut exchange = |stream: &mut TcpStream, request: serde_json::Value| -> Result<wire::Reply, String> {
            wait(&*stream)?;
            writeln!(stream, "{request}").map_err(|e| format!("send: {e}"))?;
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => Err("closed without an answer".to_owned()),
                Ok(_) => serde_json::from_str(&line).map_err(|e| e.to_string()),
                Err(e) => Err(format!("no answer: {e}")),
            }
        };
        let auth = exchange(&mut stream, serde_json::json!({"id":0,"method":"auth","params":{"token":token,"client":{"name":"stopper","version":"test"}}}))?;
        if !auth.ok {
            return Err(format!("auth: {auth:?}"));
        }
        let stop = exchange(&mut stream, serde_json::json!({"id":id,"method":"machine.stop","params":{}}))?;
        if stop.ok && stop.id == serde_json::json!(id) { Ok(()) } else { Err(format!("stop: {stop:?}")) }
    }

    /// The pending pool spares a socket that has spoken: when it is full, the oldest SILENT socket makes room, and
    /// the oldest of all only once every pending socket has spoken.
    #[test]
    fn pending_pool_evicts_the_oldest_silent_socket_first() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut keep = Vec::new();
        let mut pending = |conn: u64, spoke: bool| {
            keep.push(TcpStream::connect(listener.local_addr().unwrap()).unwrap());
            let (accepted, _) = listener.accept().unwrap();
            let closer = Closer { socket: Arc::new(accepted), closed: Arc::new(AtomicBool::new(false)) };
            Pending { conn, closer, spoke: Arc::new(AtomicBool::new(spoke)) }
        };
        let mut pools = Pools::default();
        let max = budgets::MAX_PENDING_AUTH as u64;
        // conn 1 (the oldest) has spoken; conns 2..=max have not.
        for conn in 1..=max {
            pools.add_pending(pending(conn, conn == 1));
        }
        let flags: Vec<(u64, Arc<AtomicBool>)> = pools.pending.iter().map(|p| (p.conn, Arc::clone(&p.closer.closed))).collect();
        pools.add_pending(pending(max + 1, false));
        pools.add_pending(pending(max + 2, false));
        let conns: Vec<u64> = pools.pending.iter().map(|p| p.conn).collect();
        assert_eq!(conns.len(), budgets::MAX_PENDING_AUTH);
        assert_eq!(conns.first(), Some(&1), "the socket that had spoken was evicted for a silent one: {conns:?}");
        assert!(!conns.contains(&2) && !conns.contains(&3), "the oldest silent sockets were not the ones evicted: {conns:?}");
        let closed: Vec<u64> = flags.iter().filter(|(_, flag)| flag.load(Ordering::Acquire)).map(|(conn, _)| *conn).collect();
        assert_eq!(closed, [2, 3], "only the evicted sockets are told so");
        // Every pending socket has spoken: the oldest makes room.
        for p in &pools.pending {
            p.spoke.store(true, Ordering::Release);
        }
        pools.add_pending(pending(max + 3, false));
        assert_eq!(pools.pending.front().map(|p| p.conn), Some(4));
        assert_eq!(pools.pending.back().map(|p| p.conn), Some(max + 3));
    }

    /// X1 at the wire, without relying on thread scheduling: a client whose bytes have arrived (here half of its auth
    /// frame, so it is still pending) is not pushed out by a flood of silent sockets that overflows the pending pool
    /// many times over, and its auth, completed afterwards, is served.
    #[test]
    fn pending_socket_that_has_spoken_outlives_a_silent_flood() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let port = server.port();
        let mut honest = silent(port);
        let frame = format!("{}\n", serde_json::json!({"id":0,"method":"auth","params":{"token":token,"client":{"name":"slow","version":"1"}}}));
        let (head, tail) = frame.split_at(frame.len() / 2);
        honest.write_all(head.as_bytes()).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !lock_pools(&server.pools).pending.iter().any(|p| p.spoke.load(Ordering::Acquire)) && Instant::now() < until {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(lock_pools(&server.pools).pending.iter().any(|p| p.spoke.load(Ordering::Acquire)), "the server never saw the bytes");
        let flood: Vec<TcpStream> = (0..budgets::MAX_PENDING_AUTH * 3).map(|_| silent(port)).collect();
        // With the honest socket spared, every silent socket but the newest MAX_PENDING_AUTH - 1 is evicted; the
        // last of those evictions happens only once the whole flood has been accepted.
        let last_evicted = &flood[flood.len() - budgets::MAX_PENDING_AUTH];
        assert!(closed_within(last_evicted, Duration::from_secs(5)), "the flood was not accepted and evicted in full");
        wait_pending(&server, budgets::MAX_PENDING_AUTH);
        honest.write_all(tail.as_bytes()).unwrap();
        let reply = recv(&honest).expect("the client that had spoken was evicted by silent sockets");
        assert!(reply.ok, "{reply:?}");
        assert_eq!(server.clients(), ["slow 1"]);
        drop(flood);
        server.shutdown();
    }

    /// `disconnect_all` (the GUI turning agent connections off) ends every connection's thread promptly on every
    /// platform (Windows does not wake a read waiting on a socket shut down from another thread), and nothing sent
    /// on a dropped connection afterwards reaches the UI queue.
    #[test]
    fn disconnect_all_ends_every_connection_and_serves_nothing_after() {
        let (server, receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut agents: Vec<TcpStream> = (0..3).map(|index| connect(server.port(), &token, &format!("agent-{index}"))).collect();
        let _pending = silent(server.port());
        wait_live(&server, 4);
        server.disconnect_all();
        assert!(server.clients().is_empty());
        for agent in &mut agents {
            let line = format!("{}\n", serde_json::json!({"id":1,"method":"doc.inspect","params":{}}));
            let _ = agent.write_all(line.as_bytes());
        }
        wait_live(&server, 0);
        assert!(receiver.recv_timeout(Duration::from_millis(300)).is_err(), "a request on a dropped connection reached the UI queue");
        for agent in &agents {
            assert!(closed_within(agent, Duration::from_secs(5)), "a dropped connection was left open");
        }
        server.shutdown();
    }

    /// Keeps `n` silent sockets open, reopening each as soon as the server closes it.
    struct Flood {
        stop: Arc<AtomicBool>,
        reopened: Arc<AtomicUsize>,
        thread: Option<JoinHandle<()>>,
    }

    impl Flood {
        fn start(port: u16, n: usize) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let reopened = Arc::new(AtomicUsize::new(0));
            let (flag, count) = (Arc::clone(&stop), Arc::clone(&reopened));
            let thread = thread::spawn(move || {
                let open = || {
                    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).ok()?;
                    stream.set_nonblocking(true).ok()?;
                    Some(stream)
                };
                let mut sockets: Vec<Option<TcpStream>> = (0..n).map(|_| open()).collect();
                let mut byte = [0_u8; 1];
                while !flag.load(Ordering::Acquire) {
                    for socket in &mut sockets {
                        let closed = match socket {
                            Some(stream) => match stream.read(&mut byte) {
                                Ok(_) => true,
                                Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
                            },
                            None => true,
                        };
                        if closed {
                            *socket = open();
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    thread::yield_now();
                }
            });
            Flood { stop, reopened, thread: Some(thread) }
        }

        /// Waits until the flood holds every socket the server allows it (at least the stop reserve, next to 16
        /// agents, whatever the design), then lets it churn a little.
        fn wait_saturated(&self, server: &ControlServer) {
            let until = Instant::now() + Duration::from_secs(5);
            while server.live_connections() < budgets::MAX_CONNECTIONS + budgets::STOP_ONLY_RESERVE && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(server.live_connections() >= budgets::MAX_CONNECTIONS + budgets::STOP_ONLY_RESERVE, "the flood did not start");
            thread::sleep(Duration::from_millis(200));
        }

        fn reopened(&self) -> usize {
            self.reopened.load(Ordering::Relaxed)
        }

        fn finish(mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    /// `hello` says who the server is and nothing secret: never the token, before or after a failed auth.
    #[test]
    fn hello_never_leaks_the_token() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut stream = silent(server.port());
        send(&mut stream, 0, "hello", serde_json::json!({"token": "please", "verbose": true}));
        let reply = recv(&stream).unwrap();
        let result = reply.result.clone().unwrap();
        let mut keys: Vec<&str> = result.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["pid", "protocol", "server"]);
        let text = serde_json::to_string(&reply).unwrap();
        assert!(!text.contains(&token) && !text.to_lowercase().contains("token"), "{text}");
        send(&mut stream, 1, "auth", serde_json::json!({"token": "0".repeat(64)}));
        let failed = serde_json::to_string(&recv(&stream).unwrap()).unwrap();
        assert!(failed.contains("authentication failed") && !failed.contains(&token), "{failed}");
        server.shutdown();
    }

    /// `hello` opens nothing: whatever follows it except a valid `auth` (a stop, a ping, a second hello, any
    /// method, a bad token) is refused, closes the connection, reaches neither FastStop nor the UI queue, and
    /// registers no client.
    #[test]
    fn hello_followed_by_any_method_does_not_authenticate() {
        let stops = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&stops);
        let (server, receiver, _token) = start(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"latched":true})
        }));
        let followers = [
            ("machine.stop", serde_json::json!({})),
            ("doc.inspect", serde_json::json!({})),
            (PING_METHOD, serde_json::json!({})),
            ("hello", serde_json::json!({})),
            ("engine.execute", serde_json::json!({"command":"file.new"})),
            ("", serde_json::json!(null)),
            ("auth", serde_json::json!({"token": "f".repeat(64)})),
            ("auth", serde_json::json!({})),
        ];
        for (method, params) in followers {
            let mut stream = silent(server.port());
            send(&mut stream, 0, "hello", serde_json::json!({}));
            assert!(recv(&stream).unwrap().ok);
            send(&mut stream, 1, method, params);
            let reply = recv(&stream).unwrap();
            assert!(!reply.ok, "`{method}` after hello was served: {reply:?}");
            assert!(reply.error.as_deref().unwrap().starts_with("authentication"), "{method}: {reply:?}");
            assert!(recv(&stream).is_none(), "`{method}` after hello left the connection open");
        }
        // The frame without an id that names machine.stop is refused before auth like any other.
        let mut stream = silent(server.port());
        stream.write_all(b"{\"method\":\"machine.stop\"}\n").unwrap();
        assert_eq!(recv(&stream).unwrap().error.as_deref(), Some("authentication required"));
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        assert!(receiver.try_recv().is_err());
        assert!(server.clients().is_empty());
        server.shutdown();
    }

    /// On the wire, `machine.stop` trips whatever its params are (every machine unless `machine` is a string), and
    /// a stop frame with no id still trips.
    #[test]
    fn wire_stop_with_junk_params_still_trips() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&seen);
        let (server, receiver, token) = start(Arc::new(move |machine: Option<&str>| {
            observed.lock().unwrap().push(machine.map(str::to_owned));
            serde_json::json!({"latched":true})
        }));
        let mut stream = connect(server.port(), &token, "junk");
        let junk = [
            serde_json::json!(5),
            serde_json::json!("everything"),
            serde_json::json!([1, 2]),
            serde_json::json!(null),
            serde_json::json!({"machine": 5}),
            serde_json::json!({"machine": null}),
            serde_json::json!({"machine": {"id": "m1"}}),
            serde_json::json!({"machine": "m1", "junk": true, "dryRun": true}),
        ];
        for (id, params) in junk.iter().enumerate() {
            send(&mut stream, id as u64, "machine.stop", params.clone());
            let reply = recv(&stream).unwrap();
            assert!(reply.ok && reply.id == serde_json::json!(id), "{params}: {reply:?}");
        }
        stream.write_all(b"{\"method\":\"machine.stop\"}\n").unwrap();
        let reply = recv(&stream).unwrap();
        assert!(reply.ok && reply.id.is_null(), "{reply:?}");
        stream.write_all(b"{\"method\":\"machine.stop\",\"params\":{\"machine\":\"m2\"},\"extra\":1}\n").unwrap();
        assert!(recv(&stream).unwrap().ok);
        let expected: Vec<Option<String>> = vec![None, None, None, None, None, None, None, Some("m1".to_owned()), None, Some("m2".to_owned())];
        assert_eq!(*seen.lock().unwrap(), expected);
        assert!(receiver.try_recv().is_err(), "a stop reached the UI queue");
        server.shutdown();
    }

    /// `ping` keeps a connection warm without the UI thread (nobody serves the queue here); a stop-only connection
    /// is refused it.
    #[test]
    fn ping_is_answered_on_the_transport_thread() {
        let (server, receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut stream = connect(server.port(), &token, "pinger");
        for id in 1..=3 {
            let started = Instant::now();
            send(&mut stream, id, PING_METHOD, serde_json::json!({}));
            let reply = recv(&stream).unwrap();
            assert!(reply.ok && reply.result == Some(serde_json::json!({"pong":true})), "{reply:?}");
            assert!(started.elapsed() < Duration::from_secs(1));
        }
        assert!(receiver.try_recv().is_err(), "ping reached the UI queue");
        let agents: Vec<TcpStream> = (1..budgets::MAX_CONNECTIONS).map(|index| connect(server.port(), &token, &format!("agent-{index}"))).collect();
        let (mut extra, reply) = try_connect(server.port(), &token, "stop-only");
        assert!(stop_only(&reply));
        send(&mut extra, 9, PING_METHOD, serde_json::json!({}));
        assert_eq!(recv(&extra).unwrap().error.as_deref(), Some("connection limit reached"));
        drop(agents);
        server.shutdown();
    }

    /// B7: a connection that never sends its auth frame is closed after AUTH_DEADLINE, not the 30 s idle timeout.
    #[test]
    fn silent_connection_is_closed_at_the_auth_deadline() {
        let (server, _receiver, _token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        let started = Instant::now();
        assert!(recv(&stream).is_none(), "the server answered a socket that sent nothing");
        assert!(started.elapsed() < budgets::AUTH_DEADLINE + Duration::from_secs(2), "closed after {:?}", started.elapsed());
        wait_live(&server, 0);
        server.shutdown();
    }

    #[test]
    fn trickled_auth_frame_still_hits_one_deadline() {
        let (server, _receiver, _token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        let started = Instant::now();
        // One byte every 300 ms never completes a line; each read is under any per-read timeout.
        while started.elapsed() < budgets::AUTH_DEADLINE + Duration::from_secs(2) {
            if stream.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(300));
        }
        wait_live(&server, 0);
        server.shutdown();
    }

    #[test]
    fn oversized_request_gets_an_error_then_close() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut stream = connect(server.port(), &token, "big");
        // Exactly one byte over, and no newline: the server must answer without waiting for the line to end.
        stream.write_all(&vec![b'x'; budgets::MAX_REQUEST_BYTES + 1]).unwrap();
        let reply = recv(&stream).unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("exceeds"));
        server.shutdown();
    }

    #[test]
    fn bad_json_is_answered_and_the_connection_keeps_serving() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut stream = connect(server.port(), &token, "typo");
        stream.write_all(b"{not json\n").unwrap();
        assert!(recv(&stream).unwrap().error.unwrap().starts_with("bad JSON"));
        send(&mut stream, 3, "auth", serde_json::json!({}));
        assert_eq!(recv(&stream).unwrap().error.as_deref(), Some("already authenticated"));
        send(&mut stream, 4, "machine.stop", serde_json::json!({}));
        assert!(recv(&stream).unwrap().ok);
        server.shutdown();
    }

    #[test]
    fn hello_reports_the_pid_before_auth() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        send(&mut stream, 0, "hello", serde_json::json!({}));
        let hello = recv(&stream).unwrap().result.unwrap();
        assert_eq!(hello["pid"], serde_json::json!(std::process::id()));
        send(&mut stream, 1, "auth", serde_json::json!({"token":token}));
        assert!(recv(&stream).unwrap().ok);
        server.shutdown();
    }

    #[test]
    fn client_names_are_sanitized_and_bounded() {
        let (server, _receiver, token) = start(Arc::new(|_| serde_json::json!({"latched":true})));
        let long = format!("evil\u{1b}[31m\nname{}", "x".repeat(500));
        let _stream = connect(server.port(), &token, &long);
        let clients = server.clients();
        let name = clients.first().unwrap();
        assert!(!name.chars().any(char::is_control), "{name:?}");
        assert!(name.chars().count() <= 2 * budgets::CLIENT_NAME_MAX_CHARS + 1);
        server.shutdown();
    }

    #[test]
    fn fast_stop_panic_is_answered_and_leaves_no_stale_client() {
        let (server, _receiver, token) = start(Arc::new(|_: Option<&str>| -> serde_json::Value { panic!("fast stop panicked") }));
        let mut stream = connect(server.port(), &token, "panicky");
        assert_eq!(server.clients().len(), 1);
        send(&mut stream, 1, "machine.stop", serde_json::json!({}));
        let reply = recv(&stream).unwrap();
        assert!(!reply.ok && reply.error.unwrap().contains("panicked"));
        drop(stream);
        let until = Instant::now() + Duration::from_secs(5);
        while !server.clients().is_empty() && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(server.clients().is_empty());
        server.shutdown();
    }

    /// A closed connection's read gives up within CLOSE_POLL from the flag alone, pending (under its auth deadline)
    /// or authenticated (under the 30 s idle timeout), so its thread ends promptly even where shutting the socket
    /// down from another thread does not wake a blocked read (Windows). The time is measured from the moment the
    /// flag is set, so a late flag thread cannot fail it.
    #[test]
    fn closed_connection_read_gives_up_without_a_socket_close() {
        for deadline in [Some(Instant::now() + Duration::from_secs(10)), None] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (accepted, _) = listener.accept().unwrap();
            let closed = Arc::new(AtomicBool::new(false));
            let mut stream = DeadlineStream {
                stream: Arc::new(accepted),
                deadline,
                closed: Arc::clone(&closed),
                server_stop: Arc::new(AtomicBool::new(false)),
                spoke: Arc::new(AtomicBool::new(false)),
            };
            let flag = thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                closed.store(true, Ordering::Release);
                Instant::now()
            });
            let error = stream.read(&mut [0_u8; 8]).unwrap_err();
            let returned = Instant::now();
            let set = flag.join().unwrap();
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionAborted, "{deadline:?}");
            assert!(returned.saturating_duration_since(set) < CLOSE_POLL * 4, "{deadline:?}: {:?}", returned.saturating_duration_since(set));
        }
        // The server's own shutdown flag ends it too (stop-only connections are on no list).
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let mut stream = DeadlineStream {
            stream: Arc::new(accepted),
            deadline: None,
            closed: Arc::new(AtomicBool::new(false)),
            server_stop: Arc::new(AtomicBool::new(true)),
            spoke: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(stream.read(&mut [0_u8; 8]).unwrap_err().kind(), std::io::ErrorKind::ConnectionAborted);
    }

    #[test]
    fn dispatch_can_be_settled_only_once() {
        let dispatch = Dispatch::default();
        assert!(dispatch.abandon());
        assert!(!dispatch.try_dispatch());
        assert!(!dispatch.abandon());
        let dispatch = Dispatch::default();
        assert!(dispatch.try_dispatch());
        assert!(!dispatch.abandon());
    }
}
