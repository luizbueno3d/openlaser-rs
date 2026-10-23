//! `openlaser-cli serve`: the control protocol over a headless session, on stdio (no auth) or loopback TCP (the
//! auth frame, through `ControlServer`). Both paths answer the same way: a malformed line gets "bad JSON" and the
//! next line is served; an oversized line gets an error and is skipped (stdio) or closes the connection (TCP); a
//! panicking method is an error reply and the next request works; `machine.stop` reports that nothing was stopped.

use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use openlaser_control::budgets;
use openlaser_control::server::{ControlRequest, ServerConfig};
use openlaser_control::token::Token;
use openlaser_control::wire::{self, Reply, Request};
use serde_json::{Value, json};

use crate::backend::{ToolOutput, headless_stop_report};
use crate::{Error, TokenSource};

/// What `serve` dispatches to (the headless backend; tests inject others).
pub(crate) trait ControlBackend {
    fn dispatch_control(&mut self, method: &str, params: Value) -> ToolOutput;
}

/// One reply, whatever the method does (a panic included).
pub(crate) fn guarded_reply(backend: &mut dyn ControlBackend, id: Value, method: &str, params: Value) -> Reply {
    match catch_unwind(AssertUnwindSafe(|| backend.dispatch_control(method, params))) {
        Ok(output) => control_reply(id, output),
        Err(_) => Reply { id, ok: false, result: None, error: Some(format!("internal error: `{method}` panicked; the session continues")) },
    }
}

/// A tool output on the control wire: JSON as the result; a PNG as the note's fields plus `pngBase64`; an error as
/// `{ok:false, error}` with any structured data appended.
pub(crate) fn control_reply(id: Value, output: ToolOutput) -> Reply {
    use base64::Engine as _;
    match output {
        ToolOutput::Json(result) => Reply { id, ok: true, result: Some(result), error: None },
        ToolOutput::Png { png, note } => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(png);
            let result = match note {
                Value::Object(mut map) => {
                    map.insert("pngBase64".to_owned(), json!(encoded));
                    Value::Object(map)
                }
                other => json!({"note": other, "pngBase64": encoded}),
            };
            Reply { id, ok: true, result: Some(result), error: None }
        }
        ToolOutput::Error { message, data } => {
            let error = match data {
                Some(data) => format!("{message} {data}"),
                None => message,
            };
            Reply { id, ok: false, result: None, error: Some(error) }
        }
    }
}

