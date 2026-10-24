//! Authenticated loopback JSON-lines client for the GUI control channel (AUT A8, B10).
//!
//! One `Link` per MCP session holds the target, the agent's client info (forwarded in the auth frame) and, once
//! resolved, the GUI it is bound to (`--bridge auto` reads the discovery file at the first call and stays bound). Two
//! kinds of connection hang off it:
//! - `BridgeClient`: the call connection. Replies are matched by id (stale ones skipped), a connection idle for
//!   `RECONNECT_IDLE` is replaced before the next call (the GUI drops idle sockets at 30 s), writes are never resent,
//!   vouched reads are retried once on a fresh connection.
//! - `StopHandle`: a connection of its own, used only for `machine.stop`, created on first use (so a GUI that starts
//!   after the MCP server is still reachable) and then kept warm with a transport-level `ping` every
//!   `STOP_KEEPALIVE`, so a stop rarely needs a new slot. A stop is idempotent, so on a dead, dropped or refused
//!   connection it reconnects and resends, with short timeouts and a short backoff, until a reply arrives or
//!   `STOP_DELIVERY_BUDGET` runs out; the GUI's report is passed through unchanged, never fabricated, and a report
//!   that latched nothing is an error.
//!
//! Stop delivery contract: AT-LEAST-ONCE, with a truthful report. A stop sent on a connection the GUI has just
//! closed may still reach it (Windows delivers bytes to a read already waiting on a socket shut down under it) while
//! the client sees only the close, so the client resends on a fresh connection and the GUI's FastStop runs twice,
//! both times for the same machine (or for every machine). That is safe: the latch is idempotent, and nothing the
//! agent can do in between could re-arm a machine (arming needs a human). No stop id is sent to deduplicate: a resend
//! is never suppressed, so a delivery that tripped nothing cannot hide behind one that did. The report returned to
//! the agent is the GUI's own answer to the delivery that was answered (matched by request id on its connection),
//! unchanged; an unanswered delivery never contributes to it, and with no answer at all the stop is reported as not
//! confirmed.
//!
//! The server answers a connection it will not serve (no slot left, a thread it cannot start) with an id-null error
//! and closes it; that reply is surfaced as such ("connection limit reached"), never as a lost auth.
//!
//! Before any token is sent to a GUI found through the discovery file, the connection's `hello` must report the pid
//! the file names.

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use openlaser_control::{
    budgets, discovery,
    server::PING_METHOD,
    wire::{self, Reply, Request},
};
use serde_json::{Value, json};

use crate::backend::ToolOutput;
use crate::tools::ClientInfo;
use crate::{BridgeTarget, TokenSource};

/// Connecting and authenticating the call connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The whole stop exchange, reconnects and resends included (the control server's tests hold it to the same budget).
pub(crate) use openlaser_control::budgets::STOP_DELIVERY_BUDGET;
/// A stop connection is local: anything slower than this is a dead GUI.
const STOP_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
/// The GUI waits ≤ STOP_WAIT for the driver's StopReport before it answers.
const STOP_REPLY_WAIT: Duration = Duration::from_millis(1000);
/// The pause after a failed stop attempt, doubled each time up to `STOP_RETRY_MAX_PAUSE`; attempts go on until
/// `STOP_DELIVERY_BUDGET` runs out, however many that takes.
const STOP_RETRY_FIRST_PAUSE: Duration = Duration::from_millis(5);
const STOP_RETRY_MAX_PAUSE: Duration = Duration::from_millis(50);
/// How often the keep-alive looks at the stop connection; it pings one idle for half of this, so the GUI never sees
/// it idle for more than 1.5× this (well under `RECONNECT_IDLE` and the GUI's 30 s idle timeout).
const STOP_KEEPALIVE: Duration = Duration::from_secs(10);

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
        let mut conn = Conn { stream, reader, last_used: Instant::now(), next_id: 1, stop_only: false };
        let auth = json!({"token": endpoint.token, "client": {"name": info.name, "version": info.version}});
        match conn.exchange("auth", auth, Instant::now() + setup_timeout) {
            Ok(result) => {
                // Every agent slot taken: the GUI serves this connection for one `machine.stop` and then closes it.
                conn.stop_only = result.get("stopOnly").and_then(Value::as_bool).unwrap_or(false);
                Ok(conn)
            }
            // Nothing but the auth frame went out.
            Err(Failure::Transport(e) | Failure::TimedOut(e) | Failure::Oversized(e)) => {
                Err(Failure::NotSent(format!("authentication did not complete ({e})")))
            }
            // `Refused` (the server's id-null "connection limit reached") and `Definite` ("authentication failed")
            // keep the server's own words.
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
    /// The server refused the connection itself with an id-null error ("connection limit reached", "server busy")
    /// and closed it: nothing was run.
    Refused(String),
    /// Sent, reply lost: the outcome is uncertain.
    Transport(String),
    /// Sent, no reply in time.
    TimedOut(String),
    /// The reply was over the budget: the operation may have completed, and the connection, left inside that
    /// line, is dropped.
    Oversized(String),
    /// The app answered with an error, or the request was refused before it was sent.
    Definite(String),
}

