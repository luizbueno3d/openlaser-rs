//! The control server's lifecycle (SPEC §3.19 `AutomationToggle`, §4.5, §4.6 mechanism 6). It starts only from
//! `--control` on the command line or the human-only Settings › Automation › Allow agent connections switch, and
//! `machine.stop` is answered on the transport thread by [`fast_stop`] before anything reaches the UI queue.
use openlaser_control::{
    discovery,
    server::{ControlRequest, ControlServer, ServerConfig},
    token,
};
use openlaser_engine::StopRegistry;
use openlaser_ui_egui::serde_json::{Value, json};
use openlaser_ui_egui::{AutomationToggle, ControlInfo};
use std::path::PathBuf;
use std::sync::{Arc, mpsc::Receiver};

/// The `machine.stop {machine?}` fast path. A stop is never narrowed: whatever machine it names, known or unknown,
/// every registered machine is latched (a stop that names a machine this session does not know must still stop the
/// one that is running). The reply always carries a boolean `latched`.
pub fn fast_stop(registry: &Arc<StopRegistry>, machine: Option<&str>) -> Value {
    let started = std::time::Instant::now();
    let acks = crate::stop_all_bounded(registry, crate::STOP_BUDGET);
    let known = machine.map(|id| acks.iter().any(|(m, _)| m.0 == id));
    let latched = acks.iter().all(|(_, ack)| ack.latched);
    let confirmed = !acks.is_empty() && acks.iter().all(|(_, ack)| ack.report.as_ref().is_some_and(|r| r.confirmed));
    json!({
        "latched": latched,
        "confirmed": confirmed,
        "requested": machine,
        "requestedKnown": known,
        "stoppedAll": true,
        "machines": acks.iter().map(|(id, ack)| json!({
            "machine": id.0,
            "latched": ack.latched,
            "confirmed": ack.report.as_ref().is_some_and(|r| r.confirmed),
            "elapsedMs": ack.elapsed_ms,
        })).collect::<Vec<_>>(),
        "elapsedMs": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

pub struct Automation {
    pub port: u16,
    pub token_file: Option<PathBuf>,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    pub registry: Arc<StopRegistry>,
    pub ctx: egui::Context,
    server: Option<ControlServer>,
    pending: Option<Receiver<ControlRequest>>,
}

impl Automation {
    pub fn new(args: &crate::Args, registry: Arc<StopRegistry>, ctx: egui::Context) -> Self {
        Self {
            port: args.control.unwrap_or(0),
            token_file: args.token_file.clone(),
            read_root: args.read_roots.first().cloned(),
            write_root: args.write_roots.first().cloned(),
            registry,
            ctx,
            server: None,
            pending: None,
        }
    }
}

impl AutomationToggle for Automation {
    fn set(&mut self, on: bool) -> Result<Option<ControlInfo>, String> {
        if !on {
            self.pending = None;
            if let Some(server) = self.server.take() {
                server.disconnect_all();
                server.shutdown();
                // No configuration folder means no discovery file was ever written: nothing to remove.
                if let Ok(path) = discovery::default_path() {
                    discovery::remove(&path);
                }
            }
            return Ok(None);
        }
        if self.server.is_some() {
            return Ok(self.info());
        }
        let token = match &self.token_file {
            Some(path) => token::create_or_reuse_file(path),
            None => token::Token::generate(),
        }
        .map_err(|e| e.to_string())?;
        let discovery_token = token.to_hex();
        let stop = Arc::clone(&self.registry);
        let ctx = self.ctx.clone();
        let (server, rx) = ControlServer::start(ServerConfig {
            port: self.port,
            token,
            fast_stop: Arc::new(move |machine| fast_stop(&stop, machine)),
            wake: Arc::new(move || ctx.request_repaint()),
        })
        .map_err(|e| e.to_string())?;
        let record = discovery::Discovery {
            version: 1,
            pid: std::process::id(),
            port: server.port(),
            token: discovery_token,
            started: format!("{:?}", std::time::SystemTime::now()),
            roots: discovery::Roots {
                read: self.read_root.as_ref().map(|p| p.to_string_lossy().into_owned()),
                write: self.write_root.as_ref().map(|p| p.to_string_lossy().into_owned()),
            },
        };
        // Agents find the server through the discovery file; without a configuration folder it cannot be
        // written, so the server is shut down rather than left listening undiscoverably.
        let discovery_path = match discovery::default_path() {
            Ok(path) => path,
            Err(e) => {
                server.shutdown();
                return Err(e.to_string());
            }
        };
        if let Err(e) = discovery::write(&discovery_path, &record) {
            server.shutdown();
            return Err(e.to_string());
        }
        self.server = Some(server);
        self.pending = Some(rx);
        Ok(self.info())
    }

    fn info(&self) -> Option<ControlInfo> {
        self.server.as_ref().map(|s| ControlInfo { port: s.port(), clients: s.clients() })
    }

    fn take_requests(&mut self) -> Option<Receiver<ControlRequest>> {
        self.pending.take()
    }
}

impl Drop for Automation {
    fn drop(&mut self) {
        let _ = self.set(false);
    }
}

pub struct ScopedFiles(pub openlaser_control::workspace::AuthorizedWorkspace);

impl openlaser_engine::FileAccess for ScopedFiles {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.0.read(path).map_err(|e| e.to_string())
    }
    fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.0.write_atomic(path, bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_stop_stops_everything_for_unknown_and_absent_ids_and_reports_a_boolean_latch() {
        let registry = Arc::new(StopRegistry::default());
        for machine in [Some("no-such-machine"), None, Some("")] {
            let reply = fast_stop(&registry, machine);
            assert!(reply["latched"].is_boolean(), "{reply}");
            assert!(reply["confirmed"].is_boolean(), "{reply}");
            assert_eq!(reply["stoppedAll"], true);
            assert!(reply["machines"].is_array());
            assert_eq!(reply["requested"], json!(machine));
        }
        assert_eq!(fast_stop(&registry, Some("no-such-machine"))["requestedKnown"], false);
    }

    #[test]
    fn automation_starts_only_when_asked_and_fails_closed() {
        let args = crate::parse_args(["--control", "4321"].map(std::ffi::OsString::from)).unwrap();
        let mut automation = Automation::new(&args, Arc::new(StopRegistry::default()), egui::Context::default());
        assert_eq!(automation.port, 4321);
        assert!(automation.info().is_none(), "nothing listens before the flag or the switch turns it on");
        assert!(automation.take_requests().is_none());
        // Turning it off when it never started is a no-op (it does not touch the discovery file's folder).
        assert_eq!(automation.set(false), Ok(None));
        assert!(automation.info().is_none());
    }
}
