//! Simple desktop tray controller for the Windie runtime.
//!
//! The tray is a mode of the main `windie` executable. It invokes the sibling
//! executable path with `gateway`, `api`, and `inspector` lifecycle commands;
//! those commands still detach and manage their own independent processes.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn run() -> anyhow::Result<()> {
    eprintln!("windie tray is currently supported on macOS and Windows only");
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod app {
    use std::env;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow};
    use reqwest::blocking::Client;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    use winit::event::Event;
    use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};

    const STATUS_INTERVAL: Duration = Duration::from_millis(500);
    const HEALTH_TIMEOUT: Duration = Duration::from_millis(500);

    /// A local service controlled by one tray toggle.
    #[derive(Clone, Copy)]
    enum Component {
        Gateway,
        Api,
        Inspector,
    }

    /// Events sent to the tray event loop by non-UI shutdown sources.
    enum TrayEvent {
        Shutdown,
    }

    impl Component {
        /// Returns the lifecycle command subject accepted by the CLI parser.
        const fn command(self) -> &'static str {
            match self {
                Self::Gateway => "gateway",
                Self::Api => "api",
                Self::Inspector => "inspector",
            }
        }

        /// Returns the localhost endpoint used for status polling.
        const fn health_url(self) -> &'static str {
            match self {
                Self::Gateway => "http://127.0.0.1:8080/health",
                Self::Api => "http://127.0.0.1:8787/api/health",
                Self::Inspector => "http://127.0.0.1:3000/",
            }
        }
    }

    /// Current availability of the three independently managed components.
    #[derive(Clone, Copy, Default)]
    struct StatusSnapshot {
        gateway_running: bool,
        api_running: bool,
        inspector_running: bool,
    }

    impl StatusSnapshot {
        /// Returns the status for one component.
        const fn running(self, component: Component) -> bool {
            match component {
                Component::Gateway => self.gateway_running,
                Component::Api => self.api_running,
                Component::Inspector => self.inspector_running,
            }
        }
    }

    /// Lifecycle transition currently being performed for one component.
    #[derive(Clone, Copy)]
    enum PendingAction {
        Starting,
        Stopping,
    }

    /// Pending lifecycle transitions displayed before the next health poll.
    #[derive(Clone, Copy, Default)]
    struct PendingActions {
        gateway: Option<PendingAction>,
        api: Option<PendingAction>,
        inspector: Option<PendingAction>,
    }

    impl PendingActions {
        /// Returns the pending transition for one component.
        const fn get(self, component: Component) -> Option<PendingAction> {
            match component {
                Component::Gateway => self.gateway,
                Component::Api => self.api,
                Component::Inspector => self.inspector,
            }
        }

        /// Sets or clears the pending transition for one component.
        fn set(&mut self, component: Component, action: Option<PendingAction>) {
            match component {
                Component::Gateway => self.gateway = action,
                Component::Api => self.api = action,
                Component::Inspector => self.inspector = action,
            }
        }
    }

    /// Locates the main executable and runs CLI lifecycle commands for it.
    #[derive(Clone)]
    struct RuntimeController {
        client: Client,
        windie_binary: PathBuf,
        action_running: Arc<AtomicBool>,
        pending_actions: Arc<std::sync::Mutex<PendingActions>>,
    }

    impl RuntimeController {
        /// Creates a controller using the current `windie` executable path.
        fn new() -> Result<Self> {
            let windie_binary = env::current_exe().context("failed to locate windie")?;
            let client = Client::builder()
                .timeout(HEALTH_TIMEOUT)
                .build()
                .context("failed to create local health client")?;

            Ok(Self {
                client,
                windie_binary,
                action_running: Arc::new(AtomicBool::new(false)),
                pending_actions: Arc::new(std::sync::Mutex::new(PendingActions::default())),
            })
        }

        /// Reads service availability without depending on process ownership.
        fn status(&self) -> StatusSnapshot {
            StatusSnapshot {
                gateway_running: self.is_running(Component::Gateway),
                api_running: self.is_running(Component::Api),
                inspector_running: self.is_running(Component::Inspector),
            }
        }

        /// Runs one toggle in a worker thread so the tray remains responsive.
        fn toggle_async(&self, component: Component, running: bool) {
            if self
                .action_running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                eprintln!("Windie is already handling another tray action");
                return;
            }

            let action = if running {
                PendingAction::Stopping
            } else {
                PendingAction::Starting
            };
            self.set_pending_action(component, Some(action));

            let controller = self.clone();
            thread::spawn(move || {
                let result = controller.toggle(component);
                controller.set_pending_action(component, None);
                if let Err(error) = result {
                    eprintln!("Windie tray action failed: {error:#}");
                }
                controller.action_running.store(false, Ordering::Release);
            });
        }

        /// Chooses start or stop from the current health state.
        fn toggle(&self, component: Component) -> Result<()> {
            let action = if self.is_running(component) {
                "stop"
            } else {
                "start"
            };
            self.run_cli(component, action)
        }

        /// Sets or clears the transition shown in the tray menu.
        fn set_pending_action(&self, component: Component, action: Option<PendingAction>) {
            let mut pending = self
                .pending_actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.set(component, action);
        }

        /// Reads the transition shown in the tray menu.
        fn pending_action(&self, component: Component) -> Option<PendingAction> {
            let pending = self
                .pending_actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.get(component)
        }

        /// Invokes the current executable as a short-lived CLI command.
        fn run_cli(&self, component: Component, action: &str) -> Result<()> {
            let status = Command::new(&self.windie_binary)
                .args([component.command(), action])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .with_context(|| {
                    format!("failed to run windie {} {}", component.command(), action)
                })?;

            if status.success() {
                Ok(())
            } else {
                Err(anyhow!(
                    "windie {} {} exited with {status}",
                    component.command(),
                    action
                ))
            }
        }

        /// Invokes the shared non-interactive uninstall command after the
        /// tray event loop has released its PID registration.
        fn run_uninstall(&self) -> Result<()> {
            let status = Command::new(&self.windie_binary)
                .args(["uninstall", "--yes"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .context("failed to run windie uninstall --yes")?;

            if status.success() {
                Ok(())
            } else {
                Err(anyhow!("windie uninstall --yes exited with {status}"))
            }
        }

        /// Stops every managed component, continuing if one stop command
        /// fails so that a failure in one component cannot leave the others
        /// running unintentionally.
        fn stop_all(&self) -> Result<()> {
            let mut failures = Vec::new();
            for component in [Component::Gateway, Component::Api, Component::Inspector] {
                if let Err(error) = self.run_cli(component, "stop") {
                    failures.push(format!("{}: {error:#}", component.command()));
                }
            }

            if failures.is_empty() {
                Ok(())
            } else {
                Err(anyhow!(
                    "one or more Windie components failed to stop: {}",
                    failures.join("; ")
                ))
            }
        }

        /// Returns whether a component responds successfully on localhost.
        fn is_running(&self, component: Component) -> bool {
            self.client
                .get(component.health_url())
                .send()
                .is_ok_and(|response| response.status().is_success())
        }
    }

    /// Polls health endpoints in the background for menu label updates.
    fn start_status_monitor(
        controller: RuntimeController,
        sender: Sender<StatusSnapshot>,
        stopping: Arc<AtomicBool>,
    ) {
        thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                if sender.send(controller.status()).is_err() {
                    break;
                }
                thread::sleep(STATUS_INTERVAL);
            }
        });
    }

    /// Waits for Ctrl+C and forwards shutdown to the UI event loop.
    ///
    /// The signal monitor runs on its own thread because the tray event loop
    /// is synchronous. A watch channel lets it exit when the event loop has
    /// already shut down through the menu or another event.
    fn start_ctrl_c_monitor(
        proxy: EventLoopProxy<TrayEvent>,
        mut stopping: tokio::sync::watch::Receiver<bool>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("failed to create tray Ctrl+C monitor: {error}");
                    return;
                }
            };

            runtime.block_on(async move {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result {
                            eprintln!("failed to receive tray Ctrl+C: {error}");
                        } else {
                            let _ = proxy.send_event(TrayEvent::Shutdown);
                        }
                    }
                    changed = stopping.changed() => {
                        let _ = changed;
                    }
                }
            });
        })
    }

    /// Runs the macOS or Windows tray event loop.
    #[allow(deprecated)]
    pub fn run() -> Result<()> {
        let controller = RuntimeController::new()?;
        crate::process::register_tray()?;
        let (status_sender, status_receiver) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        start_status_monitor(controller.clone(), status_sender, stopping.clone());

        let gateway = MenuItem::new("Start Gateway", true, None);
        let api = MenuItem::new("Start API", true, None);
        let inspector = MenuItem::new("Start Inspector", true, None);
        let uninstall = MenuItem::new("Uninstall Windie", true, None);
        let quit = MenuItem::new("Quit and Stop Services", true, None);
        let menu = Menu::new();
        menu.append(&gateway)?;
        menu.append(&api)?;
        menu.append(&inspector)?;
        menu.append(&uninstall)?;
        menu.append(&quit)?;

        let event_loop = EventLoop::<TrayEvent>::with_user_event()
            .build()
            .context("failed to create tray event loop")?;
        let event_loop_proxy = event_loop.create_proxy();
        let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
        let mut icon = Some(tray_icon()?);
        let ctrl_c_monitor = start_ctrl_c_monitor(event_loop_proxy, shutdown_receiver);
        let mut tray: Option<TrayIcon> = None;
        let event_loop_controller = controller.clone();
        let event_loop_stopping = stopping.clone();
        let uninstall_requested = Arc::new(AtomicBool::new(false));
        let event_loop_uninstall_requested = uninstall_requested.clone();
        let mut current_status = StatusSnapshot::default();
        let result = event_loop.run(move |event, event_loop| {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + STATUS_INTERVAL,
            ));

            if matches!(event, Event::Resumed) && tray.is_none() {
                match TrayIconBuilder::new()
                    .with_tooltip("Windie")
                    .with_icon(icon.take().expect("tray icon is created once"))
                    .with_icon_as_template(true)
                    .with_menu(Box::new(menu.clone()))
                    .build()
                {
                    Ok(created) => tray = Some(created),
                    Err(error) => {
                        eprintln!("failed to create Windie tray icon: {error}");
                        event_loop.exit();
                    }
                }
            }

            if matches!(event, Event::UserEvent(TrayEvent::Shutdown)) {
                event_loop_stopping.store(true, Ordering::Release);
                event_loop.exit();
            }

            if let Some(status) = update_menu_labels(
                &status_receiver,
                &gateway,
                &api,
                &inspector,
                &event_loop_controller,
            ) {
                current_status = status;
            }

            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id() == gateway.id() {
                    event_loop_controller
                        .toggle_async(Component::Gateway, current_status.gateway_running);
                } else if event.id() == api.id() {
                    event_loop_controller.toggle_async(Component::Api, current_status.api_running);
                } else if event.id() == inspector.id() {
                    event_loop_controller
                        .toggle_async(Component::Inspector, current_status.inspector_running);
                } else if event.id() == uninstall.id() {
                    event_loop_uninstall_requested.store(true, Ordering::Release);
                    event_loop_stopping.store(true, Ordering::Release);
                    event_loop.exit();
                } else if event.id() == quit.id() {
                    event_loop_stopping.store(true, Ordering::Release);
                    event_loop.exit();
                }
            }
        });

        stopping.store(true, Ordering::Release);
        let _ = shutdown_sender.send(true);
        let _ = ctrl_c_monitor.join();

        let event_loop_result = result.context("Windie tray event loop failed");
        let shutdown_result = controller.stop_all();
        let unregister_result = crate::process::unregister_tray();
        match (event_loop_result, shutdown_result, unregister_result) {
            (Ok(()), Ok(()), Ok(())) => Ok(()),
            (Err(event_error), Ok(()), Ok(())) => Err(event_error),
            (Ok(()), Err(shutdown_error), Ok(())) => Err(shutdown_error),
            (Ok(()), Ok(()), Err(unregister_error)) => Err(unregister_error),
            (Err(event_error), Err(shutdown_error), Ok(())) => Err(anyhow!(
                "{event_error}; tray shutdown cleanup failed: {shutdown_error:#}"
            )),
            (Err(event_error), Ok(()), Err(unregister_error)) => Err(anyhow!(
                "{event_error}; tray PID cleanup failed: {unregister_error:#}"
            )),
            (Ok(()), Err(shutdown_error), Err(unregister_error)) => Err(anyhow!(
                "{shutdown_error:#}; tray PID cleanup failed: {unregister_error:#}"
            )),
            (Err(event_error), Err(shutdown_error), Err(unregister_error)) => Err(anyhow!(
                "{event_error}; tray shutdown cleanup failed: {shutdown_error:#}; tray PID cleanup failed: {unregister_error:#}"
            )),
        }?;

        if uninstall_requested.load(Ordering::Acquire) {
            controller.run_uninstall()?;
        }

        Ok(())
    }

    /// Applies the newest health snapshot to the toggle labels.
    fn update_menu_labels(
        receiver: &Receiver<StatusSnapshot>,
        gateway: &MenuItem,
        api: &MenuItem,
        inspector: &MenuItem,
        controller: &RuntimeController,
    ) -> Option<StatusSnapshot> {
        let mut latest = None;
        while let Ok(status) = receiver.try_recv() {
            latest = Some(status);
        }
        let status = latest?;

        gateway.set_text(toggle_label(
            "Gateway",
            status.running(Component::Gateway),
            controller.pending_action(Component::Gateway),
        ));
        api.set_text(toggle_label(
            "API",
            status.running(Component::Api),
            controller.pending_action(Component::Api),
        ));
        inspector.set_text(toggle_label(
            "Inspector",
            status.running(Component::Inspector),
            controller.pending_action(Component::Inspector),
        ));
        Some(status)
    }

    /// Returns the menu label for one component's current lifecycle state.
    fn toggle_label(name: &str, running: bool, pending: Option<PendingAction>) -> String {
        if let Some(action) = pending {
            return match action {
                PendingAction::Starting => format!("Starting {name}"),
                PendingAction::Stopping => format!("Stopping {name}"),
            };
        }

        if running {
            format!("Stop {name}")
        } else {
            format!("Start {name}")
        }
    }

    /// Creates the small monochrome tray icon.
    fn tray_icon() -> Result<Icon> {
        let width = 32;
        let height = 32;
        let mut rgba = vec![0_u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let dx = x as i32 - 16;
                let dy = y as i32 - 16;
                if dx * dx + dy * dy <= 225 {
                    let offset = (y * width + x) * 4;
                    rgba[offset..offset + 4].copy_from_slice(&[30, 41, 59, 255]);
                }
            }
        }

        Icon::from_rgba(rgba, width as u32, height as u32)
            .context("failed to create Windie tray icon")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn run() -> anyhow::Result<()> {
    app::run()
}
