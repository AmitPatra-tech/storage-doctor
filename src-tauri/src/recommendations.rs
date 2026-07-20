use crate::apps::{dir_size, expand_glob};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::PathBuf;
use sysinfo::Disks;

const OLD_INSTALLER_DAYS: i64 = 180;

struct Candidate {
    id: &'static str,
    name: &'static str,
    description: String,
    recoverable_bytes: u64,
    risk: &'static str,
    recommended: bool,
    paths: Vec<String>,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Sums the sizes of the paths that exist; returns (bytes, existing paths).
fn measure(paths: Vec<Option<PathBuf>>) -> (u64, Vec<String>) {
    let existing: Vec<PathBuf> = paths.into_iter().flatten().filter(|p| p.exists()).collect();
    let bytes = existing.iter().map(|p| dir_size(p)).sum();
    (bytes, existing.iter().map(|p| p.to_string_lossy().into_owned()).collect())
}

fn simple(
    id: &'static str,
    name: &'static str,
    description: &str,
    risk: &'static str,
    recommended: bool,
    paths: Vec<Option<PathBuf>>,
) -> Option<Candidate> {
    let (bytes, existing) = measure(paths);
    if bytes == 0 {
        return None;
    }
    Some(Candidate {
        id,
        name,
        description: description.to_string(),
        recoverable_bytes: bytes,
        risk,
        recommended,
        paths: existing,
    })
}

fn build_candidates(conn: &Connection, scan_id: i64) -> Vec<Candidate> {
    let local = env_path("LOCALAPPDATA");
    let program_data = env_path("ProgramData");
    let windir = env_path("SystemRoot").or_else(|| Some(PathBuf::from("C:\\Windows")));
    let temp = env_path("TEMP");
    let join = |base: &Option<PathBuf>, rel: &str| base.as_ref().map(|b| b.join(rel));

    let mut candidates = Vec::new();

    candidates.extend(simple(
        "windows-temp",
        "Temporary Files",
        "Temporary files created by Windows and applications. They are recreated automatically when needed.",
        "low",
        true,
        vec![temp.clone(), join(&windir, "Temp")],
    ));

    candidates.extend(simple(
        "chrome-cache",
        "Chrome Browser Cache",
        "Web cache that Chrome rebuilds automatically while browsing.",
        "low",
        true,
        vec![
            join(&local, "Google\\Chrome\\User Data\\Default\\Cache"),
            join(&local, "Google\\Chrome\\User Data\\Default\\Code Cache"),
            join(&local, "Google\\Chrome\\User Data\\Default\\GPUCache"),
        ],
    ));

    candidates.extend(simple(
        "edge-cache",
        "Edge Browser Cache",
        "Web cache that Microsoft Edge rebuilds automatically while browsing.",
        "low",
        true,
        vec![
            join(&local, "Microsoft\\Edge\\User Data\\Default\\Cache"),
            join(&local, "Microsoft\\Edge\\User Data\\Default\\Code Cache"),
        ],
    ));

    let firefox_caches: Vec<Option<PathBuf>> = local
        .as_ref()
        .map(|l| {
            expand_glob(&l.join("Mozilla\\Firefox\\Profiles\\*\\cache2"))
                .into_iter()
                .map(Some)
                .collect()
        })
        .unwrap_or_default();
    candidates.extend(simple(
        "firefox-cache",
        "Firefox Browser Cache",
        "Web cache that Firefox rebuilds automatically while browsing.",
        "low",
        true,
        firefox_caches,
    ));

    let roaming = env_path("APPDATA");
    candidates.extend(simple(
        "discord-cache",
        "Discord Cache",
        "Images, video previews and code cached by Discord. Rebuilt automatically.",
        "low",
        true,
        vec![
            join(&roaming, "discord\\Cache"),
            join(&roaming, "discord\\Code Cache"),
            join(&roaming, "discord\\GPUCache"),
        ],
    ));

    candidates.extend(simple(
        "vscode-cache",
        "VS Code Cache",
        "Editor caches that VS Code rebuilds automatically on next launch.",
        "low",
        true,
        vec![
            join(&roaming, "Code\\Cache"),
            join(&roaming, "Code\\CachedData"),
            join(&roaming, "Code\\Code Cache"),
        ],
    ));

    candidates.extend(simple(
        "crash-dumps",
        "Crash Dumps",
        "Memory snapshots saved when programs crashed. Only useful for debugging past crashes.",
        "low",
        true,
        vec![
            join(&local, "CrashDumps"),
            join(&windir, "Minidump"),
            join(&windir, "LiveKernelReports"),
        ],
    ));

    candidates.extend(simple(
        "windows-logs",
        "Windows Log Files",
        "Diagnostic logs Windows keeps for troubleshooting. Not needed for normal use; removing system logs requires administrator rights.",
        "low",
        true,
        vec![join(&windir, "Logs")],
    ));

    candidates.extend(simple(
        "delivery-optimization",
        "Delivery Optimization Cache",
        "Pieces of Windows updates kept for sharing between PCs. Windows clears this automatically when space runs low.",
        "low",
        true,
        vec![join(&program_data, "Microsoft\\Windows\\DeliveryOptimization")],
    ));

    candidates.extend(simple(
        "windows-old",
        "Previous Windows Installation",
        "Your previous Windows version, kept so you can roll back after an upgrade. Removing it (via Disk Cleanup, requires admin) makes rollback impossible.",
        "medium",
        false,
        vec![Some(PathBuf::from("C:\\Windows.old"))],
    ));

    let recycle_bins: Vec<Option<PathBuf>> = Disks::new_with_refreshed_list()
        .iter()
        .map(|disk| Some(disk.mount_point().join("$Recycle.Bin")))
        .collect();
    candidates.extend(simple(
        "recycle-bin",
        "Recycle Bin",
        "Files you already deleted, held for recovery. Emptying the Recycle Bin permanently removes them.",
        "low",
        true,
        recycle_bins,
    ));

    candidates.extend(simple(
        "gradle-cache",
        "Gradle Cache",
        "Build cache and downloaded dependencies used by Gradle / Android Studio. Re-downloaded on demand.",
        "low",
        true,
        vec![env_path("USERPROFILE").map(|p| p.join(".gradle\\caches"))],
    ));

    candidates.extend(simple(
        "npm-cache",
        "npm Cache",
        "Package cache used by npm. Packages are re-downloaded on demand.",
        "low",
        true,
        vec![join(&local, "npm-cache")],
    ));

    candidates.extend(simple(
        "yarn-cache",
        "Yarn Cache",
        "Package cache used by Yarn. Packages are re-downloaded on demand.",
        "low",
        true,
        vec![join(&local, "Yarn\\Cache")],
    ));

    candidates.extend(simple(
        "nvidia-shader-cache",
        "NVIDIA Shader Cache",
        "Graphics shader cache that Windows recreates automatically.",
        "low",
        true,
        vec![
            join(&local, "NVIDIA\\DXCache"),
            join(&local, "NVIDIA\\GLCache"),
            join(&program_data, "NVIDIA Corporation\\NV_Cache"),
        ],
    ));

    candidates.extend(simple(
        "directx-shader-cache",
        "DirectX Shader Cache",
        "Compiled shader cache managed by Windows. Rebuilt automatically by games and apps.",
        "low",
        true,
        vec![join(&local, "D3DSCache")],
    ));

    candidates.extend(simple(
        "explorer-thumbnails",
        "Thumbnail Cache",
        "Image and video thumbnails cached by Windows Explorer. Regenerated when folders are viewed.",
        "low",
        true,
        vec![join(&local, "Microsoft\\Windows\\Explorer")],
    ));

    candidates.extend(simple(
        "windows-update-cache",
        "Windows Update Download Cache",
        "Already-installed update packages kept by Windows Update. Removing them is safe once updates are complete, but requires administrator rights.",
        "medium",
        false,
        vec![join(&windir, "SoftwareDistribution\\Download")],
    ));

    candidates.extend(simple(
        "adobe-media-cache",
        "Adobe Media Cache",
        "Preview and conform files created by Premiere Pro and After Effects. Regenerated when projects are opened.",
        "low",
        true,
        vec![
            env_path("APPDATA").map(|p| p.join("Adobe\\Common\\Media Cache Files")),
            env_path("APPDATA").map(|p| p.join("Adobe\\Common\\Media Cache")),
        ],
    ));

    if let Some(c) = old_installers(conn, scan_id) {
        candidates.push(c);
    }

    candidates
}

/// Old installers and disc images in Downloads, based on the last scan's
/// large-file records.
fn old_installers(conn: &Connection, scan_id: i64) -> Option<Candidate> {
    let mut stmt = conn
        .prepare(
            "SELECT path, size_bytes, modified_at FROM large_files
             WHERE scan_id = ?1 AND extension IN ('exe', 'msi', 'iso')
               AND path LIKE '%\\Downloads\\%'",
        )
        .ok()?;
    let rows: Vec<(String, u64, String)> = stmt
        .query_map([scan_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .ok()?
        .flatten()
        .collect();

    let cutoff = chrono::Utc::now() - chrono::Duration::days(OLD_INSTALLER_DAYS);
    let old: Vec<&(String, u64, String)> = rows
        .iter()
        .filter(|(_, _, modified)| {
            chrono::DateTime::parse_from_rfc3339(modified)
                .map(|m| m.with_timezone(&chrono::Utc) < cutoff)
                .unwrap_or(false)
        })
        .collect();

    if old.is_empty() {
        return None;
    }
    let bytes: u64 = old.iter().map(|(_, size, _)| size).sum();
    Some(Candidate {
        id: "old-installers",
        name: "Old Installers in Downloads",
        description: format!(
            "{} installer/ISO file(s) in Downloads not modified in over {} months. Delete them if no longer required.",
            old.len(),
            OLD_INSTALLER_DAYS / 30
        ),
        recoverable_bytes: bytes,
        risk: "medium",
        recommended: false,
        paths: old.iter().map(|(p, _, _)| p.clone()).collect(),
    })
}

/// Regenerates recommendations for a completed scan, preserving ignored flags.
/// Returns the total recoverable bytes across non-ignored recommendations.
pub fn generate(conn: &Connection, scan_id: i64) -> Result<u64, String> {
    let ignored: HashSet<String> = conn
        .prepare("SELECT id FROM recommendations WHERE ignored = 1")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    let candidates = build_candidates(conn, scan_id);

    conn.execute("DELETE FROM recommendations", [])
        .map_err(|e| e.to_string())?;

    let mut total = 0u64;
    for c in &candidates {
        let is_ignored = ignored.contains(c.id);
        if !is_ignored {
            total += c.recoverable_bytes;
        }
        conn.execute(
            "INSERT INTO recommendations
                (id, scan_id, name, description, recoverable_bytes, risk, recommended, ignored, paths)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                c.id,
                scan_id,
                c.name,
                c.description,
                c.recoverable_bytes,
                c.risk,
                c.recommended,
                is_ignored,
                serde_json::to_string(&c.paths).unwrap_or_default(),
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute(
        "UPDATE scans SET recoverable_bytes = ?2 WHERE id = ?1",
        rusqlite::params![scan_id, total],
    )
    .map_err(|e| e.to_string())?;

    Ok(total)
}
