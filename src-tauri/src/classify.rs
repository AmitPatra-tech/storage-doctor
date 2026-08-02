use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub category: String,
    /// safe | review | personal | apps | system
    pub safety: String,
    pub explanation: String,
}

fn c(category: &str, safety: &str, explanation: &str) -> Option<Classification> {
    Some(Classification {
        category: category.to_string(),
        safety: safety.to_string(),
        explanation: explanation.to_string(),
    })
}

const CACHE_NAMES: &[&str] = &[
    "cache", "caches", "cache2", "code cache", "gpucache", "cachestorage",
    "shadercache", "cachedata", "cacheddata", "componentmodelcache", "dxcache",
    "glcache", "d3dscache", "media cache", "media cache files", "webcache",
    "cachedextensionvsixs", "cached thumbnails", "cachedextensions",
];

const LOG_NAMES: &[&str] = &["logs", "log", "crashdumps", "minidump", "livekernelreports", "crash reports"];

/// The safety verdict alone — `safe`, `review`, `personal`, `apps`, `system`.
/// Every screen that reports recoverable space must agree on this, so they all
/// route through here rather than re-deriving it.
pub fn safety(path: &Path, is_dir: bool) -> Option<String> {
    classify(path, is_dir).map(|c| c.safety)
}

/// Explains what a folder or file is and whether deleting it is safe.
/// Returns None when nothing meaningful can be said.
pub fn classify(path: &Path, is_dir: bool) -> Option<Classification> {
    let full = path.to_string_lossy().to_lowercase();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if !is_dir {
        if matches!(name.as_str(), "pagefile.sys" | "hiberfil.sys" | "swapfile.sys") {
            return c(
                "System",
                "system",
                "Windows virtual memory / hibernation file. Managed by Windows — deleting it manually is not possible and not safe.",
            );
        }
        if name.ends_with(".log") || name.ends_with(".dmp") {
            return c(
                "Logs & dumps",
                "safe",
                "Diagnostic record. Safe to delete — only needed when troubleshooting a problem.",
            );
        }
    }

    if full.contains("\\$recycle.bin") {
        return c(
            "Recycle Bin",
            "safe",
            "Files you already deleted, held for recovery. Emptying the Recycle Bin permanently removes them and frees this space.",
        );
    }

    if is_dir {
        if CACHE_NAMES.contains(&name.as_str()) {
            return c(
                "Cache",
                "safe",
                "Safe to delete — applications rebuild caches automatically. The only cost is a slightly slower first launch.",
            );
        }
        if matches!(name.as_str(), "temp" | "tmp") {
            return c(
                "Temporary files",
                "safe",
                "Safe to delete — programs recreate temporary files whenever they need them.",
            );
        }
        if LOG_NAMES.contains(&name.as_str()) {
            return c(
                "Logs & dumps",
                "safe",
                "Diagnostic records only. Safe to delete — not needed for normal use.",
            );
        }
        if name == "node_modules" {
            return c(
                "Project dependencies",
                "safe",
                "Downloaded packages for a code project. Safe to delete for projects you are not working on — `npm install` recreates it.",
            );
        }
    }

    if full.contains("\\windows\\winsxs") {
        return c(
            "System",
            "system",
            "Windows component store. Never delete manually — files here are shared by Windows itself. Use Disk Cleanup instead.",
        );
    }
    if full.contains("\\windows\\installer") {
        return c(
            "System",
            "system",
            "Windows Installer cache. Deleting it breaks repairing and uninstalling of installed applications.",
        );
    }
    if full.contains("\\softwaredistribution\\download") {
        return c(
            "Windows Update",
            "review",
            "Downloaded update packages. Safe to remove once updates are installed, but requires administrator rights.",
        );
    }
    if full.contains("\\windows.old") {
        return c(
            "Previous Windows",
            "review",
            "Your previous Windows installation, kept so you can roll back. Windows removes it automatically after ~10 days; Disk Cleanup can remove it sooner.",
        );
    }
    if is_drive_child(&full, "windows") {
        return c(
            "System",
            "system",
            "Windows operating system files. Do not delete manually — removing files here can break Windows.",
        );
    }
    if full.contains("\\steamapps\\common") {
        return c(
            "Game files",
            "apps",
            "Installed games. Uninstall through Steam to remove them cleanly — deleting the folder leaves broken library entries.",
        );
    }
    if is_drive_child(&full, "program files") || is_drive_child(&full, "program files (x86)") {
        return c(
            "Applications",
            "apps",
            "Installed applications. Remove them via Settings → Apps → Uninstall, not by deleting folders — deleting here leaves broken registry entries.",
        );
    }
    if is_drive_child(&full, "programdata") {
        return c(
            "App data",
            "apps",
            "Shared data for installed applications. Deleting can reset or break applications.",
        );
    }

    if let Some(after_user) = user_relative(&full) {
        let first = after_user.split('\\').next().unwrap_or("");
        match first {
            "downloads" => {
                return c(
                    "Downloads",
                    "review",
                    "Files you downloaded. Old installers, archives and ISO images here are usually safe to delete — check anything you might still need.",
                )
            }
            "documents" | "pictures" | "videos" | "music" | "desktop" | "onedrive" => {
                return c(
                    "Personal files",
                    "personal",
                    "Your personal files. They cannot be recreated — review carefully and back up before deleting anything.",
                )
            }
            "appdata" => {
                if after_user == "appdata"
                    || matches!(after_user, "appdata\\local" | "appdata\\roaming" | "appdata\\locallow")
                {
                    return c(
                        "App data",
                        "apps",
                        "Settings and data for your applications. Deleting wholesale resets or breaks apps — clear specific caches instead.",
                    );
                }
                return None;
            }
            _ => return None,
        }
    }

    if is_dir && path.parent().map(|p| p.parent().is_none()).unwrap_or(false) && name == "users" {
        return c(
            "Personal files",
            "personal",
            "All user profiles on this PC, including your documents and app data.",
        );
    }

    None
}

/// Like `classify`, but falls back to the nearest classified ancestor —
/// a file inside a cache folder inherits "safe to clear", a file in
/// Program Files inherits "App files", and so on.
pub fn classify_effective(path: &Path, is_dir: bool) -> Option<Classification> {
    if let Some(c) = classify(path, is_dir) {
        return Some(c);
    }
    let mut current = path.parent();
    while let Some(ancestor) = current {
        if let Some(c) = classify(ancestor, true) {
            return Some(c);
        }
        current = ancestor.parent();
    }
    None
}

/// True when the path is exactly `<drive>:\<child>` or inside it.
fn is_drive_child(full_lower: &str, child: &str) -> bool {
    if full_lower.len() < 3 {
        return false;
    }
    let rest = &full_lower[3..]; // strip "c:\"
    rest == child || rest.starts_with(&format!("{child}\\"))
}

/// For `c:\users\<name>\rest` returns `rest` (lowercased input expected).
fn user_relative(full_lower: &str) -> Option<&str> {
    let idx = full_lower.find(":\\users\\")?;
    let after_users = &full_lower[idx + ":\\users\\".len()..];
    let (_user, rest) = after_users.split_once('\\')?;
    Some(rest)
}