/// The stdio loop.
pub(crate) fn serve_lines(backend: &mut dyn ControlBackend, mut input: impl BufRead, mut output: impl Write) -> Result<(), Error> {
    loop {
        let line = match wire::read_line_bounded(&mut input, budgets::MAX_REQUEST_BYTES) {
            Ok(Some(line)) => line,
            Ok(None) => return Ok(()),
            Err(openlaser_control::Error::LineTooLong { max }) => {
                let reply = Reply { id: Value::Null, ok: false, result: None, error: Some(format!("request exceeds {max} bytes; line skipped")) };
                wire::write_reply(&mut output, &reply)?;
                if !wire::discard_line(&mut input)? {
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(Error::Control(e)),
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let reply = match serde_json::from_slice::<Request>(&line) {
            Err(e) => Reply { id: Value::Null, ok: false, result: None, error: Some(format!("bad JSON: {e}")) },
            Ok(request) if request.method == "auth" => {
                Reply { id: request.id, ok: true, result: Some(json!({"authenticated": true, "note": "stdio needs no authentication"})), error: None }
            }
            Ok(request) => guarded_reply(backend, request.id, &request.method, request.params),
        };
        wire::write_reply(&mut output, &reply)?;
    }
}

/// The TCP loop: requests the control server queued (it already answered `machine.stop` on its own thread).
pub(crate) fn serve_requests(backend: &mut dyn ControlBackend, requests: &Receiver<ControlRequest>) {
    for request in requests.iter() {
        if !request.dispatch.try_dispatch() {
            continue;
        }
        let reply = guarded_reply(backend, request.request.id, &request.request.method, request.request.params);
        let _ = request.reply.send(reply);
    }
}

/// The TCP server's configuration: its fast stop says, like stdio, that nothing was stopped.
pub(crate) fn server_config(port: u16, token: Token) -> ServerConfig {
    ServerConfig { port, token, fast_stop: Arc::new(headless_stop_report), wake: Arc::new(|| {}) }
}

/// The token for `serve --port`: given inline, kept in a private token file (created if missing), from the
/// environment, or generated (returned with `true` so the caller prints it once).
pub(crate) fn serve_token(source: Option<TokenSource>) -> Result<(Token, bool), Error> {
    use openlaser_control::token;
    Ok(match source {
        Some(TokenSource::Inline(value)) => (Token::from_hex(value.trim())?, false),
        Some(TokenSource::File(path)) => (token::create_or_reuse_file(&path)?, false),
        Some(TokenSource::Env) => match std::env::var_os("OPENLASER_CONTROL_TOKEN_FILE") {
            Some(path) => (token::create_or_reuse_file(std::path::Path::new(&path))?, false),
            None => match std::env::var("OPENLASER_CONTROL_TOKEN") {
                Ok(value) => (Token::from_hex(value.trim())?, false),
                Err(_) => (Token::generate()?, true),
            },
        },
        None => (Token::generate()?, true),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};
    use std::net::{Ipv4Addr, TcpStream};

    use openlaser_control::server::ControlServer;

    use super::*;
    use crate::backend::Headless;

    fn replies(output: &[u8]) -> Vec<Reply> {
        output.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect()
    }

    #[test]
    fn serve_stdio_recovers_from_bad_and_oversized_lines() {
        let mut input = b"{bad json\n".to_vec();
        input.extend(vec![b'x'; budgets::MAX_REQUEST_BYTES + 10]);
        input.extend_from_slice(b"\n{\"id\":3,\"method\":\"session.list\"}\n");
        let mut output = Vec::new();
        let mut backend = Headless::new(None, None).unwrap();
        serve_lines(&mut backend, Cursor::new(input), &mut output).unwrap();
        let replies = replies(&output);
        assert_eq!(replies.len(), 3);
        assert!(replies[0].error.as_deref().unwrap().starts_with("bad JSON"));
        assert!(replies[1].error.as_deref().unwrap().contains("exceeds"));
        assert!(replies[2].ok && replies[2].id == json!(3));
    }

    /// B4: headless serve says nothing was stopped, identically on stdio and TCP, never `latched:true`.
    #[test]
    fn headless_serve_reports_nothing_stopped_on_stdio_and_tcp() {
        let mut output = Vec::new();
        let mut backend = Headless::new(None, None).unwrap();
        serve_lines(&mut backend, Cursor::new(b"{\"id\":1,\"method\":\"machine.stop\",\"params\":{}}\n".to_vec()), &mut output).unwrap();
        let stdio = replies(&output).remove(0).result.unwrap();
        assert_eq!(stdio["latched"], false);

        let token = Token::generate().unwrap();
        let hex = token.to_hex();
        let (server, _requests) = ControlServer::start(server_config(0, token)).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id":0,"method":"auth","params":{"token":hex}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        writeln!(stream, "{}", json!({"id":1,"method":"machine.stop","params":{}})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let tcp: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(tcp.result.unwrap(), stdio, "stdio and TCP must report the same");
        server.shutdown();
    }

    struct Panicky;

    impl ControlBackend for Panicky {
        fn dispatch_control(&mut self, method: &str, _: Value) -> ToolOutput {
            assert_ne!(method, "boom", "method panicked");
            ToolOutput::Json(json!({"method": method}))
        }
    }

    /// B5: in both serve paths a panicking method is answered and the next request works.
    #[test]
    fn serve_panic_contained_on_stdio_and_tcp() {
        let mut output = Vec::new();
        serve_lines(&mut Panicky, Cursor::new(b"{\"id\":1,\"method\":\"boom\"}\n{\"id\":2,\"method\":\"session.list\"}\n".to_vec()), &mut output).unwrap();
        let replies = replies(&output);
        assert!(replies[0].error.as_deref().unwrap().contains("panicked"));
        assert!(replies[1].ok);

        let token = Token::generate().unwrap();
        let hex = token.to_hex();
        let (server, requests) = ControlServer::start(server_config(0, token)).unwrap();
        let worker = std::thread::spawn(move || serve_requests(&mut Panicky, &requests));
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut read = || {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            serde_json::from_str::<Reply>(&line).unwrap()
        };
        writeln!(stream, "{}", json!({"id":0,"method":"auth","params":{"token":hex}})).unwrap();
        assert!(read().ok);
        writeln!(stream, "{}", json!({"id":1,"method":"boom"})).unwrap();
        assert!(read().error.unwrap().contains("panicked"));
        writeln!(stream, "{}", json!({"id":2,"method":"session.list"})).unwrap();
        assert!(read().ok, "the request after a panic failed");
        drop(stream);
        server.shutdown();
        worker.join().unwrap();
    }

    #[test]
    fn serve_token_file_is_created_private_and_reused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("serve.token");
        let (first, generated) = serve_token(Some(TokenSource::File(path.clone()))).unwrap();
        assert!(!generated);
        let (second, _) = serve_token(Some(TokenSource::File(path.clone()))).unwrap();
        assert_eq!(first.to_hex(), second.to_hex());
        assert!(serve_token(None).unwrap().1, "a generated token is reported so it can be printed");
    }

    #[test]
    fn png_travels_as_png_base64() {
        let reply = control_reply(json!(1), ToolOutput::Png { png: b"\x89PNG".to_vec(), note: json!({"fingerprint":"ab"}) });
        let result = reply.result.unwrap();
        assert_eq!(result["fingerprint"], "ab");
        assert!(result["pngBase64"].is_string());
    }
}
