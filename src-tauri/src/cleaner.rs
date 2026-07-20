use crate::apps::dir_stats;
use crate::classify;
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

const MAX_DEPTH: usize = 10;
const MIN_FILE_BYTES: u64 = 512 * 1024;
const MAX_ITEMS: usize = 500;

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
                if size_bytes >= MIN_FILE_BYTES {
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
