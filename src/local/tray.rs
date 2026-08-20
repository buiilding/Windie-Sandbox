//! Simple desktop tray controller for the Windie runtime.
//!
//! The tray is an independent presentation component. It observes component
//! availability and requests explicit lifecycle actions through the same shared
//! operations as the CLI; it never supervises or shuts down the whole runtime.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn run() -> anyhow::Result<()> {
    eprintln!("windie tray is currently supported on macOS and Windows only");
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod app {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::Duration;

    use anyhow::{Context, Result};

    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut std::ffi::c_void;
    }

    #[cfg(windows)]
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(window: *mut std::ffi::c_void, command: i32) -> i32;
    }
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    use winit::event::Event;
    use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};

    const STATUS_INTERVAL: Duration = Duration::from_millis(500);

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
        /// Returns the lifecycle identity used by shared component status.
        const fn managed_component(self) -> crate::local::process::ManagedComponent {
            match self {
                Self::Gateway => crate::local::process::ManagedComponent::Gateway,
                Self::Api => crate::local::process::ManagedComponent::Api,
                Self::Inspector => crate::local::process::ManagedComponent::Inspector,
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

    /// Requests individual component actions while keeping the tray event loop
    /// responsive. Lifecycle ownership remains in `operation::system`.
    #[derive(Clone)]
    struct RuntimeController {
        action_running: Arc<AtomicBool>,
        pending_actions: Arc<std::sync::Mutex<PendingActions>>,
    }

    impl RuntimeController {
        /// Creates the tray's local presentation controller.
        fn new() -> Self {
            Self {
                action_running: Arc::new(AtomicBool::new(false)),
                pending_actions: Arc::new(std::sync::Mutex::new(PendingActions::default())),
            }
        }

        /// Reads the shared component status without changing any process.
        async fn status(&self) -> StatusSnapshot {
            let statuses = crate::operation::component_statuses(
                crate::llm::gateway::GatewayUrl::new(crate::config::gateway_url()),
            )
            .await
            .unwrap_or_default();
            let running = |component| {
                statuses
                    .iter()
                    .any(|status| status.component == component && status.running)
            };

            StatusSnapshot {
                gateway_running: running(Component::Gateway.managed_component()),
                api_running: running(Component::Api.managed_component()),
                inspector_running: running(Component::Inspector.managed_component()),
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

        /// Chooses one explicit lifecycle request from the current health
        /// state. This does not make the tray a process supervisor.
        fn toggle(&self, component: Component) -> Result<()> {
            if self.status_for(component) {
                self.stop_component(component)
            } else {
                self.start_component(component)
            }
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

        /// Starts only the selected component through the shared lifecycle
        /// boundary used by its CLI command.
        fn start_component(&self, component: Component) -> Result<()> {
            match component {
                Component::Gateway => tray_runtime().block_on(crate::operation::start_gateway(
                    crate::llm::gateway::GatewayUrl::new(crate::config::gateway_url()),
                ))?,
                Component::Api => crate::operation::start_api()?,
                Component::Inspector => crate::operation::start_inspector()?,
            };
            Ok(())
        }

        /// Stops only the selected component through the shared lifecycle
        /// boundary used by its CLI command.
        fn stop_component(&self, component: Component) -> Result<()> {
            match component {
                Component::Gateway => tray_runtime().block_on(crate::operation::stop_gateway(
                    crate::llm::gateway::GatewayUrl::new(crate::config::gateway_url()),
                ))?,
                Component::Api => crate::operation::stop_api()?,
                Component::Inspector => crate::operation::stop_inspector()?,
            };
            Ok(())
        }

        /// Reads one component status for an explicit menu action. This runs
        /// only in the tray worker thread, never in the UI event loop.
        fn status_for(&self, component: Component) -> bool {
            tray_runtime()
                .block_on(self.status())
                .running(component)
        }
    }

    /// Builds a small runtime for the tray worker thread's asynchronous gateway
    /// lifecycle request. The UI event loop itself remains synchronous.
    fn tray_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tray lifecycle runtime can be created")
    }

    /// Polls health endpoints in the background for menu label updates.
    fn start_status_monitor(
        controller: RuntimeController,
        sender: Sender<StatusSnapshot>,
        stopping: Arc<AtomicBool>,
    ) {
        thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("failed to create tray status runtime: {error}");
                    return;
                }
            };
            while !stopping.load(Ordering::Acquire) {
                if sender.send(runtime.block_on(controller.status())).is_err() {
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
        #[cfg(windows)]
        hide_console_window();

        let controller = RuntimeController::new();
        crate::local::process::register_tray()?;
        let (status_sender, status_receiver) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        start_status_monitor(controller.clone(), status_sender, stopping.clone());

        let gateway = MenuItem::new("Start Gateway", true, None);
        let api = MenuItem::new("Start API", true, None);
        let inspector = MenuItem::new("Start Inspector", true, None);
        let quit = MenuItem::new("Quit Tray", true, None);
        let menu = Menu::new();
        menu.append(&gateway)?;
        menu.append(&api)?;
        menu.append(&inspector)?;
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
        let unregister_result = crate::local::process::unregister_tray();
        match (event_loop_result, unregister_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(event_error), Ok(())) => Err(event_error),
            (Ok(()), Err(unregister_error)) => Err(unregister_error),
            (Err(event_error), Err(unregister_error)) => Err(anyhow::anyhow!(
                "{event_error}; tray PID cleanup failed: {unregister_error:#}"
            )),
        }?;

        Ok(())
    }

    #[cfg(windows)]
    fn hide_console_window() {
        let window = unsafe { GetConsoleWindow() };
        if !window.is_null() {
            unsafe {
                ShowWindow(window, 0);
            }
        }
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
