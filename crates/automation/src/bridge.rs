//! Authenticated loopback JSON-lines client for the GUI control channel (AUT A8, B10).
//!
//! One `Link` per MCP session holds the target, the agent's client info (forwarded in the auth frame) and, once
//! resolved, the GUI it is bound to (`--bridge auto` reads the discovery file at the first call and stays bound). Two
//! kinds of connection hang off it:
//! - `BridgeClient`: the call connection. Replies are matched by id (stale ones skipped), a connection idle for
//!   `RECONNECT_IDLE` is replaced before the next call (the GUI drops idle sockets at 30 s), writes are never resent,
//!   vouched reads are retried once on a fresh connection.
//! - `StopHandle`: a connection of its own, used only for `machine.stop`, created on first use (so a GUI that starts
//!   after the MCP server is still reachable). A stop is idempotent, so on a dead or dropped connection it reconnects
//!   and resends, with short timeouts, until a reply arrives or `STOP_DELIVERY_BUDGET` runs out; the GUI's report is
//!   passed through unchanged, never fabricated.
//!
//! Before any token is sent to a GUI found through the discovery file, the connection's `hello` must report the pid
//! the file names.

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use openlaser_control::{
    budgets, discovery,
    wire::{self, Reply, Request},
};
use serde_json::{Value, json};

use crate::backend::ToolOutput;
use crate::tools::ClientInfo;
use crate::{BridgeTarget, TokenSource};

/// Connecting and authenticating the call connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The whole stop exchange, reconnects and resends included.
pub(crate) const STOP_DELIVERY_BUDGET: Duration = Duration::from_secs(2);
/// A stop connection is local: anything slower than this is a dead GUI.
const STOP_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
/// The GUI waits ≤ STOP_WAIT for the driver's StopReport before it answers.
const STOP_REPLY_WAIT: Duration = Duration::from_millis(1000);

/// The message for `--bridge auto` / `--bridge-file` without a usable GUI (AUT B10.2).
pub(crate) const NOT_RUNNING: &str = "OpenLaser is not running with Automation › Allow Agent Connections on";

#[derive(Clone)]
struct Endpoint {
    addr: SocketAddr,
    token: String,
    /// Known for discovery targets: checked with `hello` before the token is sent.
    pid: Option<u32>,
}

/// What a session's connections share.
pub(crate) struct Link {
    target: BridgeTarget,
    info: Mutex<ClientInfo>,
    bound: Mutex<Option<Endpoint>>,
}

impl Link {
    pub(crate) fn new(target: BridgeTarget, info: ClientInfo) -> Arc<Self> {
        Arc::new(Link { target, info: Mutex::new(sanitized(&info)), bound: Mutex::new(None) })
    }

    /// The agent's identity for the auth frames of connections opened from now on.
    pub(crate) fn set_info(&self, info: &ClientInfo) {
        *self.info.lock().unwrap_or_else(PoisonError::into_inner) = sanitized(info);
    }

