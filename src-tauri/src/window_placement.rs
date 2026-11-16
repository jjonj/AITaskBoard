//! Keep the native window's last placement across normal closes and forced rebuild restarts.
//! The Tauri plugin restores before the initially hidden window is shown; we also
//! checkpoint after drag/resize because the production restart script force-stops it.
use std::{
    sync::mpsc::{self, RecvTimeoutError, Sender},
    time::Duration,
};
use tauri::{Manager, Window, WindowEvent};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

pub(crate) fn state_flags() -> StateFlags {
    // Visibility is intentionally excluded: reopening the app must show it even
    // if it was previously hidden/minimized. Preserve normal bounds and maximize.
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

struct PlacementChanges(Sender<()>);

pub(crate) fn start(app: &mut tauri::App) {
    let (tx, rx) = mpsc::channel();
    app.manage(PlacementChanges(tx));
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // A drag produces many Moved/Resized events. Save once at rest, but
            // while the process is still alive (Exit is skipped on a forced stop).
            loop {
                match rx.recv_timeout(Duration::from_millis(450)) {
                    Ok(()) => {}
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if let Err(error) = handle.save_window_state(state_flags()) {
                eprintln!("TaskBoard: unable to persist window placement: {error}");
            }
        }
    });
}

pub(crate) fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }

    match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            if let Some(changes) = window.app_handle().try_state::<PlacementChanges>() {
                let _ = changes.0.send(());
            }
        }
        WindowEvent::CloseRequested { .. } => {
            // Capture the final geometry on ordinary close, without a debounce.
            let _ = window.app_handle().save_window_state(state_flags());
        }
        _ => {}
    }
}
