use crate::apps::dir_size;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use sysinfo::System;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub install_location: Option<String>,
    pub estimated_bytes: u64,
    pub uninstall_string: Option<String>,
    /// Registry `DisplayIcon` — where the app's logo lives. May carry a
    /// trailing `,<index>`; see `thumbs::app_icon_source`.
    pub display_icon: Option<String>,
}

/// Exposes command-line splitting to sibling modules (icon resolution needs
/// the program out of an uninstall string).
pub fn split_command_line_public(s: &str) -> (String, String) {
    split_command_line(&expand_env_vars(s))
}

/// Exposes MSI product-code extraction (icon resolution falls back to the
/// Windows Installer database for MSI packages).
pub fn msi_product_code_public(s: &str) -> Option<String> {
    msi_product_code(&expand_env_vars(s))
}

/// Exposes `%VAR%` expansion; MSI properties often contain `%APPDATA%`.
pub fn expand_env_vars_public(s: &str) -> String {
    expand_env_vars(s)
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Leftover {
    pub path: String,
    pub size_bytes: u64,
}

const UNINSTALL_PATHS: &[(&str, &str)] = &[
    ("HKLM", "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ("HKLM", "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ("HKCU", "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
];

/// Installed applications from the Windows registry — the same database
/// Settings → Apps uses. Store (UWP) apps are not included.
pub fn list_installed() -> Vec<InstalledApp> {
    let mut by_name: HashMap<String, InstalledApp> = HashMap::new();

    for (hive, path) in UNINSTALL_PATHS {
        let root = match *hive {
            "HKLM" => RegKey::predef(HKEY_LOCAL_MACHINE),
            _ => RegKey::predef(HKEY_CURRENT_USER),
        };
        let Ok(key) = root.open_subkey(path) else {
            continue;
        };
        for sub_name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey(&sub_name) else {
                continue;
            };
            let Ok(name) = sub.get_value::<String, _>("DisplayName") else {
                continue;
            };
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            // Filter system components, patches and updates.
            if sub.get_value::<u32, _>("SystemComponent").unwrap_or(0) == 1 {
                continue;
            }
            if sub.get_value::<String, _>("ParentKeyName").is_ok() {
                continue;
            }
            let release_type = sub.get_value::<String, _>("ReleaseType").unwrap_or_default();
            if release_type.contains("Update") || name.starts_with("Security Update") || name.starts_with("Update for") {
                continue;
            }

            let app = InstalledApp {
                version: sub.get_value("DisplayVersion").unwrap_or_default(),
                publisher: sub.get_value("Publisher").unwrap_or_default(),
                install_location: sub
                    .get_value::<String, _>("InstallLocation")
                    .ok()
                    .filter(|s| !s.trim().is_empty()),
                estimated_bytes: sub.get_value::<u32, _>("EstimatedSize").unwrap_or(0) as u64 * 1024,
                uninstall_string: sub
                    .get_value::<String, _>("UninstallString")
                    .ok()
                    .filter(|s| !s.trim().is_empty()),
                display_icon: sub
                    .get_value::<String, _>("DisplayIcon")
                    .ok()
                    .filter(|s| !s.trim().is_empty()),
                name: name.clone(),
            };

            match by_name.get(&name) {
                Some(existing)
                    if existing.uninstall_string.is_some()
                        && existing.estimated_bytes >= app.estimated_bytes => {}
                _ => {
                    by_name.insert(name, app);
                }
            }
        }
    }

    let mut apps: Vec<InstalledApp> = by_name.into_values().collect();
    apps.sort_by(|a, b| b.estimated_bytes.cmp(&a.estimated_bytes).then(a.name.cmp(&b.name)));
    apps
}

/// Launches the application's own uninstaller (shows a UAC prompt when the
/// uninstaller requires administrator rights).
///
/// This goes through `ShellExecuteEx`, not `CreateProcess` (what
/// `std::process::Command` uses). Nearly every machine-wide uninstaller
/// (Inno Setup's `unins000.exe`, NSIS' `uninstall.exe`, InstallShield…) is
/// manifested `requireAdministrator`, and `CreateProcess` refuses to start
/// those — it fails with ERROR_ELEVATION_REQUIRED (os error 740) instead of
/// prompting. `ShellExecuteEx` performs the elevation handshake, so the UAC
/// dialog appears and the uninstaller actually runs.
///
/// The registry `UninstallString` is a raw command line. Arguments are passed
/// through verbatim rather than re-quoted per token, which matters for entries
/// like NVIDIA's `RunDll32.EXE "…\NVI2.DLL",UninstallPackage Display.Driver`.
pub fn launch_uninstaller(uninstall_string: &str) -> Result<(), String> {
    let expanded = expand_env_vars(uninstall_string.trim());
    let s = expanded.trim();
    if s.is_empty() {
        return Err("This application does not provide an uninstaller command.".into());
    }

    // MSI: run msiexec with the uninstall flag directly, regardless of whether
    // the stored string used /I (modify) or /X (uninstall).
    if let Some(code) = msi_product_code(s) {
        return shell_execute("msiexec.exe", &format!("/x {code}"));
    }

    let (program, args) = split_command_line(s);
    if program.is_empty() {
        return Err("Could not parse the uninstaller command.".into());
    }
    // A rooted path that is not on disk means the registry entry is stale —
    // report the whole command so the user can see what Windows recorded.
    let rooted = program.contains(":\\") || program.starts_with("\\\\");
    if rooted && !std::path::Path::new(&program).is_file() {
        return Err(format!(
            "The uninstaller is no longer on disk. Windows still lists this command:\n{s}\n\n\
             Remove it from Settings → Apps instead."
        ));
    }
    shell_execute(&program, &args)
}

/// Runs a program through the Windows shell so that manifest-declared
/// elevation is honored (UAC prompt) instead of failing outright.
fn shell_execute(program: &str, args: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let file = wide(program);
    let params = wide(args);
    // The working directory matters for uninstallers that look for sibling
    // data files; use the program's own folder when we know it.
    let directory = std::path::Path::new(program)
        .parent()
        .filter(|p| p.is_dir())
        .map(|p| wide(&p.to_string_lossy()));

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    // NOASYNC is required because this runs on a worker thread with no message
    // loop — without it the call can return before the shell has finished.
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpFile = file.as_ptr();
    info.lpParameters = if args.is_empty() {
        std::ptr::null()
    } else {
        params.as_ptr()
    };
    info.lpDirectory = directory
        .as_ref()
        .map(|d| d.as_ptr())
        .unwrap_or(std::ptr::null());
    info.nShow = SW_SHOWNORMAL;

    let started = unsafe { ShellExecuteExW(&mut info) };
    if started == 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            return Err(
                "Administrator permission was declined, so the uninstaller did not start.".into(),
            );
        }
        return Err(format!("Windows could not start the uninstaller: {err}"));
    }
    if !info.hProcess.is_null() {
        unsafe { CloseHandle(info.hProcess) };
    }
    Ok(())
}