    fn info(&self) -> ClientInfo {
        self.info.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The bound GUI, resolving (and binding) it on first use.
    fn endpoint(&self) -> Result<Endpoint, BridgeError> {
        let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(endpoint) = bound.as_ref() {
            return Ok(endpoint.clone());
        }
        let endpoint = resolve(&self.target)?;
        *bound = Some(endpoint.clone());
        Ok(endpoint)
    }

    /// For a stop retry: the discovery file again (a GUI restarted since we bound), else the bound endpoint.
    fn endpoint_for_retry(&self) -> Result<Endpoint, BridgeError> {
        match self.target {
            BridgeTarget::Auto | BridgeTarget::File(_) => resolve(&self.target).or_else(|_| self.endpoint()),
            BridgeTarget::Addr { .. } => self.endpoint(),
        }
    }

    /// Connects, checks `hello` when the pid is known, authenticates.
    fn open(&self, endpoint: &Endpoint, connect_timeout: Duration, setup_timeout: Duration) -> Result<Conn, Failure> {
        let mut stream = TcpStream::connect_timeout(&endpoint.addr, connect_timeout)
            .map_err(|e| Failure::NotSent(format!("cannot connect to OpenLaser at {} ({e})", endpoint.addr)))?;
        let setup = |e: std::io::Error| Failure::NotSent(e.to_string());
        let _ = stream.set_nodelay(true);
        stream.set_read_timeout(Some(setup_timeout)).map_err(setup)?;
        stream.set_write_timeout(Some(setup_timeout)).map_err(setup)?;
        let mut reader = BufReader::new(stream.try_clone().map_err(setup)?);
        if let Some(pid) = endpoint.pid {
            discovery::check_hello(&mut stream, &mut reader, pid).map_err(|e| Failure::NotSent(format!("{NOT_RUNNING} ({e})")))?;
        }
        let info = self.info();
        let mut conn = Conn { stream, reader, last_used: Instant::now(), next_id: 1 };
        let auth = json!({"token": endpoint.token, "client": {"name": info.name, "version": info.version}});
        match conn.exchange("auth", auth, Instant::now() + setup_timeout) {
            Ok(_) => Ok(conn),
            // Nothing but the auth frame went out.
            Err(Failure::Transport(e) | Failure::TimedOut(e)) => Err(Failure::NotSent(format!("authentication did not complete ({e})"))),
            Err(other) => Err(other),
        }
    }
}

fn sanitized(info: &ClientInfo) -> ClientInfo {
    let clean = |text: &str| -> String {
        let text: String = text.chars().filter(|c| !c.is_control()).take(budgets::CLIENT_NAME_MAX_CHARS).collect();
        if text.trim().is_empty() { "unknown".to_owned() } else { text.trim().to_owned() }
    };
    ClientInfo { name: clean(&info.name), version: clean(&info.version) }
}

/// How one attempt failed; decides the retry and the wording.
enum Failure {
    /// Nothing was sent (could not connect or authenticate).
    NotSent(String),
    /// Sent, reply lost: the outcome is uncertain.
    Transport(String),
    /// Sent, no reply in time.
    TimedOut(String),
    /// The app answered with an error, or the request was refused before it was sent.
    Definite(String),
}

impl Failure {
    fn into_error(self, read: bool) -> BridgeError {
        match self {
            Failure::NotSent(e) => BridgeError::NotSent(e),
            Failure::Transport(e) => BridgeError::Transport(e),
            Failure::TimedOut(_) => BridgeError::TimedOut { read },
            Failure::Definite(e) => BridgeError::Definite(e),
        }
    }

    fn describe(&self) -> &str {
        match self {
            Failure::NotSent(e) | Failure::Transport(e) | Failure::TimedOut(e) | Failure::Definite(e) => e,
        }
    }
}

/// One authenticated connection.
struct Conn {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    last_used: Instant,
    next_id: u64,
}

impl Conn {
    /// Sends one request and reads until the reply with its id arrives (other ids are stale replies of earlier,
    /// abandoned requests and are skipped) or `deadline` passes.
    fn exchange(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, Failure> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut line = serde_json::to_vec(&Request { id: json!(id), method: method.to_owned(), params }).map_err(|e| Failure::Definite(e.to_string()))?;
        line.push(b'\n');
        if line.len() > budgets::MAX_REQUEST_BYTES {
            return Err(Failure::Definite(format!("request is {} bytes; the maximum is {}; it was not sent", line.len(), budgets::MAX_REQUEST_BYTES)));
        }
        let remaining = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
        let _ = self.stream.set_write_timeout(Some(remaining));
        self.stream.write_all(&line).and_then(|()| self.stream.flush()).map_err(|e| Failure::Transport(format!("sending `{method}` failed ({e})")))?;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Failure::TimedOut(format!("no reply to `{method}`")));
            }
            self.stream.set_read_timeout(Some(remaining)).map_err(|e| Failure::Transport(e.to_string()))?;
            let bytes = match wire::read_line_bounded(&mut self.reader, budgets::MAX_RESPONSE_BYTES) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => return Err(Failure::Transport("connection closed by the app".to_owned())),
                Err(openlaser_control::Error::LineTooLong { max }) => {
                    return Err(Failure::Definite(format!("bridge response exceeds {max} bytes; operation may have completed; inspect state before retrying")));
                }
                Err(openlaser_control::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                    return Err(Failure::TimedOut(format!("no reply to `{method}`")));
                }
                Err(e) => return Err(Failure::Transport(e.to_string())),
            };
            let Ok(reply) = serde_json::from_slice::<Reply>(&bytes) else { continue };
            if reply.id != json!(id) {
                continue; // a stale reply from an earlier request
            }
            return if reply.ok {
                Ok(reply.result.unwrap_or(Value::Null))
            } else {
                Err(Failure::Definite(reply.error.unwrap_or_else(|| "control request failed".to_owned())))
            };
        }
    }
}

