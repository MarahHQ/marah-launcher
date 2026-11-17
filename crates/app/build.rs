//! On Windows: the icon and version info in marah-launcher.exe, which Explorer and the taskbar show.
//! The version comes from Cargo's package version.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/marah-launcher.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../assets/marah-launcher.ico")
        .set("ProductName", "Marah Launcher")
        .set("FileDescription", "Marah Launcher: games from every emulator, in one place")
        .set("OriginalFilename", "marah-launcher.exe")
        .set("InternalName", "marah-launcher");
    if let Err(error) = resource.compile() {
        // CI must ship the icon; a local build without the resource compiler still works.
        if std::env::var_os("CI").is_some() {
            panic!("no icon or version info in marah-launcher.exe: {error}");
        }
        println!("cargo:warning=no icon or version info in marah-launcher.exe: {error}");
    }
}
