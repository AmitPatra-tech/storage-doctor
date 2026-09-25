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
        let (script, attempted_edge) = uninstall::build_force_uninstall_script(&target);
        run_elevated_powershell(&script, "storage_doctor_force_uninstall.ps1")?;
        // Edge's own uninstaller keeps tearing down scheduled tasks briefly
        // after setup.exe returns; give it a moment before checking whether
        // it is really gone.
        std::thread::sleep(std::time::Duration::from_secs(1));
        Ok(uninstall::verify_force_uninstall(&target, attempted_edge))
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
    /// Paths that could not be deleted (typically permission denied or the
    /// file is locked). These can be retried with `delete_paths_elevated`.
    pub failed: Vec<String>,
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

            if !ok {
                failed.push(path.clone());
                continue;
            }
            // How much was freed is measured rather than assumed: folders like
            // Temp and the Recycle Bin are recreated empty straight away, so
            // the "before" size alone would overstate it.
            let after = measure_path(p);
            removals.push(Removal {
                path: path.clone(),
                bytes: before.size_bytes.saturating_sub(after.size_bytes),
                files: before.file_count.saturating_sub(after.file_count),
                recoverable: before
                    .recoverable_bytes
                    .saturating_sub(after.recoverable_bytes),
                vanished: !p.exists(),
            });
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
    let tmp = std::env::temp_dir().join(temp_name);
    std::fs::write(&tmp, script).map_err(|e| e.to_string())?;

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
        let before: Vec<(String, cleaner::Measure)> = paths
            .iter()
            .map(|p| (p.clone(), measure_path(Path::new(p))))
            .collect();

        let script = build_delete_script(&paths, permanent);
        run_elevated_powershell(&script, "storage_doctor_recycle.ps1")?;

        let mut removals: Vec<Removal> = Vec::new();
        let mut failed = Vec::new();
        for (path, before) in before {
            let p = Path::new(&path);
            let after = measure_path(p);
            // Directories like Temp and the Recycle Bin are recreated instantly
            // by Windows, so "still exists" is not the same as "not deleted" —
            // compare what it holds now against what it held before.
            let bytes = before.size_bytes.saturating_sub(after.size_bytes);
            let removed_files = before.file_count.saturating_sub(after.file_count);
            if !p.exists() || bytes > 0 || removed_files > 0 {
                removals.push(Removal {
                    path: path.clone(),
                    bytes,
                    files: removed_files,
                    recoverable: before
                        .recoverable_bytes
                        .saturating_sub(after.recoverable_bytes),
                    vanished: !p.exists(),
                });
            } else {
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
const FORCE_DELETE_HELPER: &str = r#"
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

    /// Process IDs Windows reports as currently holding `path` open. Best
    /// effort throughout: any failure just means an empty list, and the
    /// caller moves straight on to deleting.
    public static int[] FindLockerPids(string path) {
        var pids = new List<int>();
        uint handle;
        string key = Guid.NewGuid().ToString("N").Substring(0, 32);
        if (RmStartSession(out handle, 0, key) != 0) return pids.ToArray();
        try {
            var files = new string[] { path };
            if (RmRegisterResources(handle, (uint)files.Length, files, 0, null, 0, null) != 0) {
                return pids.ToArray();
            }
            uint needed = 0, have = 0, reasons = 0;
            int rc = RmGetList(handle, out needed, ref have, null, ref reasons);
            if ((rc != 0 && rc != 234 /* ERROR_MORE_DATA */) || needed == 0) return pids.ToArray();
            var infos = new RM_PROCESS_INFO[needed];
            have = needed;
            if (RmGetList(handle, out needed, ref have, infos, ref reasons) != 0) return pids.ToArray();
            for (int i = 0; i < have; i++) pids.Add(infos[i].Process.dwProcessId);
        } finally {
            RmEndSession(handle);
        }
        return pids.ToArray();
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
    public static bool ScheduleForReboot(string path) {
        const int cap = 2000;
        if (Directory.Exists(path)) {
            var entries = new List<string>();
            try { CollectDepthFirst(path, entries, cap); } catch { return false; }
            if (entries.Count >= cap) return false;
            entries.Add(path);
            foreach (var p in entries) MoveFileEx(p, null, 0x4);
            return true;
        }
        if (File.Exists(path)) {
            return MoveFileEx(path, null, 0x4);
        }
        return false;
    }
}
"@
"#;

/// Builds the elevated script: for each path, find and stop whatever has it
/// open, retry the normal delete, and if it is still there afterward,
/// schedule it for removal on next boot.
pub(crate) fn build_force_delete_script(paths: &[String], permanent: bool) -> String {
    let mut script = String::from("Add-Type -AssemblyName Microsoft.VisualBasic\n");
    script.push_str(FORCE_DELETE_HELPER);

    for path in paths {
        let esc = path.replace('\'', "''");
        script.push_str(&format!(
            "try {{ foreach ($procId in [SDForceDelete]::FindLockerPids('{esc}')) {{ \
                Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue }} }} catch {{ }}\n"
        ));
        script.push_str("Start-Sleep -Milliseconds 400\n");
        if permanent {
            script.push_str(&format!(
                "try {{ Remove-Item -LiteralPath '{esc}' -Recurse -Force -ErrorAction SilentlyContinue }} catch {{ }}\n"
            ));
        } else {
            script.push_str(&recycle_item_script_commands(&esc));
        }
        script.push_str(&format!(
            "if (Test-Path -LiteralPath '{esc}') {{ [SDForceDelete]::ScheduleForReboot('{esc}') | Out-Null }}\n"
        ));
    }
    script
}

/// Same shape as `build_delete_script`'s per-item recycle-bin snippet,
/// factored out because `build_force_delete_script` needs it between two
/// other steps rather than as the whole body of the loop.
fn recycle_item_script_commands(esc: &str) -> String {
    format!(
        "try {{ if (Test-Path -LiteralPath '{esc}' -PathType Container) {{ \
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('{esc}','OnlyErrorDialogs','SendToRecycleBin') \
         }} elseif (Test-Path -LiteralPath '{esc}') {{ \
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{esc}','OnlyErrorDialogs','SendToRecycleBin') \
         }} }} catch {{ }}\n"
    )
}

/// True if `path` (or, for a folder, its own entry — `ScheduleForReboot`
/// always adds the root last) is registered to be removed the next time
/// Windows starts. Reading this key needs no elevation, unlike writing it.
pub(crate) fn has_pending_reboot_delete(path: &str) -> bool {
    let Ok(key) =
        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager")
    else {
        return false;
    };
    let Ok(entries) = key.get_value::<Vec<String>, _>("PendingFileRenameOperations") else {
        return false;
    };
    let needle = path.to_lowercase();
    entries.iter().any(|e| e.to_lowercase().contains(&needle))
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ForceDeleteResult {
    pub freed_bytes: u64,
    /// Removed immediately, once whatever had them open was closed.
    pub removed: Vec<String>,
    /// Not free yet — scheduled to be removed automatically the next time
    /// the PC restarts, because nothing running right now could be made to
    /// let go of them (a protected system process, most often).
    pub scheduled_for_reboot: Vec<String>,
    /// Could not be removed and could not even be scheduled — genuinely
    /// stuck (very rare: a protected system file, or a folder too large to
    /// schedule item-by-item safely).
    pub failed: Vec<String>,
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
        let before: Vec<(String, cleaner::Measure)> = paths
            .iter()
            .map(|p| (p.clone(), measure_path(Path::new(p))))
            .collect();

        let script = build_force_delete_script(&paths, permanent);
        run_elevated_powershell(&script, "storage_doctor_force_delete.ps1")?;

        let mut removals: Vec<Removal> = Vec::new();
        let mut scheduled_for_reboot = Vec::new();
        let mut failed = Vec::new();

        for (path, prior) in before {
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
            } else if has_pending_reboot_delete(&path) {
                scheduled_for_reboot.push(path);
            } else {
                failed.push(path);
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
            scheduled_for_reboot,
            failed,
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

    let confirm = message_box(
        &format!(
            "Force delete this item?\n\n{path}\n\nAny program currently using it will be \
             closed, and it will be moved to the Recycle Bin where possible."
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
            &format!("Deleted — moved to the Recycle Bin:\n{path}"),
            front | MB_ICONINFORMATION,
        );
        return;
    }

    // 2) Still there → locked or permission-denied. One elevated pass closes
    //    the locker and retries, falling back to removal on next restart.
    let script = build_force_delete_script(&[path.to_string()], false);
    if let Err(e) = run_elevated_powershell(&script, "storage_doctor_force_delete_cli.ps1") {
        message_box(
            &format!("Could not force delete:\n{path}\n\n{e}"),
            front | MB_ICONERROR,
        );
        return;
    }

    if !Path::new(path).exists() {
        message_box(&format!("Deleted:\n{path}"), front | MB_ICONINFORMATION);
    } else if has_pending_reboot_delete(path) {
        message_box(
            &format!(
                "This item is still in use and could not be closed.\nWindows will remove it \
                 automatically the next time you restart your PC:\n{path}"
            ),
            front | MB_ICONINFORMATION,
        );
    } else {
        message_box(
            &format!("Could not delete — the item is still in use:\n{path}"),
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

    #[test]
    fn force_delete_script_stops_lockers_before_deleting_and_schedules_reboot_fallback() {
        let script = build_force_delete_script(&[r"C:\stuck\file.txt".to_string()], false);
        assert!(script.contains("FindLockerPids"));
        assert!(script.contains("Stop-Process -Id $procId"));
        assert!(script.contains("SendToRecycleBin"));
        assert!(script.contains("ScheduleForReboot"));
        // Recycle Bin path is used, not a permanent Remove-Item.
        assert!(!script.contains(r"Remove-Item -LiteralPath 'C:\stuck\file.txt' -Recurse"));
        // The kill step must run, and finish, before the delete attempt.
        let kill_pos = script.find("FindLockerPids").unwrap();
        let delete_pos = script.find("SendToRecycleBin").unwrap();
        assert!(kill_pos < delete_pos, "processes must be stopped before deleting");
    }

    #[test]
    fn force_delete_script_permanent_mode_skips_the_recycle_bin() {
        let script = build_force_delete_script(&[r"C:\stuck\file.txt".to_string()], true);
        assert!(script.contains(r"Remove-Item -LiteralPath 'C:\stuck\file.txt' -Recurse -Force"));
        assert!(!script.contains("SendToRecycleBin"));
    }

    #[test]
    fn force_delete_script_escapes_single_quotes_in_paths() {
        // A literal `'` inside a PowerShell single-quoted string must be
        // doubled, or the script fails to parse (or worse, terminates the
        // string early against attacker-controlled-looking content).
        let script = build_force_delete_script(&[r"C:\it's\stuck.txt".to_string()], false);
        assert!(script.contains(r"it''s"));
        assert!(!script.contains(r"'C:\it's\stuck.txt'"));
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
