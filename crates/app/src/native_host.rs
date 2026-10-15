//! Lets the browser extension start Tachyon when it isn't running, through the
//! browsers' "native messaging": Tachyon registers itself as a messaging host that
//! only its own extension may call. When a browser calls it, this copy starts
//! Tachyon in the notification area, answers and exits.

use std::io::{Read, Write};
use std::path::Path;

/// The name the extension asks the browser for.
pub const HOST_NAME: &str = "com.thesaikodelik.tachyon";
/// The extension's ID in Chromium browsers, fixed by the key in its manifest.
const CHROMIUM_EXTENSION: &str = "chrome-extension://ifbhemdglfcmphdabagjlcljljcfglib/";
/// The extension's ID in Firefox, from its manifest.
const FIREFOX_EXTENSION: &str = "tachyon@localhost";

/// Where each browser looks for messaging hosts, under HKEY_CURRENT_USER.
const CHROMIUM_KEYS: [&str; 5] = [
    "Software\\Google\\Chrome\\NativeMessagingHosts",
    "Software\\Microsoft\\Edge\\NativeMessagingHosts",
    "Software\\Chromium\\NativeMessagingHosts",
    "Software\\BraveSoftware\\Brave-Browser\\NativeMessagingHosts",
    "Software\\Vivaldi\\NativeMessagingHosts",
];
const FIREFOX_KEY: &str = "Software\\Mozilla\\NativeMessagingHosts";

/// Whether a browser started this copy of Tachyon as its messaging host: Chromium
/// browsers pass the calling extension's origin, Firefox its ID.
pub fn is_host_launch() -> bool {
    std::env::args().skip(1).any(|arg| arg.starts_with("chrome-extension://") || arg == FIREFOX_EXTENSION)
}

/// Answers the browser: starts Tachyon, then replies to the one message it sent.
pub fn serve() {
    let mut stdin = std::io::stdin();
    let mut length = [0u8; 4];
    if stdin.read_exact(&mut length).is_ok() {
        // The message itself only says "launch"; read it so the browser isn't blocked.
        let mut message = vec![0u8; u32::from_ne_bytes(length).min(64 * 1024) as usize];
        let _ = stdin.read_exact(&mut message);
    }
    let launched = launch();
    let reply = format!(r#"{{"ok":{launched}}}"#);
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&(reply.len() as u32).to_ne_bytes());
    let _ = stdout.write_all(reply.as_bytes());
    let _ = stdout.flush();
}

/// Starts Tachyon in the notification area, outside the browser's process group so
/// it keeps running after this copy exits.
#[cfg(windows)]
fn launch() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let Ok(exe) = std::env::current_exe() else { return false };
    keep_pipes_from_children();
    let start = |flags: u32| {
        Command::new(&exe)
            .arg(crate::platform::START_IN_TRAY_FLAG)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    if start(detached | CREATE_BREAKAWAY_FROM_JOB).is_ok() {
        return true;
    }
    // The browser's job doesn't allow leaving it: Explorer starts it instead.
    Command::new("explorer.exe").arg(&exe).spawn().is_ok() || start(detached).is_ok()
}

/// The browser reads this copy's output until every holder of the pipe closes it;
/// Tachyon started from here mustn't inherit it and keep the browser waiting.
#[cfg(windows)]
fn keep_pipes_from_children() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: plain Win32 calls on this process's own standard handles.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(not(windows))]
fn launch() -> bool {
    false
}

/// Registers Tachyon as the extension's messaging host for every browser, pointing
/// at this program. Cheap, so it runs at every start and follows Tachyon if moved.
#[cfg(windows)]
pub fn register(data_dir: &Path) {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};

    let Ok(exe) = std::env::current_exe() else { return };
    let manifest = |allowed: serde_json::Value| {
        let mut m = serde_json::json!({
            "name": HOST_NAME,
            "description": "Tachyon Download Manager",
            "path": exe,
            "type": "stdio",
        });
        m.as_object_mut().unwrap().extend(allowed.as_object().unwrap().clone());
        m
    };
    let files = [
        ("native-host-chromium.json", manifest(serde_json::json!({ "allowed_origins": [CHROMIUM_EXTENSION] })), &CHROMIUM_KEYS[..]),
        ("native-host-firefox.json", manifest(serde_json::json!({ "allowed_extensions": [FIREFOX_EXTENSION] })), &[FIREFOX_KEY][..]),
    ];
    for (file, content, keys) in files {
        let path = data_dir.join(file);
        if let Err(e) = std::fs::write(&path, serde_json::to_vec_pretty(&content).unwrap_or_default()) {
            log::warn!("could not write {}: {e}", path.display());
            continue;
        }
        let value: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        for key in keys {
            let key: Vec<u16> = format!("{key}\\{HOST_NAME}").encode_utf16().chain(Some(0)).collect();
            // SAFETY: NUL-terminated strings; the data length is in bytes and includes the NUL.
            let status = unsafe {
                RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), std::ptr::null(), REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32)
            };
            if status != 0 {
                log::warn!("could not register the browser messaging host: error {status}");
            }
        }
    }
}

#[cfg(not(windows))]
pub fn register(_: &Path) {}

/// Takes Tachyon off every browser's list of messaging hosts (when uninstalled).
#[cfg(windows)]
pub fn unregister() {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
    for key in CHROMIUM_KEYS.iter().chain([&FIREFOX_KEY]) {
        let key: Vec<u16> = format!("{key}\\{HOST_NAME}").encode_utf16().chain(Some(0)).collect();
        // SAFETY: NUL-terminated string.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
    }
}

#[cfg(not(windows))]
pub fn unregister() {}
