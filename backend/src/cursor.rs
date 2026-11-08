//! The macOS web view hides the pointer on every key the page handles, as a text
//! field would. In the table grid the arrow keys move a picked cell, and the pointer
//! should stay put: while the grid has focus, that hiding is skipped.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the grid has focus, so key presses leave the pointer alone.
static KEEP: AtomicBool = AtomicBool::new(false);

/// The grid gained (`true`) or lost (`false`) focus.
#[tauri::command]
pub fn keep_cursor(keep: bool) {
    KEEP.store(keep, Ordering::Relaxed);
}

/// Routes `+[NSCursor setHiddenUntilMouseMoves:]` through [`KEEP`]. Call once at start-up.
#[cfg(target_os = "macos")]
pub fn install() {
    use objc2::runtime::{AnyClass, Bool, Imp, Sel};
    use objc2::sel;
    use std::sync::OnceLock;

    type Setter = unsafe extern "C-unwind" fn(&AnyClass, Sel, Bool);
    static ORIGINAL: OnceLock<Setter> = OnceLock::new();

    unsafe extern "C-unwind" fn set_hidden(class: &AnyClass, sel: Sel, hidden: Bool) {
        if hidden.as_bool() && KEEP.load(Ordering::Relaxed) {
            return;
        }
        if let Some(original) = ORIGINAL.get() {
            unsafe { original(class, sel, hidden) }
        }
    }

    let Some(method) = AnyClass::get(c"NSCursor").and_then(|class| class.class_method(sel!(setHiddenUntilMouseMoves:)))
    else {
        return;
    };
    // SAFETY: `set_hidden` has the method's signature (class, selector, BOOL) -> void,
    // and the replaced implementation is kept with that same signature to call on.
    unsafe {
        let new: Imp = std::mem::transmute::<Setter, Imp>(set_hidden);
        let original = method.set_implementation(new);
        let _ = ORIGINAL.set(std::mem::transmute::<Imp, Setter>(original));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
