//! DBForge's release tooling. [`signing`] signs the updates the app's
//! updater installs, in the format installed copies verify: the Tauri
//! updater of 0.3.x and the GPUI app's own both read a minisign signature,
//! base64-encoded, whose trusted comment names the version it was made for.

pub mod signing;
