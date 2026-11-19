//! On Windows, puts DBForge's icon and version into the executable: Explorer
//! and the taskbar show it, and GPUI loads it (resource 1) as the window's.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/icons/icon.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../packaging/icons/icon.ico")
            .set("ProductName", "DBForge")
            .set("FileDescription", "DBForge")
            .set("OriginalFilename", "DBForge.exe");
        resource.compile().expect("the Windows resources (icon, version) couldn't be compiled");
    }
}