/// Expands `%VAR%` references, which some uninstall strings still use.
fn expand_env_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                out.push_str(&rest[..start]);
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => out.push_str(&value),
                    // Not a variable (e.g. a literal `%` in a path) — keep it.
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Extracts the `{GUID}` product code from an MsiExec uninstall string.
fn msi_product_code(s: &str) -> Option<String> {
    // Only the *program* may be msiexec — bundle uninstallers such as
    // `"…\Package Cache\{GUID}\VC_redist.x64.exe" /uninstall` also contain a
    // GUID and must not be rerouted through msiexec.
    let head = s.split_whitespace().next()?.trim_matches('"').to_lowercase();
    if !(head == "msiexec" || head == "msiexec.exe" || head.ends_with("\\msiexec.exe")) {
        return None;
    }
    let start = s.find('{')?;
    let end = s[start..].find('}')? + start;
    Some(s[start..=end].to_string())
}

/// Splits a Windows command line into (program, raw argument string).
///
/// Registry uninstall strings come in every shape: quoted paths, *unquoted*
/// paths containing spaces (`C:\Program Files\Android\Android Studio\uninstall.exe`),
/// and bare commands resolved through PATH (`CMD /C "…\Doc_Uninstall.cmd"`).
/// Splitting on the first `.exe` or on whitespace alone gets each of those
/// wrong, so candidate prefixes are checked against the filesystem.
fn split_command_line(s: &str) -> (String, String) {
    let s = s.trim();

    // 1. Quoted program.
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(i) = rest.find('"') {
            return (rest[..i].to_string(), rest[i + 1..].trim().to_string());
        }
    }

    // 2. The entire string is the program — unquoted path with spaces, no args.
    if std::path::Path::new(s).is_file() {
        return (s.to_string(), String::new());
    }

    // 3. Longest prefix ending at an executable extension that exists on disk.
    let lower = s.to_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".com"] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(ext) {
            let end = from + pos + ext.len();
            if std::path::Path::new(&s[..end]).is_file() {
                return (s[..end].to_string(), s[end..].trim().to_string());
            }
            from = end;
        }
    }

    // 4. First whitespace-delimited token; the shell resolves it against PATH.
    match s.split_once(char::is_whitespace) {
        Some((program, args)) => (program.to_string(), args.trim().to_string()),
        None => (s.to_string(), String::new()),
    }
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// App name without trailing version-like tokens ("Foo 2.3.1 (x64)" → "Foo").
fn base_name(name: &str) -> String {
    name.split_whitespace()
        .filter(|token| {
            let t = token.trim_matches(|c| c == '(' || c == ')');
            !(t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == 'v' || c == 'V')
                && t.chars().any(|c| c.is_ascii_digit()))
                && !matches!(t.to_lowercase().as_str(), "x64" | "x86" | "64-bit" | "32-bit")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Normalized name forms an app can plausibly appear under on disk: the full
/// name, the name without version/arch suffixes ("Foo 2.3.1 (x64)" → "foo"),
/// and — for a name like "Google Chrome" published by Google — the name with
/// the publisher prefix dropped, since installers commonly use just "Chrome"
/// for the folder or shortcut. Shared by leftover-folder, shortcut and
/// process matching so all three agree on what counts as "this app".
fn name_variants(name: &str, publisher: &str) -> Vec<String> {
    let base = base_name(name);
    let mut variants: Vec<String> = vec![norm(name), norm(&base)];
    let name_tokens: Vec<&str> = base.split_whitespace().collect();
    let publisher_first = publisher.split_whitespace().next().map(norm).unwrap_or_default();
    // "Google Chrome" by Google → also try "Chrome".
    if name_tokens.len() >= 2 && !publisher_first.is_empty() && norm(name_tokens[0]) == publisher_first {
        variants.push(norm(&name_tokens[1..].join(" ")));
    }
    variants.retain(|v| v.len() >= 3);
    variants.dedup();
    variants
}

/// Conservative search for folders an application leaves behind. Matches
/// exact app-name folders, and `<Publisher>\<App>` two-level folders — never
/// a bare publisher folder (it may hold other apps' data).
pub fn find_leftovers(
    name: &str,
    publisher: &str,
    install_location: Option<&str>,
) -> Vec<Leftover> {
    let variants = name_variants(name, publisher);
    let base = base_name(name);
    let name_tokens: Vec<&str> = base.split_whitespace().collect();
    let publisher_first = publisher.split_whitespace().next().map(norm).unwrap_or_default();

    let single_roots = [
        env_path("LOCALAPPDATA"),
        env_path("APPDATA"),
        env_path("LOCALAPPDATA").map(|p| p.join("Programs")),
        env_path("ProgramData"),
        env_path("ProgramFiles"),
        env_path("ProgramFiles(x86)"),
    ];
    let publisher_norms: Vec<String> = [norm(publisher), publisher_first.clone()]
        .into_iter()
        .filter(|s| s.len() >= 3)
        .collect();

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(location) = install_location {
        let p = PathBuf::from(location);
        if p.exists() {
            candidates.push(p);
        }
    }

    for root in single_roots.iter().flatten() {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let child_name = norm(&entry.file_name().to_string_lossy());
            if variants.iter().any(|v| *v == child_name) {
                candidates.push(entry.path());
            } else if publisher_norms.iter().any(|p| *p == child_name) {
                // Publisher folder: only match app-named subfolders inside it.
                if let Ok(inner) = std::fs::read_dir(entry.path()) {
                    for sub in inner.flatten() {
                        if !sub.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            continue;
                        }
                        let sub_name = norm(&sub.file_name().to_string_lossy());
                        let last_token = name_tokens.last().map(|t| norm(t)).unwrap_or_default();
                        if variants.iter().any(|v| *v == sub_name)
                            || (last_token.len() >= 3 && sub_name == last_token)
                        {
                            candidates.push(sub.path());
                        }
                    }
                }
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    // Drop paths nested inside another candidate.
    let roots: Vec<PathBuf> = candidates
        .iter()
        .filter(|c| !candidates.iter().any(|other| *c != other && c.starts_with(other)))
        .cloned()
        .collect();

    let mut leftovers: Vec<Leftover> = roots
        .par_iter()
        .map(|path| Leftover {
            path: path.to_string_lossy().into_owned(),
            size_bytes: dir_size(path),
        })
        .collect();
    leftovers.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    leftovers
}

// ---------------------------------------------------------------------------
// Force uninstall — for apps whose own uninstaller is missing, broken, or
// (Microsoft Edge being the textbook case) refuses to run through the usual
// path. Mirrors what a third-party tool like Revo Uninstaller does in its
// "forced" mode: stop whatever is running, then remove the program, its
// registry entry and its shortcuts directly — no longer waiting on the
// vendor's own uninstaller to cooperate.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ForceStep {
    pub label: String,
    pub ok: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ForceUninstallReport {
    pub steps: Vec<ForceStep>,
    /// No matching registry entry and (if it had one) no install folder left
    /// — the practical definition of "actually uninstalled now". Leftover
    /// data and shortcuts are cleaned up too but do not gate this.
    pub complete: bool,
}

/// Background processes to stop that a plain install-folder match would
/// miss — updaters and helpers that run from a separate location.
fn known_process_names(name: &str) -> &'static [&'static str] {
    let n = norm(name);
    if n.contains("microsoftedge") {
        &["msedge.exe", "msedgewebview2.exe", "identity_helper.exe", "MicrosoftEdgeUpdate.exe"]
    } else if n.contains("googlechrome") {
        &["chrome.exe", "GoogleUpdate.exe", "elevation_service.exe"]
    } else if n.contains("mozillafirefox") {
        &["firefox.exe"]
    } else {
        &[]
    }
}

/// True if any process matching this app is currently running: launched from
/// its install folder, or one of its known background helpers.
fn any_related_process_running(app: &InstalledApp) -> bool {
    let sys = System::new_all();
    let install_dir = install_dir_of(app);
    let known = known_process_names(&app.name);
    sys.processes().values().any(|process| {
        let name = process.name().to_string_lossy().to_string();
        let under_install_dir = install_dir
            .as_deref()
            .zip(process.exe())
            .map(|(dir, exe)| exe.starts_with(dir))
            .unwrap_or(false);
        under_install_dir || known.iter().any(|k| k.eq_ignore_ascii_case(&name))
    })
}

/// Stops every process matching this app — works without administrator
/// rights for processes the signed-in user owns, which covers ordinary
/// desktop apps even when installed machine-wide. Best effort: callers
/// re-check `any_related_process_running` rather than trusting this.
fn stop_related_processes(app: &InstalledApp) {
    let sys = System::new_all();
    let install_dir = install_dir_of(app);
    let known = known_process_names(&app.name);
    for process in sys.processes().values() {
        let name = process.name().to_string_lossy().to_string();
        let under_install_dir = install_dir
            .as_deref()
            .zip(process.exe())
            .map(|(dir, exe)| exe.starts_with(dir))
            .unwrap_or(false);
        if under_install_dir || known.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
            process.kill();
        }
    }
}

fn install_dir_of(app: &InstalledApp) -> Option<PathBuf> {
    app.install_location
        .as_deref()
        .map(|s| s.trim().trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Everything this app's continued presence can be checked against: its own
/// install folder plus whatever `find_leftovers` would still find.
fn remaining_paths(app: &InstalledApp) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = install_dir_of(app).into_iter().filter(|p| p.exists()).collect();
    paths.extend(
        find_leftovers(&app.name, &app.publisher, app.install_location.as_deref())
            .into_iter()
            .map(|l| PathBuf::from(l.path)),
    );
    paths.sort();
    paths.dedup();
    paths
}

/// Re-locates every registry Uninstall subkey whose DisplayName matches
/// exactly (a 32-bit and 64-bit entry can coexist under the same name), so
/// the app's own entry can be removed once nothing is left of it.
fn find_uninstall_key_paths(display_name: &str) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    for (hive, path) in UNINSTALL_PATHS {
        let root = match *hive {
            "HKLM" => RegKey::predef(HKEY_LOCAL_MACHINE),
            _ => RegKey::predef(HKEY_CURRENT_USER),
        };
        let Ok(key) = root.open_subkey(path) else {
            continue;
        };
        for sub_name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey(&sub_name) else {
                continue;
            };
            let Ok(name) = sub.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if name.trim() == display_name {
                found.push((*hive, format!("{path}\\{sub_name}")));
            }
        }
    }
    found
}

