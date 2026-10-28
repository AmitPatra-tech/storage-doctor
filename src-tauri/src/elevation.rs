//! "Run as administrator" — one UAC prompt per session instead of one per
//! Force Delete / Force Uninstall. When the user turns it on, Storage Doctor
//! relaunches itself elevated at startup; every admin step in that session
//! then runs directly, with no further prompt. On by default (one prompt per
//! launch, like other system tools); it can be turned off in Settings.

use std::path::PathBuf;

/// Argument passed to the elevated relaunch: the ID of the (non-elevated)
/// process that started it. The new process waits for that one to exit, so
/// the single-instance check doesn't mistake the old window for a running copy.
const WAIT_PID_FLAG: &str = "--wait-pid=";
/// Name of the mutex every running UI instance holds.
const INSTANCE_MUTEX: &str = "Local\\HutZon.StorageDoctor.Instance";

/// Present only when the user turned "Run as administrator" off.
fn off_flag_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("StorageDoctor")
        .join("run_as_admin_off")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether this process is running with administrator rights (elevated).
pub fn is_elevated() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Whether "Run as administrator" is on (the default).
pub fn run_as_admin_enabled() -> bool {
    !off_flag_path().exists()
}

pub fn set_run_as_admin(enabled: bool) -> Result<(), String> {
    let path = off_flag_path();
    if enabled {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, b"1").map_err(|e| e.to_string())
    }
}

/// Starts this exe again with administrator rights (one UAC prompt).
/// Returns false if the user declined the prompt or it failed.
pub fn relaunch_elevated() -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let params = format!("{WAIT_PID_FLAG}{}", std::process::id());
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(&params);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success as a value greater than 32.
    result as isize > 32
}

/// Whether another Storage Doctor window is already running (elevated or not).
fn another_instance_running() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let name = wide(INSTANCE_MUTEX);
    unsafe {
        let h = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr());
        if !h.is_null() {
            CloseHandle(h);
            return true;
        }
        // An elevated instance's mutex exists but can't be opened from here.
        GetLastError() == ERROR_ACCESS_DENIED
    }
}

/// Marks this process as the running UI instance, for its whole lifetime.
pub fn mark_instance() {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = wide(INSTANCE_MUTEX);
    // The handle is deliberately never closed: it lives as long as the process.
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
    }
}

/// If this launch was started by `relaunch_elevated`, wait (up to 10 s) for
/// the process that started it to exit before the app initialises.
fn wait_for_parent(args: &[String]) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    let Some(pid) = args
        .iter()
        .find_map(|a| a.strip_prefix(WAIT_PID_FLAG))
        .and_then(|p| p.parse::<u32>().ok())
    else {
        return;
    };
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 10_000);
            CloseHandle(h);
        }
    }
}

/// Called first thing at startup. Returns true when this process should exit
/// because an elevated copy has been started in its place.
pub fn startup() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a.starts_with(WAIT_PID_FLAG)) {
        wait_for_parent(&args);
        return false;
    }
    // Opening a second window just focuses the first: no prompt for that.
    if run_as_admin_enabled() && !is_elevated() && !another_instance_running() {
        // Declined prompt: carry on without administrator rights.
        return relaunch_elevated();
    }
    false
}
