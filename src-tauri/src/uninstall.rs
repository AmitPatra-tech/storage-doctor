use crate::apps::dir_size;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub install_location: Option<String>,
    pub estimated_bytes: u64,
    pub uninstall_string: Option<String>,
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

/// Launches the application's own uninstaller (may show a UAC prompt).
///
/// The registry `UninstallString` is a raw command line — often a quoted
/// path with arguments, or an `MsiExec.exe /I{GUID}` entry. We parse it into
/// program + args and spawn the program directly, so Windows argument
/// escaping does not mangle the quoted path (which previously caused the
/// uninstaller to silently fail to launch).
pub fn launch_uninstaller(uninstall_string: &str) -> Result<(), String> {
    let s = uninstall_string.trim();
    if s.is_empty() {
        return Err("This application does not provide an uninstaller command.".into());
    }

    // MSI: run msiexec with the uninstall flag directly, regardless of whether
    // the stored string used /I (modify) or /X (uninstall).
    if let Some(code) = msi_product_code(s) {
        std::process::Command::new("msiexec")
            .args(["/x", &code])
            .spawn()
            .map_err(|e| format!("Failed to launch msiexec: {e}"))?;
        return Ok(());
    }

    let (program, args) = split_command_line(s);
    if program.is_empty() {
        return Err("Could not parse the uninstaller command.".into());
    }
    if !std::path::Path::new(&program).exists() {
        return Err(format!("Uninstaller not found at {program}"));
    }
    std::process::Command::new(&program)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to launch uninstaller: {e}"))?;
    Ok(())
}

/// Extracts the `{GUID}` product code from an MsiExec uninstall string.
fn msi_product_code(s: &str) -> Option<String> {
    if !s.to_lowercase().contains("msiexec") {
        return None;
    }
    let start = s.find('{')?;
    let end = s[start..].find('}')? + start;
    Some(s[start..=end].to_string())
}

/// Splits a Windows command line into (program, args), honoring a leading
/// quoted path and falling back to splitting after the first `.exe`.
fn split_command_line(s: &str) -> (String, Vec<String>) {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(i) = rest.find('"') {
            let program = rest[..i].to_string();
            let args = rest[i + 1..]
                .split_whitespace()
                .map(String::from)
                .collect();
            return (program, args);
        }
    }
    let lower = s.to_lowercase();
    if let Some(pos) = lower.find(".exe") {
        let split_at = pos + 4;
        let program = s[..split_at].to_string();
        let args = s[split_at..].split_whitespace().map(String::from).collect();
        return (program, args);
    }
    (s.to_string(), Vec::new())
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

/// Conservative search for folders an application leaves behind. Matches
/// exact app-name folders, and `<Publisher>\<App>` two-level folders — never
/// a bare publisher folder (it may hold other apps' data).
pub fn find_leftovers(
    name: &str,
    publisher: &str,
    install_location: Option<&str>,
) -> Vec<Leftover> {
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
