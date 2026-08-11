//! Small, platform-specific process lifecycle helpers.

use std::path::Path;

#[cfg(windows)]
use std::ffi::{c_void, OsString};
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;

#[cfg(windows)]
type Handle = *mut c_void;

#[cfg(windows)]
const PROCESS_TERMINATE: u32 = 0x0001;
#[cfg(windows)]
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
#[cfg(windows)]
const WM_CLOSE: u32 = 0x0010;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit_handle: i32, pid: u32) -> Handle;
    fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        path: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
    fn CloseHandle(object: Handle) -> i32;
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn EnumWindows(callback: extern "system" fn(Handle, isize) -> i32, data: isize) -> i32;
    fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
    fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
}

#[cfg(windows)]
pub fn is_process_alive(pid: i64) -> bool {
    const STILL_ACTIVE: u32 = 259;

    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut exit_code = 0;
    let readable = unsafe { GetExitCodeProcess(process, &mut exit_code) != 0 };
    unsafe { CloseHandle(process) };
    readable && exit_code == STILL_ACTIVE
}

#[cfg(unix)]
pub fn is_process_alive(pid: i64) -> bool {
    extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    if pid <= 0 || pid > i32::MAX as i64 {
        return false;
    }
    unsafe { kill(pid as i32, 0) == 0 }
}

#[cfg(not(any(unix, windows)))]
pub fn is_process_alive(_pid: i64) -> bool {
    true
}

#[cfg(windows)]
fn query_executable_path(process: Handle) -> Result<OsString, String> {
    let mut path = vec![0_u16; 32_768];
    let mut len = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    path.truncate(len as usize);
    Ok(OsString::from_wide(&path))
}

#[cfg(windows)]
pub fn executable_path(pid: u32) -> Result<std::path::PathBuf, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(format!(
            "cannot open client process: {}",
            std::io::Error::last_os_error()
        ));
    }
    let path = query_executable_path(process).map(std::path::PathBuf::from);
    unsafe { CloseHandle(process) };
    path
}

#[cfg(not(windows))]
pub fn executable_path(_pid: u32) -> Result<std::path::PathBuf, String> {
    Err("client process inspection is only supported on Windows".to_string())
}

#[cfg(windows)]
fn normalized(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_lowercase()
}

#[cfg(windows)]
fn open_verified(pid: u32, expected_exe: &Path, access: u32) -> Result<Handle, String> {
    let process = unsafe { OpenProcess(access | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(format!(
            "cannot open client process: {}",
            std::io::Error::last_os_error()
        ));
    }
    let actual = query_executable_path(process);
    match actual {
        Ok(actual) if normalized(Path::new(&actual)) == normalized(expected_exe) => Ok(process),
        Ok(actual) => {
            unsafe { CloseHandle(process) };
            Err(format!(
                "client process identity mismatch: expected {}, got {}",
                expected_exe.display(),
                Path::new(&actual).display()
            ))
        }
        Err(error) => {
            unsafe { CloseHandle(process) };
            Err(format!("cannot verify client process identity: {error}"))
        }
    }
}

#[cfg(windows)]
pub fn is_expected_process(pid: u32, expected_exe: &Path) -> bool {
    let Ok(process) = open_verified(pid, expected_exe, 0) else {
        return false;
    };
    unsafe { CloseHandle(process) };
    true
}

#[cfg(not(windows))]
pub fn is_expected_process(pid: u32, _expected_exe: &Path) -> bool {
    is_process_alive(pid.into())
}

#[cfg(windows)]
pub fn request_close(pid: u32, expected_exe: &Path) -> Result<(), String> {
    struct CloseTarget {
        pid: u32,
        sent: bool,
    }

    extern "system" fn close_window(window: Handle, data: isize) -> i32 {
        let target = unsafe { &mut *(data as *mut CloseTarget) };
        let mut window_pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut window_pid) };
        if window_pid == target.pid && unsafe { PostMessageW(window, WM_CLOSE, 0, 0) } != 0 {
            target.sent = true;
        }
        1
    }

    let process = open_verified(pid, expected_exe, 0)?;
    unsafe { CloseHandle(process) };

    let mut target = CloseTarget { pid, sent: false };
    unsafe { EnumWindows(close_window, &mut target as *mut CloseTarget as isize) };
    if target.sent {
        Ok(())
    } else {
        Err("no window found for the active client".to_string())
    }
}

#[cfg(not(windows))]
pub fn request_close(_pid: u32, _expected_exe: &Path) -> Result<(), String> {
    Err("client lifecycle controls are only supported on Windows".to_string())
}

#[cfg(windows)]
pub fn kill(pid: u32, expected_exe: &Path) -> Result<(), String> {
    let process = open_verified(pid, expected_exe, PROCESS_TERMINATE)?;
    let terminated = unsafe { TerminateProcess(process, 1) != 0 };
    let error = std::io::Error::last_os_error();
    unsafe { CloseHandle(process) };
    if terminated {
        Ok(())
    } else {
        Err(format!("cannot terminate client process: {error}"))
    }
}

#[cfg(not(windows))]
pub fn kill(_pid: u32, _expected_exe: &Path) -> Result<(), String> {
    Err("client lifecycle controls are only supported on Windows".to_string())
}
