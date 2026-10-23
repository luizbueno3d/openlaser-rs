//! Authorized control-server lifecycle; invoked only by the startup flag or the human settings switch.
use openlaser_control::{
    discovery,
    server::{ControlRequest, ControlServer, ServerConfig},
    token,
};
use openlaser_ui_egui::{AutomationToggle, ControlInfo};
use std::path::PathBuf;
use std::sync::{Arc, mpsc::Receiver};

pub struct Automation {
    pub port: u16,
    pub token_file: Option<PathBuf>,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    pub registry: Arc<openlaser_engine::StopRegistry>,
    pub ctx: egui::Context,
    server: Option<ControlServer>,
    pending: Option<Receiver<ControlRequest>>,
}
impl Automation {
    pub fn new(args: &crate::Args, registry: Arc<openlaser_engine::StopRegistry>, ctx: egui::Context) -> Self {
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
                discovery::remove(&discovery::default_path());
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
            fast_stop: Arc::new(move |_machine| {
                let acks = stop.stop_all(std::time::Duration::from_millis(500));
                format!("{{\"latched\":{},\"machines\":{}}}", acks.iter().all(|(_, a)| a.latched), acks.len()).parse().unwrap_or_default()
            }),
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
        if let Err(e) = discovery::write(&discovery::default_path(), &record) {
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
