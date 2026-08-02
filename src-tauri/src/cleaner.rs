use crate::apps::dir_stats;
use crate::classify;
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

/// Bounded only to stop pathological recursion. It deliberately exceeds any
/// real directory tree: if the listing stopped short of what `measure` counts,
/// a row would advertise more recoverable space than the dialog can offer.
const MAX_DEPTH: usize = 32;
/// A file smaller than this is not worth listing on its own.
pub const MIN_FILE_BYTES: u64 = 512 * 1024;
const MAX_ITEMS: usize = 500;

/// What a folder holds, and how much of that is safe to clear.
#[derive(Default, Clone, Copy)]
pub struct Measure {
    pub size_bytes: u64,
    pub file_count: u64,
    pub recoverable_bytes: u64,
}

impl Measure {
    fn plus(self, other: Measure) -> Measure {
        Measure {
            size_bytes: self.size_bytes + other.size_bytes,
            file_count: self.file_count + other.file_count,
            recoverable_bytes: self.recoverable_bytes + other.recoverable_bytes,
        }
    }
}

/// Measures `dir` — size, file count, and how many of those bytes are safe to
/// clear — in a single walk, taking `dir`'s own classification into account.
///
/// The breakdown needs both numbers for every row it draws. Walking twice
/// would double the cost of opening a folder, and answering "how much can I
/// reclaim here?" only when the user clicks is what made a 72 GB row appear
/// to promise 72 GB of cleanup when it held 390 MB.
pub fn measure_dir(dir: &Path) -> Measure {
    let mut measured = measure_children(dir);
    match classify::safety(dir, true).as_deref() {
        // The whole folder is disposable, so all of it counts.
        Some("safe") => measured.recoverable_bytes = measured.size_bytes,
        // Nothing inside a system area is ever offered for cleanup.
        Some("system") => measured.recoverable_bytes = 0,
        _ => {}
    }
    measured
}

fn measure_children(dir: &Path) -> Measure {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Measure::default();
    };
    let entries: Vec<_> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| {
            let Ok(file_type) = entry.file_type() else {
                return Measure::default();
            };
            if file_type.is_symlink() {
                return Measure::default();
            }
            if file_type.is_dir() {
                return measure_dir(&entry.path());
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            Measure {
                size_bytes: size,
                file_count: 1,
                recoverable_bytes: if is_clearable_file(&entry.path(), size) {
                    size
                } else {
                    0
                },
            }
        })
        .reduce(Measure::default, Measure::plus)
}

/// The rule `find_safe_items` uses for loose files, shared so the total on a
/// row always matches the items the cleanup dialog lists.
pub fn is_clearable_file(path: &Path, size: u64) -> bool {
    size >= MIN_FILE_BYTES && classify::safety(path, false).as_deref() == Some("safe")
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeItem {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub category: String,
    pub explanation: String,
}

/// Recursively finds everything inside `root` that is classified as safe to
/// delete (caches, temp, logs, dumps, node_modules, …). Never descends into
/// system areas. Nothing is deleted here — this only reports candidates.
pub fn find_safe_items(root: &Path) -> Vec<SafeItem> {
    let out = Mutex::new(Vec::new());
    visit(root, 0, &out);
    let mut items = out.into_inner().unwrap_or_default();
    items.retain(|i| i.size_bytes > 0);
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    items.truncate(MAX_ITEMS);
    items
}

fn visit(dir: &Path, depth: usize, out: &Mutex<Vec<SafeItem>>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    entries.par_iter().for_each(|entry| {
        let Ok(file_type) = entry.file_type() else {
            return;
        };
        if file_type.is_symlink() {
            return;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(c) = classify::classify(&path, true) {
                if c.safety == "safe" {
                    let (size_bytes, file_count) = dir_stats(&path);
                    if let Ok(mut items) = out.lock() {
                        items.push(SafeItem {
                            path: path.to_string_lossy().into_owned(),
                            size_bytes,
                            file_count,
                            category: c.category,
                            explanation: c.explanation,
                        });
                    }
                    // Whole folder is deletable — no need to look inside.
                    return;
                }
                if c.safety == "system" {
                    // Never suggest cleanup inside system areas.
                    return;
                }
            }
            visit(&path, depth + 1, out);
        } else if let Some(c) = classify::classify(&path, false) {
            if c.safety == "safe" {
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if is_clearable_file(&path, size_bytes) {
                    if let Ok(mut items) = out.lock() {
                        items.push(SafeItem {
                            path: path.to_string_lossy().into_owned(),
                            size_bytes,
                            file_count: 1,
                            category: c.category,
                            explanation: c.explanation,
                        });
                    }
                }
            }
        }
    });
}


#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    /// A tree with one cache folder, one loose log, and personal files that
    /// must never be counted as reclaimable.
    fn fixture(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("Documents").join("thesis.docx"), 3_000_000);
        write(&root.join("Cache").join("blob-1"), 2_000_000);
        write(&root.join("Cache").join("blob-2"), 1_000_000);
        write(&root.join("app").join("run.log"), 1_500_000);
        // Under the 512 KB floor, so neither side should offer it.
        write(&root.join("app").join("tiny.log"), 1_000);
        root
    }

    /// The number a breakdown row advertises and the total the cleanup dialog
    /// can actually delete must be the same number. When they drifted apart, a
    /// 72 GB row appeared to promise 72 GB and delivered 390 MB.
    #[test]
    fn row_figure_matches_what_the_cleanup_dialog_offers() {
        let root = fixture("storage doctor measure test");

        let measured = measure_dir(&root);
        let listed: u64 = find_safe_items(&root).iter().map(|i| i.size_bytes).sum();

        assert_eq!(
            measured.recoverable_bytes, listed,
            "row says {} but the dialog can only offer {}",
            measured.recoverable_bytes, listed
        );
        // Cache folder (3 MB) + the log over the floor (1.5 MB).
        assert_eq!(measured.recoverable_bytes, 4_500_000);
        // Personal files are counted in the size but never as recoverable.
        assert_eq!(measured.size_bytes, 7_501_000);
        assert_eq!(measured.file_count, 5);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A folder holding nothing disposable reports zero, which is what lets
    /// the UI drop the cleanup action from that row entirely.
    #[test]
    fn folders_with_nothing_to_clear_report_zero() {
        let root = std::env::temp_dir().join("storage doctor personal only test");
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("Pictures").join("holiday.jpg"), 4_000_000);

        let measured = measure_dir(&root);
        assert_eq!(measured.recoverable_bytes, 0);
        assert_eq!(measured.size_bytes, 4_000_000);
        assert!(find_safe_items(&root).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}