impl Failure {
    fn into_error(self, read: bool) -> BridgeError {
        match self {
            Failure::NotSent(e) => BridgeError::NotSent(e),
            Failure::Refused(e) => BridgeError::NotSent(format!("OpenLaser refused the connection: {e}")),
            Failure::Transport(e) => BridgeError::Transport(e),
            Failure::TimedOut(_) => BridgeError::TimedOut { read },
            Failure::Oversized(e) | Failure::Definite(e) => BridgeError::Definite(e),
        }
    }

    fn describe(&self) -> &str {
        match self {
            Failure::NotSent(e) | Failure::Refused(e) | Failure::Transport(e) | Failure::TimedOut(e) | Failure::Oversized(e) | Failure::Definite(e) => e,
        }
    }
}

/// One authenticated connection.
struct Conn {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    last_used: Instant,
    next_id: u64,
    /// The GUI gave it a stop-only slot: one `machine.stop`, then it closes it.
    stop_only: bool,
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
                    return Err(Failure::Oversized(format!(
                        "bridge response exceeds {max} bytes; operation may have completed; inspect state before retrying"
                    )));
                }
                Err(openlaser_control::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                    return Err(Failure::TimedOut(format!("no reply to `{method}`")));
                }
                Err(e) => return Err(Failure::Transport(e.to_string())),
            };
            let Ok(reply) = serde_json::from_slice::<Reply>(&bytes) else { continue };
            if reply.id.is_null() && !reply.ok {
                // We never send id null, so this is no stale reply: the server is refusing this connection
                // ("connection limit reached", …) and closing it.
                return Err(Failure::Refused(reply.error.unwrap_or_else(|| "the connection was refused".to_owned())));
            }
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
            Err(Failure::NotSent(_) | Failure::Refused(_) | Failure::Transport(_)) => self.attempt(method, params).map_err(|f| f.into_error(true)),
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
        let conn = self.ensure()?;
        let stop_only = conn.stop_only;
        if stop_only && method != "machine.stop" {
            // Every agent slot is taken: this connection would serve one stop and refuse anything else. Close it and
            // say so; the call is not sent (the next call tries for a slot again).
            self.conn = None;
            return Err(Failure::Refused(format!(
                "connection limit reached (all {} agent connections are in use; only a stop is accepted)",
                budgets::MAX_CONNECTIONS
            )));
        }
        let outcome = conn.exchange(method, params, deadline);
        match &outcome {
            // The GUI closes a stop-only connection after its one exchange.
            _ if stop_only => self.conn = None,
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

/// The dedicated stop connection: never shared with tool calls, opened on first use, kept warm, reconnected on
/// failure.
#[derive(Clone)]
pub struct StopHandle {
    inner: Arc<StopInner>,
}

struct StopInner {
    link: Arc<Link>,
    conn: Mutex<Option<Conn>>,
    /// The keep-alive's period; it pings a connection idle for half of it.
    keepalive: Duration,
    /// A connection idle this long may already be closed by the GUI: a stop opens a fresh one instead.
    reconnect_idle: Duration,
    keeper_started: AtomicBool,
}

impl StopHandle {
    pub(crate) fn lazy(link: Arc<Link>) -> Self {
        Self::tuned(link, STOP_KEEPALIVE, budgets::RECONNECT_IDLE)
    }

    fn tuned(link: Arc<Link>, keepalive: Duration, reconnect_idle: Duration) -> Self {
        StopHandle { inner: Arc::new(StopInner { link, conn: Mutex::new(None), keepalive, reconnect_idle, keeper_started: AtomicBool::new(false) }) }
    }

    /// The agent's identity (from MCP `initialize`) for the stop connection's auth frame.
    pub(crate) fn set_client(&self, info: &ClientInfo) {
        self.inner.link.set_info(info);
    }

    fn connect_now(&self) -> Result<(), BridgeError> {
        let endpoint = self.inner.link.endpoint()?;
        let conn = self.inner.link.open(&endpoint, STOP_CONNECT_TIMEOUT, STOP_CONNECT_TIMEOUT).map_err(|f| f.into_error(false))?;
        // A stop-only connection is good for one stop only: the GUI is reachable, but do not keep it.
        if !conn.stop_only {
            *self.inner.conn.lock().unwrap_or_else(PoisonError::into_inner) = Some(conn);
            self.keep_warm();
        }
        Ok(())
    }

    /// Starts the keep-alive once a connection worth keeping exists. It holds only a weak reference, so it ends
    /// within one period of the last handle being dropped.
    fn keep_warm(&self) {
        if self.inner.keeper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = Arc::downgrade(&self.inner);
        let every = self.inner.keepalive;
        let spawned = std::thread::Builder::new().name("openlaser-stop-keepalive".to_owned()).spawn(move || keepalive(&inner, every));
        if spawned.is_err() {
            // Try again with the next connection; until then a stop reconnects when it has to.
            self.inner.keeper_started.store(false, Ordering::Release);
        }
    }

    /// Sends `machine.stop` (every machine when `machine` is None) and returns the GUI's report unchanged. A dead,
    /// idle-dropped, refused or never-opened connection is replaced and the stop resent (it is idempotent), with a
    /// short backoff between failed attempts, until `STOP_DELIVERY_BUDGET` runs out; the GUI itself waits
    /// ≤ STOP_WAIT for the StopReport before it answers.
    ///
    /// At-least-once (module docs): an attempt whose reply was lost may still have tripped the GUI, so one call can
    /// reach FastStop more than once, every time with these same params. The report returned is the answered
    /// delivery's, unchanged.
    pub fn stop(&self, machine: Option<&str>) -> ToolOutput {
        let deadline = Instant::now() + STOP_DELIVERY_BUDGET;
        let params = machine.map_or_else(|| json!({}), |m| json!({"machine": m}));
        let mut guard = self.inner.conn.lock().unwrap_or_else(PoisonError::into_inner);
        let mut last: Option<Failure> = None;
        let mut first_open = true;
        let mut pause = STOP_RETRY_FIRST_PAUSE;
        let back_off = |pause: &mut Duration| {
            std::thread::sleep((*pause).min(deadline.saturating_duration_since(Instant::now())));
            *pause = pause.saturating_mul(2).min(STOP_RETRY_MAX_PAUSE);
        };
        while Instant::now() < deadline {
            if guard.as_ref().is_some_and(|conn| conn.last_used.elapsed() >= self.inner.reconnect_idle) {
                *guard = None;
            }
            // A connection kept from before may have been dropped by the GUI: if it fails, reconnect at once.
            let fresh = guard.is_none();
            if fresh {
                let endpoint = if first_open { self.inner.link.endpoint() } else { self.inner.link.endpoint_for_retry() };
                first_open = false;
                let remaining = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
                let opened = endpoint
                    .map_err(|e| Failure::NotSent(bridge_text(&e)))
                    .and_then(|endpoint| self.inner.link.open(&endpoint, STOP_CONNECT_TIMEOUT.min(remaining), STOP_CONNECT_TIMEOUT.min(remaining)));
                match opened {
                    Ok(conn) => *guard = Some(conn),
                    // "authentication failed": the token is wrong, and resending will not change that.
                    Err(Failure::Definite(e)) => return stop_error(&Failure::Definite(e)),
                    Err(failure) => {
                        // Nothing listening, the slots full, or this socket closed before it authenticated.
                        last = Some(failure);
                        back_off(&mut pause);
                        continue;
                    }
                }
            }
            let Some(conn) = guard.as_mut() else { continue };
            let reply_by = deadline.min(Instant::now() + STOP_REPLY_WAIT);
            let outcome = conn.exchange("machine.stop", params.clone(), reply_by);
            conn.last_used = Instant::now();
            let keep = !conn.stop_only;
            match outcome {
                Ok(report) => {
                    if keep {
                        drop(guard);
                        self.keep_warm();
                    } else {
                        *guard = None;
                    }
                    return stop_report(report);
                }
                Err(Failure::Definite(e)) => {
                    if !keep {
                        *guard = None;
                    }
                    return stop_error(&Failure::Definite(e));
                }
                Err(failure) => {
                    // Dropped (idle), reset, refused or silent: a fresh connection and the same idempotent stop.
                    *guard = None;
                    last = Some(failure);
                    if fresh {
                        back_off(&mut pause);
                    }
                }
            }
        }
        stop_error(&last.unwrap_or_else(|| Failure::NotSent("no attempt fitted in the stop budget".to_owned())))
    }
}

/// The keep-alive thread: every `every`, ping the stop connection if it has been idle for half of that.
fn keepalive(inner: &Weak<StopInner>, every: Duration) {
    loop {
        std::thread::sleep(every);
        let Some(inner) = inner.upgrade() else { return };
        inner.ping_if_idle();
    }
}

impl StopInner {
    /// Takes the connection out while it is pinged, so a stop never waits behind a ping (it would open a fresh
    /// connection instead), and puts it back unless a stop has meanwhile opened another.
    fn ping_if_idle(&self) {
        let taken = {
            let mut guard = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.as_ref().is_some_and(|conn| !conn.stop_only && conn.last_used.elapsed() >= self.keepalive / 2) { guard.take() } else { None }
        };
        let Some(mut conn) = taken else { return };
        if conn.last_used.elapsed() >= self.reconnect_idle {
            return; // the GUI may already have closed it; the next stop opens a fresh one
        }
        if conn.exchange(PING_METHOD, json!({}), Instant::now() + STOP_REPLY_WAIT).is_ok() {
            conn.last_used = Instant::now();
            let mut guard = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.is_none() {
                *guard = Some(conn);
            }
        }
    }
}

/// The GUI's report, unchanged, when it reports a latch. A report that latched nothing, or none at all, is an error
/// that says so (with the report as data): nothing confirms that the machine stopped.
fn stop_report(report: Value) -> ToolOutput {
    match report.get("latched").and_then(Value::as_bool) {
        Some(true) => ToolOutput::Json(report),
        Some(false) => {
            let note = report.get("note").and_then(Value::as_str).unwrap_or("it reported latched:false");
            let stopped_nothing = report.get("stopped").and_then(Value::as_array).is_none_or(Vec::is_empty);
            let message = if stopped_nothing {
                format!(
                    "not_connected: OpenLaser latched no stop ({note}), so nothing confirms a machine stopped; if anything is running, press STOP in the OpenLaser window or the E-stop"
                )
            } else {
                format!("not_latched: OpenLaser did not latch the stop on every machine ({note}); press STOP in the OpenLaser window or the E-stop")
            };
            ToolOutput::Error { message, data: Some(json!({"reply": report})) }
        }
        None => ToolOutput::Error {
            message: "transport: OpenLaser answered the stop without a latch report, so nothing confirms the machine stopped; press STOP in the OpenLaser window or the E-stop".to_owned(),
            data: Some(json!({"reply": report})),
        },
    }
}

fn stop_error(failure: &Failure) -> ToolOutput {
    let prefix = match failure {
        Failure::NotSent(_) | Failure::Refused(_) => "not_connected",
        Failure::Definite(_) => "refused",
        Failure::Transport(_) | Failure::TimedOut(_) | Failure::Oversized(_) => "transport",
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

    /// How the scripted GUI treats one connection.
    #[derive(Clone, Copy, PartialEq)]
    enum Treat {
        /// What the control server says when no slot is left: an id-null "connection limit reached", then close.
        Refuse,
        /// Auth, then ping / machine.stop answered for as long as the client stays.
        Serve,
        /// Auth with `stopOnly: true`, one machine.stop, then close.
        StopOnly,
        /// Auth, then an over-budget reply to the first request, then silence.
        Oversized,
        /// Auth, then the first machine.stop is taken (FastStop runs) and the connection closed without an answer:
        /// what a client sees when the GUI drops a connection its stop still reached (Windows does this).
        Swallow,
    }

    /// A scripted GUI on a real socket: `treat(n)` decides connection n (0-based); it counts what it saw.
    struct FakeGui {
        addr: SocketAddr,
        connections: Arc<AtomicUsize>,
        pings: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        /// The `machine` param of every machine.stop that reached it, in order (null for every machine).
        machines: Arc<Mutex<Vec<Value>>>,
    }

    impl FakeGui {
        fn start(treat: impl Fn(usize) -> Treat + Send + 'static) -> Self {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();
            let (connections, pings, stops) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            let machines = Arc::new(Mutex::new(Vec::new()));
            let (seen, pinged, stopped, stopped_for) = (Arc::clone(&connections), Arc::clone(&pings), Arc::clone(&stops), Arc::clone(&machines));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { return };
                    let how = treat(seen.fetch_add(1, Ordering::SeqCst));
                    let (pinged, stopped, stopped_for) = (Arc::clone(&pinged), Arc::clone(&stopped), Arc::clone(&stopped_for));
                    std::thread::spawn(move || serve_fake(stream, how, &pinged, &stopped, &stopped_for));
                }
            });
            FakeGui { addr, connections, pings, stops, machines }
        }

        fn target(&self) -> BridgeTarget {
            BridgeTarget::Addr { addr: self.addr, token: TokenSource::Inline(Token::generate().unwrap().to_hex()) }
        }

        fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }
    }

    fn serve_fake(stream: TcpStream, how: Treat, pings: &AtomicUsize, stops: &AtomicUsize, machines: &Mutex<Vec<Value>>) {
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if how == Treat::Refuse {
            // As the control server does: the auth frame is read (closing with it unread would reset the
            // connection and could discard the refusal), then refused with id null, then closed.
            let _ = reader.read_line(&mut line);
            let _ = writeln!(writer, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
            return;
        }
        while matches!(reader.read_line(&mut line), Ok(n) if n > 0) {
            let Ok(request) = serde_json::from_str::<Request>(&line) else { return };
            line.clear();
            let result = match request.method.as_str() {
                "auth" => json!({"authenticated": true, "stopOnly": how == Treat::StopOnly}),
                "ping" => {
                    pings.fetch_add(1, Ordering::SeqCst);
                    json!({"pong": true})
                }
                "machine.stop" => {
                    let machine = request.params.get("machine").cloned().unwrap_or(Value::Null);
                    machines.lock().unwrap().push(machine.clone());
                    let delivery = stops.fetch_add(1, Ordering::SeqCst) + 1;
                    if how == Treat::Swallow {
                        return; // tripped, and the connection closes before any answer
                    }
                    json!({"latched": true, "confirmed": true, "delivery": delivery, "machine": machine})
                }
                _ => json!("done"),
            };
            if how == Treat::Oversized && request.method != "auth" {
                let mut big = vec![b'x'; budgets::MAX_RESPONSE_BYTES + 2];
                big.push(b'\n');
                let _ = writer.write_all(&big);
                let _ = reader.read(&mut [0_u8; 1]); // silent until the client goes
                return;
            }
            if writeln!(writer, "{}", json!({"id": request.id, "ok": true, "result": result})).is_err() {
                return;
            }
            if how == Treat::StopOnly && request.method == "machine.stop" {
                return;
            }
        }
    }

    /// X1 (4) and (2): every slot taken, the GUI answers each new connection with an id-null "connection limit
    /// reached". The stop keeps trying (well past 4 attempts) for the whole delivery budget, then says why it failed
    /// in the GUI's words, not "authentication did not complete".
    #[test]
    fn stop_names_the_connection_limit_and_retries_for_the_whole_budget() {
        let gui = FakeGui::start(|_| Treat::Refuse);
        let handle = StopHandle::lazy(Link::new(gui.target(), info()));
        let started = Instant::now();
        let ToolOutput::Error { message, .. } = handle.stop(None) else { panic!("a refused stop was reported as done") };
        let elapsed = started.elapsed();
        assert!(message.starts_with("not_connected:") && message.contains("connection limit reached"), "{message}");
        assert!(!message.contains("authentication did not complete"), "{message}");
        assert!(gui.connections() > 8, "gave up after {} attempts", gui.connections());
        assert!(elapsed >= STOP_DELIVERY_BUDGET - Duration::from_millis(100), "gave up after {elapsed:?}");
        assert!(elapsed < STOP_DELIVERY_BUDGET + Duration::from_millis(500), "overran the budget: {elapsed:?}");
    }

    /// X1 (2): the slots free up after a while (here, after 12 refused connections): the stop gets through.
    #[test]
    fn stop_gets_through_once_a_slot_frees() {
        let gui = FakeGui::start(|n| if n < 12 { Treat::Refuse } else { Treat::Serve });
        let handle = StopHandle::lazy(Link::new(gui.target(), info()));
        let started = Instant::now();
        let outcome = handle.stop(Some("m1"));
        assert!(matches!(&outcome, ToolOutput::Json(report) if report["latched"] == json!(true)), "{:?}", stop_text(&outcome));
        assert!(started.elapsed() < STOP_DELIVERY_BUDGET);
        assert_eq!(gui.stops.load(Ordering::SeqCst), 1);
    }

    /// The at-least-once contract, deterministically on every OS (CI saw it on Windows through a real GUI drop): the
    /// first delivery trips the GUI but its connection closes before any answer, so the handle resends on a fresh
    /// connection. FastStop runs twice, both times for the requested machine, and the agent gets exactly the report
    /// of the delivery that was answered (the second), unchanged, never one made up for the first.
    #[test]
    fn stop_is_delivered_at_least_once_and_reports_the_answered_delivery() {
        let gui = FakeGui::start(|n| if n == 0 { Treat::Swallow } else { Treat::Serve });
        let handle = StopHandle::lazy(Link::new(gui.target(), info()));
        let outcome = handle.stop(Some("m1"));
        let ToolOutput::Json(report) = &outcome else { panic!("a stop answered on the resend was not reported: {}", stop_text(&outcome)) };
        assert_eq!(*report, json!({"latched": true, "confirmed": true, "delivery": 2, "machine": "m1"}));
        assert_eq!(*gui.machines.lock().unwrap(), [json!("m1"), json!("m1")]);
        assert_eq!(gui.connections(), 2);
        // Every machine: the resend says every machine too.
        let gui = FakeGui::start(|n| if n == 0 { Treat::Swallow } else { Treat::Serve });
        let handle = StopHandle::lazy(Link::new(gui.target(), info()));
        assert!(matches!(handle.stop(None), ToolOutput::Json(report) if report["delivery"] == json!(2)));
        assert_eq!(*gui.machines.lock().unwrap(), [Value::Null, Value::Null]);
        // A delivery that is never answered is never reported as a stop: with every connection swallowing it, the
        // budget runs out and the agent is told the stop was not confirmed, however many deliveries tripped.
        let gui = FakeGui::start(|_| Treat::Swallow);
        let handle = StopHandle::lazy(Link::new(gui.target(), info()));
        let outcome = handle.stop(Some("m1"));
        assert!(matches!(&outcome, ToolOutput::Error { message, .. } if message.starts_with("transport:")), "{}", stop_text(&outcome));
        assert!(gui.stops.load(Ordering::SeqCst) >= 2);
        assert!(gui.machines.lock().unwrap().iter().all(|m| *m == json!("m1")));
    }

    /// Waits until the keep-alive has just put the stop connection back after a ping, so a stop made now does not
    /// land while the ping has it out (by design such a stop opens a fresh connection rather than wait).
    fn just_pinged(handle: &StopHandle) -> bool {
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            let fresh = handle.inner.conn.lock().unwrap().as_ref().is_some_and(|conn| conn.last_used.elapsed() < Duration::from_millis(10));
            if fresh {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// X1 (3): once open, the stop connection is kept warm with pings, so the next stop needs no new connection
    /// (here with a 100 ms keep-alive and a 500 ms reconnect threshold standing in for 10 s and 25 s; the idle time is
    /// over twice the threshold, so only the pings can have kept the connection).
    #[test]
    fn stop_connection_is_kept_warm() {
        let gui = FakeGui::start(|_| Treat::Serve);
        let handle = StopHandle::tuned(Link::new(gui.target(), info()), Duration::from_millis(100), Duration::from_millis(500));
        assert!(matches!(handle.stop(None), ToolOutput::Json(_)));
        assert_eq!(gui.connections(), 1);
        std::thread::sleep(Duration::from_millis(1200));
        assert!(gui.pings.load(Ordering::SeqCst) >= 3, "{} pings", gui.pings.load(Ordering::SeqCst));
        assert!(just_pinged(&handle), "the keep-alive did not keep the connection");
        assert!(matches!(handle.stop(None), ToolOutput::Json(_)));
        assert_eq!(gui.connections(), 1, "the stop after an idle second opened a new connection");
        assert_eq!(gui.stops.load(Ordering::SeqCst), 2);
        // The keep-alive holds only a weak reference: it ends with the handle.
        drop(handle);
        let pings = gui.pings.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(400));
        assert!(gui.pings.load(Ordering::SeqCst) <= pings + 1);
    }

    /// A stop-only connection serves one stop: it is neither kept nor pinged, and the next stop opens another
    /// straight away instead of failing on the closed one first.
    #[test]
    fn stop_only_connection_is_not_kept() {
        let gui = FakeGui::start(|_| Treat::StopOnly);
        let handle = StopHandle::tuned(Link::new(gui.target(), info()), Duration::from_millis(50), Duration::from_secs(25));
        assert!(matches!(handle.stop(None), ToolOutput::Json(_)));
        std::thread::sleep(Duration::from_millis(200));
        assert!(matches!(handle.stop(None), ToolOutput::Json(_)));
        assert_eq!((gui.connections(), gui.stops.load(Ordering::SeqCst), gui.pings.load(Ordering::SeqCst)), (2, 2, 0));
        // A call connection that could only be stop-only is refused before anything is sent...
        let mut client = BridgeClient::connect(&gui.target(), &info()).unwrap();
        let refused = client.call("engine.execute", json!({"command":"edit.selectAll"})).unwrap_err();
        assert!(matches!(&refused, BridgeError::NotSent(m) if m.contains("connection limit reached")), "{refused:?}");
        // ...but `machine.stop` on it is served (`openlaser-cli ui machine.stop` with every slot taken), and the
        // connection, closed by the GUI after that one exchange, is not reused.
        let stops = gui.stops.load(Ordering::SeqCst);
        assert_eq!(client.call("machine.stop", json!({})).unwrap()["latched"], json!(true));
        assert_eq!(client.call("machine.stop", json!({})).unwrap()["latched"], json!(true));
        assert_eq!(gui.stops.load(Ordering::SeqCst), stops + 2);
    }

    /// An over-budget reply leaves the connection inside that line: it is dropped, and the next call goes out on a
    /// fresh one (the old connection is never answered again).
    #[test]
    fn oversized_reply_drops_the_call_connection() {
        let gui = FakeGui::start(|n| if n == 0 { Treat::Oversized } else { Treat::Serve });
        let mut client = BridgeClient::lazy(Link::new(gui.target(), info()));
        client.timeout = Duration::from_secs(2);
        let first = client.call("doc.inspect", json!({})).unwrap_err();
        assert!(matches!(&first, BridgeError::Definite(m) if m.contains("exceeds")), "{first:?}");
        assert_eq!(client.call("doc.inspect", json!({})).unwrap(), json!("done"));
        assert_eq!(gui.connections(), 2);
    }

    /// X1 review item: a report that latched nothing is an error result carrying the report, never a success.
    #[test]
    fn unlatched_stop_report_is_an_error() {
        let none = json!({"latched": false, "confirmed": false, "elapsedMs": 0, "stopped": [], "note": "no machine is connected"});
        let echoed = none.clone();
        let (server, _requests, target) = gui(Arc::new(move |_| echoed.clone()));
        let handle = StopHandle::lazy(Link::new(target, info()));
        match handle.stop(None) {
            ToolOutput::Error { message, data } => {
                assert!(message.starts_with("not_connected:") && message.contains("no machine is connected"), "{message}");
                assert_eq!(data.unwrap()["reply"], none);
            }
            other => panic!("latched:false was reported as a stop: {:?}", stop_text(&other)),
        }
        server.shutdown();
        let partial = json!({"latched": false, "confirmed": false, "stopped": [{"machine": "m1", "latched": false}]});
        let echoed = partial.clone();
        let (server, _requests, target) = gui(Arc::new(move |_| echoed.clone()));
        let handle = StopHandle::lazy(Link::new(target, info()));
        assert!(matches!(handle.stop(Some("m1")), ToolOutput::Error { message, .. } if message.starts_with("not_latched:")));
        server.shutdown();
    }

    fn stop_text(output: &ToolOutput) -> String {
        match output {
            ToolOutput::Json(value) => value.to_string(),
            ToolOutput::Error { message, .. } => message.clone(),
            ToolOutput::Png { .. } => "png".to_owned(),
        }
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
