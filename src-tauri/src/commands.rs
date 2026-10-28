use crate::apps;
use crate::classify::{self, Classification};
use crate::cleaner;
use crate::dupes;
use crate::recommendations;
use crate::scanner;
use crate::shell;
use crate::thumbs;
use crate::uninstall;
use crate::AppState;
use rayon::prelude::*;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::Disks;
use tauri::{AppHandle, Emitter, Manager, State};
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

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
    /// How much of this folder is safe to clear — the figure behind the
    /// cleanup action, so the row never implies more than it can deliver.
    /// `None` for scans taken before this was measured, which is not the
    /// same as zero and must not be shown as such.
    pub recoverable_bytes: Option<u64>,
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
    pub recoverable_bytes: u64,
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

/// Live drive capacity/usage, read fresh from the OS every call — this is
/// what keeps the dashboard's free-space figures current between scans.
#[tauri::command(async)]
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

#[tauri::command(async)]
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
            "SELECT path, name, size_bytes, file_count, recoverable_bytes, recoverable_measured
             FROM folders
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
                // A row from before this was measured holds 0, which would
                // read as "nothing to clear". Say "not measured" instead —
                // `measure_recoverable` fills those in.
                recoverable_bytes: (row.get::<_, i64>(5)? != 0)
                    .then(|| row.get(4))
                    .transpose()?,
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

        let reporter = app.clone();
        let outcome = scanner::scan(
            Some(Box::new(move |progress| {
                let _ = reporter.emit("scan-progress", progress);
            })),
            scan_id,
            &roots,
        );

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
            "INSERT INTO folders
                (scan_id, path, name, size_bytes, file_count,
                 recoverable_bytes, recoverable_measured, depth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            rusqlite::params![
                scan_id,
                f.path,
                f.name,
                f.size_bytes,
                f.file_count,
                f.recoverable_bytes,
                f.depth
            ],
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

#[tauri::command(async)]
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
pub async fn browse_folder(app: AppHandle, path: String) -> Result<Vec<BrowseEntry>, String> {
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
                // One walk yields both the size and how much of it is
                // clearable, so every row can state the latter up front.
                let measured = if *is_dir {
                    cleaner::measure_dir(child)
                } else {
                    let size = std::fs::metadata(child).map(|m| m.len()).unwrap_or(0);
                    cleaner::Measure {
                        size_bytes: size,
                        file_count: 1,
                        recoverable_bytes: if cleaner::is_clearable_file(child, size) {
                            size
                        } else {
                            0
                        },
                    }
                };
                BrowseEntry {
                    path: child.to_string_lossy().into_owned(),
                    name: child
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    is_dir: *is_dir,
                    size_bytes: measured.size_bytes,
                    file_count: measured.file_count,
                    recoverable_bytes: measured.recoverable_bytes,
                    classification: classify::classify(child, *is_dir),
                }
            })
            .collect();

        // Totalled before truncation, or a folder with more than 300 children
        // would look like it had shrunk.
        let total_bytes: u64 = result.iter().map(|e| e.size_bytes).sum();
        let total_files: u64 = result.iter().map(|e| e.file_count).sum();
        // A folder that is itself safe to clear counts in full, exactly as
        // `cleaner::measure_dir` would have it.
        let total_recoverable: u64 =
            if classify::safety(Path::new(&path), true).as_deref() == Some("safe") {
                total_bytes
            } else {
                result.iter().map(|e| e.recoverable_bytes).sum()
            };
        let complete = result.len() <= 300;

        result.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        result.truncate(300);

        if complete {
            reconcile_folder_size(&app, &path, total_bytes, total_files, total_recoverable);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Folds a live folder measurement back into the stored scan.
///
/// The top-level breakdown is served from the last scan, while any folder you
/// open is measured live — so the two disagree whenever something shrank in
/// between. An emptied Recycle Bin read 330 MB in the list and 129 B inside,
/// with nothing to explain the gap. Opening a folder now corrects its recorded
/// size and every ancestor's, so the list converges on reality as you explore.
fn reconcile_folder_size(
    app: &AppHandle,
    path: &str,
    bytes: u64,
    files: u64,
    recoverable: u64,
) {
    /// Ignore ordinary churn; only rewrite the snapshot for real differences.
    const MIN_CORRECTION: u64 = 4 * 1024 * 1024;

    let state = app.state::<AppState>();
    let Ok(mut db) = state.db.lock() else {
        return;
    };
    let scan_id: Option<i64> = db
        .query_row(
            "SELECT id FROM scans WHERE status = 'completed' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    let Some(scan_id) = scan_id else {
        return;
    };

    // Only folders the scan actually recorded can be corrected — anything
    // deeper was never stored, so there is no stale number to fix.
    let recorded: Option<(u64, u64, u64, bool)> = db
        .query_row(
            "SELECT size_bytes, file_count, recoverable_bytes, recoverable_measured
             FROM folders WHERE scan_id = ?1 AND path = ?2",
            rusqlite::params![scan_id, path],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            },
        )
        .ok();
    let Some((old_bytes, old_files, old_recoverable, was_measured)) = recorded else {
        return;
    };
    // An unmeasured row always gets written, however small the difference —
    // otherwise the folder keeps reporting "not measured" forever.
    if was_measured
        && old_bytes.abs_diff(bytes) < MIN_CORRECTION
        && old_recoverable.abs_diff(recoverable) < MIN_CORRECTION
    {
        return;
    }

    let Ok(tx) = db.transaction() else {
        return;
    };
    let _ = tx.execute(
        "UPDATE folders
         SET size_bytes = ?3, file_count = ?4,
             recoverable_bytes = ?5, recoverable_measured = 1
         WHERE scan_id = ?1 AND path = ?2",
        rusqlite::params![scan_id, path, bytes, files, recoverable],
    );
    adjust_ancestors(
        &tx,
        scan_id,
        path,
        bytes as i64 - old_bytes as i64,
        files as i64 - old_files as i64,
        // A row that held no measurement contributed nothing to its parents,
        // so there is no earlier figure to subtract.
        recoverable as i64 - if was_measured { old_recoverable as i64 } else { 0 },
    );
    let _ = tx.commit();
}

