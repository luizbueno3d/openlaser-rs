//! Native OpenLaser GUI entry point (SPEC §3.21). Default builds inject the simulator and never link hardware
//! transport. Everything testable lives in `src/lib.rs`.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::mpsc;

use openlaser_engine::{DeviceAccess, FsAuthority, Origin, Session, SessionConfig};
use openlaser_format::machines;
use openlaser_lmc_sim::{SimConfig, SimProvider};
use openlaser_types::{Clock, SystemClock};
use openlaser_ui_egui::{AutomationToggle, Clipboard, OpenLaserApp, Services, SettingsStore, UiSettings};

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
        if self.0.get() != settings {
            self.0.update(|value| *value = settings.clone());
            let _ = self.0.flush();
        }
    }
}

/// The system clipboard through egui (text only).
struct EguiClipboard(egui::Context);

impl Clipboard for EguiClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, text: &str) {
        self.0.copy_text(text.to_owned());
    }
}

fn main() -> ExitCode {
    let args = match openlaser::preflight(std::env::args_os().skip(1), &mut std::io::stderr()) {
        Ok(args) => args,
        Err(code) => return ExitCode::from(code),
    };
    let (mut profiles, profile_messages) = machines::load_all(machines::profiles_dir().as_deref());
    let simulator = openlaser_ui_egui::simulator_profile();
    if !profiles.iter().any(|p| p.id == simulator.id) {
        profiles.push(simulator);
    }
    for message in profile_messages {
        log::warn!("machine profile: {message}");
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let files = if args.read_roots.is_empty() && args.write_roots.is_empty() {
        FsAuthority::Denied
    } else {
        match openlaser_control::workspace::AuthorizedWorkspace::open(
            args.read_roots.first().map(|p| p.as_path()),
            args.write_roots.first().map(|p| p.as_path()),
        ) {
            Ok(workspace) => FsAuthority::Scoped(Box::new(openlaser::control_runtime::ScopedFiles(workspace))),
            Err(e) => {
                eprintln!("openlaser: {e}");
                return ExitCode::FAILURE;
            }
        }
    };
    // S24: profiles are written only through this store (human-only Device Settings Save).
    let profile_store = machines::profiles_dir()
        .or_else(|| craft_settings::paths::data_dir("OpenLaser").map(|p| p.join("machines")))
        .map(|dir| Arc::new(machines::DirProfileStore::new(dir)) as Arc<dyn openlaser_engine::ProfileStore>);
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
        profiles: profile_store,
        files,
        clock: Arc::clone(&clock),
    });
    // §3.21: before run_native, a panic hook and a signal watcher that stop every machine first.
    let registry = session.stop_registry();
    openlaser::install_panic_hook(openlaser::registry_stop(Arc::clone(&registry)));
    let _signals = match openlaser::install_signal_watcher(openlaser::registry_stop(Arc::clone(&registry)), |code| std::process::exit(code)) {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("openlaser: cannot install the signal watcher: {error}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("OpenLaser").with_inner_size([1440.0, 900.0]).with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "OpenLaser",
        options,
        Box::new(move |cc| {
            let settings = PersistentSettings::open();
            let in_window_menus = args.in_window_menus || settings.load().in_window_menus;
            let quit_ctx = cc.egui_ctx.clone();
            let mut automation = openlaser::control_runtime::Automation::new(&args, Arc::clone(&registry), cc.egui_ctx.clone());
            if args.control.is_some() {
                automation.set(true).map_err(|e| std::io::Error::other(format!("--control: {e}")))?;
            }
            let services = Services {
                clock: Arc::clone(&clock),
                human: Box::new(openlaser_human::Human::new(clock)),
                files: Box::new(openlaser::native_files::NativeFiles::new(cc)),
                settings: Box::new(settings),
                clipboard: Box::new(EguiClipboard(cc.egui_ctx.clone())),
                automation: Box::new(automation),
                // Asks the root window to close: the close guard in `logic` stops the machines first.
                quit: Box::new(move || quit_ctx.send_viewport_cmd(egui::ViewportCommand::Close)),
            };
            #[cfg(target_os = "macos")]
            let (menu, commands) = openlaser::native_menu::install(&cc.egui_ctx, in_window_menus)?;
            let mut app = OpenLaserApp::new(cc, session, services);
            app.set_in_window_menus(in_window_menus);
            app.open_startup_files(&args.files);
            Ok(Box::new(DesktopApp {
                app,
                #[cfg(target_os = "macos")]
                menu,
                #[cfg(target_os = "macos")]
                commands,
            }))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openlaser: {error}");
            ExitCode::FAILURE
        }
    }
}

struct DesktopApp {
    app: OpenLaserApp,
    #[cfg(target_os = "macos")]
    menu: muda::Menu,
    #[cfg(target_os = "macos")]
    commands: mpsc::Receiver<String>,
}

impl eframe::App for DesktopApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            let _ = &self.menu; // The native items must outlive the installed menu.
            for id in self.commands.try_iter() {
                self.app.invoke_menu_command(ctx, &id);
            }
        }
        // The app's close guard runs inside: while a machine is active it cancels the close and stops first.
        self.app.logic(ctx, frame);
        // Last, so it sees the guard's CancelClose (craft-quit docs; UI §7.5).
        craft_quit::on_close_request(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }
}
