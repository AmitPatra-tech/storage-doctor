use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Files below this size are not considered for duplicate detection.
/// 1 byte means every non-empty file is eligible; empty (0-byte) files are
/// skipped because they are trivially identical and only add noise.
const MIN_SIZE: u64 = 1;
/// First-pass hash length; only prefix collisions get a full hash.
const PREFIX_LEN: u64 = 128 * 1024;
const MAX_GROUPS: usize = 500;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeProgress {
    pub phase: String,
    pub files_processed: u64,
    pub total_files: u64,
    pub bytes_hashed: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeFile {
    pub path: String,
    pub modified_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeGroup {
    pub hash: String,
    pub size_bytes: u64,
    pub files: Vec<DupeFile>,
}

/// Downloads, Documents, Pictures, Videos, Music, Desktop.
pub fn default_roots() -> Vec<PathBuf> {
    [
        dirs::download_dir(),
        dirs::document_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        dirs::audio_dir(),
        dirs::desktop_dir(),
    ]
    .into_iter()
    .flatten()
    .filter(|p| p.exists())
    .collect()
}

fn collect_files(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(&entry.path(), out);
        } else if let Ok(meta) = entry.metadata() {
            if meta.len() >= MIN_SIZE {
                out.push((entry.path(), meta.len()));
            }
        }
    }
}

fn hash_file(path: &Path, limit: Option<u64>, bytes_counter: &AtomicU64) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut remaining = limit.unwrap_or(u64::MAX);
    while remaining > 0 {
        let want = buf.len().min(remaining.min(usize::MAX as u64) as usize);
        let n = reader.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        bytes_counter.fetch_add(n as u64, Ordering::Relaxed);
        remaining -= n as u64;
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn modified_rfc3339(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or_default()
}

pub fn find(app: AppHandle, roots: Vec<PathBuf>) -> Vec<DupeGroup> {
    find_with_progress(roots, &|progress| {
        let _ = app.emit("dupe-progress", progress);
    })
}

/// Core duplicate detection. Progress is delivered through a callback so the
/// algorithm can be exercised without a Tauri app handle.
pub fn find_with_progress(
    roots: Vec<PathBuf>,
    on_progress: &(dyn Fn(DupeProgress) + Sync),
) -> Vec<DupeGroup> {
    let roots = if roots.is_empty() { default_roots() } else { roots };

    on_progress(DupeProgress {
        phase: "collecting".into(),
        files_processed: 0,
        total_files: 0,
        bytes_hashed: 0,
    });

    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    for root in &roots {
        collect_files(root, &mut files);
    }

    // Only files sharing an exact size can be duplicates.
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for (path, size) in files {
        by_size.entry(size).or_default().push(path);
    }
    let candidates: Vec<(u64, Vec<PathBuf>)> =
        by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    let total_files: u64 = candidates.iter().map(|(_, v)| v.len() as u64).sum();

    let processed = AtomicU64::new(0);
    let bytes = AtomicU64::new(0);
    let last_emit = Mutex::new(Instant::now());
    let report = || {
        let Ok(mut last) = last_emit.try_lock() else {
            return;
        };
        if last.elapsed() < Duration::from_millis(250) {
            return;
        }
        *last = Instant::now();
        on_progress(DupeProgress {
            phase: "hashing".into(),
            files_processed: processed.load(Ordering::Relaxed),
            total_files,
            bytes_hashed: bytes.load(Ordering::Relaxed),
        });
    };

    let mut groups: Vec<DupeGroup> = candidates
        .par_iter()
        .flat_map(|(size, paths)| {
            let mut by_prefix: HashMap<String, Vec<&PathBuf>> = HashMap::new();
            for path in paths {
                processed.fetch_add(1, Ordering::Relaxed);
                report();
                if let Some(h) = hash_file(path, Some(PREFIX_LEN), &bytes) {
                    by_prefix.entry(h).or_default().push(path);
                }
            }

            let mut out: Vec<DupeGroup> = Vec::new();
            for (prefix_hash, group) in by_prefix.into_iter().filter(|(_, g)| g.len() > 1) {
                if *size <= PREFIX_LEN {
                    // The prefix covered the whole file — already a full match.
                    out.push(make_group(prefix_hash, *size, &group));
                    continue;
                }
                let mut by_full: HashMap<String, Vec<&PathBuf>> = HashMap::new();
                for path in group {
                    report();
                    if let Some(h) = hash_file(path, None, &bytes) {
                        by_full.entry(h).or_default().push(path);
                    }
                }
                for (full_hash, full_group) in by_full.into_iter().filter(|(_, g)| g.len() > 1) {
                    out.push(make_group(full_hash, *size, &full_group));
                }
            }
            out
        })
        .collect();

    // Largest wasted space first (size × extra copies).
    groups.sort_by_key(|g| std::cmp::Reverse(g.size_bytes * (g.files.len() as u64 - 1)));
    groups.truncate(MAX_GROUPS);

    on_progress(DupeProgress {
        phase: "done".into(),
        files_processed: total_files,
        total_files,
        bytes_hashed: bytes.load(Ordering::Relaxed),
    });

    groups
}

fn make_group(hash: String, size_bytes: u64, paths: &[&PathBuf]) -> DupeGroup {
    let mut files: Vec<DupeFile> = paths
        .iter()
        .map(|p| DupeFile {
            path: p.to_string_lossy().into_owned(),
            modified_at: modified_rfc3339(p),
        })
        .collect();
    // Oldest first — treated as the "original" to keep.
    files.sort_by(|a, b| a.modified_at.cmp(&b.modified_at));
    DupeGroup {
        hash: hash[..16].to_string(),
        size_bytes,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, byte: u8, len: usize) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, vec![byte; len]).unwrap();
        path
    }

    /// Small files are in scope: MIN_SIZE is 1 byte, so 100 B and 100 KB
    /// duplicates are detected just like large ones. Only 0-byte files are
    /// skipped (every empty file is trivially "identical" — pure noise).
    #[test]
    fn detects_small_duplicates_and_skips_empty_files() {
        let dir = std::env::temp_dir().join("storage doctor dupes test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 100 bytes, identical pair.
        write(&dir, "tiny-a.bin", 0xAA, 100);
        write(&dir, "tiny-b.bin", 0xAA, 100);
        // 100 KB, identical pair.
        write(&dir, "small-a.bin", 0xBB, 100 * 1024);
        write(&dir, "small-b.bin", 0xBB, 100 * 1024);
        // Same size as the 100-byte pair but different content — must not group.
        write(&dir, "tiny-different.bin", 0xCC, 100);
        // Empty files are deliberately ignored.
        write(&dir, "empty-a.bin", 0x00, 0);
        write(&dir, "empty-b.bin", 0x00, 0);

        let groups = find_with_progress(vec![dir.clone()], &|_| {});

        let sizes: Vec<u64> = groups.iter().map(|g| g.size_bytes).collect();
        assert!(sizes.contains(&100), "100-byte duplicates not detected: {sizes:?}");
        assert!(
            sizes.contains(&(100 * 1024)),
            "100 KB duplicates not detected: {sizes:?}"
        );
        assert!(!sizes.contains(&0), "empty files should not be grouped: {sizes:?}");

        // Each group holds exactly the two identical copies — the same-size
        // file with different content is excluded.
        for group in &groups {
            assert_eq!(group.files.len(), 2, "unexpected group {:?}", group.files);
        }
        assert_eq!(groups.len(), 2, "expected exactly two groups");

        let _ = fs::remove_dir_all(&dir);
    }
}