/// Applies a signed size change to every folder above `path`.
fn adjust_ancestors(
    tx: &rusqlite::Transaction,
    scan_id: i64,
    path: &str,
    delta_bytes: i64,
    delta_files: i64,
    delta_recoverable: i64,
) {
    for ancestor in ancestors_of(path) {
        let _ = tx.execute(
            "UPDATE folders
             SET size_bytes = MAX(0, size_bytes + ?3),
                 file_count = MAX(0, file_count + ?4),
                 recoverable_bytes = MAX(0, recoverable_bytes + ?5)
             WHERE scan_id = ?1 AND path = ?2",
            rusqlite::params![scan_id, ancestor, delta_bytes, delta_files, delta_recoverable],
        );
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderMeasurement {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub recoverable_bytes: u64,
}

/// Works out how much can be cleared from folders the last scan never
/// measured, and stores the answer.
///
/// The breakdown's opening screen is served from stored scan data, so a folder
/// recorded before this was measured showed no cleanup figure until you opened
/// it — which measured it live. Rather than demanding another full scan, fill
/// the gaps in the background: each folder is emitted as `folder-measured` the
/// moment it is done, so the figures appear one by one.
#[tauri::command]
pub async fn measure_recoverable(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    const MAX_FOLDERS: usize = 40;

    tauri::async_runtime::spawn_blocking(move || {
        // Sequential on purpose: each measurement is already parallel inside,
        // and walking every top-level folder at once only thrashes the disk.
        for path in paths.into_iter().take(MAX_FOLDERS) {
            let dir = PathBuf::from(&path);
            if !dir.is_dir() {
                continue;
            }
            let measured = cleaner::measure_dir(&dir);
            reconcile_folder_size(
                &app,
                &path,
                measured.size_bytes,
                measured.file_count,
                measured.recoverable_bytes,
            );
            let _ = app.emit(
                "folder-measured",
                FolderMeasurement {
                    path,
                    size_bytes: measured.size_bytes,
                    file_count: measured.file_count,
                    recoverable_bytes: measured.recoverable_bytes,
                },
            );
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Compares the two most recent completed scans to answer "why did my
/// storage increase". Prefers the most specific folder that explains a
/// change over its parents.
#[tauri::command(async)]
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
#[tauri::command(async)]
pub fn get_installed_apps() -> Vec<uninstall::InstalledApp> {
    installed_apps_cached()
}

/// Launches the application's own uninstaller. The user completes it there;
/// leftovers can then be scanned separately.
///
/// Runs off the main thread: the shell blocks on the UAC prompt, which would
/// otherwise freeze the window until the user answers it.
#[tauri::command]
pub async fn launch_uninstaller(uninstall_string: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || uninstall::launch_uninstaller(&uninstall_string))
        .await
        .map_err(|e| e.to_string())?
}

/// Force-removes an application the normal uninstaller could not — closing
/// related processes and removing the program, its registry entry and
/// shortcuts directly. The last resort for when an app's own uninstaller is
/// missing, broken, or (Microsoft Edge being the standard example) refuses
/// to run at all, the same job a third-party tool like Revo Uninstaller does
/// in its "forced" mode.
///
/// Runs without administrator rights first, which is enough on its own for
/// most per-user installs; whatever still needs elevation (Program Files,
/// the HKLM registry, all-users shortcuts) is reported so the caller can
/// offer `force_uninstall_elevated`.
#[tauri::command(async)]
pub fn force_uninstall(target: uninstall::InstalledApp) -> uninstall::ForceUninstallReport {
    uninstall::force_uninstall(&target)
}

/// Finishes whatever `force_uninstall` could not without administrator
/// rights, in a single elevated pass (one UAC prompt). The elevated script's
/// own exit code is not trusted — everything is re-checked afterward, so the
/// report reflects what is actually gone rather than what the script claimed.
#[tauri::command]
pub async fn force_uninstall_elevated(
    target: uninstall::InstalledApp,
) -> Result<uninstall::ForceUninstallReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let report_path = new_force_report_path();
        let (script, attempted_edge) =
            uninstall::build_force_uninstall_script(&target, &report_path);
        run_elevated_powershell(&script, "storage_doctor_force_uninstall.ps1")?;
        let report_text = std::fs::read_to_string(&report_path).unwrap_or_default();
        let _ = std::fs::remove_file(&report_path);
        // Edge's own uninstaller keeps tearing down scheduled tasks briefly
        // after setup.exe returns; give it a moment before checking whether
        // it is really gone.
        std::thread::sleep(std::time::Duration::from_secs(1));
        let mut report = uninstall::verify_force_uninstall(&target, attempted_edge);
        if attempted_edge && !report.complete {
            report.notes = uninstall::edge_refusal_notes(&report_text);
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// PNG data URIs for the given files, keyed by path. Images and videos get a
/// real content thumbnail; everything else gets its file-type icon. Paths that
/// cannot be rendered are simply absent from the map, so the UI falls back to
/// a generic icon rather than showing a broken image.
///
/// Rendering is parallel but capped — a grid of thumbnails is requested in one
/// call, and each one is a shell round-trip.
#[tauri::command]
pub async fn get_thumbnails(
    paths: Vec<String>,
    size: Option<u32>,
    icon_only: Option<bool>,
) -> Result<HashMap<String, String>, String> {
    const MAX_PATHS: usize = 300;
    let size = size.unwrap_or(64).clamp(16, 256);
    let icon_only = icon_only.unwrap_or(false);

    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .take(MAX_PATHS)
            .collect::<Vec<_>>()
            .par_iter()
            .filter_map(|path| {
                thumbs::shell_image(Path::new(path), size, icon_only)
                    .map(|uri| (path.clone(), uri))
            })
            .collect::<HashMap<String, String>>()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Logos for installed applications, keyed by application name.
#[tauri::command]
pub async fn get_app_icons(
    apps: Vec<uninstall::InstalledApp>,
    size: Option<u32>,
) -> Result<HashMap<String, String>, String> {
    let size = size.unwrap_or(32).clamp(16, 256);
    tauri::async_runtime::spawn_blocking(move || {
        apps.par_iter()
            .filter_map(|app| {
                let source = thumbs::app_icon_source(
                    app.display_icon.as_deref(),
                    app.install_location.as_deref(),
                    app.uninstall_string.as_deref(),
                )?;
                thumbs::shell_image(&source, size, true).map(|uri| (app.name.clone(), uri))
            })
            .collect::<HashMap<String, String>>()
    })
    .await
    .map_err(|e| e.to_string())
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
    /// Paths Windows refused to delete. These can be retried with
    /// `delete_paths_elevated`.
    pub failed: Vec<String>,
    /// Paths this app will not delete at all (a drive, a core Windows or
    /// user folder, or anything inside Windows), each with the reason. Not
    /// retried: no retry would make deleting them safe.
    pub refused: Vec<BlockedItem>,
}

/// Why this app refuses to delete `path`, if it does. Deleting a drive or a
/// core folder would break the PC. `force` adds one more rule: force delete
/// closes every program using files in the target, so it is also refused for
/// anything inside Windows — except the Windows cleanup folders this app's
/// own recommendations target (Temp, Logs, crash dumps, update downloads).
pub(crate) fn delete_refusal(path: &str, force: bool) -> Option<String> {
    let p = Path::new(path);
    let canon = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    let target = canon(p);
    if p.parent().is_none() || target.len() <= 2 {
        return Some("This is an entire drive. Storage Doctor never deletes a whole drive.".into());
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let core = [
        env("SystemRoot"),
        env("ProgramFiles"),
        env("ProgramFiles(x86)"),
        env("ProgramW6432"),
        env("ProgramData"),
        env("PUBLIC"),
        env("USERPROFILE"),
        env("LOCALAPPDATA"),
        env("APPDATA"),
        env("SystemDrive").map(|d| PathBuf::from(format!("{}\\Users", d.to_string_lossy()))),
    ];
    if core.iter().flatten().any(|c| canon(c) == target) {
        return Some(
            "This is a core Windows or user folder. Deleting it would break Windows or your \
             programs, so Storage Doctor never deletes it."
                .into(),
        );
    }
    if force {
        if let Some(win) = env("SystemRoot") {
            let win = canon(&win);
            let cleanup = ["temp", "logs", "minidump", "livekernelreports", "softwaredistribution\\download"];
            let in_cleanup = cleanup.iter().any(|c| {
                let root = format!("{win}\\{c}");
                target == root || target.starts_with(&format!("{root}\\"))
            });
            if target.starts_with(&format!("{win}\\")) && !in_cleanup {
                return Some(
                    "This is part of Windows. Force delete would close the Windows components \
                     using it, so Storage Doctor never force deletes inside the Windows folder."
                        .into(),
                );
            }
        }
    }
    None
}

/// Splits `paths` into those that may be deleted and those that are refused.
fn split_refused(paths: Vec<String>, force: bool) -> (Vec<String>, Vec<BlockedItem>) {
    let mut allowed = Vec::new();
    let mut refused = Vec::new();
    for path in paths {
        match delete_refusal(&path, force) {
            Some(why) => refused.push(BlockedItem { path, reasons: vec![why] }),
            None => allowed.push(path),
        }
    }
    (allowed, refused)
}

/// What is on disk at `p` right now — size, file count and clearable bytes.
/// Returns zeroes for anything that no longer exists, so it doubles as the
/// "after" measurement of a deletion.
fn measure_path(p: &Path) -> cleaner::Measure {
    if p.is_dir() {
        return cleaner::measure_dir(p);
    }
    match std::fs::metadata(p) {
        Ok(m) => cleaner::Measure {
            size_bytes: m.len(),
            file_count: 1,
            recoverable_bytes: if cleaner::is_clearable_file(p, m.len()) {
                m.len()
            } else {
                0
            },
        },
        Err(_) => cleaner::Measure::default(),
    }
}

/// What a single delete actually removed, measured rather than assumed.
/// `bytes`/`files` are the difference between the before and after
/// measurements, so a folder Windows immediately recreates (Temp, the Recycle
/// Bin) is counted for its contents only.
struct Removal {
    path: String,
    bytes: u64,
    files: u64,
    /// How much of `bytes` had been counted as clearable, so the figure beside
    /// the cleanup action shrinks along with the folder.
    recoverable: u64,
    /// The path is gone entirely, so its rows can be dropped from the scan
    /// snapshot instead of resized.
    vanished: bool,
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
        let (paths, refused) = split_refused(paths, false);
        let mut removals: Vec<Removal> = Vec::new();
        let mut failed = Vec::new();
        for path in &paths {
            let p = Path::new(path);
            let before = measure_path(p);

            // The Recycle Bin is a protected system folder — deleting it as a
            // directory hangs. Empty it through the shell API instead.
            let ok = if let Some(drive) = recycle_bin_drive(path) {
                empty_recycle_bin(&drive)
            } else if !p.exists() {
                continue;
            } else if permanent {
                if p.is_dir() {
                    std::fs::remove_dir_all(p).is_ok()
                } else {
                    std::fs::remove_file(p).is_ok()
                }
            } else {
                trash::delete(p).is_ok()
            };

            // How much was removed is measured rather than assumed: folders
            // like Temp and the Recycle Bin are recreated empty straight away,
            // and a permanent delete that fails part-way has still removed
            // part of the folder.
            let after = measure_path(p);
            let bytes = before.size_bytes.saturating_sub(after.size_bytes);
            let files = before.file_count.saturating_sub(after.file_count);
            if ok || bytes > 0 || files > 0 {
                removals.push(Removal {
                    path: path.clone(),
                    bytes,
                    files,
                    recoverable: before
                        .recoverable_bytes
                        .saturating_sub(after.recoverable_bytes),
                    vanished: !p.exists(),
                });
            }
            if !ok {
                failed.push(path.clone());
            }
        }

        let freed: u64 = removals.iter().map(|r| r.bytes).sum();
        sync_scan_after_delete(&app, &removals);
        log_operation(
            &app,
            &label,
            removals.len(),
            freed,
            if permanent { "permanent" } else { "recycle" },
        );
        DeleteResult {
            freed_bytes: freed,
            failed,
            refused,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Rewrites the stored scan snapshot so it matches what is now on disk.
///
/// Without this the Breakdown, Dashboard and Search screens keep serving the
/// sizes captured by the last full scan — deleting 30 GB changed nothing on
/// screen until the user scanned again. Deleted subtrees are dropped, folders
/// Windows recreated are resized, every ancestor folder shrinks by the same
/// amount, and drive free space is re-read live.
fn sync_scan_after_delete(app: &AppHandle, removals: &[Removal]) {
    if removals.is_empty() {
        return;
    }
    // Everything that touches the disk happens before the lock is taken —
    // holding the connection across a directory walk would stall every other
    // query behind it.
    let drives = get_drives();
    let survivors: HashMap<&str, cleaner::Measure> = removals
        .iter()
        .filter(|r| !r.vanished)
        .map(|r| (r.path.as_str(), measure_path(Path::new(&r.path))))
        .collect();

    let state = app.state::<AppState>();
    let Ok(mut db) = state.db.lock() else {
        return;
    };
    let scan_id: Option<i64> = db
        .query_row(
            "SELECT id FROM scans WHERE status = 'completed' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    let Some(scan_id) = scan_id else {
        return;
    };
    let Ok(tx) = db.transaction() else {
        return;
    };

    for removal in removals {
        let subtree = like_subtree(&removal.path);
        if removal.vanished {
            let _ = tx.execute(
                "DELETE FROM folders
                 WHERE scan_id = ?1 AND (path = ?2 OR path LIKE ?3 ESCAPE '\\')",
                rusqlite::params![scan_id, removal.path, subtree],
            );
        } else {
            // Recreated (empty) folder: keep the row, correct its size.
            let now = survivors
                .get(removal.path.as_str())
                .copied()
                .unwrap_or_default();
            let _ = tx.execute(
                "UPDATE folders
                 SET size_bytes = ?3, file_count = ?4,
                     recoverable_bytes = ?5, recoverable_measured = 1
                 WHERE scan_id = ?1 AND path = ?2",
                rusqlite::params![
                    scan_id,
                    removal.path,
                    now.size_bytes,
                    now.file_count,
                    now.recoverable_bytes
                ],
            );
            let _ = tx.execute(
                "DELETE FROM folders WHERE scan_id = ?1 AND path LIKE ?2 ESCAPE '\\'",
                rusqlite::params![scan_id, subtree],
            );
        }
        let _ = tx.execute(
            "DELETE FROM large_files
             WHERE scan_id = ?1 AND (path = ?2 OR path LIKE ?3 ESCAPE '\\')",
            rusqlite::params![scan_id, removal.path, subtree],
        );

        // Every folder above it now holds that much less.
        adjust_ancestors(
            &tx,
            scan_id,
            &removal.path,
            -(removal.bytes as i64),
            -(removal.files as i64),
            -(removal.recoverable as i64),
        );
    }

    for d in &drives {
        let _ = tx.execute(
            "UPDATE scan_drives SET total_bytes = ?3, used_bytes = ?4, free_bytes = ?5
             WHERE scan_id = ?1 AND letter = ?2",
            rusqlite::params![scan_id, d.letter, d.total_bytes, d.used_bytes, d.free_bytes],
        );
    }

    let _ = tx.commit();
}

/// LIKE pattern matching everything strictly below `path`. Escaped for
/// `ESCAPE '\'` because Windows paths are full of `_`, which LIKE would
/// otherwise treat as a single-character wildcard.
fn like_subtree(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for ch in path.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    if !path.ends_with('\\') {
        out.push_str("\\\\");
    }
    out.push('%');
    out
}

/// Every parent directory of `path`, nearest first.
fn ancestors_of(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = Path::new(path).parent();
    while let Some(dir) = current {
        let text = dir.to_string_lossy().to_string();
        if text.is_empty() {
            break;
        }
        out.push(text);
        current = dir.parent();
    }
    out
}

/// If the path is a drive's Recycle Bin, returns its drive letter.
fn recycle_bin_drive(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    if lower.len() >= 3 && lower[1..].starts_with(":\\$recycle.bin") {
        return Some(path[..1].to_string());
    }
    None
}

/// Empties one drive's Recycle Bin through the shell API Explorer itself uses.
///
/// The previous implementation shelled out to `Clear-RecycleBin
/// -ErrorAction SilentlyContinue`, which exits 0 whether or not it emptied
/// anything — so a bin that had not been touched was still reported as freed.
/// `SHEmptyRecycleBin` reports the truth, and the caller re-measures anyway.
fn empty_recycle_bin(drive: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::{
        SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND,
    };

    crate::thumbs::ensure_com();
    let root = HSTRING::from(format!("{drive}:\\"));
    let result = unsafe {
        SHEmptyRecycleBinW(
            None,
            &root,
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    };
    match result {
        Ok(()) => true,
        // E_UNEXPECTED comes back when the bin is already empty — not a failure.
        Err(e) => e.code() == windows::Win32::Foundation::E_UNEXPECTED,
    }
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

/// Runs a PowerShell script with administrator rights and waits for it to
/// finish — one UAC prompt. Shared by every operation that needs elevation
/// (deleting, force-uninstalling, force-deleting): the script is written to a
/// temp file first so its content never has to survive command-line quoting,
/// then an outer non-elevated PowerShell launches an elevated one to run it.
pub(crate) fn run_elevated_powershell(script: &str, temp_name: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // Unique per run: two force-deletes started together from Explorer must
    // not overwrite each other's script.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("{}_{stamp}_{temp_name}", std::process::id()));
    std::fs::write(&tmp, script).map_err(|e| e.to_string())?;

    // Already administrator ("Run as administrator" is on): run it directly —
    // no UAC prompt.
    if crate::elevation::is_elevated() {
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
            .arg(&tmp)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .status()
            .map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&tmp);
        return match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("The administrator step failed (exit code {:?}).", s.code())),
            Err(e) => Err(e),
        };
    }

    let inner = format!(
        "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden \
         -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','\"{}\"'",
        tmp.display().to_string().replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &inner])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);

    if !status.success() {
        return Err("Administrator permission was denied.".into());
    }
    Ok(())
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
        let (paths, refused) = split_refused(paths, false);
        let before: Vec<(String, cleaner::Measure)> = paths
            .iter()
            .map(|p| (p.clone(), measure_path(Path::new(p))))
            .collect();

        if !paths.is_empty() {
            let script = build_delete_script(&paths, permanent);
            run_elevated_powershell(&script, "storage_doctor_recycle.ps1")?;
        }

        let mut removals: Vec<Removal> = Vec::new();
        let mut failed = Vec::new();
        for (path, before) in before {
            let p = Path::new(&path);
            let after = measure_path(p);
            // Whatever was removed is counted, even if some of it remains.
            // Done means: gone; or, for a permanent cache cleanup (or the
            // Recycle Bin), emptied — Windows recreates Temp and similar
            // folders straight away. A user's own item sent to the Recycle
            // Bin must be gone entirely.
            let bytes = before.size_bytes.saturating_sub(after.size_bytes);
            let removed_files = before.file_count.saturating_sub(after.file_count);
            if bytes > 0 || removed_files > 0 || !p.exists() {
                removals.push(Removal {
                    path: path.clone(),
                    bytes,
                    files: removed_files,
                    recoverable: before
                        .recoverable_bytes
                        .saturating_sub(after.recoverable_bytes),
                    vanished: !p.exists(),
                });
            }
            let emptied_ok = permanent || recycle_bin_drive(&path).is_some();
            if p.exists() && (after.file_count > 0 || !emptied_ok) {
                failed.push(path);
            }
        }
        let freed: u64 = removals.iter().map(|r| r.bytes).sum();
        sync_scan_after_delete(&app, &removals);
        log_operation(
            &app,
            &label,
            removals.len(),
            freed,
            if permanent { "permanent-admin" } else { "recycle-admin" },
        );
        Ok(DeleteResult {
            freed_bytes: freed,
            failed,
            refused,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Force delete — for the "this file is open in another program" case, once
// an elevated retry has already failed. Identifies whatever holds the item
// open via a Restart Manager session (the same mechanism Windows Update and
// MSI installers use before replacing a file — there is no other supported
// way to ask this question) and closes it; anything still locked afterward
// is scheduled to be removed the moment Windows next starts, which always
// succeeds because nothing can hold a lock across a restart before Windows
// itself processes the request.
//
// This whole mechanism was built and verified against a real locked file on
// a real machine before being wired in here — including the two sharp edges
// that would otherwise have shipped silently broken: `Marshal
// .GetLastWin32Error()` must be read in the same P/Invoke call as the API it
// reports on (a separate PowerShell statement in between clobbers it), and
// `PendingFileRenameOperations` already holds legitimate Windows Update and
// Office entries on a real machine — the script only ever appends to it via
// `MoveFileEx` itself, never rewrites the value directly.
// ---------------------------------------------------------------------------

/// C#, compiled once per script via `Add-Type`, giving the elevated
/// PowerShell process the two Win32 building blocks force-delete needs:
/// asking Restart Manager what has a path open, and scheduling a path for
/// removal on next boot.
pub(crate) const FORCE_DELETE_HELPER: &str = r#"
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;

public class SDForceDelete {
    [StructLayout(LayoutKind.Sequential)]
    struct RM_UNIQUE_PROCESS {
        public int dwProcessId;
        public System.Runtime.InteropServices.ComTypes.FILETIME ProcessStartTime;
    }
    const int CCH_RM_MAX_APP_NAME = 255;
    const int CCH_RM_MAX_SVC_NAME = 63;
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct RM_PROCESS_INFO {
        public RM_UNIQUE_PROCESS Process;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = CCH_RM_MAX_APP_NAME + 1)]
        public string strAppName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = CCH_RM_MAX_SVC_NAME + 1)]
        public string strServiceShortName;
        public int ApplicationType;
        public uint AppStatus;
        public uint TSSessionId;
        [MarshalAs(UnmanagedType.Bool)]
        public bool bRestartable;
    }

    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmStartSession(out uint pSessionHandle, int dwSessionFlags, string strSessionKey);
    [DllImport("rstrtmgr.dll")]
    static extern int RmEndSession(uint pSessionHandle);
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmRegisterResources(uint pSessionHandle, uint nFiles, string[] rgsFilenames,
        uint nApplications, RM_UNIQUE_PROCESS[] rgApplications, uint nServices, string[] rgsServiceNames);
    [DllImport("rstrtmgr.dll")]
    static extern int RmGetList(uint dwSessionHandle, out uint pnProcInfoNeeded, ref uint pnProcInfo,
        [In, Out] RM_PROCESS_INFO[] rgAffectedApps, ref uint lpdwRebootReasons);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool MoveFileEx(string lpExistingFileName, string lpNewFileName, uint dwFlags);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SHFILEOPSTRUCT {
        public IntPtr hwnd;
        public uint wFunc;
        public string pFrom;
        public string pTo;
        public ushort fFlags;
        [MarshalAs(UnmanagedType.Bool)] public bool fAnyOperationsAborted;
        public IntPtr hNameMappings;
        public string lpszProgressTitle;
    }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern int SHFileOperation(ref SHFILEOPSTRUCT lpFileOp);

    /// Sends a file or folder to the Recycle Bin with no UI at all. The
    /// VisualBasic API used before shows "File In Use" error dialogs from this
    /// hidden elevated process on every retry. Returns 0 on success. Like
    /// Explorer with confirmations off, an item too large for the Recycle Bin
    /// (or on a drive without one) is deleted permanently.
    public static int Recycle(string path) {
        var op = new SHFILEOPSTRUCT();
        op.wFunc = 3;                               // FO_DELETE
        op.pFrom = path + "\0\0";
        op.fFlags = (ushort)(0x40 | 0x10 | 0x400 | 0x4); // ALLOWUNDO|NOCONFIRMATION|NOERRORUI|SILENT
        return SHFileOperation(ref op);
    }

    /// Files to hand Restart Manager for a target. Restart Manager only
    /// reports a process as using the exact resource registered — asking about
    /// a *folder* never finds a process holding a file *inside* it. So for a
    /// directory we register every file within (bounded), which is what makes
    /// "the file is open somewhere in this folder" actually resolve.
    static string[] ResourcesFor(string path) {
        const int cap = 5000;
        var files = new List<string>();
        try {
            if (Directory.Exists(path)) {
                // Only real files may be registered — RmRegisterResources
                // rejects the whole call if a directory path is included, which
                // is why registering the folder itself found nothing.
                CollectFilesForRm(path, files, cap);
            } else if (File.Exists(path)) {
                files.Add(path);
            }
        } catch { }
        return files.ToArray();
    }

    static void CollectFilesForRm(string dir, List<string> into, int cap) {
        if (into.Count >= cap) return;
        string[] files;
        try { files = Directory.GetFiles(dir); } catch { return; }
        foreach (var f in files) { into.Add(f); if (into.Count >= cap) return; }
        string[] subs;
        try { subs = Directory.GetDirectories(dir); } catch { return; }
        foreach (var s in subs) { CollectFilesForRm(s, into, cap); if (into.Count >= cap) return; }
    }

    /// Processes that are never closed, whatever Restart Manager says: killing
    /// any of these crashes Windows, takes down unrelated services, or hard-
    /// stops a running virtual machine. They are reported to the user instead.
    static readonly HashSet<string> NeverClose = new HashSet<string>(StringComparer.OrdinalIgnoreCase) {
        "System", "Registry", "Idle", "Memory Compression", "smss", "csrss", "wininit",
        "winlogon", "services", "lsass", "lsaiso", "svchost", "dwm", "fontdrvhost",
        "sihost", "ctfmon", "audiodg", "conhost", "MsMpEng", "NisSrv", "SecurityHealthService",
        "vmwp", "vmmem", "vmcompute", "vmms", "storage-doctor"
    };

    public class Locker {
        public int Pid;
        public string Exe;      // process name, e.g. "WINWORD"
        public string App;      // Restart Manager's friendly name, e.g. "Microsoft Word"
        public string Service;  // service short name when the holder is a service
        public int Type;        // RM_APP_TYPE
        public bool Closable;
    }

    /// Everything Windows reports as holding `path` — or, for a folder, any
    /// file inside it — open. Includes the System process (PID 4) and
    /// critical processes: those are never closed, but they must still be
    /// reported, or the user is told nothing about why the delete failed.
    public static Locker[] GetLockers(string path) {
        var list = new List<Locker>();
        var resources = ResourcesFor(path);
        if (resources.Length == 0) return list.ToArray();
        uint handle;
        string key = Guid.NewGuid().ToString("N").Substring(0, 32);
        if (RmStartSession(out handle, 0, key) != 0) return list.ToArray();
        try {
            if (RmRegisterResources(handle, (uint)resources.Length, resources, 0, null, 0, null) != 0) {
                return list.ToArray();
            }
            uint needed = 0, have = 0, reasons = 0;
            int rc = RmGetList(handle, out needed, ref have, null, ref reasons);
            if ((rc != 0 && rc != 234 /* ERROR_MORE_DATA */) || needed == 0) return list.ToArray();
            var infos = new RM_PROCESS_INFO[needed];
            have = needed;
            if (RmGetList(handle, out needed, ref have, infos, ref reasons) != 0) return list.ToArray();
            int self = System.Diagnostics.Process.GetCurrentProcess().Id;
            for (int i = 0; i < have; i++) {
                int pid = infos[i].Process.dwProcessId;
                if (pid == self) continue;
                string exe = "";
                if (pid <= 4) {
                    exe = "System";
                } else {
                    try { exe = System.Diagnostics.Process.GetProcessById(pid).ProcessName; } catch { }
                    // Exited between the query and now: it no longer holds anything.
                    if (exe.Length == 0) continue;
                }
                var l = new Locker();
                l.Pid = pid;
                l.Exe = exe;
                l.App = infos[i].strAppName ?? "";
                l.Service = infos[i].strServiceShortName ?? "";
                l.Type = infos[i].ApplicationType;
                l.Closable = pid > 4
                    && l.Type != 1000 /* RmCritical */
                    && l.Type != 3    /* RmService */
                    && l.Service.Length == 0
                    && !NeverClose.Contains(exe);
                list.Add(l);
            }
        } finally {
            RmEndSession(handle);
        }
        return list.ToArray();
    }

    /// Only the lockers it is safe to close.
    public static int[] FindLockerPids(string path) {
        var pids = new List<int>();
        foreach (var l in GetLockers(path)) { if (l.Closable) pids.Add(l.Pid); }
        return pids.ToArray();
    }

    public static string DisplayName(Locker l) {
        if (l.Pid <= 4) return "Windows (System process)";
        string exe = l.Exe + ".exe";
        if (l.App.Length > 0 && !l.App.Equals(l.Exe, StringComparison.OrdinalIgnoreCase)
            && !l.App.Equals(exe, StringComparison.OrdinalIgnoreCase)) {
            return l.App + " (" + exe + ")";
        }
        return exe;
    }

    /// One line telling the user exactly what holds the item and why this
    /// app did not close it. `closedExes` = process names this run already
    /// closed once (so a survivor of the same name started again).
    public static string Describe(Locker l, HashSet<string> closedExes) {
        string name = DisplayName(l);
        if (l.Pid <= 4) {
            return "Windows itself (the System process). This happens when a virtual machine " +
                   "(Hyper-V, WSL, an app's built-in VM) or a driver is using the file. Quit the " +
                   "app that owns the virtual machine, then try again.";
        }
        string lower = l.Exe.ToLowerInvariant();
        if (lower == "vmwp" || lower == "vmmem" || lower == "vmcompute" || lower == "vmms") {
            return name + ": a running virtual machine. Quit the app that owns the virtual machine, then try again.";
        }
        if (l.Service.Length > 0 || l.Type == 3) {
            string svc = l.Service.Length > 0 ? l.Service : l.Exe;
            return name + ": the Windows service \"" + svc + "\". Services are not stopped automatically; " +
                   "stop it in the Services app, or uninstall the program it belongs to, then try again.";
        }
        if (l.Type == 1000 || NeverClose.Contains(l.Exe)) {
            return name + ": a Windows system process that cannot be closed safely.";
        }
        if (closedExes != null && closedExes.Contains(l.Exe)) {
            return name + ": it was closed, but started again and re-opened the file. " +
                   "Quit it fully (check the system tray) or disable it from starting, then try again.";
        }
        return name + ": Windows refused to close it. Close it yourself, then try again.";
    }

    static void CollectDepthFirst(string dir, List<string> into, int cap) {
        if (into.Count >= cap) return;
        foreach (var sub in Directory.GetDirectories(dir)) {
            CollectDepthFirst(sub, into, cap);
            if (into.Count >= cap) return;
            into.Add(sub);
        }
        foreach (var file in Directory.GetFiles(dir)) {
            into.Add(file);
            if (into.Count >= cap) return;
        }
    }

    /// Schedules `path` — and, for a folder, everything inside it, deepest
    /// first — to be removed the moment Windows starts up next. A folder
    /// with more than a couple of thousand entries is skipped rather than
    /// scheduled partially: PendingFileRenameOperations is a single registry
    /// value shared with Windows Update and Office, and this app has no
    /// business growing it without bound.
    /// 0 = scheduled; 1 = folder too large to schedule; 2 = Windows refused
    /// (or the folder could not be listed); 3 = the path no longer exists.
    public static int ScheduleForReboot(string path) {
        const int cap = 2000;
        if (Directory.Exists(path)) {
            var entries = new List<string>();
            try { CollectDepthFirst(path, entries, cap); } catch { return 2; }
            if (entries.Count >= cap) return 1;
            foreach (var p in entries) MoveFileEx(p, null, 0x4);
            // The folder's own entry decides the outcome: it is what the
            // caller later checks for, and it is added last.
            return MoveFileEx(path, null, 0x4) ? 0 : 2;
        }
        if (File.Exists(path)) {
            return MoveFileEx(path, null, 0x4) ? 0 : 2;
        }
        return 3;
    }
}
"@
"#;

/// Where the elevated force-delete script records the names of whatever still
/// holds a path open after every attempt — so the caller can tell the user
/// what to close, instead of falsely promising a restart will remove it.
/// A unique report file per force-delete run, so two right-click deletes
/// started together can't read each other's results.
fn new_force_report_path() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "storage_doctor_fd_report_{}_{stamp}.txt",
        std::process::id()
    ))
}

/// Builds the elevated force-delete script. For each path: close what holds
/// it open (only processes that are safe to close) and retry the delete. If
/// it still exists afterwards, it records exactly what holds it and does NOT
/// schedule a restart deletion — whatever holds it (a VM, a service, an app
/// that re-launches) would simply hold it again, and promising otherwise was
/// a lie. A restart deletion is scheduled only when nothing holds it at all.
///
/// Every outcome is written as a tab-separated line to `report`:
/// `closed\t<path>\t<program>`, `blocked\t<path>\t<explanation>`,
/// `unschedulable\t<path>\t`, and a final `done` line proving the script ran
/// to the end (it is absent if the UAC prompt was declined).
pub(crate) fn build_force_delete_script(paths: &[String], permanent: bool, report: &Path) -> String {
    let mut script = String::from("Add-Type -AssemblyName Microsoft.VisualBasic\n");
    script.push_str(FORCE_DELETE_HELPER);
    script.push_str("$sdOut = New-Object 'System.Collections.Generic.List[string]'\n");

    for path in paths {
        let esc = path.replace('\'', "''");
        script.push_str(&robust_remove_block(&esc, permanent, false));
        script.push_str(&format!(
            "if (Test-Path -LiteralPath '{esc}') {{\n\
                $sdLeft = @(); try {{ $sdLeft = @([SDForceDelete]::GetLockers('{esc}')) }} catch {{ }}\n\
                if ($sdLeft.Count -gt 0) {{\n\
                    foreach ($l in $sdLeft) {{ $sdOut.Add(\"blocked`t\" + '{esc}' + \"`t\" + [SDForceDelete]::Describe($l, $sdClosedExes)) }}\n\
                }} else {{\n\
                    $sdR = [SDForceDelete]::ScheduleForReboot('{esc}')\n\
                    if ($sdR -ne 0) {{ $sdOut.Add(\"unschedulable`t\" + '{esc}' + \"`t\" + $sdR) }}\n\
                }}\n\
             }}\n"
        ));
    }

    let rf = report.to_string_lossy().replace('\'', "''");
    script.push_str(&format!(
        "$sdOut.Add('done')\n\
         try {{ [System.IO.File]::WriteAllLines('{rf}', $sdOut) }} catch {{ }}\n"
    ));
    script
}

/// What the elevated force-delete script reported, parsed from its file.
#[derive(Default)]
struct ForceReport {
    /// The script ran to the end (false if UAC was declined or it crashed).
    completed: bool,
    /// (path, program closed for it)
    closed: Vec<(String, String)>,
    /// (path, explanation of what still holds it)
    blocked: Vec<(String, String)>,
    /// Paths nothing holds, but which could not even be scheduled, with the
    /// `ScheduleForReboot` code (1 = too many entries, 2 = Windows refused).
    unschedulable: Vec<(String, String)>,
}

impl ForceReport {
    fn blocked_for(&self, path: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .blocked
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, why)| why.clone())
            .collect();
        v.dedup();
        v
    }
    fn closed_programs(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for (_, name) in &self.closed {
            if !v.contains(name) {
                v.push(name.clone());
            }
        }
        v
    }
}

/// Reads and deletes the report file.
fn read_force_report(report: &Path) -> ForceReport {
    let text = std::fs::read_to_string(report).unwrap_or_default();
    let _ = std::fs::remove_file(report);
    let mut r = ForceReport::default();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let kind = parts.next().unwrap_or("").trim_start_matches('\u{feff}');
        let path = parts.next().unwrap_or("").to_string();
        let detail = parts.next().unwrap_or("").to_string();
        match kind {
            "done" => r.completed = true,
            "closed" => r.closed.push((path, detail)),
            "blocked" => r.blocked.push((path, detail)),
            "unschedulable" => r.unschedulable.push((path, detail)),
            _ => {}
        }
    }
    r
}

