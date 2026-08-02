//! The Storage Breakdown's opening screen is served straight from what a scan
//! recorded. When the scan failed to work out recoverable space, every row on
//! that screen quietly claimed there was nothing to clean — the cleanup action
//! only appeared after opening a folder and coming back, because opening it
//! measured the folder live.
//!
//! These run as an integration test rather than a `#[cfg(test)]` module: this
//! crate is built as a `cdylib`, and its unit-test harness fails to load on
//! Windows (`STATUS_ENTRYPOINT_NOT_FOUND`) once a test touches the scanner.

use std::fs;
use std::path::{Path, PathBuf};
use storage_doctor_lib::{cleaner, scanner};

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

/// A tree shaped like the one that exposed the bug: a folder named `cache`
/// nested two levels down, alongside content that must never be offered.
fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&root);
    write(&root.join("project").join("cache").join("blob"), 12_000_000);
    write(&root.join("project").join("src").join("main.rs"), 11_000_000);
    write(&root.join("media").join("clip.mp4"), 14_000_000);
    root
}

#[test]
fn scan_records_recoverable_space_for_every_folder() {
    let root = fixture("storage doctor scan measure test");
    let outcome = scanner::scan(None, 1, &[root.clone()]);

    let find = |name: &str| {
        outcome
            .folders
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "{name} was not recorded; got {:?}",
                    outcome.folders.iter().map(|f| &f.name).collect::<Vec<_>>()
                )
            })
    };

    let project = find("project");
    assert_eq!(project.size_bytes, 23_000_000);
    assert_eq!(
        project.recoverable_bytes, 12_000_000,
        "the cache folder nested inside it should be reclaimable"
    );

    // A folder that is itself disposable counts in full.
    let cache = find("cache");
    assert_eq!(cache.recoverable_bytes, cache.size_bytes);

    // Nothing here is disposable, so the row must offer no cleanup at all.
    assert_eq!(find("media").recoverable_bytes, 0);

    let _ = fs::remove_dir_all(&root);
}

/// What the scan records for a folder and what opening that folder measures
/// have to be the same number, or the figure on a row changes the moment you
/// click into it.
#[test]
fn scanned_figure_matches_a_live_measurement() {
    let root = fixture("storage doctor scan agreement test");
    let outcome = scanner::scan(None, 1, &[root.clone()]);

    for recorded in &outcome.folders {
        let live = cleaner::measure_dir(Path::new(&recorded.path));
        assert_eq!(
            recorded.recoverable_bytes, live.recoverable_bytes,
            "{} was scanned as {} recoverable but measures {} live",
            recorded.path, recorded.recoverable_bytes, live.recoverable_bytes
        );
        assert_eq!(recorded.size_bytes, live.size_bytes, "{}", recorded.path);
    }

    let _ = fs::remove_dir_all(&root);
}
