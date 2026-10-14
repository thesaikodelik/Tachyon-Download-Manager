//! Finds which program sent a request to the local endpoint, so captures can be
//! limited to the browsers the user picked.

use std::net::SocketAddr;
use std::path::PathBuf;

/// The executable that owns the loopback connection from `peer` to our `port`.
#[cfg(windows)]
pub(crate) fn program_of(peer: SocketAddr, port: u16) -> Option<PathBuf> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_CONNECTIONS,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    let SocketAddr::V4(peer) = peer else { return None };
    let mut size = 0u32;
    let mut buf: Vec<u32> = Vec::new();
    // The table can grow between asking for its size and reading it.
    for _ in 0..4 {
        buf.resize(size as usize / 4 + 1, 0);
        let status = unsafe {
            GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_CONNECTIONS, 0)
        };
        if status == 0 {
            let table = buf.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
            let rows: &[MIB_TCPROW_OWNER_PID] = unsafe {
                std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize)
            };
            // Ports are stored in network byte order in the low 16 bits.
            let port_of = |raw: u32| u16::from_be(raw as u16);
            let row = rows.iter().find(|row| {
                port_of(row.dwLocalPort) == peer.port()
                    && port_of(row.dwRemotePort) == port
                    && row.dwLocalAddr == u32::from_ne_bytes(peer.ip().octets())
            })?;
            return process_path(row.dwOwningPid);
        }
        if status != 122 {
            // Anything but ERROR_INSUFFICIENT_BUFFER.
            return None;
        }
    }
    None
}

#[cfg(windows)]
fn process_path(pid: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut name = [0u16; 1024];
    let mut len = name.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut len) };
    unsafe { CloseHandle(process) };
    (ok != 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&name[..len as usize])))
}

#[cfg(not(windows))]
pub(crate) fn program_of(_peer: SocketAddr, _port: u16) -> Option<PathBuf> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn finds_this_process_behind_a_loopback_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (_server, peer) = listener.accept().unwrap();
        let program = program_of(peer, port).expect("owner found");
        let name = |p: &std::path::Path| p.file_name().unwrap().to_ascii_lowercase();
        assert_eq!(name(&program), name(&std::env::current_exe().unwrap()));
    }
}
