//! Native OpenLaser GUI entry point. Default builds inject the simulator and never link hardware transport.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};

use openlaser_engine::{DeviceAccess, FsAuthority, Origin, Session, SessionConfig};
use openlaser_format::machines;
use openlaser_lmc_sim::{SimConfig, SimProvider};
use openlaser_types::{Clock, SystemClock};
use openlaser_ui_egui::{AutomationToggle, Clipboard, ControlInfo, DialogKind, DialogTicket, FileDialogs, OpenLaserApp, Services, SettingsStore, UiSettings};

type PickResult = Option<(String, Vec<u8>)>;
type DialogReceiver = mpsc::Receiver<PickResult>;

struct NativeFiles {
    next: u64,
    pending: HashMap<u64, DialogReceiver>,
}
impl NativeFiles {
    fn new() -> Self {
        Self { next: 1, pending: HashMap::new() }
    }
    fn queue(&mut self, task: impl FnOnce() -> Option<(String, Vec<u8>)> + Send + 'static) -> DialogTicket {
        let ticket = self.next;
        self.next = self.next.saturating_add(1).max(1);
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("openlaser-file-dialog".to_owned()).spawn(move || {
            let _ = tx.send(task());
        });
        self.pending.insert(ticket, rx);
        DialogTicket(ticket)
    }
}
impl FileDialogs for NativeFiles {
    fn open(&mut self, kind: DialogKind) -> DialogTicket {
        self.queue(move || {
            let mut dialog = rfd::FileDialog::new();
            match kind {
                DialogKind::OpenProject => {
                    dialog = dialog.add_filter("OpenLaser Project", &["olp"]);
                }
                DialogKind::Import => {
                    dialog = dialog.add_filter("Vector and image files", &["svg", "dxf", "png", "jpg", "jpeg", "bmp", "tif", "tiff", "webp", "gif"]);
                }
                _ => {}
            }
            let path = dialog.pick_file()?;
            let bytes = std::fs::read(&path).ok()?;
            Some((path.to_string_lossy().into_owned(), bytes))
        })
    }
    fn poll(&mut self, ticket: DialogTicket) -> Option<Option<(String, Vec<u8>)>> {
        let rx = self.pending.get(&ticket.0)?;
        match rx.try_recv() {
            Ok(value) => {
                self.pending.remove(&ticket.0);
                Some(value)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending.remove(&ticket.0);
                Some(None)
            }
        }
    }
    fn save(&mut self, _kind: DialogKind, bytes: Vec<u8>) -> DialogTicket {
        self.queue(move || {
            let path = rfd::FileDialog::new().add_filter("OpenLaser Project", &["olp"]).save_file()?;
            std::fs::write(&path, bytes).ok()?;
            Some((path.to_string_lossy().into_owned(), Vec::new()))
        })
    }
}

struct PersistentSettings(craft_settings::SettingsStore<UiSettings>);
impl PersistentSettings {
    fn open() -> Self {
        let (store, _) = craft_settings::SettingsStore::open(craft_settings::Location::Auto);
        Self(store)
    }
}
impl SettingsStore for PersistentSettings {
    fn load(&self) -> UiSettings {
        self.0.get().clone()
    }
    fn save(&mut self, settings: &UiSettings) {
        self.0.update(|value| *value = settings.clone());
        let _ = self.0.flush();
    }
}

struct EmptyClipboard;
impl Clipboard for EmptyClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _text: &str) {}
}

struct DisabledAutomation;
impl AutomationToggle for DisabledAutomation {
    fn set(&mut self, on: bool) -> Result<Option<ControlInfo>, String> {
        if on { Err("control server is unavailable".to_owned()) } else { Ok(None) }
    }
    fn info(&self) -> Option<ControlInfo> {
        None
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = validate_args(&args) {
        eprintln!("openlaser: {message}");
        return ExitCode::from(2);
    }
    let (profiles, profile_messages) = machines::load_all(machines::profiles_dir().as_deref());
    for message in profile_messages {
        log::warn!("machine profile: {message}");
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let session = Session::new(SessionConfig {
        origin: Origin::Human,
        window: true,
        devices: DeviceAccess::Provider(Arc::new(SimProvider::new(SimConfig {
            buffer_depth: 3,
            transfer_latency: std::time::Duration::ZERO,
            status_extra_bits: 0,
            pid: 0x9899,
            faults: Vec::new(),
            emission_cap_pct: None,
        }))),
        machines: profiles,
        profiles: None,
        files: FsAuthority::Denied,
        clock: Arc::clone(&clock),
    });
    let services = Services {
        human: Box::new(openlaser_human::Human::new(clock)),
        files: Box::new(NativeFiles::new()),
        settings: Box::new(PersistentSettings::open()),
        clipboard: Box::new(EmptyClipboard),
        automation: Box::new(DisabledAutomation),
        quit: Box::new(|| {}),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native("OpenLaser", options, Box::new(move |cc| Ok(Box::new(OpenLaserApp::new(cc, session, services)))));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openlaser: {error}");
            ExitCode::FAILURE
        }
    }
}

fn validate_args(args: &[String]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        let arg = args.get(i).map(String::as_str).unwrap_or_default();
        if arg == "--control" {
            i += 1;
            let value = args.get(i).ok_or_else(|| "--control requires a port".to_owned())?;
            let port = value.parse::<u16>().map_err(|_| format!("invalid --control port `{value}`"))?;
            if port == 0 {
                return Err("--control port must be between 1 and 65535".to_owned());
            }
        } else if arg == "--control-token-file" || arg == "--automation-read-root" || arg == "--automation-write-root" {
            i += 1;
            if args.get(i).is_none() {
                return Err(format!("{arg} requires a path"));
            }
        } else if arg == "--in-window-menus" || arg.starts_with('-') {
            if arg.starts_with('-') && arg != "--in-window-menus" {
                return Err(format!("unknown option `{arg}`"));
            }
        } else {
            let _path = PathBuf::from(arg);
        }
        i += 1;
    }
    Ok(())
}
