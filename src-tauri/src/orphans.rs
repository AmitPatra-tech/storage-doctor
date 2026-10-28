//! Leftovers of apps that are no longer installed: top-level folders in the
//! usual app-data and program locations that nothing installed owns.
//!
//! Deliberately conservative — a folder that belongs to an installed app must
//! never be offered for deletion. A folder is listed only when ALL hold:
//!   * no installed program or Store app matches it (name, publisher, any
//!     word of the name, or its install path);
//!   * no running process is using a program inside it;
//!   * it is not a known Windows, driver or shared folder.
//! Anything that could plausibly match is kept out.

use crate::uninstall::{name_variants, norm};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Orphan {
    pub path: String,
    pub size_bytes: u64,
    /// Seconds since the Unix epoch anything inside the folder last changed
    /// (0 = unknown). Recent means it may still be in use.
    pub modified_unix: i64,
    /// The folder's name as it appears on disk.
    pub name: String,
    /// Where it was found, e.g. "AppData (Roaming)".
    pub location: String,
}

/// Folders that are Windows', a driver's, or shared between many apps —
/// never an app's leftovers. Compared after `norm` (lowercase alphanumerics).
const NEVER: &[&str] = &[
    "microsoft", "windows", "packages", "temp", "tmp", "programs", "commonfiles",
    "windowsapps", "modifiablewindowsapps", "windowsdefender", "windowsdefenderadvancedthreatprotection",
    "windowsnt", "windowsmail", "windowsmediaplayer", "windowsphotoviewer", "windowsportabledevices",
    "windowssidebar", "windowspowershell", "windowsmultimediaplatform", "internetexplorer",
    "referenceassemblies", "msbuild", "dotnet", "packagemanagement", "uninstallinformation",
    "packagecache", "usoshared", "usoprivate", "softwaredistribution", "applicationdata",
    "desktop", "documents", "startmenu", "templates", "history", "crashdumps", "d3dscache",
    "connecteddevicesplatform", "comms", "microsoftcorporation", "microsoftedge", "edge",
    "intel", "nvidia", "nvidiacorporation", "amd", "ati", "realtek", "hp", "hpinc", "dell",
    "lenovo", "asus", "acer", "msi", "synaptics", "elevatedagent", "regid19910710commicrosoft",
    "ssh", "git", "npm", "npmcache", "pip", "nodejs", "python", "java", "oracle", "ruby",
    "gradle", "nuget", "sun", "adobe", "google", "mozilla", "apple", "vmware", "virtualbox",
    "oem", "drivers", "driverstore", "license", "licenses", "sharedfiles", "fonts", "icons",
    "logs", "log", "cache", "caches", "data", "default", "public", "users", "system",
    "system32", "syswow64", "inetpub", "perflogs", "recovery", "boot", "esd", "fontcache",
    "deliveryoptimization", "identitycrl", "mpsigstub", "ntuser", "regid", "storagedoctor",
    "ms", "mspc", "systemcertificates", "crypto", "protect", "vault", "credentials",
    // Developer kits and tool data: used by tools that never register as
    // installed programs (deleting them breaks builds).
    "windowskits", "windowsappcertificationkit", "android", "androidsdk", "msplaywright",
    "msplaywrightgo", "tauri", "aws", "azure", "dart", "pub", "flutter", "rustup", "cargo", "go",
    "docker", "jetbrains", "dotslash", "unrealengine", "epicgames", "xdgconfig", "xdgdata",
    "sentry", "comgr", "squirreltemp", "electron", "electronbuilder", "yarn", "pnpm", "bun",
    // Windows feature and shell folders.
    "speech", "publishers", "placeholdertilelogofolder", "toastnotificationmanagercompat",
    "backup", "propagation", "copilot", "hewlettpackard", "microsoftonedrive", "onedrive",
];

fn is_never(name_norm: &str) -> bool {
    NEVER.contains(&name_norm) || name_norm.len() < 3
}

/// Everything an installed thing could be known by, normalized.
struct Owners {
    exact: HashSet<String>,
    /// Longer words/names a folder may contain or be contained in.
    words: HashSet<String>,
    install_paths: Vec<String>,
}

fn owners() -> Owners {
    let mut exact = HashSet::new();
    let mut words = HashSet::new();
    let mut install_paths = Vec::new();
    let add_text = |text: &str, exact: &mut HashSet<String>, words: &mut HashSet<String>| {
        let n = norm(text);
        if n.len() >= 3 {
            exact.insert(n);
        }
        for w in text.split(|c: char| !c.is_ascii_alphanumeric()) {
            let w = norm(w);
            if w.len() >= 4 {
                words.insert(w);
            }
        }
    };

    for app in crate::uninstall::list_installed() {
        for v in name_variants(&app.name, &app.publisher) {
            exact.insert(v);
        }
        add_text(&app.name, &mut exact, &mut words);
        add_text(&app.publisher, &mut exact, &mut words);
        for loc in [app.install_location.as_deref(), app.display_icon.as_deref(), app.uninstall_string.as_deref()]
            .into_iter()
            .flatten()
        {
            install_paths.push(loc.trim_matches('"').to_lowercase());
        }
    }
    if let Ok(store) = crate::store_apps::list() {
        for app in store {
            add_text(&app.display_name, &mut exact, &mut words);
            add_text(&app.name, &mut exact, &mut words);
            add_text(&app.publisher, &mut exact, &mut words);
        }
    }
    // Generic words that would make everything match.
    for generic in ["microsoft", "windows", "corporation", "software", "update", "setup", "program",
        "files", "system", "data", "tools", "service", "services", "runtime", "driver", "x64", "x86",
        "limited", "company", "studio", "player", "manager", "client", "edition", "version", "inc"]
    {
        words.remove(generic);
    }
    Owners { exact, words, install_paths }
}

