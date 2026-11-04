//! Native OpenLaser GUI entry point (SPEC §3.21). Default builds inject the simulator and never link hardware
//! transport. Everything testable lives in `src/lib.rs`.
#![forbid(unsafe_code)]
// Release builds on Windows open no console window behind the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::mpsc;

use openlaser::diagnostics;
use openlaser_engine::{DeviceAccess, DeviceProvider, FsAuthority, Origin, Session, SessionConfig};
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
    let log_dir = diagnostics::log_dir();
    let supervised = std::env::var_os(diagnostics::SUPERVISED_VAR);
    if diagnostics::supervisor_wanted(supervised.as_deref(), std::env::var_os(diagnostics::SUPERVISOR_VAR).as_deref())
        && let Some(dir) = &log_dir
        && let Some(code) = supervise(dir)
    {
        return code;
    }
    // The window process: its own log records (the supervisor already rotated the log), and the session marker.
    if let Some(dir) = &log_dir
        && diagnostics::install(dir, supervised.is_none()).is_some()
    {
        log::info!(target: "openlaser::start", "{}{}", diagnostics::build_line(), if supervised.is_some() { ", supervised" } else { "" });
        if let Some(text) = diagnostics::describe_previous(&diagnostics::begin_session(dir)) {
            log::warn!(target: "openlaser::start", "{text}");
        }
        log::info!(
            target: "openlaser::start",
            "{}={:?}, {} file argument(s)",
            openlaser::WGPU_BACKEND,
            std::env::var_os(openlaser::WGPU_BACKEND).unwrap_or_default(),
            args.files.len()
        );
        diagnostics::start_heartbeat();
    }
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
    // Phase 6b: the camera↔bed calibration is machine-local setup data (never the project file).
    let camera_store = openlaser_format::camera_cal::calibration_path()
        .map(|path| Arc::new(openlaser_format::camera_cal::DirCalibrationStore::new(path)) as Arc<dyn openlaser_engine::CameraCalibrationStore>);
    let (provider, real_laser) = device_provider();
    log::info!(target: "openlaser::start", "machines: {}", if real_laser { "REAL LASER (USB)" } else { "simulator" });
    if real_laser {
        eprintln!("openlaser: REAL LASER: this window drives the USB board (hw build, OPENLASER_HW_PRESENT=1)");
    }
    let session = Session::new(SessionConfig {
        origin: Origin::Human,
        window: true,
        devices: DeviceAccess::Provider(provider),
        machines: profiles,
        profiles: profile_store,
        camera_store,
        files,
        clock: Arc::clone(&clock),
    });
    // §3.21: before run_native, a panic hook and a signal watcher that stop every machine first.
    let registry = session.stop_registry();
    openlaser::install_panic_hook(openlaser::registry_stop(Arc::clone(&registry)));
    let exit_dir = log_dir.clone();
    let _signals = match openlaser::install_signal_watcher(openlaser::registry_stop(Arc::clone(&registry)), move |code| {
        if let Some(dir) = &exit_dir {
            diagnostics::end_session(dir, "signal watcher (exit 143)");
        }
        std::process::exit(code)
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("openlaser: cannot install the signal watcher: {error}");
            return ExitCode::FAILURE;
        }
    };
    // `main` keeps `registry` for the post-run fail-safe stop; the app closure gets its own clone.
    let registry_for_app = Arc::clone(&registry);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(if real_laser { "OpenLaser — REAL LASER (USB)" } else { "OpenLaser" })
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(app_icon()),
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    let result = eframe::run_native(
        "OpenLaser",
        options,
        Box::new(move |cc| {
            if let Some(render) = &cc.wgpu_render_state {
                openlaser::install_gpu_error_handlers(&render.device);
                let info = render.adapter.get_info();
                log::info!(
                    target: "openlaser::start",
                    "graphics: {} ({:?}, {:?}), driver {} {}, max texture {}",
                    info.name,
                    info.backend,
                    info.device_type,
                    info.driver,
                    info.driver_info,
                    render.device.limits().max_texture_dimension_2d
                );
            }
            let settings = PersistentSettings::open();
            let in_window_menus = args.in_window_menus || settings.load().in_window_menus;
            // Before the native menu is built: its labels are fixed until the next start.
            openlaser_ui_egui::i18n::set_language(settings.load().language);
            let quit_ctx = cc.egui_ctx.clone();
            let mut automation = openlaser::control_runtime::Automation::new(&args, Arc::clone(&registry_for_app), cc.egui_ctx.clone());
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
                beat: std::time::Instant::now(),
                frames: 0,
            }))
        }),
    );
    let failure = result.err().map(|error| {
        eprintln!("openlaser: {error}");
        log::error!(target: diagnostics::EXIT_TARGET, "the window could not run: {error}");
        let graphics = matches!(error, eframe::Error::Wgpu(_));
        (error.to_string(), graphics)
    });
    // Fail-safe stop (UI §7.5): Dock Quit, logout and `terminate:` end the event loop through
    // `applicationWillTerminate`, which bypasses `CloseRequested`, so the close guard never ran.
    // A repeated stop on an already-stopped machine is a no-op.
    let _ = openlaser::stop_all_bounded(&registry, openlaser::STOP_BUDGET);
    if let Some(dir) = &log_dir {
        let reason = diagnostics::logger()
            .and_then(diagnostics::FileLogger::exit_reason)
            .unwrap_or_else(|| "the event loop ended with no close request (Dock Quit, logout, or the system ended it)".to_owned());
        log::info!(target: "openlaser::start", "clean end: {reason}");
        diagnostics::end_session(dir, &reason);
    }
    match failure {
        None => ExitCode::SUCCESS,
        Some((error, graphics)) => explain_failure(&error, graphics),
    }
}