/// Start Menu (current user and all users) and Desktop (current user and
/// Public) — every place Windows itself puts a shortcut for an installed app.
fn shortcut_dirs() -> Vec<PathBuf> {
    [
        env_path("ProgramData").map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs")),
        env_path("APPDATA").map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs")),
        env_path("Public").map(|p| p.join("Desktop")),
        env_path("USERPROFILE").map(|p| p.join("Desktop")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn walk_shortcuts(dir: &Path, variants: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk_shortcuts(&path, variants, out);
            continue;
        }
        let is_shortcut = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url"))
            .unwrap_or(false);
        if !is_shortcut {
            continue;
        }
        let stem = norm(&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        // Exact match only: Explorer names a shortcut after the app's
        // DisplayName, and a fuzzy match here risks catching an unrelated
        // shortcut that merely contains the same short word.
        if variants.contains(&stem) {
            out.push(path);
        }
    }
}

/// Shortcuts (Start Menu, Desktop — current user and all users) matching this
/// app by filename. Reading a `.lnk`'s actual target needs COM
/// (`IShellLink`); a name match is what Explorer's own "Pin to Start"
/// already relies on, and is enough for something about to be uninstalled.
fn find_shortcuts(name: &str, publisher: &str) -> Vec<PathBuf> {
    let variants = name_variants(name, publisher);
    let mut out = Vec::new();
    for dir in shortcut_dirs() {
        walk_shortcuts(&dir, &variants, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn is_edge(name: &str) -> bool {
    norm(name) == "microsoftedge"
}

fn parse_version(s: &str) -> Option<Vec<u32>> {
    let parts: Result<Vec<u32>, _> = s.split('.').map(str::parse).collect();
    parts.ok().filter(|p: &Vec<u32>| !p.is_empty())
}

fn newest_edge_setup(app_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(app_dir).ok()?;
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(version) = parse_version(&name) else {
            continue;
        };
        let setup = entry.path().join("Installer").join("setup.exe");
        if !setup.is_file() {
            continue;
        }
        if best.as_ref().map(|(v, _)| version > *v).unwrap_or(true) {
            best = Some((version, setup));
        }
    }
    best.map(|(_, path)| path)
}

/// Locates Microsoft Edge's own uninstaller. Edge does not remove itself
/// through a plain `UninstallString` — it insists on `setup.exe --uninstall`,
/// run from inside its own versioned application folder, which is exactly
/// why "the uninstaller won't run" is the standard complaint about Edge.
/// Checked machine-wide first (the common case for Edge), then per-user.
/// Returns whether the install found is machine-wide (needs administrator
/// rights to remove).
fn edge_setup_exe() -> Option<(PathBuf, bool)> {
    for base in [env_path("ProgramFiles(x86)"), env_path("ProgramFiles")]
        .into_iter()
        .flatten()
    {
        if let Some(setup) = newest_edge_setup(&base.join(r"Microsoft\Edge\Application")) {
            return Some((setup, true));
        }
    }
    if let Some(base) = env_path("LOCALAPPDATA") {
        if let Some(setup) = newest_edge_setup(&base.join(r"Microsoft\Edge\Application")) {
            return Some((setup, false));
        }
    }
    None
}

/// Runs a program and waits for it to finish — used for the per-user Edge
/// uninstaller, which needs no elevation. Edge's own flags are all bare
/// tokens, so a naive whitespace split is safe here (unlike the general
/// uninstall-string parsing above, which has to handle quoted paths).
fn run_and_wait(program: &Path, args: &str) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args.split_whitespace())
        .status()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Sends a file or folder to the Recycle Bin — a forced uninstall is still
/// not a reason to make a mistake unrecoverable, matching how every other
/// deletion in this app behaves.
fn trash_path(path: &Path) {
    let _ = trash::delete(path);
}

/// Builds the step report by checking real, current state — never by
/// trusting what a removal attempt claimed to do. `attempted_edge` controls
/// whether the Edge-specific step is shown at all (only relevant when this
/// pass actually tried it).
pub fn verify_force_uninstall(app: &InstalledApp, attempted_edge: bool) -> ForceUninstallReport {
    let mut steps = vec![ForceStep {
        label: "Closed running processes".into(),
        ok: !any_related_process_running(app),
    }];
    if attempted_edge {
        steps.push(ForceStep {
            label: "Ran Microsoft Edge's own uninstaller".into(),
            ok: edge_setup_exe().is_none(),
        });
    }
    let remaining = remaining_paths(app);
    steps.push(ForceStep {
        label: "Removed the program's files".into(),
        ok: remaining.is_empty(),
    });
    let registry_gone = find_uninstall_key_paths(&app.name).is_empty();
    steps.push(ForceStep {
        label: "Removed it from Installed Apps".into(),
        ok: registry_gone,
    });
    steps.push(ForceStep {
        label: "Removed shortcuts".into(),
        ok: find_shortcuts(&app.name, &app.publisher).is_empty(),
    });

    let complete = registry_gone && install_dir_of(app).map(|p| !p.exists()).unwrap_or(true);
    ForceUninstallReport { steps, complete }
}

/// Best-effort force removal without administrator rights: stops related
/// processes, removes the install folder and any leftovers, the registry
/// entry and shortcuts. Succeeds outright for per-user installs; whatever a
/// machine-wide install still needs elevation for is left for
/// `build_force_uninstall_script` to finish.
pub fn force_uninstall(app: &InstalledApp) -> ForceUninstallReport {
    stop_related_processes(app);
    // Give processes a moment to actually exit before touching their files.
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Edge's per-user install needs no elevation; the machine-wide one is
    // left for the elevated pass rather than prompting for UAC twice.
    let mut attempted_edge = false;
    if is_edge(&app.name) {
        if let Some((setup, system_level)) = edge_setup_exe() {
            if !system_level {
                attempted_edge = true;
                let _ = run_and_wait(&setup, "--uninstall --force-uninstall --verbose-logging");
            }
        }
    }

    for path in remaining_paths(app) {
        trash_path(&path);
    }
    for (hive, path) in find_uninstall_key_paths(&app.name) {
        let root = match hive {
            "HKLM" => RegKey::predef(HKEY_LOCAL_MACHINE),
            _ => RegKey::predef(HKEY_CURRENT_USER),
        };
        let _ = root.delete_subkey_all(&path);
    }
    for shortcut in find_shortcuts(&app.name, &app.publisher) {
        trash_path(&shortcut);
    }

    verify_force_uninstall(app, attempted_edge)
}

/// One item's removal, sent to the Recycle Bin like every other deletion in
/// this app rather than wiped outright.
fn recycle_item_script(path: &Path) -> String {
    let esc = path.to_string_lossy().replace('\'', "''");
    format!(
        "try {{ if (Test-Path -LiteralPath '{esc}' -PathType Container) {{ \
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('{esc}','OnlyErrorDialogs','SendToRecycleBin') \
         }} elseif (Test-Path -LiteralPath '{esc}') {{ \
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{esc}','OnlyErrorDialogs','SendToRecycleBin') \
         }} }} catch {{ }}\n"
    )
}

/// Assembles the PowerShell script that finishes whatever `force_uninstall`
/// could not do without administrator rights: stop processes again
/// (defensive — a process owned by another session may have survived the
/// first pass), run Edge's machine-wide uninstaller if this is Edge, then
/// remove the install folder, leftovers, registry entry and shortcuts.
/// Returns the script together with whether it attempts the Edge step, so
/// the caller's step report can match. Pure — runs no privileged code
/// itself; `commands::run_elevated_powershell` executes what this builds.
pub fn build_force_uninstall_script(app: &InstalledApp) -> (String, bool) {
    let mut script = String::from("Add-Type -AssemblyName Microsoft.VisualBasic\n");

    for name in known_process_names(&app.name) {
        let base = name.trim_end_matches(".exe").replace('\'', "''");
        script.push_str(&format!(
            "Stop-Process -Name '{base}' -Force -ErrorAction SilentlyContinue\n"
        ));
    }
    if let Some(dir) = install_dir_of(app) {
        let esc = dir.to_string_lossy().replace('\'', "''");
        script.push_str(&format!(
            "Get-Process | Where-Object {{ $_.Path -like '{esc}\\*' }} | Stop-Process -Force -ErrorAction SilentlyContinue\n"
        ));
    }
    script.push_str("Start-Sleep -Milliseconds 500\n");

    let mut attempted_edge = false;
    if is_edge(&app.name) {
        if let Some((setup, _)) = edge_setup_exe() {
            attempted_edge = true;
            let esc = setup.to_string_lossy().replace('\'', "''");
            script.push_str(&format!(
                "try {{ Start-Process -FilePath '{esc}' \
                 -ArgumentList '--uninstall','--system-level','--verbose-logging','--force-uninstall' \
                 -Wait -ErrorAction SilentlyContinue }} catch {{ }}\n"
            ));
        }
    }

    for path in remaining_paths(app) {
        script.push_str(&recycle_item_script(&path));
    }
    for (hive, path) in find_uninstall_key_paths(&app.name) {
        let root = match hive {
            "HKLM" => "HKEY_LOCAL_MACHINE",
            _ => "HKEY_CURRENT_USER",
        };
        let esc = path.replace('\'', "''");
        script.push_str(&format!(
            "Remove-Item -Path 'Registry::{root}\\{esc}' -Recurse -Force -ErrorAction SilentlyContinue\n"
        ));
    }
    for shortcut in find_shortcuts(&app.name, &app.publisher) {
        script.push_str(&recycle_item_script(&shortcut));
    }

    (script, attempted_edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_variants_drops_publisher_prefix_and_short_tokens() {
        let variants = name_variants("Google Chrome", "Google LLC");
        assert!(variants.contains(&"googlechrome".to_string()));
        assert!(variants.contains(&"chrome".to_string()));
    }

    #[test]
    fn is_edge_matches_only_the_exact_display_name() {
        assert!(is_edge("Microsoft Edge"));
        // A near-miss must not be treated as Edge itself — running Edge's
        // machine-wide force-uninstall command against the wrong product
        // would be a serious mistake.
        assert!(!is_edge("Microsoft Edge WebView2 Runtime"));
        assert!(!is_edge("Notepad++"));
    }

    #[test]
    fn parse_version_reads_dotted_numeric_folders() {
        assert_eq!(parse_version("124.0.2478.51"), Some(vec![124, 0, 2478, 51]));
        assert_eq!(parse_version("Installer"), None);
        assert_eq!(parse_version(""), None);
    }

    /// The highest-versioned `Installer\setup.exe` must win, since that is
    /// the one still valid to run — Edge leaves old version folders behind
    /// after updating itself.
    #[test]
    fn newer_edge_version_folder_wins() {
        let dir = std::env::temp_dir().join("storage doctor edge version test");
        let _ = std::fs::remove_dir_all(&dir);
        for version in ["120.0.100.1", "124.0.2478.51", "119.9.9.9"] {
            let installer = dir.join(version).join("Installer");
            std::fs::create_dir_all(&installer).unwrap();
            std::fs::write(installer.join("setup.exe"), b"").unwrap();
        }

        let found = newest_edge_setup(&dir).expect("should find a setup.exe");
        assert!(found.to_string_lossy().contains("124.0.2478.51"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The force-uninstall script must never target Edge's system-level
    /// uninstaller for an app that merely has "edge" somewhere in its name —
    /// only an exact match should trigger it.
    #[test]
    fn force_uninstall_script_only_runs_edges_own_uninstaller_for_edge_itself() {
        let not_edge = InstalledApp {
            name: "Notepad++".to_string(),
            version: "8.6".to_string(),
            publisher: "Don Ho".to_string(),
            install_location: None,
            estimated_bytes: 0,
            uninstall_string: None,
            display_icon: None,
        };
        let (script, attempted_edge) = build_force_uninstall_script(&not_edge);
        assert!(!attempted_edge);
        assert!(!script.contains("--system-level"));
    }

    // Shapes taken verbatim from real registry UninstallString values.

    #[test]
    fn quoted_program_keeps_arguments_verbatim() {
        let (program, args) = split_command_line(
            r#""C:\windows\SysWOW64\RunDll32.EXE" "C:\Program Files\NVIDIA Corporation\Installer2\InstallerCore\NVI2.DLL",UninstallPackage Display.Driver"#,
        );
        assert_eq!(program, r"C:\windows\SysWOW64\RunDll32.EXE");
        // Quoting inside the argument string must survive — splitting on
        // whitespace here is what broke the NVIDIA uninstallers.
        assert_eq!(
            args,
            r#""C:\Program Files\NVIDIA Corporation\Installer2\InstallerCore\NVI2.DLL",UninstallPackage Display.Driver"#
        );
    }

    #[test]
    fn quoted_program_without_arguments() {
        let (program, args) = split_command_line(r#""C:\Program Files\Audacity\unins000.exe""#);
        assert_eq!(program, r"C:\Program Files\Audacity\unins000.exe");
        assert_eq!(args, "");
    }

    #[test]
    fn bare_command_falls_back_to_first_token() {
        let (program, args) =
            split_command_line(r#"CMD /C "C:\Program Files\HP\Documentation\Doc_Uninstall.cmd""#);
        assert_eq!(program, "CMD");
        assert_eq!(args, r#"/C "C:\Program Files\HP\Documentation\Doc_Uninstall.cmd""#);
    }

    #[test]
    fn unquoted_path_with_spaces_resolves_against_disk() {
        // The whole string is the program only when it exists on disk;
        // otherwise the first token is used. Build the case with a real file.
        let dir = std::env::temp_dir().join("storage doctor test dir");
        let exe = dir.join("uninstall.exe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&exe, b"").unwrap();

        let full = exe.to_string_lossy().into_owned();
        let (program, args) = split_command_line(&full);
        assert_eq!(program, full);
        assert_eq!(args, "");

        let (program, args) = split_command_line(&format!("{full} /S"));
        assert_eq!(program, full);
        assert_eq!(args, "/S");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn msi_product_code_only_matches_msiexec_programs() {
        assert_eq!(
            msi_product_code("MsiExec.exe /I{20C01991-CCD1-2C06-7A9A-B10A9B4AF807}").as_deref(),
            Some("{20C01991-CCD1-2C06-7A9A-B10A9B4AF807}")
        );
        assert_eq!(
            msi_product_code(
                r"msiexec.exe /x {AF599C42-A2E5-4251-B7EE-49251227A340} /L*V C:\Temp\hss.log"
            )
            .as_deref(),
            Some("{AF599C42-A2E5-4251-B7EE-49251227A340}")
        );
        // A bundle uninstaller whose *path* contains a GUID is not an MSI.
        assert_eq!(
            msi_product_code(
                r#""C:\ProgramData\Package Cache\{d8bbe9f9-7c5b-42c6-b715-9ee898a2e515}\VC_redist.x64.exe"  /uninstall"#
            ),
            None
        );
    }

    /// End-to-end: an unquoted path containing spaces plus an argument, run
    /// through the real `ShellExecuteEx` call. The target writes a marker file
    /// so we can confirm it actually started with the argument intact.
    #[test]
    fn launches_a_command_with_spaces_in_its_path() {
        let dir = std::env::temp_dir().join("storage doctor launch test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("uninstall.cmd");
        let marker = dir.join("ran.txt");
        std::fs::write(&script, "@echo off\r\necho %1> \"%~dp0ran.txt\"\r\n").unwrap();

        let command = format!("{} MARKER-ARG", script.to_string_lossy());
        launch_uninstaller(&command).expect("uninstaller should launch");

        // The shell starts the process asynchronously; give it a moment.
        let mut contents = String::new();
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&marker) {
                contents = text;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            contents.contains("MARKER-ARG"),
            "target did not run with its argument; marker contained {contents:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_uninstaller_reports_the_stale_command() {
        let err = launch_uninstaller(r"C:\No Such Dir\uninstall.exe /S").unwrap_err();
        assert!(err.contains("no longer on disk"), "unexpected error: {err}");
    }

    /// Diagnostic: resolves every uninstall string in this machine's registry
    /// and reports which ones the parser cannot point at a real program.
    /// Run with `cargo test -- --ignored --nocapture audit`.
    #[test]
    #[ignore]
    fn audit_every_installed_app() {
        let apps = list_installed();
        let mut msi = 0;
        let mut resolved = 0;
        let mut via_path = 0;
        let mut unresolved: Vec<(String, String)> = Vec::new();

        for app in &apps {
            let Some(raw) = app.uninstall_string.as_deref() else {
                continue;
            };
            let expanded = expand_env_vars(raw.trim());
            let s = expanded.trim();
            if msi_product_code(s).is_some() {
                msi += 1;
                continue;
            }
            let (program, _) = split_command_line(s);
            if std::path::Path::new(&program).is_file() {
                resolved += 1;
            } else if !(program.contains(":\\") || program.starts_with("\\\\")) {
                // Bare command such as `CMD` — the shell resolves it via PATH.
                via_path += 1;
            } else {
                unresolved.push((app.name.clone(), raw.to_string()));
            }
        }

        println!("\n=== uninstall string audit ===");
        println!("total apps with an uninstall string: {}", msi + resolved + via_path + unresolved.len());
        println!("  msi (msiexec /x GUID):   {msi}");
        println!("  resolved to a real file: {resolved}");
        println!("  bare command via PATH:   {via_path}");
        println!("  UNRESOLVED:              {}", unresolved.len());
        for (name, raw) in &unresolved {
            println!("    - {name}\n        {raw}");
        }
    }

    #[test]
    fn env_vars_are_expanded_and_stray_percents_kept() {
        std::env::set_var("STORAGE_DOCTOR_TEST_VAR", r"C:\Apps");
        assert_eq!(
            expand_env_vars(r"%STORAGE_DOCTOR_TEST_VAR%\uninstall.exe"),
            r"C:\Apps\uninstall.exe"
        );
        assert_eq!(expand_env_vars("50% off"), "50% off");
        assert_eq!(
            expand_env_vars("%NOT_A_REAL_VAR_12345%\\x"),
            "%NOT_A_REAL_VAR_12345%\\x"
        );
    }
}
