//! The existing --create-task hotkey opens a standalone editor overlay, not the
//! entire TaskBoard. Reusing one window preserves unsaved input on repeat presses.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl, WebviewWindowBuilder};

// A WebView must be created off the Windows event-loop thread. The
// single-instance callback runs inside WM_COPYDATA on that thread; constructing
// a WebView there can deadlock the entire board. Coalesce repeated hotkeys while
// one background open is in flight.
static OPEN_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn request_open(app: &tauri::AppHandle) {
    if OPEN_PENDING.swap(true, Ordering::AcqRel) { return; }
    let app = app.clone();
    std::thread::spawn(move || {
        struct ResetPending;
        impl Drop for ResetPending {
            fn drop(&mut self) { OPEN_PENDING.store(false, Ordering::Release); }
        }
        let _reset = ResetPending;
        if let Err(error) = open(&app) {
            eprintln!("TaskBoard: quick create overlay failed: {error}");
        }
    });
}

#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    #[repr(C)]
    struct Point { x: i32, y: i32 }
    extern "system" { fn GetCursorPos(point: *mut Point) -> i32; }
    let mut point = Point { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

#[cfg(not(windows))]
fn cursor_position() -> Option<(i32, i32)> { None }

pub(crate) fn open(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("quick-create") {
        return show(&window);
    }

    WebviewWindowBuilder::new(
        app, "quick-create", WebviewUrl::App("index.html?quick-create=1".into()),
    )
    .title("New Task — TaskBoard")
    .inner_size(756.0, 850.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()?;
    // React requests visibility after mounting. Position only then, so neither
    // initial WebView creation nor the window-state plugin can override the size.
    Ok(())
}

pub(crate) fn show(overlay: &tauri::WebviewWindow) -> tauri::Result<()> {
    if overlay.label() != "quick-create" { return Ok(()); }
    // A global hotkey should appear on the monitor under the pointer, rather
    // than wherever the main TaskBoard happens to be. Use physical pixels for
    // correct work-area placement with mixed-DPI or negative-coordinate screens.
    let monitors = overlay.available_monitors()?;
    let cursor = cursor_position();
    let target = monitors.iter().find(|monitor| {
        let origin = monitor.position();
        let size = monitor.size();
        cursor.is_some_and(|(x, y)| {
            i64::from(x) >= i64::from(origin.x)
                && i64::from(x) < i64::from(origin.x) + i64::from(size.width)
                && i64::from(y) >= i64::from(origin.y)
                && i64::from(y) < i64::from(origin.y) + i64::from(size.height)
        })
    }).or_else(|| monitors.first());
    if let Some(monitor) = target {
        let work = monitor.work_area();
        let width = ((756.0 * monitor.scale_factor()).round() as u32).min(work.size.width);
        // Move first: changing monitors can trigger a DPI-induced resize.
        // Apply the intended physical size on the destination monitor last.
        overlay.set_position(Position::Physical(PhysicalPosition::new(
            work.position.x + (work.size.width - width) as i32,
            work.position.y,
        )))?;
        overlay.set_size(Size::Physical(PhysicalSize::new(width, work.size.height)))?;
    }
    overlay.unminimize()?;
    overlay.show()?;
    overlay.set_focus()?;
    Ok(())
}