/// The supervisor (Windows): this process opens no window and no device. It starts itself again as the window
/// process, with stderr into the log, waits, and records how it ended; after a death it says so in a message box.
/// It never touches a machine: it opens no device and stops nothing. `None` when the child could not start (then this process
/// opens the window itself, as before).
fn supervise(dir: &std::path::Path) -> Option<ExitCode> {
    let logger = diagnostics::install(dir, true);
    log::info!(target: "openlaser::supervisor", "{}, supervisor", diagnostics::build_line());
    let exe = std::env::current_exe().ok()?;
    let mut child = std::process::Command::new(exe);
    child.args(std::env::args_os().skip(1));
    match diagnostics::supervise(child, dir) {
        Ok(seen) => {
            if !seen.exit.clean {
                died_message(&seen.exit.text, logger.map(diagnostics::FileLogger::path));
            }
            Some(ExitCode::from(u8::try_from(seen.code).unwrap_or(1)))
        }
        Err(error) => {
            log::error!(target: "openlaser::supervisor", "could not start the window process ({error}); opening the window in this process");
            None
        }
    }
}

#[cfg(windows)]
fn died_message(what: &str, log: Option<&std::path::Path>) {
    let log = log.map(|p| p.display().to_string()).unwrap_or_default();
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("OpenLaser closed unexpectedly")
        .set_description(format!(
            "OpenLaser closed unexpectedly ({what}).\n\nA job the laser board had already received may still be running on the machine. \
             The details are in {log}. Run \"OpenLaser diagnostics\" and send its report."
        ))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(not(windows))]
fn died_message(what: &str, _log: Option<&std::path::Path>) {
    eprintln!("openlaser: the window process died ({what})");
}

/// 32-bit Windows: OpenGL (WGL) only, unless `WGPU_BACKEND` says otherwise. CI (i686 on windows-latest) found the other
/// backends broken in a 32-bit program: Vulkan aborts inside wgpu's debug-message callback, and DirectX 12 (WARP)
/// fails to compile egui's shader, which wgpu reports as a panic, not an error. OpenGL opened the window. The old
/// Intel graphics such PCs have ship no 32-bit Vulkan or DirectX 12 driver anyway. Every other build keeps eframe's
/// default backends. Every build sets the OpenGL fence behaviour ([`openlaser::gl_fence_behavior`]), which only the
/// OpenGL backend reads.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        let fence = openlaser::gl_fence_behavior(std::env::var_os(openlaser::GL_FENCE_VAR).as_deref());
        log::info!(target: "openlaser::start", "OpenGL fences (used only on the OpenGL backend): {fence:?}");
        setup.instance_descriptor.backend_options.gl.fence_behavior = fence;
    }
    #[cfg(all(windows, target_arch = "x86"))]
    if std::env::var_os(openlaser::WGPU_BACKEND).is_none()
        && let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup
    {
        setup.instance_descriptor.backends = eframe::wgpu::Backends::GL;
    }
    options
}

