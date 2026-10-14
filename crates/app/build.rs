fn main() {
    // On Windows, embed the app icon and version details in tachyon.exe. GPUI uses
    // icon resource 1 for every window's title bar and taskbar button.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/windows/tachyon.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/windows/tachyon.ico")
            .set("ProductName", "Tachyon Download Manager")
            .set("FileDescription", "Tachyon Download Manager")
            .set("LegalCopyright", "MIT License");
        if let Err(e) = resource.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