fn owned(folder: &Path, owners: &Owners) -> bool {
    let name = folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let n = norm(&name);
    if owners.exact.contains(&n) {
        return true;
    }
    // A word of an installed app's name inside the folder name, or the folder
    // name inside an installed app's name ("Zoom" vs "ZoomWorkplace").
    if owners.words.iter().any(|w| n.contains(w.as_str()))
        || (n.len() >= 4 && owners.exact.iter().any(|e| e.contains(n.as_str())))
        // Short names ("vlc"): an installed name starting with it ("vlcmediaplayer").
        || owners.exact.iter().any(|e| e.starts_with(n.as_str()))
    {
        return true;
    }
    // Any installed program's recorded path lies in (or is) this folder.
    let lower = format!("{}\\", folder.to_string_lossy().to_lowercase());
    owners
        .install_paths
        .iter()
        .any(|p| p.starts_with(&lower) || format!("{p}\\").starts_with(&lower))
}

/// Folders containing a running program — certainly in use.
fn in_use_dirs() -> Vec<String> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .filter_map(|p| p.exe().map(|e| e.to_string_lossy().to_lowercase()))
        .collect()
}

fn roots() -> Vec<(PathBuf, &'static str)> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    [
        (env("APPDATA"), "AppData (Roaming)"),
        (env("LOCALAPPDATA"), "AppData (Local)"),
        (env("LOCALAPPDATA").map(|p| p.join("Programs")), "AppData (Local)\\Programs"),
        (env("ProgramData"), "ProgramData"),
        (env("ProgramFiles"), "Program Files"),
        (env("ProgramFiles(x86)"), "Program Files (x86)"),
    ]
    .into_iter()
    .filter_map(|(p, label)| p.map(|p| (p, label)))
    .collect()
}

/// Total size and the newest modification time of anything inside `dir`,
/// without following junctions/symlinks.
fn size_and_newest(dir: &Path) -> (u64, i64) {
    fn secs(m: &std::fs::Metadata) -> i64 {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
    let mut size = 0u64;
    let mut newest = std::fs::metadata(dir).map(|m| secs(&m)).unwrap_or(0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if let Ok(m) = e.metadata() {
                newest = newest.max(secs(&m));
                if ft.is_dir() {
                    stack.push(e.path());
                } else {
                    size += m.len();
                }
            }
        }
    }
    (size, newest)
}

pub fn find() -> Vec<Orphan> {
    let owners = owners();
    let running = in_use_dirs();
    let mut candidates: Vec<(PathBuf, &'static str)> = Vec::new();
    for (root, label) in roots() {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            // Real directories only — never follow junctions/symlinks.
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.starts_with('$') || name.starts_with('{') {
                continue;
            }
            if is_never(&norm(&name)) || owned(&path, &owners) {
                continue;
            }
            let prefix = format!("{}\\", path.to_string_lossy().to_lowercase());
            if running.iter().any(|exe| exe.starts_with(&prefix)) {
                continue;
            }
            candidates.push((path, label));
        }
    }

    let mut found: Vec<Orphan> = candidates
        .par_iter()
        .map(|(path, label)| {
            let (size, modified_unix) = size_and_newest(path);
            Orphan {
                path: path.to_string_lossy().to_string(),
                size_bytes: size,
                modified_unix,
                name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                location: label.to_string(),
            }
        })
        // Empty folders are not worth anyone's attention.
        .filter(|o| o.size_bytes > 0)
        .collect();
    found.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owners_of(exact: &[&str], words: &[&str], paths: &[&str]) -> Owners {
        Owners {
            exact: exact.iter().map(|s| s.to_string()).collect(),
            words: words.iter().map(|s| s.to_string()).collect(),
            install_paths: paths.iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    #[test]
    fn installed_apps_own_their_folders_by_name_word_or_path() {
        let o = owners_of(&["googlechrome", "zoomworkplace"], &["chrome", "zoom", "slack"], &[r"C:\Program Files\Acme Thing\app.exe"]);
        assert!(owned(Path::new(r"C:\Users\me\AppData\Local\Chrome"), &o));
        assert!(owned(Path::new(r"C:\Users\me\AppData\Roaming\Zoom"), &o));
        assert!(owned(Path::new(r"C:\Users\me\AppData\Roaming\slack-desktop"), &o), "word inside folder name");
        assert!(owned(Path::new(r"C:\Program Files\Acme Thing"), &o), "install path inside folder");
        assert!(!owned(Path::new(r"C:\Users\me\AppData\Roaming\OldGoneApp"), &o));
        let vlc = owners_of(&["vlcmediaplayer", "videolan"], &["media", "player"], &[]);
        assert!(owned(Path::new(r"C:\Users\me\AppData\Roaming\vlc"), &vlc), "short name of an installed app");
    }

    #[test]
    fn windows_and_shared_folders_are_never_candidates() {
        for n in ["Microsoft", "Packages", "Temp", "NVIDIA Corporation", "Common Files", "WindowsApps", "ab"] {
            assert!(is_never(&norm(n)), "{n}");
        }
        assert!(!is_never(&norm("OldGoneApp")));
    }

    /// LIVE, read-only: prints what would be listed on this machine.
    #[test]
    #[ignore]
    fn live_find() {
        for o in find() {
            println!("{} | {} | {} bytes", o.location, o.path, o.size_bytes);
        }
    }
}
