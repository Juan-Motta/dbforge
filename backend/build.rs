fn main() {
    // `generate_context!` builds the app and Dock icons into the binary, but
    // nothing tells Cargo to watch them: without this, a new icon only shows
    // after an unrelated change forces a rebuild.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