/// Closing File Explorer to free a file also removes the taskbar; bring it
/// back from this (non-elevated) process so it runs as the user, not admin.
fn ensure_explorer_running() {
    use std::os::windows::process::CommandExt;
    let running = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq explorer.exe", "/NH"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("explorer.exe"))
        .unwrap_or(true);
    if !running {
        if crate::elevation::is_elevated() {
            // Started from an elevated app it would become an elevated
            // desktop; start it with an ordinary user's rights instead.
            let _ = std::process::Command::new("runas")
                .args(["/trustlevel:0x20000", "explorer.exe"])
                .creation_flags(0x0800_0000)
                .spawn();
        } else {
            let _ = std::process::Command::new("explorer.exe").spawn();
        }
    }
}

/// The shared "remove one stubborn path" routine, used by both force-delete
/// and force-uninstall. `esc` is a PowerShell-single-quote-escaped path; the
/// caller must have emitted `FORCE_DELETE_HELPER` earlier.
///
/// Each cycle: close the processes holding the path (or a file within it)
/// that are safe to close — never Windows itself, services, critical system
/// processes, a virtual machine, or this app — then retry the delete, since
/// handles are released asynchronously after a kill and a program can
/// re-open the file. If only uncloseable holders remain on two checks in a
/// row, it stops early: retrying cannot succeed. Ownership is taken (to get
/// past restrictive permissions) only when nothing holds the item open, so a
/// file that ends up not deleted is not left with changed ownership.
///
/// Programs closed are appended to `$sdOut` when the caller defined it.
/// `schedule_on_failure` schedules removal at the next restart if the item
/// still exists afterwards (force-uninstall uses it; force-delete decides
/// for itself).
pub(crate) fn robust_remove_block(esc: &str, permanent: bool, schedule_on_failure: bool) -> String {
    let delete_attempt = if permanent {
        format!(
            "try {{ Remove-Item -LiteralPath '{esc}' -Recurse -Force -ErrorAction SilentlyContinue }} catch {{ }}\n"
        )
    } else {
        recycle_item_script_commands(esc)
    };
    let schedule = if schedule_on_failure {
        format!("if (Test-Path -LiteralPath '{esc}') {{ [SDForceDelete]::ScheduleForReboot('{esc}') | Out-Null }}\n")
    } else {
        String::new()
    };
    format!(
        "if ($null -eq $sdClosedExes) {{ $sdClosedExes = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase) }}\n\
         $sdStuck = 0\n\
         for ($i = 0; $i -lt 10; $i++) {{\n\
            if (-not (Test-Path -LiteralPath '{esc}')) {{ break }}\n\
            $sdL = @(); try {{ $sdL = @([SDForceDelete]::GetLockers('{esc}')) }} catch {{ }}\n\
            foreach ($l in $sdL) {{ if ($l.Closable) {{ try {{\n\
                $sdName = [SDForceDelete]::DisplayName($l)\n\
                Stop-Process -Id $l.Pid -Force -ErrorAction Stop\n\
                [void]$sdClosedExes.Add($l.Exe)\n\
                if ($null -ne $sdOut) {{ $sdOut.Add(\"closed`t\" + '{esc}' + \"`t\" + $sdName) }}\n\
            }} catch {{ }} }} }}\n\
            $sdHeld = @($sdL | Where-Object {{ -not $_.Closable }}).Count\n\
            if ($sdHeld -gt 0) {{ $sdStuck++; if ($sdStuck -ge 2) {{ break }} }} else {{ $sdStuck = 0 }}\n\
            if ($i -eq 1 -and $sdL.Count -eq 0) {{\n\
                try {{ & takeown /f '{esc}' /r /d y *>$null }} catch {{ }}\n\
                try {{ & icacls '{esc}' /grant '*S-1-5-32-544:F' /t /c *>$null }} catch {{ }}\n\
            }}\n\
            Start-Sleep -Milliseconds 300\n\
            if ($sdHeld -eq 0) {{\n\
            {delete_attempt}\
            }}\n\
         }}\n\
         {schedule}"
    )
}

