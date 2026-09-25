//! Windows Explorer right-click "Force delete with Storage Doctor" entry.
//!
//! Registered per-user under `HKCU\Software\Classes` — no administrator rights
//! — for both files (`*`) and folders (`Directory`). Choosing it launches this
//! same executable with `--force-delete "<path>"`, which surfaces the
//! force-delete flow for that one path without the user opening the app first.
//!
//! Windows 11 shows classic shell verbs like this under **Show more options**
//! (Shift+F10), not the top-level modern menu. Getting into the modern menu
//! would require shipping a packaged `IExplorerCommand` handler; the registry
//! verb here is the standard, admin-free approach every comparable tool uses.

use std::path::Path;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

/// Registry key name for our verb — unique enough not to collide.
const VERB: &str = "StorageDoctorForceDelete";
const LABEL: &str = "Force delete with Storage Doctor";
/// The shell classes the verb attaches to: every file, and every directory.
const CLASSES: [&str; 2] = ["*", "Directory"];

/// The CLI flag Explorer's command line passes back to us, followed by the
/// selected path. Shared with `lib.rs`, which parses it on launch.
pub const FORCE_DELETE_FLAG: &str = "--force-delete";

fn verb_key_path(class: &str) -> String {
    format!(r"Software\Classes\{class}\shell\{VERB}")
}

/// The `--force-delete "<path>"` argument, extracted from a process argv.
/// Returns the path when the flag is present with a following value. Shared by
/// the initial launch (`std::env::args`) and the single-instance callback so
/// both parse it identically.
pub fn force_delete_target(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == FORCE_DELETE_FLAG {
            return it.next().filter(|p| !p.is_empty()).cloned();
        }
        // Also accept `--force-delete=<path>`.
        if let Some(rest) = arg.strip_prefix(&format!("{FORCE_DELETE_FLAG}=")) {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// The exact command line Explorer runs for the verb, quoted so paths with
/// spaces survive. `%1` is the file or folder the user right-clicked.
fn command_line(exe: &str) -> String {
    format!("\"{exe}\" {FORCE_DELETE_FLAG} \"%1\"")
}

/// The command line currently registered under the verb (from the `*` class),
/// or `None` if the verb is not registered at all.
fn registered_command() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let command = hkcu
        .open_subkey_with_flags(format!("{}\\command", verb_key_path("*")), KEY_READ)
        .ok()?;
    command.get_value::<String, _>("").ok()
}

fn current_exe() -> Result<String, String> {
    std::env::current_exe()
        .map_err(|e| e.to_string())
        .map(|p| p.to_string_lossy().into_owned())
}

/// True only when the verb is registered *and* points at the executable
/// running now. A stale entry left by a previous install path reads as not
/// registered, so re-enabling heals it rather than leaving a dead command.
pub fn is_registered() -> bool {
    match (registered_command(), current_exe()) {
        (Some(cmd), Ok(exe)) => cmd == command_line(&exe),
        _ => false,
    }
}

/// Adds (or refreshes) the context-menu verb for files and folders.
pub fn register() -> Result<(), String> {
    let exe = current_exe()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for class in CLASSES {
        let (key, _) = hkcu
            .create_subkey(verb_key_path(class))
            .map_err(|e| e.to_string())?;
        key.set_value("", &LABEL).map_err(|e| e.to_string())?;
        // Shows this app's icon beside the menu entry.
        key.set_value("Icon", &format!("{exe},0"))
            .map_err(|e| e.to_string())?;
        let (command, _) = key.create_subkey("command").map_err(|e| e.to_string())?;
        command
            .set_value("", &command_line(&exe))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Removes the verb from both classes. Missing keys are not an error — the end
/// state is what matters, and it is "not registered" either way.
pub fn unregister() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for class in CLASSES {
        match hkcu.delete_subkey_all(verb_key_path(class)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

/// True if `path` is a plain file or directory that exists — the only thing
/// worth force-deleting. Guards against a junk `%1` (a virtual shell item,
/// a deleted path) launching the flow against nothing.
pub fn is_deletable_target(path: &str) -> bool {
    let p = Path::new(path);
    p.is_file() || p.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_force_delete_target_in_both_forms() {
        let split = vec![
            "storage-doctor.exe".to_string(),
            "--force-delete".to_string(),
            r"C:\stuck folder".to_string(),
        ];
        assert_eq!(force_delete_target(&split).as_deref(), Some(r"C:\stuck folder"));

        let joined = vec![
            "storage-doctor.exe".to_string(),
            r"--force-delete=C:\a\b.txt".to_string(),
        ];
        assert_eq!(force_delete_target(&joined).as_deref(), Some(r"C:\a\b.txt"));
    }

    #[test]
    fn no_target_without_the_flag_or_value() {
        assert_eq!(force_delete_target(&["storage-doctor.exe".to_string()]), None);
        assert_eq!(
            force_delete_target(&[
                "storage-doctor.exe".to_string(),
                "--force-delete".to_string(),
            ]),
            None
        );
    }

    #[test]
    fn command_line_is_quoted_for_spaces_and_carries_the_flag() {
        let cmd = command_line(r"C:\Program Files\Storage Doctor\storage-doctor.exe");
        assert_eq!(
            cmd,
            r#""C:\Program Files\Storage Doctor\storage-doctor.exe" --force-delete "%1""#
        );
    }

    /// Exercises the real HKCU registry round-trip. Ignored by default because
    /// it writes to the live registry (under the test binary's own path, so
    /// harmless) — run explicitly with:
    ///   cargo test --lib shell::tests::registry_round_trip -- --ignored
    #[test]
    #[ignore]
    fn registry_round_trip() {
        // Clean slate regardless of any earlier aborted run.
        unregister().unwrap();
        assert!(!is_registered(), "should start unregistered");

        register().unwrap();
        assert!(is_registered(), "should be registered after register()");

        // The command actually stored must invoke this exe with the flag.
        let cmd = registered_command().expect("command must exist");
        assert!(cmd.contains(FORCE_DELETE_FLAG));
        assert!(cmd.contains(r#""%1""#));

        unregister().unwrap();
        assert!(!is_registered(), "should be unregistered after unregister()");
        assert!(registered_command().is_none(), "keys must be gone");
    }
}
