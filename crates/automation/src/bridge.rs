//! Authenticated loopback JSON-lines client for the GUI control channel.

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream};

use crate::backend::StopHandle;
use crate::tools::ClientInfo;
use crate::{BridgeTarget, TokenSource};
use openlaser_control::{
    budgets, discovery,
    token::Token,
    wire::{self, Reply, Request},
};

/// One authenticated loopback connection.
pub struct BridgeClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    next_id: u64,
    target: BridgeTarget,
    info: ClientInfo,
}

impl BridgeClient {
    pub fn connect(target: &BridgeTarget, client: &ClientInfo) -> Result<Self, BridgeError> {
        let (addr, token) = resolve(target)?;
        if !addr.ip().is_loopback() {
            return Err(BridgeError::NotSent("control address must be loopback".to_owned()));
        }
        let stream = TcpStream::connect_timeout(&addr, budgets::BRIDGE_TIMEOUT).map_err(|e| BridgeError::NotSent(e.to_string()))?;
        stream.set_read_timeout(Some(budgets::BRIDGE_TIMEOUT)).map_err(|e| BridgeError::NotSent(e.to_string()))?;
        stream.set_write_timeout(Some(budgets::STOP_WAIT)).map_err(|e| BridgeError::NotSent(e.to_string()))?;
        let reader_stream = stream.try_clone().map_err(|e| BridgeError::NotSent(e.to_string()))?;
        let mut client_conn = Self { stream, reader: BufReader::new(reader_stream), next_id: 1, target: target.clone(), info: client.clone() };
        let auth = Request {
            id: serde_json::json!(0),
            method: "auth".to_owned(),
            params: serde_json::json!({"token":token,"client":{"name":client.name,"version":client.version}}),
        };
        client_conn.write_request(&auth).map_err(|e| BridgeError::NotSent(e.to_string()))?;
        let reply = client_conn.read_reply().map_err(|e| BridgeError::Transport(e.to_string()))?;
        if !reply.ok {
            return Err(BridgeError::Definite(reply.error.unwrap_or_else(|| "authentication failed".to_owned())));
        }
        Ok(client_conn)
    }

    /// A write is never automatically resent.
    pub fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        self.request(method, params, false)
    }

    /// Read-only calls are retried once on an idle connection failure.
    pub fn call_read(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        match self.request(method, params.clone(), true) {
            Err(BridgeError::NotSent(_)) | Err(BridgeError::Transport(_)) | Err(BridgeError::TimedOut { read: true }) => {
                let replacement = Self::connect(&self.target, &self.info)?;
                *self = replacement;
                self.request(method, params, true)
            }
            result => result,
        }
    }

    /// Opens a second authenticated connection reserved for machine.stop.
    pub fn stop_handle(&self) -> Result<StopHandle, BridgeError> {
        let connection = Self::connect(&self.target, &self.info)?;
        Ok(StopHandle::new(Some(connection)))
    }

    pub(crate) fn call_stop(&mut self, machine: Option<&str>) -> Result<serde_json::Value, BridgeError> {
        self.stream.set_read_timeout(Some(budgets::STOP_WAIT)).map_err(|e| BridgeError::NotSent(e.to_string()))?;
        let result = self.request("machine.stop", machine.map_or_else(|| serde_json::json!({}), |m| serde_json::json!({"machine":m})), false);
        let _ = self.stream.set_read_timeout(Some(budgets::BRIDGE_TIMEOUT));
        result
    }

    fn request(&mut self, method: &str, params: serde_json::Value, read: bool) -> Result<serde_json::Value, BridgeError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_request(&Request { id: serde_json::json!(id), method: method.to_owned(), params }).map_err(|e| BridgeError::NotSent(e.to_string()))?;
        let reply = self.read_reply().map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock {
                BridgeError::TimedOut { read }
            } else {
                BridgeError::Transport(e.to_string())
            }
        })?;
        if !reply.ok {
            return Err(BridgeError::Definite(reply.error.unwrap_or_else(|| "control request failed".to_owned())));
        }
        Ok(reply.result.unwrap_or(serde_json::Value::Null))
    }

    fn write_request(&mut self, request: &Request) -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(request).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        self.stream.write_all(&bytes)?;
        self.stream.flush()
    }

    fn read_reply(&mut self) -> std::io::Result<Reply> {
        let line = wire::read_line_bounded(&mut self.reader, budgets::MAX_RESPONSE_BYTES)
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control server closed"))?;
        serde_json::from_slice(&line).map_err(std::io::Error::other)
    }
}