/// Silent Recycle Bin removal (or permanent, when the item cannot go to the
/// Recycle Bin — see `SDForceDelete.Recycle`).
fn recycle_item_script_commands(esc: &str) -> String {
    format!("try {{ [void][SDForceDelete]::Recycle('{esc}') }} catch {{ }}\n")
}

/// True only if `path` itself is registered in PendingFileRenameOperations
/// for deletion at the next restart (a folder's own entry is always added,
/// last). Exact match: a substring test would also match `C:\a\bc` for
/// `C:\a\b`. Entries are stored as `\??\C:\path`; deletions have an empty
/// destination. Reading this key needs no elevation.
pub(crate) fn has_pending_reboot_delete(path: &str) -> bool {
    let Ok(key) =
        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager")
    else {
        return false;
    };
    // Raw REG_MULTI_SZ, decoded by hand: the typed reader drops the empty
    // strings that mark a deletion, which would misalign the pairs.
    let Ok(raw) = key.get_raw_value("PendingFileRenameOperations") else {
        return false;
    };
    let wide: Vec<u16> = raw
        .bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let text = String::from_utf16_lossy(&wide);
    let mut parts: Vec<&str> = text.split('\0').collect();
    // Drop the list terminator (trailing empty strings after the last pair).
    while parts.len() % 2 == 1 || (parts.len() >= 2 && parts[parts.len() - 1].is_empty() && parts[parts.len() - 2].is_empty()) {
        parts.pop();
    }
    pending_delete_pairs_contain(&parts, path)
}

