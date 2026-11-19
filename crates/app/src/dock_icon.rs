//! The Dock icon of an unbundled run. A packaged DBForge.app shows the icon its
//! Info.plist names; the bare executable (`cargo run`, a test build) has no
//! bundle, so macOS would show a generic one. Such a run sets the same icon
//! itself. Windows embeds it as a resource (build.rs) and Linux takes it from
//! the desktop entry, so only macOS needs this.

use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::{NSBundle, NSData};

/// The icon the packaged app carries, every size in one file.
const ICON: &[u8] = include_bytes!("../../../packaging/icons/icon.icns");

/// Shows the app's icon in the Dock when DBForge runs outside its bundle.
/// Call on the main thread once the application has started.
pub fn show_when_unbundled() {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    if NSBundle::mainBundle().bundleIdentifier().is_some() {
        return;
    }
    let data = NSData::with_bytes(ICON);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    // SAFETY: called on the main thread (checked above), with a valid image the application retains.
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image)) };
}
