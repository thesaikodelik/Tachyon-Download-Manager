//! Scans finished downloads for viruses (Options → Downloads), as IDM can. By default
//! Microsoft Defender scans the file and quarantines anything it finds; any other
//! scanner can be configured with `{file}` standing for the path.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The outcome of a scan that needs the user's attention.
pub struct Finding {
    pub file: PathBuf,
    pub message: String,
}

static FINDINGS: Mutex<Vec<Finding>> = Mutex::new(Vec::new());

/// Results waiting to be shown, oldest first.
pub fn take_findings() -> Vec<Finding> {
    std::mem::take(&mut *FINDINGS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Scans `file` in the background with `scanner` (empty for Microsoft Defender).
pub fn scan(file: PathBuf, scanner: String) {
    std::thread::Builder::new()
        .name("virus-scan".into())
        .spawn(move || {
            if let Some(message) = run(&file, &scanner) {
                FINDINGS.lock().unwrap_or_else(|e| e.into_inner()).push(Finding { file, message });
            }
        })
        .ok();
}

/// Splits `program args…`, where the program may be quoted.
fn split_command(command: &str) -> (String, String) {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"')
        && let Some((program, args)) = rest.split_once('"')
    {
        return (program.to_string(), args.trim().to_string());
    }
    match command.split_once(' ') {
        Some((program, args)) => (program.to_string(), args.trim().to_string()),
        None => (command.to_string(), String::new()),
    }
}

/// Runs the scan; returns a message for the user when there is something to say.
fn run(file: &Path, scanner: &str) -> Option<String> {
    let custom = !scanner.trim().is_empty();
    let (program, args) = if custom {
        let (program, args) = split_command(scanner);
        let args = if args.contains("{file}") {
            args.replace("{file}", &format!("\"{}\"", file.display()))
        } else {
            format!("{args} \"{}\"", file.display())
        };
        (program, args)
    } else {
        (defender_path()?, format!("-Scan -ScanType 3 -File \"{}\"", file.display()))
    };
    let mut command = std::process::Command::new(&program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // raw_arg keeps the quoting exactly as written for the scanner.
        command.raw_arg(&args).creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    command.args(args.split_whitespace());
    let status = match command.status() {
        Ok(status) => status,
        Err(e) => return Some(format!("The virus scanner couldn't run ({program}): {e}")),
    };
    log::info!("virus scan of {} finished with {status}", file.display());
    match status.code() {
        Some(0) => None,
        // Microsoft Defender: 2 means a threat was found (and handled).
        Some(2) if !custom => Some("Microsoft Defender found a threat in this file and dealt with it.".into()),
        Some(code) => Some(format!("The virus scanner reported a problem (exit code {code}).")),
        None => Some("The virus scanner stopped unexpectedly.".into()),
    }
}

fn defender_path() -> Option<String> {
    let program_files = std::env::var_os("ProgramFiles")?;
    let path = Path::new(&program_files).join("Windows Defender").join("MpCmdRun.exe");
    path.exists().then(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::split_command;

    #[test]
    fn splits_quoted_and_plain_commands() {
        assert_eq!(
            split_command(r#""C:\Program Files\AV\scan.exe" /scan {file}"#),
            (r"C:\Program Files\AV\scan.exe".to_string(), "/scan {file}".to_string())
        );
        assert_eq!(split_command("clamscan --no-summary"), ("clamscan".to_string(), "--no-summary".to_string()));
        assert_eq!(split_command("scan"), ("scan".to_string(), String::new()));
    }
}
