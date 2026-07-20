use crate::apps;
use crate::classify::{self, Classification};
use crate::cleaner;
use crate::dupes;
use crate::recommendations;
use crate::scanner;
use crate::uninstall;
use crate::AppState;
use rayon::prelude::*;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use sysinfo::Disks;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub is_removable: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub classification: Option<Classification>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BrowseEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub file_count: u64,
    pub classification: Option<Classification>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderDelta {
    pub path: String,
    pub name: String,
    pub delta_bytes: i64,
    pub is_new: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanComparison {
    pub previous_at: String,
    pub current_at: String,
    pub delta_bytes: i64,
    pub changes: Vec<FolderDelta>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size_bytes: u64,
    pub modified_at: String,
    pub classification: Option<Classification>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultItem {
    pub kind: String,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub id: String,
    pub name: String,
    pub description: String,
    pub recoverable_bytes: u64,
    pub risk: String,
    pub recommended: bool,
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub scan_id: i64,
    pub started_at: String,
    pub duration_ms: u64,
    pub drives: Vec<DriveInfo>,
    pub largest_folders: Vec<FolderEntry>,
    pub largest_files: Vec<LargeFile>,
    pub recoverable_bytes: u64,
}

#[tauri::command]
pub fn get_drives() -> Vec<DriveInfo> {
    Disks::new_with_refreshed_list()
        .iter()
        .map(|disk| {
            let total = disk.total_space();
            let free = disk.available_space();
            DriveInfo {
                letter: disk.mount_point().to_string_lossy().trim_end_matches('\\').to_string(),
                label: {
                    let name = disk.name().to_string_lossy().to_string();
                    if name.is_empty() { "Local Disk".to_string() } else { name }
                },
                total_bytes: total,
                used_bytes: total.saturating_sub(free),
                free_bytes: free,
                is_removable: disk.is_removable(),
            }
        })
        .collect()
}

#[tauri::command]
pub fn get_last_scan(state: State<AppState>) -> Result<Option<ScanSummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let scan = db
        .query_row(
            "SELECT id, started_at, duration_ms, recoverable_bytes
             FROM scans WHERE status = 'completed'
             ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })?;

    let Some((scan_id, started_at, duration_ms, recoverable_bytes)) = scan else {
        return Ok(None);
    };

    let mut stmt = db
        .prepare(
            "SELECT letter, label, total_bytes, used_bytes, free_bytes, is_removable
             FROM scan_drives WHERE scan_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let drives = stmt
        .query_map([scan_id], |row| {
            Ok(DriveInfo {
                letter: row.get(0)?,
                label: row.get(1)?,
                total_bytes: row.get(2)?,
                used_bytes: row.get(3)?,
                free_bytes: row.get(4)?,
                is_removable: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Only top-level folders: nested folders would count the same bytes twice.
    let mut stmt = db
        .prepare(
            "SELECT path, name, size_bytes, file_count FROM folders
             WHERE scan_id = ?1 AND depth = 1 ORDER BY size_bytes DESC LIMIT 25",
        )
        .map_err(|e| e.to_string())?;
    let largest_folders = stmt
        .query_map([scan_id], |row| {
            let path: String = row.get(0)?;
            Ok(FolderEntry {
                classification: classify::classify(std::path::Path::new(&path), true),
                path,
                name: row.get(1)?,
                size_bytes: row.get(2)?,
                file_count: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut stmt = db
        .prepare(
            "SELECT path, name, extension, size_bytes, modified_at FROM large_files
             WHERE scan_id = ?1 ORDER BY size_bytes DESC LIMIT 200",
        )
        .map_err(|e| e.to_string())?;
    let largest_files = stmt
        .query_map([scan_id], |row| {
            let path: String = row.get(0)?;
            Ok(LargeFile {
                classification: classify::classify_effective(std::path::Path::new(&path), false),
                path,
                name: row.get(1)?,
                extension: row.get(2)?,
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(Some(ScanSummary {
        scan_id,
        started_at,
        duration_ms,
        drives,
        largest_folders,
        largest_files,
        recoverable_bytes,
    }))
}

/// Starts a scan on a background thread and returns the scan id immediately.
/// Progress is reported via `scan-progress` events, completion via
/// `scan-complete` (or `scan-error`).
#[tauri::command]
pub fn start_scan(
    app: AppHandle,
    state: State<AppState>,
    drive_letters: Vec<String>,
) -> Result<i64, String> {
    let drives: Vec<DriveInfo> = get_drives()
        .into_iter()
        .filter(|d| drive_letters.is_empty() || drive_letters.contains(&d.letter))
        .collect();
    if drives.is_empty() {
        return Err("No matching drives to scan".into());
    }

    let scan_id = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO scans (started_at, status) VALUES (datetime('now'), 'running')",
            [],
        )
        .map_err(|e| e.to_string())?;
        db.last_insert_rowid()
    };

    std::thread::spawn(move || {
        let roots: Vec<PathBuf> = drives
            .iter()
            .map(|d| PathBuf::from(format!("{}\\", d.letter)))
            .collect();

        let outcome = scanner::scan(app.clone(), scan_id, &roots);

        match persist_scan(&app, scan_id, &drives, &outcome) {
            Ok(()) => {
                let _ = app.emit(
                    "scan-complete",
                    scanner::ScanComplete {
                        scan_id,
                        duration_ms: outcome.duration_ms,
                        files_scanned: outcome.files_scanned,
                        bytes_scanned: outcome.bytes_scanned,
                        errors: outcome.errors,
                    },
                );
            }
            Err(err) => {
                let _ = app.emit("scan-error", err);
            }
        }
    });

    Ok(scan_id)
}

fn persist_scan(
    app: &AppHandle,
    scan_id: i64,
    drives: &[DriveInfo],
    outcome: &scanner::ScanOutcome,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut db = state.db.lock().map_err(|e| e.to_string())?;
    let tx = db.transaction().map_err(|e| e.to_string())?;

    for d in drives {
        tx.execute(
            "INSERT INTO scan_drives (scan_id, letter, label, total_bytes, used_bytes, free_bytes, is_removable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                scan_id,
                d.letter,
                d.label,
                d.total_bytes,
                d.used_bytes,
                d.free_bytes,
                d.is_removable
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    for f in &outcome.folders {
        tx.execute(
            "INSERT INTO folders (scan_id, path, name, size_bytes, file_count, depth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![scan_id, f.path, f.name, f.size_bytes, f.file_count, f.depth],
        )
        .map_err(|e| e.to_string())?;
    }

    for f in &outcome.large_files {
        tx.execute(
            "INSERT INTO large_files (scan_id, path, name, extension, size_bytes, modified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![scan_id, f.path, f.name, f.extension, f.size_bytes, f.modified_at],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.execute(
        "UPDATE scans SET status = 'completed', duration_ms = ?2 WHERE id = ?1",
        rusqlite::params![scan_id, outcome.duration_ms],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;

    // Regenerate cleanup recommendations against the fresh scan data.
    recommendations::generate(&db, scan_id)?;
    Ok(())
}

#[tauri::command]
pub fn get_recommendations(state: State<AppState>) -> Result<Vec<Recommendation>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(
            "SELECT id, name, description, recoverable_bytes, risk, recommended, paths
             FROM recommendations WHERE ignored = 0
             ORDER BY recoverable_bytes DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Recommendation {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                recoverable_bytes: row.get(3)?,
                risk: row.get(4)?,
                recommended: row.get::<_, i64>(5)? != 0,
                paths: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn set_recommendation_ignored(
    state: State<AppState>,
    id: String,
    ignored: bool,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.execute(
        "UPDATE recommendations SET ignored = ?2 WHERE id = ?1",
        rusqlite::params![id, ignored],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Live listing of a folder's direct children with recursively computed
/// sizes — works at any depth, independent of what the scan recorded.
#[tauri::command]
pub async fn browse_folder(path: String) -> Result<Vec<BrowseEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let entries = std::fs::read_dir(&path).map_err(|e| e.to_string())?;
        let children: Vec<(PathBuf, bool)> = entries
            .flatten()
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if file_type.is_symlink() {
                    return None;
                }
                Some((entry.path(), file_type.is_dir()))
            })
            .collect();

        let mut result: Vec<BrowseEntry> = children
            .par_iter()
            .map(|(child, is_dir)| {
                let (size_bytes, file_count) = if *is_dir {
                    apps::dir_stats(child)
                } else {
                    (
                        std::fs::metadata(child).map(|m| m.len()).unwrap_or(0),
                        1,
                    )
                };
                BrowseEntry {
                    path: child.to_string_lossy().into_owned(),
                    name: child
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    is_dir: *is_dir,
                    size_bytes,
                    file_count,
                    classification: classify::classify(child, *is_dir),
                }
            })
            .collect();

        result.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        result.truncate(300);
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Compares the two most recent completed scans to answer "why did my
/// storage increase". Prefers the most specific folder that explains a
/// change over its parents.
#[tauri::command]
pub fn get_scan_comparison(state: State<AppState>) -> Result<Option<ScanComparison>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let scans: Vec<(i64, String)> = db
        .prepare("SELECT id, started_at FROM scans WHERE status = 'completed' ORDER BY id DESC LIMIT 2")
        .map_err(|e| e.to_string())?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if scans.len() < 2 {
        return Ok(None);
    }
    let (current_id, current_at) = scans[0].clone();
    let (previous_id, previous_at) = scans[1].clone();

    let used_bytes = |scan_id: i64| -> Result<i64, String> {
        db.query_row(
            "SELECT COALESCE(SUM(used_bytes), 0) FROM scan_drives WHERE scan_id = ?1",
            [scan_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())
    };
    let delta_bytes = used_bytes(current_id)? - used_bytes(previous_id)?;

    let folder_sizes = |scan_id: i64| -> Result<HashMap<String, (String, i64)>, String> {
        db.prepare(
            "SELECT path, name, size_bytes FROM folders WHERE scan_id = ?1 AND depth <= 3",
        )
        .map_err(|e| e.to_string())?
        .query_map([scan_id], |row| {
            Ok((row.get::<_, String>(0)?, (row.get::<_, String>(1)?, row.get::<_, i64>(2)?)))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| e.to_string())
    };
    let current = folder_sizes(current_id)?;
    let previous = folder_sizes(previous_id)?;

    const MIN_DELTA: i64 = 100 * 1024 * 1024;
    let mut candidates: Vec<FolderDelta> = Vec::new();
    for (path, (name, size)) in &current {
        let prev_size = previous.get(path).map(|(_, s)| *s);
        let delta = size - prev_size.unwrap_or(0);
        if delta.abs() >= MIN_DELTA {
            candidates.push(FolderDelta {
                path: path.clone(),
                name: name.clone(),
                delta_bytes: delta,
                is_new: prev_size.is_none(),
            });
        }
    }
    for (path, (name, size)) in &previous {
        if !current.contains_key(path) && *size >= MIN_DELTA {
            candidates.push(FolderDelta {
                path: path.clone(),
                name: name.clone(),
                delta_bytes: -size,
                is_new: false,
            });
        }
    }
    candidates.sort_by_key(|c| -c.delta_bytes.abs());

    // Prefer specific subfolders: drop a parent when a child explains most of
    // its change, and drop anything whose ancestor is already listed.
    let mut changes: Vec<FolderDelta> = Vec::new();
    for candidate in &candidates {
        let prefix = format!("{}\\", candidate.path);
        let child_explains = candidates.iter().any(|other| {
            other.path.starts_with(&prefix)
                && other.delta_bytes.abs() as f64 >= 0.7 * candidate.delta_bytes.abs() as f64
        });
        if child_explains {
            continue;
        }
        let ancestor_listed = changes
            .iter()
            .any(|kept| candidate.path.starts_with(&format!("{}\\", kept.path)));
        if ancestor_listed {
            continue;
        }
        changes.push(candidate.clone());
        if changes.len() >= 8 {
            break;
        }
    }

    Ok(Some(ScanComparison {
        previous_at,
        current_at,
        delta_bytes,
        changes,
    }))
}

/// Walks the known-application registry; runs on a blocking thread because it
/// hits the disk heavily.
#[tauri::command]
pub async fn get_app_usage() -> Result<Vec<apps::AppReport>, String> {
    tauri::async_runtime::spawn_blocking(apps::analyze)
        .await
        .map_err(|e| e.to_string())
}

/// All installed applications from the Windows registry (same list as
/// Settings → Apps; Store apps excluded).
#[tauri::command]
pub fn get_installed_apps() -> Vec<uninstall::InstalledApp> {
    uninstall::list_installed()
}

/// Launches the application's own uninstaller. The user completes it there;
/// leftovers can then be scanned separately.
#[tauri::command]
pub fn launch_uninstaller(uninstall_string: String) -> Result<(), String> {
    uninstall::launch_uninstaller(&uninstall_string)
}

/// Finds folders an app (likely) left behind. Returns candidates with sizes;
/// nothing is deleted here.
#[tauri::command]
pub async fn find_app_leftovers(
    name: String,
    publisher: String,
    install_location: Option<String>,
) -> Result<Vec<uninstall::Leftover>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        uninstall::find_leftovers(&name, &publisher, install_location.as_deref())
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub freed_bytes: u64,
    /// Paths that could not be deleted (typically permission denied or the
    /// file is locked). These can be retried with `delete_paths_elevated`.
    pub failed: Vec<String>,
}

fn path_size(p: &Path) -> u64 {
    if p.is_dir() {
        apps::dir_size(p)
    } else {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
}

/// Records a completed cleanup operation in the journal (best effort).
fn log_operation(app: &AppHandle, source: &str, item_count: usize, freed: u64, method: &str) {
    if item_count == 0 && freed == 0 {
        return;
    }
    let state = app.state::<AppState>();
    let guard = state.db.lock();
    if let Ok(db) = guard {
        let _ = db.execute(
            "INSERT INTO operations (performed_at, source, item_count, freed_bytes, method)
             VALUES (datetime('now'), ?1, ?2, ?3, ?4)",
            rusqlite::params![source, item_count as i64, freed as i64, method],
        );
    }
}

/// Deletes the given folders/files. When `permanent` is true they are removed
/// outright (used for caches/temp — this actually reclaims disk space, and
/// they are recreated automatically); otherwise they are moved to the Recycle
/// Bin (recoverable). One failure does not abort the rest — failures are
/// collected so the caller can offer an elevated retry.
#[tauri::command]
pub async fn delete_paths(
    app: AppHandle,
    paths: Vec<String>,
    source: Option<String>,
    permanent: Option<bool>,
) -> Result<DeleteResult, String> {
    let label = source.unwrap_or_else(|| "Cleanup".into());
    let permanent = permanent.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let mut freed = 0u64;
        let mut deleted = 0usize;
        let mut failed = Vec::new();
        for path in &paths {
            // The Recycle Bin is a protected system folder — deleting it hangs.
            // Empty it with the proper API instead.
            if let Some(drive) = recycle_bin_drive(path) {
                let size = path_size(Path::new(path));
                if clear_recycle_bin_drive(&drive) {
                    freed += size;
                    deleted += 1;
                } else {
                    failed.push(path.clone());
                }
                continue;
            }
            let p = Path::new(path);
            if !p.exists() {
                continue;
            }
            let size = path_size(p);
            let result = if permanent {
                if p.is_dir() {
                    std::fs::remove_dir_all(p).map_err(|_| ())
                } else {
                    std::fs::remove_file(p).map_err(|_| ())
                }
            } else {
                trash::delete(p).map_err(|_| ())
            };
            match result {
                Ok(()) => {
                    freed += size;
                    deleted += 1;
                }
                Err(()) => failed.push(path.clone()),
            }
        }
        log_operation(
            &app,
            &label,
            deleted,
            freed,
            if permanent { "permanent" } else { "recycle" },
        );
        DeleteResult {
            freed_bytes: freed,
            failed,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// If the path is a drive's Recycle Bin, returns its drive letter.
fn recycle_bin_drive(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    if lower.len() >= 3 && lower[1..].starts_with(":\\$recycle.bin") {
        return Some(path[..1].to_string());
    }
    None
}

/// Empties one drive's Recycle Bin via the proper Windows API (fast, and no
/// admin needed for the current user's own bin).
fn clear_recycle_bin_drive(drive: &str) -> bool {
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &format!("Clear-RecycleBin -DriveLetter {drive} -Force -ErrorAction SilentlyContinue"),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_delete_script(paths: &[String], permanent: bool) -> String {
    let mut script = String::from("Add-Type -AssemblyName Microsoft.VisualBasic\n");
    for path in paths {
        if let Some(drive) = recycle_bin_drive(path) {
            script.push_str(&format!(
                "try {{ Clear-RecycleBin -DriveLetter {drive} -Force -ErrorAction SilentlyContinue }} catch {{ }}\n"
            ));
            continue;
        }
        let esc = path.replace('\'', "''");
        if permanent {
            script.push_str(&format!(
                "try {{ Remove-Item -LiteralPath '{esc}' -Recurse -Force -ErrorAction SilentlyContinue }} catch {{ }}\n"
            ));
        } else {
            script.push_str(&format!(
                "try {{ if (Test-Path -LiteralPath '{esc}' -PathType Container) {{ \
                    [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('{esc}','OnlyErrorDialogs','SendToRecycleBin') \
                 }} elseif (Test-Path -LiteralPath '{esc}') {{ \
                    [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{esc}','OnlyErrorDialogs','SendToRecycleBin') \
                 }} }} catch {{ }}\n"
            ));
        }
    }
    script
}

/// Retries deletion with administrator rights, still sending items to the
/// Recycle Bin (via the Windows shell). Triggers a single UAC prompt.
#[tauri::command]
pub async fn delete_paths_elevated(
    app: AppHandle,
    paths: Vec<String>,
    source: Option<String>,
    permanent: Option<bool>,
) -> Result<DeleteResult, String> {
    let label = source.unwrap_or_else(|| "Cleanup (admin)".into());
    let permanent = permanent.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let sizes: Vec<(String, u64)> = paths
            .iter()
            .map(|p| (p.clone(), path_size(Path::new(p))))
            .collect();

        // Write the delete script to a temp file to avoid command-line quoting.
        let script = build_delete_script(&paths, permanent);
        let tmp = std::env::temp_dir().join("storage_doctor_recycle.ps1");
        std::fs::write(&tmp, script).map_err(|e| e.to_string())?;

        // Outer (non-elevated) PowerShell launches an elevated PowerShell that
        // runs the script, and waits for it to finish.
        let inner = format!(
            "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden \
             -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{}'",
            tmp.display()
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &inner])
            .status()
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&tmp);

        if !status.success() {
            return Err("Administrator permission was denied.".into());
        }

        let mut freed = 0u64;
        let mut deleted = 0usize;
        let mut failed = Vec::new();
        for (path, size) in sizes {
            let p = Path::new(&path);
            if !p.exists() {
                freed += size;
                deleted += 1;
            } else if permanent && p.is_dir() {
                // Directories like Temp are recreated instantly by Windows.
                // If the current size is much smaller than before, the contents
                // were successfully deleted — count it as success.
                let current_size = path_size(p);
                if current_size < size / 2 || (size > 0 && current_size < 4096) {
                    freed += size.saturating_sub(current_size);
                    deleted += 1;
                } else {
                    failed.push(path);
                }
            } else {
                failed.push(path);
            }
        }
        log_operation(
            &app,
            &label,
            deleted,
            freed,
            if permanent { "permanent-admin" } else { "recycle-admin" },
        );
        Ok(DeleteResult {
            freed_bytes: freed,
            failed,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Re-measures the current recommendations against the latest scan so their
/// recoverable sizes reflect what was just cleaned (folders now empty drop off).
#[tauri::command]
pub fn regenerate_recommendations(state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let scan_id: Option<i64> = db
        .query_row(
            "SELECT id FROM scans WHERE status = 'completed' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = scan_id {
        recommendations::generate(&db, id)?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub id: i64,
    pub performed_at: String,
    pub source: String,
    pub item_count: i64,
    pub freed_bytes: i64,
    pub method: String,
}

/// The cleanup journal — every deletion the app has performed, newest first.
#[tauri::command]
pub fn get_operations(state: State<AppState>) -> Result<Vec<OperationRecord>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(
            "SELECT id, performed_at, source, item_count, freed_bytes, method
             FROM operations ORDER BY id DESC LIMIT 500",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OperationRecord {
                id: row.get(0)?,
                performed_at: row.get(1)?,
                source: row.get(2)?,
                item_count: row.get(3)?,
                freed_bytes: row.get(4)?,
                method: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Stored license JSON (or null if unlicensed / free).
#[tauri::command]
pub fn get_license(state: State<AppState>) -> Result<Option<String>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.query_row(
        "SELECT value FROM settings WHERE key = 'license'",
        [],
        |row| row.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.to_string()),
    })
}

#[tauri::command]
pub fn set_license(state: State<AppState>, license: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO settings (key, value) VALUES ('license', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![license],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn clear_license(state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.execute("DELETE FROM settings WHERE key = 'license'", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Writes raw bytes to a file (used to save exported PDF reports).
#[tauri::command]
pub fn write_bytes(path: String, bytes: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())
}

/// Opens a URL (e.g. the Dodo Payments checkout) in the default browser.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // Only allow http(s) links; never shell out to arbitrary schemes.
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only http(s) URLs can be opened.".into());
    }
    std::process::Command::new("cmd")
        .raw_arg(format!("/C start \"\" \"{url}\""))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Everything inside a folder that is safe to delete (caches, temp, logs…),
/// however deep. Reports candidates only — deletion is a separate, confirmed
/// step.
#[tauri::command]
pub async fn find_safe_cleanup(path: String) -> Result<Vec<cleaner::SafeItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        cleaner::find_safe_items(std::path::Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())
}

/// SHA-256 duplicate detection across the given roots (defaults to the
/// user's Downloads/Documents/Pictures/Videos/Music/Desktop). Progress is
/// emitted as `dupe-progress` events.
#[tauri::command]
pub async fn find_duplicates(
    app: AppHandle,
    roots: Vec<String>,
) -> Result<Vec<dupes::DupeGroup>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        dupes::find(app, roots.into_iter().map(PathBuf::from).collect())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Search across scanned folders, large files, recommendations and
/// installed applications.
#[tauri::command]
pub fn search(state: State<AppState>, query: String) -> Result<Vec<SearchResultItem>, String> {
    let q = query.trim().to_lowercase();
    if q.len() < 2 {
        return Ok(Vec::new());
    }
    let pattern = format!("%{}%", q.replace('%', "").replace('_', "\\_"));
    let mut results: Vec<SearchResultItem> = Vec::new();

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let scan_id: Option<i64> = db
            .query_row(
                "SELECT id FROM scans WHERE status = 'completed' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();

        if let Some(scan_id) = scan_id {
            let mut stmt = db
                .prepare(
                    "SELECT path, name, size_bytes FROM folders
                     WHERE scan_id = ?1 AND name LIKE ?2 ORDER BY size_bytes DESC LIMIT 20",
                )
                .map_err(|e| e.to_string())?;
            let folders = stmt
                .query_map(rusqlite::params![scan_id, pattern], |row| {
                    Ok(SearchResultItem {
                        kind: "folder".into(),
                        path: row.get(0)?,
                        name: row.get(1)?,
                        size_bytes: row.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .flatten();
            results.extend(folders);

            let mut stmt = db
                .prepare(
                    "SELECT path, name, size_bytes FROM large_files
                     WHERE scan_id = ?1 AND (name LIKE ?2 OR extension LIKE ?2)
                     ORDER BY size_bytes DESC LIMIT 20",
                )
                .map_err(|e| e.to_string())?;
            let files = stmt
                .query_map(rusqlite::params![scan_id, pattern], |row| {
                    Ok(SearchResultItem {
                        kind: "file".into(),
                        path: row.get(0)?,
                        name: row.get(1)?,
                        size_bytes: row.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .flatten();
            results.extend(files);
        }

        let mut stmt = db
            .prepare(
                "SELECT name, recoverable_bytes FROM recommendations
                 WHERE ignored = 0 AND (name LIKE ?1 OR description LIKE ?1)
                 ORDER BY recoverable_bytes DESC LIMIT 10",
            )
            .map_err(|e| e.to_string())?;
        let recs = stmt
            .query_map([&pattern], |row| {
                Ok(SearchResultItem {
                    kind: "recommendation".into(),
                    name: row.get(0)?,
                    path: String::new(),
                    size_bytes: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .flatten();
        results.extend(recs);
    }

    let apps: Vec<SearchResultItem> = uninstall::list_installed()
        .into_iter()
        .filter(|a| a.name.to_lowercase().contains(&q))
        .take(20)
        .map(|a| SearchResultItem {
            kind: "app".into(),
            name: a.name,
            path: a.install_location.unwrap_or_default(),
            size_bytes: a.estimated_bytes,
        })
        .collect();
    results.extend(apps);

    Ok(results)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProgress {
    pub found: usize,
    pub done: bool,
}

const SEARCH_RESULT_CAP: usize = 500;

struct SearchCtx {
    query: String,
    app: AppHandle,
    results: Mutex<Vec<SearchResultItem>>,
    found: AtomicUsize,
    last_emit: Mutex<std::time::Instant>,
}

/// Live filesystem search for files/folders whose name contains the query,
/// across all fixed drives. Unlike `search`, this finds files of any size
/// (they are not all indexed by a scan). Emits `search-progress` events and
/// is capped and run on a blocking thread.
#[tauri::command]
pub async fn search_files(
    app: AppHandle,
    query: String,
) -> Result<Vec<SearchResultItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let q = query.trim().to_lowercase();
        if q.len() < 2 {
            return Vec::new();
        }
        let roots: Vec<PathBuf> = Disks::new_with_refreshed_list()
            .iter()
            .filter(|d| !d.is_removable())
            .map(|d| d.mount_point().to_path_buf())
            .collect();

        let ctx = SearchCtx {
            query: q,
            app: app.clone(),
            results: Mutex::new(Vec::new()),
            found: AtomicUsize::new(0),
            last_emit: Mutex::new(std::time::Instant::now()),
        };
        roots.par_iter().for_each(|root| search_dir(root, &ctx));

        let _ = app.emit(
            "search-progress",
            SearchProgress {
                found: ctx.found.load(Ordering::Relaxed),
                done: true,
            },
        );

        let mut out = ctx.results.into_inner().unwrap_or_default();
        out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        out.truncate(300);
        out
    })
    .await
    .map_err(|e| e.to_string())
}

fn emit_search_progress(ctx: &SearchCtx) {
    let Ok(mut last) = ctx.last_emit.try_lock() else {
        return;
    };
    if last.elapsed() < std::time::Duration::from_millis(200) {
        return;
    }
    *last = std::time::Instant::now();
    let _ = ctx.app.emit(
        "search-progress",
        SearchProgress {
            found: ctx.found.load(Ordering::Relaxed),
            done: false,
        },
    );
}

fn search_dir(dir: &Path, ctx: &SearchCtx) {
    if ctx.found.load(Ordering::Relaxed) >= SEARCH_RESULT_CAP {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.contains(&ctx.query) {
            let (is_dir, size) = if file_type.is_dir() {
                (true, 0)
            } else {
                (false, entry.metadata().map(|m| m.len()).unwrap_or(0))
            };
            if ctx.found.fetch_add(1, Ordering::Relaxed) < SEARCH_RESULT_CAP {
                if let Ok(mut r) = ctx.results.lock() {
                    r.push(SearchResultItem {
                        kind: if is_dir { "folder".into() } else { "file".into() },
                        name: entry.file_name().to_string_lossy().into_owned(),
                        path: entry.path().to_string_lossy().into_owned(),
                        size_bytes: size,
                    });
                }
                emit_search_progress(ctx);
            }
        }
        if file_type.is_dir() {
            subdirs.push(entry.path());
        }
    }
    subdirs.par_iter().for_each(|sub| search_dir(sub, ctx));
}

#[tauri::command]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // raw_arg bypasses Rust's automatic quoting, which otherwise wraps a
    // path-with-spaces so that `explorer /select,` fails and silently opens
    // the default Documents folder. Folders are opened directly; files are
    // revealed and selected inside their parent.
    let is_dir = std::path::Path::new(&path).is_dir();
    let raw = if is_dir {
        format!("\"{path}\"")
    } else {
        format!("/select,\"{path}\"")
    };
    std::process::Command::new("explorer")
        .raw_arg(raw)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Moves a file to the Recycle Bin (never a permanent delete) and drops it
/// from the current scan's large-file list.
#[tauri::command]
pub fn delete_file(state: State<AppState>, path: String) -> Result<(), String> {
    trash::delete(&path).map_err(|e| e.to_string())?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.execute(
        "DELETE FROM large_files WHERE path = ?1",
        rusqlite::params![path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
