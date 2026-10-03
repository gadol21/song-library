// Gives SingAlongStudio.exe its icon (shown in Explorer and the taskbar) and version info.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Sing-Along Studio");
        res.set("FileDescription", "Sing-Along Studio");
        if let Err(e) = res.compile() {
            // A missing resource compiler only costs the icon, not the build
            println!("cargo:warning=could not embed the icon: {}", e);
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
}
