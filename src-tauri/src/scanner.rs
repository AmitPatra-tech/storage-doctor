use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};

/// Files below this size are never reported individually.
pub const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024;
const MAX_LARGE_FILES: usize = 200;
const MAX_FOLDERS: usize = 1000;
/// Folders deeper than this (relative to the scan root) are aggregated into
/// their parents instead of being recorded individually.
const FOLDER_RECORD_DEPTH: usize = 4;
const FOLDER_MIN_BYTES: u64 = 10 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub scan_id: i64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub current_path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanComplete {
    pub scan_id: i64,
    pub duration_ms: u64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub errors: u64,
}

pub struct FolderRecord {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub depth: usize,
}

pub struct FileRecord {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size_bytes: u64,
    pub modified_at: String,
}

pub struct ScanOutcome {
    pub folders: Vec<FolderRecord>,
    pub large_files: Vec<FileRecord>,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub errors: u64,
    pub duration_ms: u64,
}

struct ScanCtx {
    scan_id: i64,
    app: AppHandle,
    files_scanned: AtomicU64,
    bytes_scanned: AtomicU64,
    errors: AtomicU64,
    folders: Mutex<Vec<FolderRecord>>,
    large_files: Mutex<Vec<FileRecord>>,
    last_emit: Mutex<Instant>,
}

#[derive(Default, Clone, Copy)]
struct DirStats {
    size: u64,
    files: u64,
}

pub fn scan(app: AppHandle, scan_id: i64, roots: &[PathBuf]) -> ScanOutcome {
    let started = Instant::now();
    let ctx = ScanCtx {
        scan_id,
        app,
        files_scanned: AtomicU64::new(0),
        bytes_scanned: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        folders: Mutex::new(Vec::new()),
        large_files: Mutex::new(Vec::new()),
        last_emit: Mutex::new(Instant::now()),
    };

    for root in roots {
        scan_dir(root, 0, &ctx);
    }

    let mut folders = ctx.folders.into_inner().unwrap_or_default();
    folders.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    folders.truncate(MAX_FOLDERS);

    let mut large_files = ctx.large_files.into_inner().unwrap_or_default();
    large_files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    large_files.truncate(MAX_LARGE_FILES);

    ScanOutcome {
        folders,
        large_files,
        files_scanned: ctx.files_scanned.load(Ordering::Relaxed),
        bytes_scanned: ctx.bytes_scanned.load(Ordering::Relaxed),
        errors: ctx.errors.load(Ordering::Relaxed),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn scan_dir(path: &Path, depth: usize, ctx: &ScanCtx) -> DirStats {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => {
            // Permission denied, encrypted or vanished folders are skipped.
            ctx.errors.fetch_add(1, Ordering::Relaxed);
            return DirStats::default();
        }
    };

    let mut local = DirStats::default();
    let mut subdirs: Vec<PathBuf> = Vec::new();

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            ctx.errors.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        // Symlinks and junctions are skipped to avoid cycles and double counting.
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            subdirs.push(entry.path());
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        local.size += size;
        local.files += 1;

        if size >= LARGE_FILE_THRESHOLD {
            record_large_file(&entry.path(), size, ctx);
        }

        let scanned = ctx.files_scanned.fetch_add(1, Ordering::Relaxed) + 1;
        ctx.bytes_scanned.fetch_add(size, Ordering::Relaxed);
        if scanned % 512 == 0 {
            maybe_emit_progress(path, ctx);
        }
    }

    let sub_total: DirStats = subdirs
        .par_iter()
        .map(|sub| scan_dir(sub, depth + 1, ctx))
        .reduce(DirStats::default, |a, b| DirStats {
            size: a.size + b.size,
            files: a.files + b.files,
        });

    let total = DirStats {
        size: local.size + sub_total.size,
        files: local.files + sub_total.files,
    };

    if depth > 0 && depth <= FOLDER_RECORD_DEPTH && total.size >= FOLDER_MIN_BYTES {
        if let Ok(mut folders) = ctx.folders.lock() {
            folders.push(FolderRecord {
                path: path.to_string_lossy().into_owned(),
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned()),
                size_bytes: total.size,
                file_count: total.files,
                depth,
            });
        }
    }

    total
}

fn record_large_file(path: &Path, size: u64, ctx: &ScanCtx) {
    let modified_at = fs::metadata(path)
        .and_then(|m| m.modified())
        .map(system_time_to_rfc3339)
        .unwrap_or_default();

    if let Ok(mut files) = ctx.large_files.lock() {
        files.push(FileRecord {
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            extension: path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            size_bytes: size,
            modified_at,
        });
        // Bound memory: compact opportunistically once the buffer grows.
        if files.len() > MAX_LARGE_FILES * 8 {
            files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
            files.truncate(MAX_LARGE_FILES);
        }
    }
}

fn maybe_emit_progress(current: &Path, ctx: &ScanCtx) {
    let Ok(mut last) = ctx.last_emit.try_lock() else {
        return;
    };
    if last.elapsed() < PROGRESS_INTERVAL {
        return;
    }
    *last = Instant::now();

    let payload = ScanProgress {
        scan_id: ctx.scan_id,
        files_scanned: ctx.files_scanned.load(Ordering::Relaxed),
        bytes_scanned: ctx.bytes_scanned.load(Ordering::Relaxed),
        current_path: current.to_string_lossy().into_owned(),
    };
    let _ = ctx.app.emit("scan-progress", payload);
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
}
