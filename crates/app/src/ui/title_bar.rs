//! The top bars as the window's title bar. On macOS the app draws under a
//! transparent title bar, so the bars take over what the system's did: pressed
//! and dragged, they move the window; double-clicked, they do what System
//! Settings › Desktop & Dock says for a title bar (zoom by default, or
//! minimize). The controls in a bar keep their clicks. Windows and Linux keep
//! the system's title bar, so there this changes nothing.
//!
//! The traffic lights sit lower than a plain title bar (32 pt) reaches, and
//! AppKit only hovers and presses a button inside its title bar. An empty,
//! invisible toolbar makes the title bar tall enough to hold them; the app
//! still gets every press outside the buttons.

use gpui::{InteractiveElement, MouseButton, Window};

/// Makes `bar`'s empty space act as the window's title bar.
pub fn moves_window<E: InteractiveElement>(bar: E) -> E {
    if cfg!(target_os = "macos") {
        bar.on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                drag_window();
            }
        })
    } else {
        bar
    }
}

/// Keeps a control inside a title bar from moving the window when pressed.
pub fn keeps_window<E: InteractiveElement>(control: E) -> E {
    if cfg!(target_os = "macos") {
        control.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    } else {
        control
    }
}

/// Makes the system's title bar tall enough for the traffic lights where the
/// window places them. Call once, on the window just opened.
#[cfg(target_os = "macos")]
pub fn fit_traffic_lights(window: &mut Window) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSTitlebarSeparatorStyle, NSToolbar, NSView, NSWindowToolbarStyle};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: the handle points at GPUI's view of this window, alive while `window` is.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(native) = view.window() else {
        return;
    };
    native.setToolbar(Some(&NSToolbar::new(main_thread)));
    native.setToolbarStyle(NSWindowToolbarStyle::Unified);
    native.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
    // The toolbar put the buttons back where AppKit keeps them; setting the
    // title has GPUI place them again, now against the taller title bar.
    window.set_window_title(&native.title().to_string());
}

#[cfg(not(target_os = "macos"))]
pub fn fit_traffic_lights(_: &mut Window) {}

/// Lets AppKit move the window with the mouse press being handled, as a
/// native title bar does (GPUI's `start_window_move` is for Linux only).
#[cfg(target_os = "macos")]
fn drag_window() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(main_thread);
    if let Some(event) = app.currentEvent()
        && let Some(window) = event.window(main_thread)
    {
        window.performWindowDragWithEvent(&event);
    }
}

#[cfg(not(target_os = "macos"))]
fn drag_window() {}
