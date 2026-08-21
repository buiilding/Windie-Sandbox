//! Independent desktop notification component.
//!
//! This process owns the durable `session.completed` observer and the
//! development notification probe. It deliberately does not create a tray or
//! start, stop, or otherwise supervise the API, gateway, or Inspector. The
//! presenter in `tray_notification` is currently named for historical
//! compatibility; its responsibility is platform notification delivery, not
//! tray ownership.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};

/// Runs the notification component until its foreground process receives
/// Ctrl-C or its detached process is stopped by the lifecycle boundary.
pub fn run() -> Result<()> {
    #[cfg(target_os = "macos")]
    crate::local::tray_notification::initialize_native_notifications();

    crate::local::process::register_notifier()?;
    let stopping = Arc::new(AtomicBool::new(false));
    let _completion_observer =
        crate::local::session_event_observer::start_completed_session_observer(stopping.clone());
    let _test_observer =
        crate::local::tray_notification::start_test_completion_observer(stopping.clone());

    let result = wait_for_shutdown();
    stopping.store(true, Ordering::Release);
    let unregister_result = crate::local::process::unregister_notifier();
    match (result, unregister_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(run_error), Err(unregister_error)) => Err(anyhow::anyhow!(
            "{run_error}; notifier PID cleanup failed: {unregister_error:#}"
        )),
    }
}

/// Waits for the only explicit foreground shutdown request. Detached lifecycle
/// commands own forced termination after identity verification; that path does
/// not rely on a signal handler inside the child.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn wait_for_shutdown() -> Result<()> {
    use winit::event::Event;
    use winit::event_loop::{ControlFlow, EventLoop};

    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .context("failed to create macOS notification event loop")?;
    let proxy = event_loop.create_proxy();
    std::thread::Builder::new()
        .name("windie-notifier-ctrl-c".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("windie notifier: failed to create Ctrl-C runtime: {error}");
                    return;
                }
            };
            if let Err(error) = runtime.block_on(tokio::signal::ctrl_c()) {
                eprintln!("windie notifier: failed to receive Ctrl-C: {error}");
            } else {
                let _ = proxy.send_event(());
            }
        })
        .context("failed to start notifier Ctrl-C monitor")?;

    event_loop
        .run(|event, event_loop| {
            event_loop.set_control_flow(ControlFlow::Wait);
            if matches!(event, Event::UserEvent(())) {
                event_loop.exit();
            }
        })
        .context("macOS notification event loop failed")
}

#[cfg(not(target_os = "macos"))]
fn wait_for_shutdown() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async { tokio::signal::ctrl_c().await.map_err(Into::into) })
}