/// `parts` = PendingFileRenameOperations as alternating (source, destination)
/// strings. A deletion is a pair whose destination is empty. Sources are
/// stored as `*1\??\C:\path` on current Windows (a `*<n>` flag prefix, then
/// the NT `\??\` prefix) and as plain `\??\C:\path` on older entries.
fn pending_delete_pairs_contain(parts: &[&str], path: &str) -> bool {
    let norm = |s: &str| {
        let s = match s.strip_prefix('*') {
            Some(rest) => rest.trim_start_matches(|c: char| c.is_ascii_digit()),
            None => s,
        };
        s.trim_start_matches('!')
            .trim_start_matches(r"\??\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    let needle = norm(path);
    parts
        .chunks_exact(2)
        .any(|pair| pair[1].is_empty() && !pair[0].is_empty() && norm(pair[0]) == needle)
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ForceDeleteResult {
    /// Bytes of the items that are now gone. When they went to the Recycle
    /// Bin this space is NOT free yet — the UI words it accordingly.
    pub freed_bytes: u64,
    /// Gone from their location.
    pub removed: Vec<String>,
    /// Programs this run closed to free the items (their unsaved work is lost).
    pub closed_programs: Vec<String>,
    /// Still there, held open by something this app does not close — each
    /// with the exact explanation of what holds it and what to do.
    pub blocked: Vec<BlockedItem>,
    /// Still there although nothing holds it open; set to be deleted
    /// (permanently) during the next full restart. Not a promise: if a
    /// program recreates the item at startup it will be back.
    pub scheduled_for_reboot: Vec<String>,
    /// Still there, nothing reported as holding it, and it could not be
    /// scheduled either — each with the specific reason.
    pub failed: Vec<BlockedItem>,
    /// The administrator step did not run to completion.
    pub elevation_declined: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BlockedItem {
    pub path: String,
    pub reasons: Vec<String>,
}

/// Force-deletes paths that failed even an administrator-elevated delete —
/// offered only once that has already been tried, since this exists
/// specifically for the case elevation alone cannot fix: the item is open
/// somewhere, not merely permission-denied. One UAC prompt.
#[tauri::command]
pub async fn force_delete_paths(
    app: AppHandle,
    paths: Vec<String>,
    source: Option<String>,
    permanent: Option<bool>,
) -> Result<ForceDeleteResult, String> {
    let label = source.unwrap_or_else(|| "Force delete".into());
    let permanent = permanent.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let (paths, refused) = split_refused(paths, true);
        let before: Vec<(String, cleaner::Measure)> = paths
            .iter()
            .map(|p| (p.clone(), measure_path(Path::new(p))))
            .collect();

        let report = if paths.is_empty() {
            ForceReport { completed: true, ..Default::default() }
        } else {
            let report_path = new_force_report_path();
            let script = build_force_delete_script(&paths, permanent, &report_path);
            let run = run_elevated_powershell(&script, "storage_doctor_force_delete.ps1");
            let report = read_force_report(&report_path);
            ensure_explorer_running();
            if let Err(e) = run {
                if !report.completed {
                    return Err(e);
                }
            }
            report
        };

        let mut removals: Vec<Removal> = Vec::new();
        let mut blocked = Vec::new();
        let mut scheduled_for_reboot = Vec::new();
        // Refused items are reported with their reason, never processed.
        let mut failed = refused;

        for (path, prior) in before {
            let reasons = report.blocked_for(&path);
            let p = Path::new(&path);
            if !p.exists() {
                let after = measure_path(p);
                removals.push(Removal {
                    path: path.clone(),
                    bytes: prior.size_bytes.saturating_sub(after.size_bytes),
                    files: prior.file_count.saturating_sub(after.file_count),
                    recoverable: prior
                        .recoverable_bytes
                        .saturating_sub(after.recoverable_bytes),
                    vanished: true,
                });
            } else if !reasons.is_empty() {
                blocked.push(BlockedItem { path, reasons });
            } else if report.completed && has_pending_reboot_delete(&path) {
                scheduled_for_reboot.push(path);
            } else {
                let reason = if !report.completed {
                    "The administrator step stopped before finishing, so this item was not processed."
                } else if report.unschedulable.iter().any(|(p, c)| p == &path && c.trim() == "1") {
                    "No program has it open, but Windows refused to delete it, and it holds more than 2,000 files and folders — too many to schedule for deletion at restart."
                } else {
                    "No program has it open, but Windows refused to delete it and refused to schedule it for deletion at restart."
                };
                failed.push(BlockedItem { path, reasons: vec![reason.to_string()] });
            }
        }

        let freed: u64 = removals.iter().map(|r| r.bytes).sum();
        let removed: Vec<String> = removals.iter().map(|r| r.path.clone()).collect();
        sync_scan_after_delete(&app, &removals);
        log_operation(
            &app,
            &label,
            removals.len(),
            freed,
            if permanent { "force-permanent" } else { "force-recycle" },
        );

        Ok(ForceDeleteResult {
            freed_bytes: freed,
            removed,
            closed_programs: report.closed_programs(),
            blocked,
            scheduled_for_reboot,
            failed,
            elevation_declined: !report.completed,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Re-measures the current recommendations against the latest scan so their
/// recoverable sizes reflect what was just cleaned (folders now empty drop off).
#[tauri::command(async)]
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
#[tauri::command(async)]
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
    if url.contains('"') {
        return Err("Invalid URL.".into());
    }
    // Handed to Explorer, which passes it to the already-running desktop:
    // the browser opens with normal rights even when this app runs as
    // administrator.
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("\"{url}\""))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Built-in and Microsoft Store apps (no administrator rights needed to list).
#[tauri::command]
pub async fn list_store_apps() -> Result<Vec<crate::store_apps::StoreApp>, String> {
    tauri::async_runtime::spawn_blocking(crate::store_apps::list)
        .await
        .map_err(|e| e.to_string())?
}

/// Removes a built-in / Store app for every user account and stops Windows
/// adding it back for new accounts. One administrator pass (no prompt in a
/// "Run as administrator" session). The result is what is actually left,
/// re-checked afterwards.
#[tauri::command]
pub async fn remove_store_app(name: String) -> Result<crate::store_apps::StoreRemoveResult, String> {
    if !crate::store_apps::valid_package_name(&name) {
        return Err("Invalid app package name.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let report = new_force_report_path();
        let script = crate::store_apps::build_remove_script(&name, &report);
        let run = run_elevated_powershell(&script, "storage_doctor_store_remove.ps1");
        let text = std::fs::read_to_string(&report).unwrap_or_default();
        let _ = std::fs::remove_file(&report);
        if text.is_empty() {
            return Err(run.err().unwrap_or_else(|| "The administrator step did not run.".into()));
        }
        Ok(crate::store_apps::parse_remove_report(&text))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AdminStatus {
    /// This session is running with administrator rights.
    pub elevated: bool,
    /// "Run as administrator" is turned on (applies at every launch).
    pub enabled: bool,
}

#[tauri::command(async)]
pub fn admin_status() -> AdminStatus {
    AdminStatus {
        elevated: crate::elevation::is_elevated(),
        enabled: crate::elevation::run_as_admin_enabled(),
    }
}

#[tauri::command(async)]
pub fn set_run_as_admin(enabled: bool) -> Result<(), String> {
    crate::elevation::set_run_as_admin(enabled)
}

/// Restarts Storage Doctor with administrator rights now (one UAC prompt).
#[tauri::command]
pub fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    if crate::elevation::relaunch_elevated() {
        app.exit(0);
        Ok(())
    } else {
        Err("Administrator permission was declined.".into())
    }
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

/// The installed-app list is three registry hives and several hundred
/// subkeys. Search re-runs it on every keystroke and the Applications page on
/// every visit, so hold it briefly rather than rebuilding it each time.
fn installed_apps_cached() -> Vec<uninstall::InstalledApp> {
    static CACHE: Mutex<Option<(Instant, Vec<uninstall::InstalledApp>)>> = Mutex::new(None);
    const TTL: Duration = Duration::from_secs(20);

    if let Ok(guard) = CACHE.lock() {
        if let Some((measured_at, apps)) = guard.as_ref() {
            if measured_at.elapsed() < TTL {
                return apps.clone();
            }
        }
    }
    let apps = uninstall::list_installed();
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((Instant::now(), apps.clone()));
    }
    apps
}

/// Search across scanned folders, large files, recommendations and
/// installed applications.
///
/// Declared `async` so Tauri runs it off the main thread — as a plain sync
/// command it blocked the UI thread for the whole registry sweep, which is
/// what made typing in the search box feel like it stuttered.
#[tauri::command(async)]
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

    let apps: Vec<SearchResultItem> = installed_apps_cached()
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

/// Incremental search feedback: the matches found since the previous event,
/// so the UI can list results while the walk is still running instead of
/// staring at a spinner until every drive has been read.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProgress {
    pub query: String,
    pub found: usize,
    pub done: bool,
    pub items: Vec<SearchResultItem>,
}

const SEARCH_RESULT_CAP: usize = 500;
const SEARCH_EMIT_INTERVAL: Duration = Duration::from_millis(120);

/// Bumped by every new search and by `cancel_search`; a walk whose generation
/// no longer matches unwinds instead of finishing work nobody is waiting for.
static SEARCH_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Directories that are never worth walking: the component store holds
/// hundreds of thousands of hard-linked system files, and the other two are
/// either inaccessible or hold mangled names that cannot match a query.
const SKIP_DIRS: [&str; 3] = ["winsxs", "$recycle.bin", "system volume information"];

/// Reparse point (junction or symlink). Following these re-walks the same
/// bytes repeatedly — `C:\Users\All Users` alone would drag the whole of
/// ProgramData through a second time.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

struct SearchCtx {
    query: String,
    app: AppHandle,
    generation: u64,
    results: Mutex<Vec<SearchResultItem>>,
    /// Matches not yet pushed to the UI.
    pending: Mutex<Vec<SearchResultItem>>,
    found: AtomicUsize,
    last_emit: Mutex<Instant>,
}

impl SearchCtx {
    /// True once the cap is hit or a newer search superseded this one.
    fn finished(&self) -> bool {
        self.found.load(Ordering::Relaxed) >= SEARCH_RESULT_CAP
            || SEARCH_GENERATION.load(Ordering::Relaxed) != self.generation
    }
}

/// Live filesystem search for files/folders whose name contains the query,
/// across all fixed drives. Unlike `search`, this finds files of any size
/// (they are not all indexed by a scan). Matches stream out as
/// `search-progress` events; the returned list is the sorted final set.
#[tauri::command]
pub async fn search_files(
    app: AppHandle,
    query: String,
) -> Result<Vec<SearchResultItem>, String> {
    let generation = SEARCH_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
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
            query: q.clone(),
            app: app.clone(),
            generation,
            results: Mutex::new(Vec::new()),
            pending: Mutex::new(Vec::new()),
            found: AtomicUsize::new(0),
            last_emit: Mutex::new(Instant::now()),
        };
        roots.par_iter().for_each(|root| search_dir(root, &ctx));

        // Final event carries whatever is still buffered plus the done flag.
        let tail = ctx.pending.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        let _ = app.emit(
            "search-progress",
            SearchProgress {
                query: q,
                found: ctx.found.load(Ordering::Relaxed),
                done: true,
                items: tail,
            },
        );

        let mut out = ctx.results.into_inner().unwrap_or_default();
        out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        out.truncate(SEARCH_RESULT_CAP);
        out
    })
    .await
    .map_err(|e| e.to_string())
}

/// Abandons the running deep search (the user navigated away or typed a new
/// query). The walk notices on its next directory and stops.
#[tauri::command]
pub fn cancel_search() {
    SEARCH_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Pushes buffered matches to the UI, at most once per interval.
fn flush_search_results(ctx: &SearchCtx) {
    let Ok(mut last) = ctx.last_emit.try_lock() else {
        return;
    };
    if last.elapsed() < SEARCH_EMIT_INTERVAL {
        return;
    }
    let items = ctx
        .pending
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default();
    if items.is_empty() {
        return;
    }
    *last = Instant::now();
    let _ = ctx.app.emit(
        "search-progress",
        SearchProgress {
            query: ctx.query.clone(),
            found: ctx.found.load(Ordering::Relaxed),
            done: false,
            items,
        },
    );
}

fn search_dir(dir: &Path, ctx: &SearchCtx) {
    if ctx.finished() {
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
        let raw_name = entry.file_name();
        let name = raw_name.to_string_lossy().to_lowercase();
        if name.contains(&ctx.query) {
            let (is_dir, size) = if file_type.is_dir() {
                (true, 0)
            } else {
                (false, entry.metadata().map(|m| m.len()).unwrap_or(0))
            };
            if ctx.found.fetch_add(1, Ordering::Relaxed) < SEARCH_RESULT_CAP {
                let item = SearchResultItem {
                    kind: if is_dir { "folder".into() } else { "file".into() },
                    name: raw_name.to_string_lossy().into_owned(),
                    path: entry.path().to_string_lossy().into_owned(),
                    size_bytes: size,
                };
                if let Ok(mut results) = ctx.results.lock() {
                    results.push(item.clone());
                }
                if let Ok(mut pending) = ctx.pending.lock() {
                    pending.push(item);
                }
                flush_search_results(ctx);
            }
        }
        if file_type.is_dir() && !SKIP_DIRS.contains(&name.as_str()) && !is_reparse_point(&entry) {
            subdirs.push(entry.path());
        }
    }
    subdirs.par_iter().for_each(|sub| search_dir(sub, ctx));
}

fn is_reparse_point(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    // On Windows this reads the data the directory listing already returned —
    // no extra syscall.
    entry
        .metadata()
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
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

/// Moves a file to the Recycle Bin (never a permanent delete) and folds the
/// change into the stored scan so every screen reflects it immediately.
#[tauri::command(async)]
pub fn delete_file(app: AppHandle, path: String) -> Result<(), String> {
    let target = Path::new(&path);
    let before = measure_path(target);
    trash::delete(&path).map_err(|e| e.to_string())?;
    sync_scan_after_delete(
        &app,
        &[Removal {
            path: path.clone(),
            bytes: before.size_bytes,
            files: before.file_count,
            recoverable: before.recoverable_bytes,
            vanished: !target.exists(),
        }],
    );
    Ok(())
}

/// A native Windows message box with no owner window — used by the headless
/// right-click force-delete path, which never starts the webview UI.
fn message_box(text: &str, flags: u32) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text).as_ptr(),
            wide("Storage Doctor").as_ptr(),
            flags,
        )
    }
}

/// The Explorer right-click "Force delete with Storage Doctor" verb, handled
/// entirely with native dialogs — no webview, no dashboard, so it is instant.
///
/// Recycle first (enough, and fast, when nothing holds the item open); if it
/// is locked, one elevated pass closes whatever has it open and retries, or
/// schedules it for removal on the next restart. Every outcome is reported in
/// a native box. This is what `main` runs instead of `run()` when launched
/// with `--force-delete "<path>"`.
pub fn force_delete_cli(path: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST,
        MB_YESNO,
    };
    let front = MB_SETFOREGROUND | MB_TOPMOST;

    if !Path::new(path).exists() {
        message_box(
            &format!("This item no longer exists:\n{path}"),
            front | MB_ICONINFORMATION,
        );
        return;
    }

    if let Some(why) = delete_refusal(path, true) {
        message_box(&format!("Not deleted.\n\n{why}\n\n{path}"), front | MB_ICONERROR);
        return;
    }

    let confirm = message_box(
        &format!(
            "Force delete this item?\n\n{path}\n\n\
             • Programs that have it open will be closed. Unsaved work in them is lost.\n\
             • It goes to the Recycle Bin. An item too large for the Recycle Bin is \
             deleted permanently."
        ),
        front | MB_YESNO | MB_ICONWARNING,
    );
    if confirm != IDYES {
        return;
    }

    // 1) Plain recycle — fast, and all that is needed when nothing holds it.
    let _ = trash::delete(path);
    if !Path::new(path).exists() {
        message_box(
            &format!("Moved to the Recycle Bin:\n{path}"),
            front | MB_ICONINFORMATION,
        );
        return;
    }

    // 2) Still there → locked or permission-denied. One elevated pass.
    let report_path = new_force_report_path();
    let script = build_force_delete_script(&[path.to_string()], false, &report_path);
    let run = run_elevated_powershell(&script, "storage_doctor_force_delete_cli.ps1");
    let report = read_force_report(&report_path);
    ensure_explorer_running();

    let closed = report.closed_programs();
    let closed_note = if closed.is_empty() {
        String::new()
    } else {
        format!("\n\nClosed to free it:\n• {}", closed.join("\n• "))
    };

    if !Path::new(path).exists() {
        message_box(
            &format!(
                "Deleted (sent to the Recycle Bin, or deleted permanently if it was too large \
                 for it):\n{path}{closed_note}"
            ),
            front | MB_ICONINFORMATION,
        );
        return;
    }

    if !report.completed {
        let why = run.err().unwrap_or_else(|| "The administrator step did not finish.".into());
        message_box(
            &format!(
                "Not deleted. Windows refused a normal delete, and the administrator step \
                 did not run:\n{why}\n\n{path}"
            ),
            front | MB_ICONERROR,
        );
        return;
    }

    let reasons = report.blocked_for(path);
    if !reasons.is_empty() {
        message_box(
            &format!(
                "Not deleted — it is still in use by:\n\n• {}\n\nNothing was scheduled for \
                 a restart: whatever holds it would hold it again.{closed_note}\n\n{path}",
                reasons.join("\n• ")
            ),
            front | MB_ICONERROR,
        );
    } else if has_pending_reboot_delete(path) {
        message_box(
            &format!(
                "Not deleted yet. No program has it open, but Windows still refused to \
                 delete it now.\n\nIt is scheduled to be deleted permanently (not to the \
                 Recycle Bin) during the next restart — use Start ▸ Power ▸ Restart, not \
                 Shut down. If a program creates it again when Windows starts, it will \
                 be back.{closed_note}\n\n{path}"
            ),
            front | MB_ICONWARNING,
        );
    } else if report
        .unschedulable
        .iter()
        .any(|(p, code)| p == path && code.trim() == "1")
    {
        message_box(
            &format!(
                "Not deleted. No program has it open, but Windows refused to delete it, and \
                 it holds more than 2,000 files and folders — too many to schedule for \
                 deletion at restart.{closed_note}\n\n{path}"
            ),
            front | MB_ICONERROR,
        );
    } else {
        message_box(
            &format!(
                "Not deleted, and it could not be scheduled for deletion at restart \
                 (Windows refused the request).{closed_note}\n\n{path}"
            ),
            front | MB_ICONERROR,
        );
    }
}

/// Whether the "Force delete with Storage Doctor" entry is currently in
/// Explorer's right-click menu for this user.
#[tauri::command(async)]
pub fn context_menu_enabled() -> bool {
    shell::is_registered()
}

/// Adds or removes the Explorer right-click entry (per-user, no admin).
#[tauri::command(async)]
pub fn set_context_menu_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        shell::register()
    } else {
        shell::unregister()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Diagnostic: writes the exact script the app runs for the path in
    /// SD_FDTEST_PATH to SD_FDTEST_OUT, so it can be run by hand against a
    /// genuinely locked file.
    ///   SD_FDTEST_PATH=... SD_FDTEST_OUT=... cargo test --lib \
    ///     commands::tests::dump_force_delete_script -- --ignored
    #[test]
    #[ignore]
    fn dump_force_delete_script() {
        let path = std::env::var("SD_FDTEST_PATH").expect("SD_FDTEST_PATH");
        let out = std::env::var("SD_FDTEST_OUT").expect("SD_FDTEST_OUT");
        let report = std::env::var("SD_FDTEST_REPORT").expect("SD_FDTEST_REPORT");
        std::fs::write(
            &out,
            build_force_delete_script(&[path], false, Path::new(&report)),
        )
        .unwrap();
    }

    /// LIVE, elevated (one UAC prompt): runs the real force-delete pipeline
    /// on SD_LIVE_PATH and prints the outcome exactly as the app classifies it.
    #[test]
    #[ignore]
    fn live_force_delete() {
        let path = std::env::var("SD_LIVE_PATH").expect("SD_LIVE_PATH");
        let report_path = new_force_report_path();
        let script = build_force_delete_script(&[path.clone()], false, &report_path);
        let run = run_elevated_powershell(&script, "sd_live_fd.ps1");
        let r = read_force_report(&report_path);
        println!("ELEVATED_PROCESS={}", crate::elevation::is_elevated());
        println!("RUN={run:?}");
        println!("COMPLETED={}", r.completed);
        println!("EXISTS_AFTER={}", Path::new(&path).exists());
        println!("CLOSED={:?}", r.closed_programs());
        println!("BLOCKED={:?}", r.blocked_for(&path));
        println!("UNSCHEDULABLE={:?}", r.unschedulable);
        println!("PENDING_REBOOT={}", has_pending_reboot_delete(&path));
    }

    /// LIVE, elevated, READ-ONLY: reports what holds SD_LIVE_PATH open, using
    /// the same GetLockers/Describe the app uses. Closes nothing, deletes
    /// nothing — safe to point at a file in active use.
    #[test]
    #[ignore]
    fn live_describe_lockers() {
        let path = std::env::var("SD_LIVE_PATH").expect("SD_LIVE_PATH");
        let report = new_force_report_path();
        let esc = path.replace('\'', "''");
        let rf = report.to_string_lossy().replace('\'', "''");
        let script = format!(
            "{FORCE_DELETE_HELPER}\n$o = New-Object 'System.Collections.Generic.List[string]'\n\
             foreach ($l in @([SDForceDelete]::GetLockers('{esc}'))) {{ $o.Add(\"lock`t\" + $l.Pid + ' ' + $l.Exe + ' closable=' + $l.Closable + \"`t\" + [SDForceDelete]::Describe($l, $null)) }}\n\
             $o.Add('done')\n[System.IO.File]::WriteAllLines('{rf}', $o)\n"
        );
        let run = run_elevated_powershell(&script, "sd_live_describe.ps1");
        println!("RUN={run:?}");
        println!("{}", std::fs::read_to_string(&report).unwrap_or_default());
        let _ = std::fs::remove_file(&report);
    }

    /// LIVE, elevated: schedules SD_LIVE_PATH (a scratch file) for deletion at
    /// the next restart via the app's own ScheduleForReboot, then checks the
    /// exact-match detection against the real registry value.
    #[test]
    #[ignore]
    fn live_schedule_detection() {
        let path = std::env::var("SD_LIVE_PATH").expect("SD_LIVE_PATH");
        let esc = path.replace('\'', "''");
        let script = format!("{FORCE_DELETE_HELPER}\n[void][SDForceDelete]::ScheduleForReboot('{esc}')\n");
        println!("RUN={:?}", run_elevated_powershell(&script, "sd_live_sched.ps1"));
        println!("EXACT={}", has_pending_reboot_delete(&path));
        println!("PREFIX_ONLY={}", has_pending_reboot_delete(&path[..path.len() - 1]));
        println!("LONGER={}", has_pending_reboot_delete(&format!("{path}x")));
    }

    fn script_for(path: &str, permanent: bool) -> String {
        build_force_delete_script(&[path.to_string()], permanent, Path::new(r"C:\r.txt"))
    }

    #[test]
    fn force_delete_script_closes_lockers_before_deleting() {
        let script = script_for(r"C:\stuck\file.txt", false);
        assert!(script.contains("Stop-Process -Id $l.Pid"));
        assert!(script.contains("[SDForceDelete]::Recycle('C:\\stuck\\file.txt')"));
        // No VisualBasic recycle in the force path: it pops "File In Use"
        // dialogs from the hidden elevated process.
        assert!(!script.contains("SendToRecycleBin"));
        assert!(!script.contains(r"Remove-Item -LiteralPath 'C:\stuck\file.txt' -Recurse"));
        let kill_pos = script.find("Stop-Process -Id $l.Pid").unwrap();
        let delete_pos = script.find("::Recycle('").unwrap();
        assert!(kill_pos < delete_pos, "processes must be stopped before deleting");
    }

    #[test]
    fn force_delete_only_closes_processes_marked_closable() {
        let script = script_for(r"C:\stuck\file.txt", false);
        assert!(script.contains("if ($l.Closable)"));
        // Never: System, services, critical processes, the VM worker, itself.
        for name in ["\"svchost\"", "\"csrss\"", "\"lsass\"", "\"vmwp\"", "\"vmmem\"", "\"storage-doctor\""] {
            assert!(script.contains(name), "{name} must be in the never-close list");
        }
        assert!(script.contains("l.Type != 3"), "services are never killed");
        assert!(script.contains("l.Type != 1000"), "critical processes are never killed");
        assert!(script.contains("pid > 4"), "System is never killed");
    }

    #[test]
    fn force_delete_never_schedules_a_restart_when_something_holds_the_item() {
        let script = script_for(r"C:\stuck\file.txt", false);
        // The force-delete routine itself must not schedule…
        let routine = robust_remove_block("C:\\x", false, false);
        assert!(!routine.contains("ScheduleForReboot"));
        // …and the script schedules only in the branch where no locker remains.
        let blocked = script.find("if ($sdLeft.Count -gt 0)").unwrap();
        let sched = script.find("$sdR = [SDForceDelete]::ScheduleForReboot").unwrap();
        let else_pos = script[blocked..].find("} else {").unwrap() + blocked;
        assert!(sched > else_pos, "scheduling must be in the no-locker branch only");
        // Force-uninstall keeps its restart fallback.
        assert!(robust_remove_block("C:\\x", false, true).contains("ScheduleForReboot"));
    }

    #[test]
    fn ownership_is_taken_only_when_nothing_holds_the_item() {
        let routine = robust_remove_block("C:\\x", false, false);
        assert!(routine.contains("if ($i -eq 1 -and $sdL.Count -eq 0)"));
    }

    #[test]
    fn force_delete_script_permanent_mode_skips_the_recycle_bin() {
        let script = script_for(r"C:\stuck\file.txt", true);
        assert!(script.contains(r"Remove-Item -LiteralPath 'C:\stuck\file.txt' -Recurse -Force"));
        assert!(!script.contains("::Recycle('"));
    }

    #[test]
    fn force_delete_script_retries_the_delete_and_is_folder_aware() {
        let script = script_for(r"C:\stuck", false);
        assert!(script.contains("for ($i = 0; $i -lt 10; $i++)"));
        let loop_pos = script.find("for ($i = 0; $i -lt 10").unwrap();
        let call_pos = script.find("[SDForceDelete]::GetLockers('C:\\stuck')").unwrap();
        assert!(call_pos > loop_pos, "locker lookup must be inside the retry loop");
        assert!(script.contains("CollectFilesForRm"));
        assert!(script.contains("ResourcesFor"));
        // Report written and terminated with a completion marker.
        assert!(script.contains("$sdOut.Add('done')"));
        assert!(script.contains(r"WriteAllLines('C:\r.txt', $sdOut)"));
    }

    #[test]
    fn force_delete_script_escapes_single_quotes_in_paths() {
        let script = script_for(r"C:\it's\stuck.txt", false);
        assert!(script.contains(r"it''s"));
        assert!(!script.contains(r"'C:\it's\stuck.txt'"));
    }

    #[test]
    fn refuses_drives_core_folders_and_force_inside_windows_only() {
        let win = std::env::var("SystemRoot").unwrap();
        let pf = std::env::var("ProgramFiles").unwrap();
        let profile = std::env::var("USERPROFILE").unwrap();
        // Drives and core folders: refused for every kind of delete.
        for p in ["C:\\", "C:", win.as_str(), pf.as_str(), profile.as_str()] {
            assert!(delete_refusal(p, false).is_some(), "{p} must be refused");
            assert!(delete_refusal(p, true).is_some(), "{p} must be refused");
        }
        assert!(delete_refusal(&format!("{win}\\"), false).is_some(), "trailing slash");
        assert!(delete_refusal(&win.to_uppercase(), false).is_some(), "case-insensitive");
        // Inside Windows: allowed for a normal (cleanup) delete, refused for force…
        let sys32 = format!("{win}\\System32\\drivers");
        assert!(delete_refusal(&sys32, false).is_none());
        assert!(delete_refusal(&sys32, true).is_some());
        // …except the cleanup folders the app's recommendations target.
        for c in ["Temp", "Temp\\a.tmp", "Logs\\CBS", "Minidump", "SoftwareDistribution\\Download\\x"] {
            assert!(delete_refusal(&format!("{win}\\{c}"), true).is_none(), "{c} must be allowed");
        }
        assert!(delete_refusal(&format!("{win}\\SoftwareDistribution"), true).is_some());
        assert!(delete_refusal(&format!("{win}\\TempX"), true).is_some(), "no prefix match");
        // Ordinary items are never refused.
        assert!(delete_refusal(&format!("{pf}\\SomeApp"), true).is_none());
        assert!(delete_refusal(&format!("{profile}\\Downloads\\a.zip"), true).is_none());
    }

    #[test]
    fn pending_delete_pairs_match_exactly() {
        let parts = [
            r"\??\C:\a\b", "",                 // delete C:\a\b (older format)
            r"\??\C:\a\old", r"!\??\C:\a\new", // a rename, not a delete
            r"*1\??\C:\x\y.txt", "",           // delete, current Windows format
        ];
        assert!(pending_delete_pairs_contain(&parts, r"C:\x\y.txt"), "*1 prefix must be handled");
        assert!(!pending_delete_pairs_contain(&parts, r"C:\x\y.tx"));
        assert!(pending_delete_pairs_contain(&parts, r"C:\a\b"));
        assert!(pending_delete_pairs_contain(&parts, r"c:\A\B\"));
        assert!(!pending_delete_pairs_contain(&parts, r"C:\a"), "no prefix match");
        assert!(!pending_delete_pairs_contain(&parts, r"C:\a\bc"));
        assert!(!pending_delete_pairs_contain(&parts, r"C:\a\old"), "a rename is not a delete");
    }

    #[test]
    fn force_report_parses_every_line_kind() {
        let dir = std::env::temp_dir().join(format!("sd_report_test_{}", std::process::id()));
        std::fs::write(
            &dir,
            "\u{feff}closed\tC:\\a\tWINWORD.exe\nclosed\tC:\\b\tWINWORD.exe\n\
             blocked\tC:\\c\tWindows itself\nunschedulable\tC:\\d\t1\ndone\n",
        )
        .unwrap();
        let r = read_force_report(&dir);
        assert!(r.completed);
        assert_eq!(r.closed_programs(), vec!["WINWORD.exe".to_string()]);
        assert_eq!(r.blocked_for("C:\\c"), vec!["Windows itself".to_string()]);
        assert!(r.blocked_for("C:\\a").is_empty());
        assert_eq!(r.unschedulable, vec![("C:\\d".to_string(), "1".to_string())]);
        assert!(!dir.exists(), "report is deleted after reading");
        // Missing file = the elevated step never ran.
        assert!(!read_force_report(&dir).completed);
    }

    /// Read-only against the real machine — safe, no side effects. A path
    /// this app has never touched must never be reported as scheduled.
    #[test]
    fn pending_reboot_delete_is_false_for_a_path_never_scheduled() {
        assert!(!has_pending_reboot_delete(
            r"C:\this\path\was\never\scheduled\by\storage\doctor\tests.txt"
        ));
    }

    #[test]
    fn ancestors_walk_up_to_the_drive_root() {
        assert_eq!(
            ancestors_of(r"C:\Users\me\Downloads"),
            vec![r"C:\Users\me", r"C:\Users", r"C:\"]
        );
        assert!(ancestors_of(r"C:\").is_empty());
    }

    /// The subtree pattern must not treat `_` or `%` in a real path as LIKE
    /// wildcards — plenty of folders are named `my_project` or `100% done`.
    #[test]
    fn subtree_pattern_matches_descendants_only() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute("CREATE TABLE folders (path TEXT)", []).unwrap();
        for path in [
            r"C:\my_app",         // the target itself
            r"C:\my_app\cache",   // descendant — must go
            r"C:\my_app\a\b",     // deeper descendant — must go
            r"C:\myXapp\cache",   // an unescaped `_` would wrongly match this
            r"C:\my_application", // shares the prefix but not the separator
            r"C:\other",
        ] {
            db.execute("INSERT INTO folders VALUES (?1)", [path]).unwrap();
        }

        db.execute(
            r"DELETE FROM folders WHERE path = ?1 OR path LIKE ?2 ESCAPE '\'",
            rusqlite::params![r"C:\my_app", like_subtree(r"C:\my_app")],
        )
        .unwrap();

        let mut stmt = db.prepare("SELECT path FROM folders ORDER BY path").unwrap();
        let remaining: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            remaining,
            // Binary collation: `X` (0x58) sorts before `_` (0x5F).
            vec![r"C:\myXapp\cache", r"C:\my_application", r"C:\other"]
        );
    }

    #[test]
    fn recycle_bin_paths_are_recognised_per_drive() {
        assert_eq!(recycle_bin_drive(r"D:\$RECYCLE.BIN"), Some("D".into()));
        assert_eq!(
            recycle_bin_drive(r"c:\$Recycle.Bin\S-1-5-21"),
            Some("c".into())
        );
        assert_eq!(recycle_bin_drive(r"C:\Users\me\$RECYCLE.BIN"), None);
    }
}
