//! The loopback control server (std::net on 127.0.0.1 only).
//!
//! Limits (AUT A9/A10, B10): every live socket counts from `accept`, authenticated or not. Up to
//! `MAX_CONNECTIONS` are served normally; the next `STOP_ONLY_RESERVE` are accepted for exactly one `auth` +
//! `machine.stop` exchange; past that a connection gets "connection limit reached" and is closed. A connection has
//! `AUTH_DEADLINE` (one deadline, not per read) and `AUTH_MAX_BYTES` to authenticate, and a request line stops
//! being read at `MAX_REQUEST_BYTES + 1`, so a silent, trickling or endless client cannot hold a slot or a thread.

use std::io::{BufReader, Read};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Error, budgets, discovery, token, wire};

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

/// `machine.stop` is answered on the transport thread through this, before anything touches the UI queue.
pub type FastStop = Arc<dyn Fn(Option<&str>) -> serde_json::Value + Send + Sync>;

pub struct ServerConfig {
    /// 0 = free port.
    pub port: u16,
    pub token: token::Token,
    pub fast_stop: FastStop,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

type Clients = Arc<Mutex<Vec<(u64, String, TcpStream)>>>;

/// std::net on 127.0.0.1 only; ≤ 16 connections (+ a stop-only reserve); 30 s idle; first frame must be
/// {"method":"auth","params":{"token","client":{"name","version"}}} (an optional `hello` may come first, answered
/// with the server's pid so a client can check the discovery file before it sends the token).
pub struct ControlServer {
    port: u16,
    stop: Arc<AtomicBool>,
    clients: Clients,
    live: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

/// Decrements the live-connection count when the connection (or its failed spawn) is gone.
struct LiveSlot(Arc<AtomicUsize>);

impl Drop for LiveSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Removes an authenticated connection from the client list however its thread ends (a panic included).
struct Registration {
    clients: Clients,
    conn: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner).retain(|(id, _, _)| *id != self.conn);
    }
}

impl ControlServer {
    pub fn start(cfg: ServerConfig) -> Result<(Self, std::sync::mpsc::Receiver<ControlRequest>), Error> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cfg.port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let clients: Clients = Arc::new(Mutex::new(Vec::new()));
        let live = Arc::new(AtomicUsize::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_clients = Arc::clone(&clients);
        let thread_live = Arc::clone(&live);
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared { token: cfg.token, fast_stop: cfg.fast_stop, wake: cfg.wake });
        let thread = thread::Builder::new().name("openlaser-control".to_owned()).spawn(move || {
            let mut next_conn = 1_u64;
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        if stream.set_nonblocking(false).is_err() {
                            let _ = stream.shutdown(Shutdown::Both);
                            continue;
                        }
                        // Only this thread increments, so the check and the add cannot race; closes only lower it.
                        let live_now = thread_live.load(Ordering::Acquire);
                        if live_now >= budgets::MAX_CONNECTIONS.saturating_add(budgets::STOP_ONLY_RESERVE) {
                            refuse(stream, "connection limit reached");
                            continue;
                        }
                        thread_live.fetch_add(1, Ordering::AcqRel);
                        let slot = LiveSlot(Arc::clone(&thread_live));
                        let id = next_conn;
                        next_conn = next_conn.saturating_add(1);
                        let context = ConnectionContext {
                            conn: id,
                            stop_only: live_now >= budgets::MAX_CONNECTIONS,
                            shared: Arc::clone(&shared),
                            tx: tx.clone(),
                            clients: Arc::clone(&thread_clients),
                            stop: Arc::clone(&thread_stop),
                        };
                        let fallback = stream.try_clone().ok();
                        let spawned = thread::Builder::new().name(format!("openlaser-control-{id}")).spawn(move || {
                            let _slot = slot;
                            serve_connection(stream, context);
                        });
                        // The closure (stream and slot) is dropped with the error; tell the client why.
                        if spawned.is_err()
                            && let Some(stream) = fallback
                        {
                            refuse(stream, "server busy: cannot start a connection thread; retry");
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
        Ok((Self { port, stop, clients, live, thread: Some(thread) }, rx))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Authenticated clients, "name version" each.
    pub fn clients(&self) -> Vec<String> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|(_, name, _)| name.clone()).collect()
    }

    /// Every live socket, authenticated or not (diagnostics and tests).
    pub fn live_connections(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    pub fn disconnect_all(&self) {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, _, stream) in clients.iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        clients.clear();
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
    stop_only: bool,
    shared: Arc<Shared>,
    tx: std::sync::mpsc::Sender<ControlRequest>,
    clients: Clients,
    stop: Arc<AtomicBool>,
}

/// Writes one error reply on a connection that will not be served, then closes it. Never blocks the accept loop
/// for long: a fresh socket's send buffer is empty, and the write timeout bounds the rest.
fn refuse(mut stream: TcpStream, message: &str) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let _ = wire::write_reply(&mut stream, &error_reply(serde_json::Value::Null, message));
    let _ = stream.shutdown(Shutdown::Both);
}

fn error_reply(id: serde_json::Value, message: &str) -> wire::Reply {
    wire::Reply { id, ok: false, result: None, error: Some(message.to_owned()) }
}

/// Reads through one deadline for the whole pre-auth exchange (a per-read timeout would let a client trickle a
/// byte at a time forever); after authentication the deadline is cleared and the idle timeout applies per read.
struct DeadlineStream {
    stream: TcpStream,
    deadline: Option<Instant>,
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "authentication deadline passed"));
            }
            self.stream.set_read_timeout(Some(remaining))?;
        }
        self.stream.read(buf)
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
            Err(e) => Frame::Bad(e.to_string()),
        },
        Ok(None) => Frame::Closed,
        Err(Error::LineTooLong { max }) => Frame::TooLong(max),
        Err(_) => Frame::Closed,
    }
}