fn resolve(target: &BridgeTarget) -> Result<(SocketAddr, String), BridgeError> {
    match target {
        BridgeTarget::Auto => resolve(&BridgeTarget::File(discovery::default_path())),
        BridgeTarget::File(path) => {
            let info = discovery::read(path).map_err(|e| BridgeError::NotSent(e.to_string()))?;
            let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
            Ok((addr, info.token))
        }
        BridgeTarget::Addr { addr, token } => Ok((*addr, token_value(token)?)),
    }
}

fn token_value(source: &TokenSource) -> Result<String, BridgeError> {
    let value = match source {
        TokenSource::Inline(value) => value.clone(),
        TokenSource::File(path) => std::fs::read_to_string(path).map_err(|e| BridgeError::NotSent(e.to_string()))?.trim().to_owned(),
        TokenSource::Env => {
            if let Some(path) = std::env::var_os("OPENLASER_CONTROL_TOKEN_FILE") {
                std::fs::read_to_string(path).map_err(|e| BridgeError::NotSent(e.to_string()))?.trim().to_owned()
            } else {
                std::env::var("OPENLASER_CONTROL_TOKEN").map_err(|e| BridgeError::NotSent(format!("control token is missing: {e}")))?
            }
        }
    };
    Token::from_hex(&value).map_err(|e| BridgeError::NotSent(e.to_string()))?;
    Ok(value)
}

/// Wording follows the retry rules in AUT A8.
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
    use super::*;

    #[test]
    fn mcp_stop_not_blocked_by_inflight_call() {
        use openlaser_control::{
            server::{ControlServer, ServerConfig},
            token::Token,
        };
        use std::sync::Arc;
        use std::time::Instant;

        let token = Token::generate().unwrap();
        let token_hex = token.to_hex();
        let fast_stop: openlaser_control::server::FastStop = Arc::new(|machine| serde_json::json!({"latched":true,"machine":machine}));
        let (server, requests) =
            ControlServer::start(ServerConfig { port: 0, token: Token::from_hex(&token_hex).unwrap(), fast_stop, wake: Arc::new(|| {}) }).unwrap();
        let target = BridgeTarget::Addr { addr: SocketAddr::from(([127, 0, 0, 1], server.port())), token: TokenSource::Inline(token_hex) };
        let info = ClientInfo { name: "test".to_owned(), version: "1".to_owned() };
        let mut call_client = BridgeClient::connect(&target, &info).unwrap();
        let stop = call_client.stop_handle().unwrap();
        let in_flight = std::thread::spawn(move || call_client.call("doc.inspect", serde_json::json!({})));
        let request = requests.recv_timeout(budgets::BRIDGE_TIMEOUT).unwrap();
        assert!(request.dispatch.try_dispatch());

        let started = Instant::now();
        assert!(matches!(stop.stop(None), crate::ToolOutput::Json(value) if value.get("latched").and_then(serde_json::Value::as_bool) == Some(true)));
        assert!(started.elapsed() < budgets::STOP_WAIT);

        request.reply.send(Reply { id: request.request.id, ok: true, result: Some(serde_json::json!({})), error: None }).unwrap();
        assert!(in_flight.join().unwrap().is_ok());
        server.shutdown();
    }
}
