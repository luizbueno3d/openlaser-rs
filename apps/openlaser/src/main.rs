//! Native OpenLaser GUI entry point. Default builds inject the simulator and never link hardware transport.
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

fn main() -> ExitCode {
    let args = match openlaser::parse_args(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("openlaser: {message}");
            return ExitCode::from(2);
        }
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
        profiles: machines::profiles_dir()
            .or_else(|| craft_settings::paths::data_dir("OpenLaser").map(|p| p.join("machines")))
            .map(|dir| Arc::new(machines::DirProfileStore::new(dir)) as Arc<dyn openlaser_engine::ProfileStore>),
        files,
        clock: Arc::clone(&clock),
    });
    let registry = session.stop_registry();
    openlaser::install_panic_hook(Arc::clone(&registry));
    let _signals = match openlaser::install_signal_watcher(registry) {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("openlaser: cannot install signal watcher: {error}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "OpenLaser",
        options,
        Box::new(move |cc| {
            let mut settings = PersistentSettings::open();
            if args.in_window_menus {
                settings.0.update(|value| value.in_window_menus = true);
            }
            let ctx = cc.egui_ctx.clone();
            let mut automation = openlaser::control_runtime::Automation::new(&args, session.stop_registry(), cc.egui_ctx.clone());
            if args.control.is_some() {
                automation.set(true).map_err(std::io::Error::other)?;
            }
            let services = Services {
                clock: Arc::clone(&clock),
                human: Box::new(openlaser_human::Human::new(clock)),
                files: Box::new(openlaser::native_files::NativeFiles::new(cc)),
                settings: Box::new(settings),
                clipboard: Box::new(EmptyClipboard),
                automation: Box::new(automation),
                quit: Box::new(move || ctx.send_viewport_cmd(egui::ViewportCommand::Close)),
            };
            #[cfg(target_os = "macos")]
            let (menu, commands) = openlaser::native_menu::install(&cc.egui_ctx, services.settings.load().in_window_menus)?;
            let mut app = OpenLaserApp::new(cc, session, services);
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
        self.app.logic(ctx, frame);
        craft_quit::on_close_request(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }
}