/// Printable, bounded text for the GUI's "Agent connected: …" indicator and logs.
fn sanitize(text: Option<&str>) -> String {
    let clean: String = text.unwrap_or("").chars().filter(|c| !c.is_control()).take(budgets::CLIENT_NAME_MAX_CHARS).collect();
    let clean = clean.trim();
    if clean.is_empty() { "unknown".to_owned() } else { clean.to_owned() }
}

fn serve_connection(mut stream: TcpStream, context: ConnectionContext) {
    let ConnectionContext { conn, stop_only, shared, tx, clients, stop } = context;
    let _ = stream.set_write_timeout(Some(budgets::IDLE_TIMEOUT));
    let Ok(cloned) = stream.try_clone() else { return };
    let mut reader = BufReader::new(DeadlineStream { stream: cloned, deadline: Some(Instant::now() + budgets::AUTH_DEADLINE) });

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
    reader.get_mut().deadline = None;
    let _ = stream.set_read_timeout(Some(budgets::IDLE_TIMEOUT));
    let _registration = Registration { clients: Arc::clone(&clients), conn };
    if !stop_only && let Ok(handle) = stream.try_clone() {
        clients.lock().unwrap_or_else(PoisonError::into_inner).push((conn, client.clone(), handle));
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
        if request.method == "machine.stop" {
            // A machine that is not a string means every machine: a stop is never refused for its params.
            let machine = request.params.get("machine").and_then(serde_json::Value::as_str);
            let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (shared.fast_stop)(machine))) {
                Ok(value) => wire::Reply { id: request.id, ok: true, result: Some(value), error: None },
                Err(_) => error_reply(request.id, "internal error: machine.stop panicked; the stop may not have reached the machine: use the E-stop"),
            };
            if wire::write_reply(&mut stream, &reply).is_err() || stop_only {
                break;
            }
            continue;
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

    fn recv(stream: &TcpStream) -> Option<wire::Reply> {
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&line).unwrap()),
        }
    }

    fn connect(port: u16, token: &str, client: &str) -> TcpStream {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        send(&mut stream, 0, "auth", serde_json::json!({"token":token,"client":{"name":client,"version":"test"}}));
        assert!(recv(&stream).unwrap().ok);
        stream
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

    /// B7: sockets that never authenticate count too, so they cannot crowd the limit while a 17th client gets
    /// full service; the stop-only reserve still stops; past the reserve the connection is refused at once.
    #[test]
    fn every_connection_counts_from_accept() {
        let stops = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&stops);
        let (server, _receiver, token) = start(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"latched":true})
        }));
        let silent: Vec<TcpStream> = (0..budgets::MAX_CONNECTIONS).map(|_| TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap()).collect();
        wait_live(&server, budgets::MAX_CONNECTIONS);
        let mut late = connect(server.port(), &token, "late");
        send(&mut late, 1, "doc.inspect", serde_json::json!({}));
        assert_eq!(recv(&late).unwrap().error.as_deref(), Some("connection limit reached"));
        let mut stopper = connect(server.port(), &token, "stopper");
        send(&mut stopper, 2, "machine.stop", serde_json::json!({}));
        assert!(recv(&stopper).unwrap().ok);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        let reserve: Vec<TcpStream> = (0..budgets::STOP_ONLY_RESERVE).map(|_| TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap()).collect();
        wait_live(&server, budgets::MAX_CONNECTIONS + budgets::STOP_ONLY_RESERVE);
        let refused = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        assert_eq!(recv(&refused).unwrap().error.as_deref(), Some("connection limit reached"));
        drop((silent, reserve));
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