/// One authenticated loopback connection for tool calls (lazily opened, replaced when idle).
pub struct BridgeClient {
    link: Arc<Link>,
    conn: Option<Conn>,
    reconnect_idle: Duration,
    timeout: Duration,
}

impl BridgeClient {
    /// Resolves the target and opens the connection now (errors here are "not sent").
    pub fn connect(target: &BridgeTarget, client: &ClientInfo) -> Result<Self, BridgeError> {
        let mut bridge = Self::lazy(Link::new(target.clone(), client.clone()));
        bridge.ensure().map_err(|f| f.into_error(false))?;
        Ok(bridge)
    }

    pub(crate) fn lazy(link: Arc<Link>) -> Self {
        BridgeClient { link, conn: None, reconnect_idle: budgets::RECONNECT_IDLE, timeout: budgets::BRIDGE_TIMEOUT }
    }

    pub(crate) fn link(&self) -> &Arc<Link> {
        &self.link
    }

    #[cfg(test)]
    pub(crate) fn set_reconnect_idle(&mut self, idle: Duration) {
        self.reconnect_idle = idle;
    }

    /// A write is never automatically resent.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, BridgeError> {
        self.attempt(method, params).map_err(|f| f.into_error(false))
    }

    /// Read-only calls are retried once, on a fresh connection, after a failure that applied nothing.
    pub fn call_read(&mut self, method: &str, params: Value) -> Result<Value, BridgeError> {
        match self.attempt(method, params.clone()) {
            Err(Failure::NotSent(_) | Failure::Transport(_)) => self.attempt(method, params).map_err(|f| f.into_error(true)),
            other => other.map_err(|f| f.into_error(true)),
        }
    }

    /// A handle with its own authenticated connection, reserved for `machine.stop` (reconnects on use if dropped).
    pub fn stop_handle(&self) -> Result<StopHandle, BridgeError> {
        let handle = StopHandle::lazy(Arc::clone(&self.link));
        handle.connect_now()?;
        Ok(handle)
    }

    fn ensure(&mut self) -> Result<&mut Conn, Failure> {
        if self.conn.as_ref().is_some_and(|conn| conn.last_used.elapsed() >= self.reconnect_idle) {
            // The GUI may already have closed it (30 s idle): send on a fresh socket instead.
            self.conn = None;
        }
        if self.conn.is_none() {
            let endpoint = self.link.endpoint().map_err(|e| Failure::NotSent(bridge_text(&e)))?;
            self.conn = Some(self.link.open(&endpoint, CONNECT_TIMEOUT, CONNECT_TIMEOUT)?);
        }
        self.conn.as_mut().ok_or_else(|| Failure::NotSent("not connected".to_owned()))
    }

    fn attempt(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        let deadline = Instant::now() + self.timeout;
        let outcome = self.ensure()?.exchange(method, params, deadline);
        match &outcome {
            Ok(_) | Err(Failure::Definite(_)) => {
                if let Some(conn) = self.conn.as_mut() {
                    conn.last_used = Instant::now();
                }
            }
            Err(_) => self.conn = None,
        }
        outcome
    }
}

