//! Passwords in `settings.json` (site logins, proxy) are never stored in plain text
//! on Windows: they are sealed with DPAPI, so only the same Windows user can read
//! them. Other systems keep them as written; the settings file is private (0600).

use serde::{Deserialize, Deserializer, Serializer};

const SEALED_PREFIX: &str = "dpapi:";

/// Serde adapter for password fields.
pub mod password {
    use super::*;

    pub fn serialize<S: Serializer>(value: &str, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&seal(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        let stored = String::deserialize(deserializer)?;
        Ok(unseal(&stored))
    }
}

pub fn seal(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    match imp::protect(value.as_bytes()) {
        Some(sealed) => format!("{SEALED_PREFIX}{}", hex(&sealed)),
        None => value.to_string(),
    }
}

/// Reads a sealed value; a value written by hand (no prefix) is taken as is.
pub fn unseal(stored: &str) -> String {
    let Some(encoded) = stored.strip_prefix(SEALED_PREFIX) else {
        return stored.to_string();
    };
    unhex(encoded)
        .and_then(|bytes| imp::unprotect(&bytes))
        .and_then(|plain| String::from_utf8(plain).ok())
        .unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn run(input: &[u8], protect: bool) -> Option<Vec<u8>> {
        let mut blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
        let mut blob_out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: the input blob points at `input`, which outlives the call; the
        // output buffer is allocated by DPAPI and freed with LocalFree below.
        unsafe {
            let ok = if protect {
                CryptProtectData(
                    &mut blob_in,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut blob_out,
                )
            } else {
                CryptUnprotectData(
                    &mut blob_in,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut blob_out,
                )
            };
            if ok == 0 || blob_out.pbData.is_null() {
                return None;
            }
            let out = std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec();
            LocalFree(blob_out.pbData as HLOCAL);
            Some(out)
        }
    }

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        run(plain, true)
    }

    pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        run(sealed, false)
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn protect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for value in ["", "hunter2", "pässwörd with spaces"] {
            assert_eq!(unseal(&seal(value)), value);
        }
    }

    #[cfg(windows)]
    #[test]
    fn never_stores_plain_text() {
        let sealed = seal("hunter2");
        assert!(sealed.starts_with(SEALED_PREFIX));
        assert!(!sealed.contains("hunter2"));
    }

    #[test]
    fn hand_written_values_are_kept() {
        assert_eq!(unseal("typed-in-the-file"), "typed-in-the-file");
    }
}
