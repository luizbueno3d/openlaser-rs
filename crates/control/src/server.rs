//! The loopback control server (std::net on 127.0.0.1 only). WP0 stubs: it never starts.

use std::io::BufReader;
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Error, budgets, token, wire};

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

/// std::net on 127.0.0.1 only; ≤ 16 connections; 30 s idle; first frame must be
/// {"method":"auth","params":{"token","client":{"name","version"}}}.
pub struct ControlServer {
    port: u16,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<(u64, String, TcpStream)>>>,
    thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    pub fn start(cfg: ServerConfig) -> Result<(Self, std::sync::mpsc::Receiver<ControlRequest>), Error> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cfg.port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_clients = Arc::clone(&clients);
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = thread::Builder::new().name("openlaser-control".to_owned()).spawn(move || {
            let mut next_conn = 1_u64;
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        if stream.set_nonblocking(false).is_err() {
                            let _ = stream.shutdown(Shutdown::Both);
                            continue;
                        }
                        let count = thread_clients.lock().map(|v| v.len()).unwrap_or(usize::MAX);
                        let over_limit = count >= budgets::MAX_CONNECTIONS;
                        let id = next_conn;
                        next_conn = next_conn.saturating_add(1);
                        let stop = Arc::clone(&thread_stop);
                        let clients = Arc::clone(&thread_clients);
                        let tx = tx.clone();
                        let token = cfg.token.to_hex();
                        let fast_stop = Arc::clone(&cfg.fast_stop);
                        let wake = Arc::clone(&cfg.wake);
                        let _ = thread::Builder::new().name(format!("openlaser-control-{id}")).spawn(move || {
                            serve_connection(stream, ConnectionContext { conn: id, over_limit, token, fast_stop, wake, tx, clients, stop });
                        });
                    }
                    Ok((stream, _)) => {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                    Err(_) => thread::sleep(Duration::from_millis(25)),
                }
            }
        })?;
        Ok((Self { port, stop, clients, thread: Some(thread) }, rx))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn clients(&self) -> Vec<String> {
        self.clients.lock().map(|v| v.iter().map(|(_, name, _)| name.clone()).collect()).unwrap_or_default()
    }

    pub fn disconnect_all(&self) {
        if let Ok(mut clients) = self.clients.lock() {
            for (_, _, stream) in clients.iter() {
                let _ = stream.shutdown(Shutdown::Both);
            }
            clients.clear();
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

struct ConnectionContext {
    conn: u64,
    over_limit: bool,
    token: String,
    fast_stop: FastStop,
    wake: Arc<dyn Fn() + Send + Sync>,
    tx: std::sync::mpsc::Sender<ControlRequest>,
    clients: Arc<Mutex<Vec<(u64, String, TcpStream)>>>,
    stop: Arc<AtomicBool>,
}

fn serve_connection(mut stream: TcpStream, context: ConnectionContext) {
    let ConnectionContext { conn, over_limit, token, fast_stop, wake, tx, clients, stop } = context;
    let _ = stream.set_read_timeout(Some(budgets::IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(budgets::STOP_WAIT));
    let cloned = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(cloned);
    let auth = match read_request(&mut reader) {
        Ok(Some(r)) => r,
        _ => return,
    };
    let token_value = auth.params.get("token").and_then(serde_json::Value::as_str).unwrap_or("");
    let name = auth.params.get("client").and_then(|v| v.get("name")).and_then(serde_json::Value::as_str).unwrap_or("unknown");
    let version = auth.params.get("client").and_then(|v| v.get("version")).and_then(serde_json::Value::as_str).unwrap_or("unknown");
    if auth.method != "auth" || token.len() != 64 || !constant_time_eq(&token, token_value) {
        let _ = wire::write_reply(&mut stream, &wire::Reply { id: auth.id, ok: false, result: None, error: Some("authentication failed".to_owned()) });
        return;
    }
    let client = format!("{name} {version}");
    if let Ok(mut list) = clients.lock()
        && let Ok(shutdown_stream) = stream.try_clone()
    {
        list.push((conn, client.clone(), shutdown_stream));
    }
    let _ = wire::write_reply(&mut stream, &wire::Reply { id: auth.id, ok: true, result: Some(serde_json::json!({"authenticated":true})), error: None });
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let request = match read_request(&mut reader) {
            Ok(Some(r)) => r,
            _ => break,
        };
        if request.method == "machine.stop" {
            let machine = request.params.get("machine").and_then(serde_json::Value::as_str);
            let value = fast_stop(machine);
            let _ = wire::write_reply(&mut stream, &wire::Reply { id: request.id, ok: true, result: Some(value), error: None });
            if over_limit {
                break;
            }
            continue;
        }
        if over_limit {
            let _ =
                wire::write_reply(&mut stream, &wire::Reply { id: request.id, ok: false, result: None, error: Some("connection limit reached".to_owned()) });
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
            break;
        }
        wake();
        match reply_rx.recv_timeout(budgets::APP_DEADLINE) {
            Ok(reply) => {
                if wire::write_reply(&mut stream, &reply).is_err() {
                    break;
                }
            }
            Err(_) => {
                let safe = dispatch.abandon();
                let msg = if safe {
                    "request timed out before dispatch; safe to resend"
                } else {
                    "request timed out; operation may have completed; inspect state before retrying"
                };
                let _ = wire::write_reply(&mut stream, &wire::Reply { id: request_id, ok: false, result: None, error: Some(msg.to_owned()) });
                if !safe {
                    break;
                }
            }
        }
    }
    if let Ok(mut list) = clients.lock() {
        list.retain(|(id, _, _)| *id != conn);
    }
}

fn read_request(reader: &mut impl std::io::BufRead) -> Result<Option<wire::Request>, Error> {
    let Some(line) = wire::read_line_bounded(reader, budgets::MAX_REQUEST_BYTES)? else { return Ok(None) };
    Ok(Some(serde_json::from_slice(&line)?))
}

fn constant_time_eq(expected: &str, actual: &str) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected.bytes().zip(actual.bytes()).fold(0_u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::AtomicUsize;

    fn connect(port: u16, token: &str, client: &str) -> TcpStream {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        serde_json::to_writer(
            &mut stream,
            &wire::Request {
                id: serde_json::json!(0),
                method: "auth".to_owned(),
                params: serde_json::json!({"token":token,"client":{"name":client,"version":"test"}}),
            },
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(serde_json::from_str::<wire::Reply>(&line).unwrap().ok);
        stream
    }

    #[test]
    fn fast_stop_works_without_a_ui_queue_consumer() {
        let token = token::Token::generate().unwrap();
        let token_hex = token.to_hex();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let fast_stop: FastStop = Arc::new(move |machine| {
            observed.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"latched":true,"machine":machine})
        });
        let (server, _receiver) = ControlServer::start(ServerConfig { port: 0, token, fast_stop, wake: Arc::new(|| {}) }).unwrap();
        let mut stream = connect(server.port(), &token_hex, "stop-test");
        let start = Instant::now();
        let mut command =
            serde_json::to_vec(&wire::Request { id: serde_json::json!(1), method: "machine.stop".to_owned(), params: serde_json::json!({}) }).unwrap();
        command.push(b'\n');
        stream.write_all(&command).unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(serde_json::from_str::<wire::Reply>(&line).unwrap().ok);
        assert!(start.elapsed() < budgets::STOP_WAIT);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.shutdown();
    }

    #[test]
    fn stop_is_accepted_on_seventeenth_connection() {
        let token = token::Token::generate().unwrap();
        let token_hex = token.to_hex();
        let (server, _receiver) =
            ControlServer::start(ServerConfig { port: 0, token, fast_stop: Arc::new(|_| serde_json::json!({"latched":true})), wake: Arc::new(|| {}) }).unwrap();
        let mut connections = Vec::new();
        for index in 0..budgets::MAX_CONNECTIONS {
            connections.push(connect(server.port(), &token_hex, &format!("client-{index}")));
        }
        let mut last = connect(server.port(), &token_hex, "stop-only");
        serde_json::to_writer(&mut last, &wire::Request { id: serde_json::json!(17), method: "machine.stop".to_owned(), params: serde_json::json!({}) })
            .unwrap();
        last.write_all(b"\n").unwrap();
        last.flush().unwrap();
        let mut reader = BufReader::new(last.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(serde_json::from_str::<wire::Reply>(&line).unwrap().ok);
        drop(connections);
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