/// The dedicated stop connection: never shared with tool calls, opened on first use, reconnected on failure.
#[derive(Clone)]
pub struct StopHandle {
    inner: Arc<StopInner>,
}

struct StopInner {
    link: Arc<Link>,
    conn: Mutex<Option<Conn>>,
}

impl StopHandle {
    pub(crate) fn lazy(link: Arc<Link>) -> Self {
        StopHandle { inner: Arc::new(StopInner { link, conn: Mutex::new(None) }) }
    }

    /// The agent's identity (from MCP `initialize`) for the stop connection's auth frame.
    pub(crate) fn set_client(&self, info: &ClientInfo) {
        self.inner.link.set_info(info);
    }

    fn connect_now(&self) -> Result<(), BridgeError> {
        let endpoint = self.inner.link.endpoint()?;
        let conn = self.inner.link.open(&endpoint, STOP_CONNECT_TIMEOUT, STOP_CONNECT_TIMEOUT).map_err(|f| f.into_error(false))?;
        *self.inner.conn.lock().unwrap_or_else(PoisonError::into_inner) = Some(conn);
        Ok(())
    }

    /// Sends `machine.stop` (every machine when `machine` is None) and returns the GUI's report unchanged. A dead,
    /// idle-dropped or never-opened connection is replaced and the stop resent (it is idempotent) within
    /// `STOP_DELIVERY_BUDGET`; the GUI itself waits ≤ STOP_WAIT for the StopReport before it answers.
    pub fn stop(&self, machine: Option<&str>) -> ToolOutput {
        let deadline = Instant::now() + STOP_DELIVERY_BUDGET;
        let params = machine.map_or_else(|| json!({}), |m| json!({"machine": m}));
        let mut guard = self.inner.conn.lock().unwrap_or_else(PoisonError::into_inner);
        let mut last: Option<Failure> = None;
        let mut attempt = 0_u32;
        while Instant::now() < deadline && attempt < 4 {
            attempt += 1;
            if guard.as_ref().is_some_and(|conn| conn.last_used.elapsed() >= budgets::RECONNECT_IDLE) {
                *guard = None;
            }
            if guard.is_none() {
                let endpoint = if attempt == 1 { self.inner.link.endpoint() } else { self.inner.link.endpoint_for_retry() };
                let remaining = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
                let opened = endpoint
                    .map_err(|e| Failure::NotSent(bridge_text(&e)))
                    .and_then(|endpoint| self.inner.link.open(&endpoint, STOP_CONNECT_TIMEOUT.min(remaining), STOP_CONNECT_TIMEOUT.min(remaining)));
                match opened {
                    Ok(conn) => *guard = Some(conn),
                    Err(Failure::Definite(e)) => return stop_error(&Failure::Definite(e)),
                    Err(failure) => {
                        last = Some(failure);
                        // Nothing listening: a short pause before trying the discovery file again.
                        std::thread::sleep(Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())));
                        continue;
                    }
                }
            }
            let Some(conn) = guard.as_mut() else { continue };
            let reply_by = deadline.min(Instant::now() + STOP_REPLY_WAIT);
            match conn.exchange("machine.stop", params.clone(), reply_by) {
                Ok(report) => {
                    conn.last_used = Instant::now();
                    return stop_report(report);
                }
                Err(Failure::Definite(e)) => {
                    conn.last_used = Instant::now();
                    return stop_error(&Failure::Definite(e));
                }
                Err(failure) => {
                    // Dropped (idle), reset or silent: a fresh connection and the same idempotent stop.
                    *guard = None;
                    last = Some(failure);
                }
            }
        }
        stop_error(&last.unwrap_or_else(|| Failure::NotSent("no attempt fitted in the stop budget".to_owned())))
    }
}

