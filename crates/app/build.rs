use std::path::{Path, PathBuf};

/// Every file of the browser extension, for the installer to write out.
fn embed_extension() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../extension");
    println!("cargo:rerun-if-changed={}", root.display());
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                println!("cargo:rerun-if-changed={}", path.display());
                walk(&path, base, out);
            } else {
                let relative = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                println!("cargo:rerun-if-changed={}", path.display());
                out.push((relative, path.canonicalize().unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    let mut code = String::from("/// The browser extension's files: (path inside the folder, contents).\n");
    code.push_str("pub static EXTENSION_FILES: &[(&str, &[u8])] = &[\n");
    for (relative, path) in files {
        code.push_str(&format!("    ({relative:?}, include_bytes!({:?})),\n", path.display().to_string()));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("extension_files.rs");
    std::fs::write(out, code).unwrap();
}

fn main() {
    embed_extension();
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