/// Windows: a release build has no console, so a failed start says why in a message box. When the graphics failed
/// and nobody chose a wgpu backend, OpenLaser first starts once more on OpenGL alone (old Intel graphics without
/// DirectX 12 or Vulkan). The window never opened, so no machine was connected.
#[cfg(windows)]
fn explain_failure(error: &str, graphics: bool) -> ExitCode {
    let text = if graphics {
        match openlaser::graphics_failure(error, std::env::var_os(openlaser::WGPU_BACKEND).as_deref()) {
            openlaser::GraphicsFailure::RetryWithGl => {
                if let Some(code) = restart_with_gl() {
                    return code;
                }
                format!("OpenLaser could not start its graphics, and could not start again on OpenGL.\n\n{error}")
            }
            openlaser::GraphicsFailure::Explain(text) => text,
        }
    } else {
        format!("OpenLaser could not start.\n\n{error}")
    };
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("OpenLaser: cannot start")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    ExitCode::FAILURE
}

#[cfg(not(windows))]
fn explain_failure(_error: &str, _graphics: bool) -> ExitCode {
    ExitCode::FAILURE
}

/// Runs this program again with the same arguments and `WGPU_BACKEND=gl`, and returns its exit status.
#[cfg(windows)]
fn restart_with_gl() -> Option<ExitCode> {
    let exe = std::env::current_exe().ok()?;
    eprintln!("openlaser: starting again with {}=gl", openlaser::WGPU_BACKEND);
    let status = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env(openlaser::WGPU_BACKEND, "gl").status().ok()?;
    Some(ExitCode::from(status.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)))
}

struct DesktopApp {
    app: OpenLaserApp,
    #[cfg(target_os = "macos")]
    menu: muda::Menu,
    #[cfg(target_os = "macos")]
    commands: mpsc::Receiver<String>,
    /// The UI heartbeat: frames drawn and textures held, once per `diagnostics::HEARTBEAT`.
    beat: std::time::Instant,
    frames: u64,
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
        self.frames = self.frames.saturating_add(1);
        if self.beat.elapsed() >= diagnostics::HEARTBEAT {
            let secs = self.beat.elapsed().as_secs_f64();
            log::info!(
                target: "openlaser::heartbeat",
                "ui: {} frames in {secs:.0} s ({:.1} per s), {} textures, {} graphics errors so far",
                self.frames,
                self.frames as f64 / secs,
                ctx.tex_manager().read().num_allocated(),
                openlaser::GPU_ERRORS.load(std::sync::atomic::Ordering::Relaxed)
            );
            self.beat = std::time::Instant::now();
            self.frames = 0;
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

/// The machines' device provider: the simulator, or, in a `hw` build run with `OPENLASER_HW_PRESENT=1` and
/// `OPENLASER_NO_HW` unset, the real USB board (S4, S23), exactly as `openlaser-hw` opens it. The transport still
/// refuses every open without a bench session (`openlaser-hw bench start`) and caps its first emission.
fn device_provider() -> (Arc<dyn DeviceProvider>, bool) {
    #[cfg(feature = "hw")]
    {
        if !openlaser_transport::hardware_disabled() {
            return (Arc::new(openlaser_transport::UsbProvider::new()), true);
        }
    }
    let sim = SimProvider::new(SimConfig {
        buffer_depth: 3,
        transfer_latency: std::time::Duration::ZERO,
        status_extra_bits: 0,
        pid: 0x9899,
        faults: Vec::new(),
        emission_cap_pct: None,
    });
    (Arc::new(sim), false)
}

/// OpenLaser's own icon (assets/icon.svg); without it eframe shows its default icon in the Dock and taskbar.
fn app_icon() -> Arc<egui::IconData> {
    Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default())
}