/// The GUI's report, unchanged, when it reports a latch; anything else is an error that says so.
fn stop_report(report: Value) -> ToolOutput {
    if report.get("latched").and_then(Value::as_bool).is_some() {
        ToolOutput::Json(report)
    } else {
        ToolOutput::Error {
            message: "transport: OpenLaser answered the stop without a latch report, so nothing confirms the machine stopped; press STOP in the OpenLaser window or the E-stop".to_owned(),
            data: Some(json!({"reply": report})),
        }
    }
}

fn stop_error(failure: &Failure) -> ToolOutput {
    let prefix = match failure {
        Failure::NotSent(_) => "not_connected",
        Failure::Definite(_) => "refused",
        Failure::Transport(_) | Failure::TimedOut(_) => "transport",
    };
    ToolOutput::Error {
        message: format!("{prefix}: the stop was not confirmed by OpenLaser ({}); press STOP in the OpenLaser window or the E-stop", failure.describe()),
        data: None,
    }
}

fn bridge_text(error: &BridgeError) -> String {
    match error {
        BridgeError::NotSent(e) | BridgeError::Transport(e) | BridgeError::Definite(e) => e.clone(),
        BridgeError::TimedOut { .. } => "timed out".to_owned(),
        BridgeError::NotImplemented(e) => (*e).to_owned(),
    }
}

fn resolve(target: &BridgeTarget) -> Result<Endpoint, BridgeError> {
    match target {
        BridgeTarget::Auto => {
            let path = discovery::default_path().map_err(|e| BridgeError::NotSent(format!("{NOT_RUNNING} (no discovery file: {e})")))?;
            resolve(&BridgeTarget::File(path))
        }
        BridgeTarget::File(path) => {
            let info = discovery::read(path).map_err(|e| BridgeError::NotSent(format!("{NOT_RUNNING} ({}: {e})", path.display())))?;
            Ok(Endpoint { addr: SocketAddr::from(([127, 0, 0, 1], info.port)), token: info.token, pid: Some(info.pid) })
        }
        BridgeTarget::Addr { addr, token } => {
            if !addr.ip().is_loopback() {
                return Err(BridgeError::NotSent(format!("control address must be loopback, got {addr}")));
            }
            Ok(Endpoint { addr: *addr, token: token_value(token)?, pid: None })
        }
    }
}

fn token_value(source: &TokenSource) -> Result<String, BridgeError> {
    let not_sent = |e: openlaser_control::Error| BridgeError::NotSent(e.to_string());
    let token = match source {
        TokenSource::Inline(value) => openlaser_control::token::Token::from_hex(value).map_err(not_sent)?,
        TokenSource::File(path) => openlaser_control::token::read_file(path).map_err(not_sent)?,
        TokenSource::Env => {
            if let Some(path) = std::env::var_os("OPENLASER_CONTROL_TOKEN_FILE") {
                openlaser_control::token::read_file(std::path::Path::new(&path)).map_err(not_sent)?
            } else {
                let value = std::env::var("OPENLASER_CONTROL_TOKEN")
                    .map_err(|_| BridgeError::NotSent("control token is missing: pass --control-token-file or set OPENLASER_CONTROL_TOKEN_FILE".to_owned()))?;
                openlaser_control::token::Token::from_hex(value.trim()).map_err(not_sent)?
            }
        }
    };
    Ok(token.to_hex())
}

/// Wording per AUT A8: NotSent "the request was not sent"; Transport "operation may have completed; inspect state
/// before retrying"; TimedOut read "may be retried safely" / write "may have completed"; Definite = the app's error.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeError {
    NotSent(String),
    Transport(String),
    TimedOut { read: bool },
    Definite(String),
    NotImplemented(&'static str),
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Read};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use openlaser_control::server::{ControlServer, FastStop, ServerConfig};
    use openlaser_control::token::Token;

    fn gui(fast_stop: FastStop) -> (ControlServer, std::sync::mpsc::Receiver<openlaser_control::server::ControlRequest>, BridgeTarget) {
        let token = Token::generate().unwrap();
        let hex = token.to_hex();
        let (server, requests) = ControlServer::start(ServerConfig { port: 0, token, fast_stop, wake: Arc::new(|| {}) }).unwrap();
        let target = BridgeTarget::Addr { addr: SocketAddr::from(([127, 0, 0, 1], server.port())), token: TokenSource::Inline(hex) };
        (server, requests, target)
    }

    fn info() -> ClientInfo {
        ClientInfo { name: "test".to_owned(), version: "1".to_owned() }
    }

    /// B8: a reply with another id (left over from an abandoned request) is skipped, not returned for this call.
    #[test]
    fn bridge_skips_stale_reply_ids() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let token = Token::generate().unwrap().to_hex();
        let fake = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap(); // auth
            let auth: Request = serde_json::from_str(&line).unwrap();
            writeln!(writer, "{}", json!({"id": auth.id, "ok": true, "result": {"authenticated": true}})).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            let id = request.id.as_u64().unwrap();
            writeln!(writer, "{}", json!({"id": id.wrapping_sub(1), "ok": true, "result": "stale"})).unwrap();
            writeln!(writer, "not json").unwrap();
            writeln!(writer, "{}", json!({"id": id, "ok": true, "result": "fresh"})).unwrap();
            let _ = reader.read(&mut [0_u8; 1]);
        });
        let target = BridgeTarget::Addr { addr, token: TokenSource::Inline(token) };
        let mut client = BridgeClient::connect(&target, &info()).unwrap();
        assert_eq!(client.call("doc.inspect", json!({})).unwrap(), json!("fresh"));
        drop(client);
        fake.join().unwrap();
    }

    /// B8: after the GUI dropped an idle connection, the next call (a write, never resent) goes out on a fresh one.
    #[test]
    fn bridge_reconnects_when_idle() {
        let (server, requests, target) = gui(Arc::new(|_| json!({"latched": true})));
        let answered = std::thread::spawn(move || {
            for request in requests.iter().take(2) {
                assert!(request.dispatch.try_dispatch());
                let _ = request.reply.send(Reply { id: request.request.id, ok: true, result: Some(json!({"conn": request.conn})), error: None });
            }
        });
        let mut client = BridgeClient::connect(&target, &info()).unwrap();
        client.set_reconnect_idle(Duration::from_millis(200));
        let first = client.call("engine.execute", json!({"command":"edit.selectAll"})).unwrap();
        server.disconnect_all(); // what the GUI's 30 s idle timeout does
        std::thread::sleep(Duration::from_millis(300));
        let second = client.call("engine.execute", json!({"command":"edit.selectAll"})).unwrap();
        assert_ne!(first["conn"], second["conn"], "the write went out on the dropped connection");
        answered.join().unwrap();
        server.shutdown();
    }

    /// B4: the GUI's report is passed through unchanged; a reply without a latch report is an error, never
    /// `latched:true` made up by the client.
    #[test]
    fn stop_report_is_never_fabricated() {
        let (server, _requests, target) = gui(Arc::new(|_| json!({"confirmed": false, "elapsedMs": 3})));
        let handle = StopHandle::lazy(Link::new(target, info()));
        match handle.stop(None) {
            ToolOutput::Error { message, data } => {
                assert!(message.contains("without a latch report"), "{message}");
                assert_eq!(data.unwrap()["reply"], json!({"confirmed": false, "elapsedMs": 3}));
            }
            _ => panic!("a reply without `latched` was reported as a stop"),
        }
        server.shutdown();
        let report = json!({"latched": true, "confirmed": true, "elapsedMs": 41, "report": {"machine": "m1"}});
        let echoed = report.clone();
        let (server, _requests, target) = gui(Arc::new(move |_| echoed.clone()));
        let handle = StopHandle::lazy(Link::new(target, info()));
        assert!(matches!(handle.stop(Some("m1")), ToolOutput::Json(value) if value == report));
        server.shutdown();
    }

    /// B1 at the handle level (the MCP-level tests in `mcp.rs` go through the read loop): a handle made before the
    /// GUI listens still stops once it does.
    #[test]
    fn stop_handle_is_lazy() {
        let port = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let token = Token::generate().unwrap();
        let hex = token.to_hex();
        let handle =
            StopHandle::lazy(Link::new(BridgeTarget::Addr { addr: SocketAddr::from(([127, 0, 0, 1], port)), token: TokenSource::Inline(hex) }, info()));
        assert!(matches!(handle.stop(None), ToolOutput::Error { message, .. } if message.starts_with("not_connected:")));
        let stops = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&stops);
        let (server, _requests) = ControlServer::start(ServerConfig {
            port,
            token,
            fast_stop: Arc::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                json!({"latched": true})
            }),
            wake: Arc::new(|| {}),
        })
        .unwrap();
        assert!(matches!(handle.stop(None), ToolOutput::Json(_)));
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        server.shutdown();
    }

    #[test]
    fn client_info_reaches_the_gui() {
        let (server, _requests, target) = gui(Arc::new(|_| json!({"latched": true})));
        let link = Link::new(target, info());
        link.set_info(&ClientInfo { name: "claude-code\u{7}".to_owned(), version: "2.1".to_owned() });
        let _client = {
            let mut client = BridgeClient::lazy(Arc::clone(&link));
            client.ensure().ok().map(|_| ()).unwrap();
            client
        };
        assert_eq!(server.clients(), ["claude-code 2.1"]);
        server.shutdown();
    }

    /// The discovery path: hello proves the pid before the token is sent; a file naming another pid is stale and
    /// nothing is sent; with no file, the B10.2 message.
    #[test]
    fn bridge_binds_through_the_discovery_file() {
        let (server, requests, target) = gui(Arc::new(|_| json!({"latched": true})));
        let BridgeTarget::Addr { token: TokenSource::Inline(token), .. } = &target else { panic!("addr target") };
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("automation").join("control.json");
        let roots = discovery::Roots { read: None, write: None };
        let good = discovery::Discovery { version: 1, pid: std::process::id(), port: server.port(), token: token.clone(), started: "now".to_owned(), roots };
        discovery::write(&file, &good).unwrap();
        let answer = std::thread::spawn(move || {
            let request = requests.recv().unwrap();
            assert!(request.dispatch.try_dispatch());
            request.reply.send(Reply { id: request.request.id, ok: true, result: Some(json!("via discovery")), error: None }).unwrap();
        });
        let mut client = BridgeClient::connect(&BridgeTarget::File(file.clone()), &info()).unwrap();
        assert_eq!(client.call_read("session.list", json!({})).unwrap(), json!("via discovery"));
        answer.join().unwrap();
        discovery::write(&file, &discovery::Discovery { pid: std::process::id().wrapping_add(1), ..good }).unwrap();
        let stale = BridgeClient::connect(&BridgeTarget::File(file), &info()).err().unwrap();
        assert!(matches!(&stale, BridgeError::NotSent(m) if m.starts_with(NOT_RUNNING)), "{stale:?}");
        let missing = BridgeClient::connect(&BridgeTarget::File(temp.path().join("none.json")), &info()).err().unwrap();
        assert!(matches!(&missing, BridgeError::NotSent(m) if m.starts_with(NOT_RUNNING)), "{missing:?}");
        server.shutdown();
    }
}
