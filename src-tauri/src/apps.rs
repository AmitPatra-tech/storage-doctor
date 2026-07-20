use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppComponent {
    pub label: String,
    pub path: String,
    pub bytes: u64,
    pub recoverable: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppReport {
    pub name: String,
    pub total_bytes: u64,
    pub recoverable_bytes: u64,
    pub status: String,
    pub components: Vec<AppComponent>,
}

/// Recursive directory size; skips symlinks/junctions, ignores unreadable entries.
pub fn dir_size(path: &Path) -> u64 {
    dir_stats(path).0
}

/// Recursive (bytes, file count); skips symlinks/junctions, ignores unreadable entries.
pub fn dir_stats(path: &Path) -> (u64, u64) {
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    let entries: Vec<_> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| {
            let Ok(file_type) = entry.file_type() else {
                return (0, 0);
            };
            if file_type.is_symlink() {
                (0, 0)
            } else if file_type.is_dir() {
                dir_stats(&entry.path())
            } else {
                (entry.metadata().map(|m| m.len()).unwrap_or(0), 1)
            }
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Expands a single `*` path segment against the filesystem.
pub fn expand_glob(path: &Path) -> Vec<PathBuf> {
    let mut result = vec![PathBuf::new()];
    for segment in path.iter() {
        if segment == "*" {
            let mut next = Vec::new();
            for base in &result {
                if let Ok(entries) = fs::read_dir(base) {
                    for entry in entries.flatten() {
                        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            next.push(entry.path());
                        }
                    }
                }
            }
            result = next;
        } else {
            for base in &mut result {
                base.push(segment);
            }
        }
    }
    result.into_iter().filter(|p| p.exists()).collect()
}

struct Spec {
    name: &'static str,
    /// (label, env var, relative path) — counted toward the app total.
    roots: &'static [(&'static str, &'static str, &'static str)],
    /// (label, env var, relative path) — recoverable caches. Paths inside a
    /// root are not double counted in the total.
    caches: &'static [(&'static str, &'static str, &'static str)],
}

const SPECS: &[Spec] = &[
    Spec {
        name: "Google Chrome",
        roots: &[("Data", "LOCALAPPDATA", "Google\\Chrome")],
        caches: &[
            ("Cache", "LOCALAPPDATA", "Google\\Chrome\\User Data\\Default\\Cache"),
            ("Code Cache", "LOCALAPPDATA", "Google\\Chrome\\User Data\\Default\\Code Cache"),
            ("GPU Cache", "LOCALAPPDATA", "Google\\Chrome\\User Data\\Default\\GPUCache"),
            ("Media Cache", "LOCALAPPDATA", "Google\\Chrome\\User Data\\Default\\Media Cache"),
            ("Shader Cache", "LOCALAPPDATA", "Google\\Chrome\\User Data\\ShaderCache"),
        ],
    },
    Spec {
        name: "Microsoft Edge",
        roots: &[("Data", "LOCALAPPDATA", "Microsoft\\Edge")],
        caches: &[
            ("Cache", "LOCALAPPDATA", "Microsoft\\Edge\\User Data\\Default\\Cache"),
            ("Code Cache", "LOCALAPPDATA", "Microsoft\\Edge\\User Data\\Default\\Code Cache"),
            ("GPU Cache", "LOCALAPPDATA", "Microsoft\\Edge\\User Data\\Default\\GPUCache"),
        ],
    },
    Spec {
        name: "Mozilla Firefox",
        roots: &[("Data", "LOCALAPPDATA", "Mozilla\\Firefox")],
        caches: &[("Cache", "LOCALAPPDATA", "Mozilla\\Firefox\\Profiles\\*\\cache2")],
    },
    Spec {
        name: "Discord",
        roots: &[("Data", "APPDATA", "discord")],
        caches: &[
            ("Cache", "APPDATA", "discord\\Cache"),
            ("Code Cache", "APPDATA", "discord\\Code Cache"),
            ("GPU Cache", "APPDATA", "discord\\GPUCache"),
        ],
    },
    Spec {
        name: "Spotify",
        roots: &[("Application", "APPDATA", "Spotify")],
        caches: &[("Cache", "LOCALAPPDATA", "Spotify")],
    },
    Spec {
        name: "Steam",
        roots: &[("Installation", "ProgramFiles(x86)", "Steam")],
        caches: &[
            ("App Cache", "ProgramFiles(x86)", "Steam\\appcache"),
            ("Shader Cache", "ProgramFiles(x86)", "Steam\\steamapps\\shadercache"),
        ],
    },
    Spec {
        name: "Epic Games Launcher",
        roots: &[("Installation", "ProgramFiles(x86)", "Epic Games")],
        caches: &[("Web Cache", "LOCALAPPDATA", "EpicGamesLauncher\\Saved\\webcache")],
    },
    Spec {
        name: "VS Code",
        roots: &[
            ("Installation", "LOCALAPPDATA", "Programs\\Microsoft VS Code"),
            ("User Data", "APPDATA", "Code"),
            ("Extensions", "USERPROFILE", ".vscode"),
        ],
        caches: &[
            ("Cache", "APPDATA", "Code\\Cache"),
            ("Cached Data", "APPDATA", "Code\\CachedData"),
            ("Code Cache", "APPDATA", "Code\\Code Cache"),
            ("GPU Cache", "APPDATA", "Code\\GPUCache"),
        ],
    },
    Spec {
        name: "Visual Studio",
        roots: &[
            ("Installation", "ProgramFiles", "Microsoft Visual Studio"),
            ("Local Data", "LOCALAPPDATA", "Microsoft\\VisualStudio"),
        ],
        caches: &[("Component Cache", "LOCALAPPDATA", "Microsoft\\VisualStudio\\*\\ComponentModelCache")],
    },
    Spec {
        name: "Node.js",
        roots: &[("Installation", "ProgramFiles", "nodejs")],
        caches: &[],
    },
    Spec {
        name: "npm",
        roots: &[("Cache", "LOCALAPPDATA", "npm-cache")],
        caches: &[("Cache", "LOCALAPPDATA", "npm-cache")],
    },
    Spec {
        name: "pnpm",
        roots: &[("Store", "LOCALAPPDATA", "pnpm")],
        caches: &[("Store", "LOCALAPPDATA", "pnpm\\store")],
    },
    Spec {
        name: "Yarn",
        roots: &[("Cache", "LOCALAPPDATA", "Yarn")],
        caches: &[("Cache", "LOCALAPPDATA", "Yarn\\Cache")],
    },
    Spec {
        name: "Docker",
        roots: &[("Data", "LOCALAPPDATA", "Docker")],
        caches: &[],
    },
    Spec {
        name: "Android Studio",
        roots: &[
            ("Data", "LOCALAPPDATA", "Google\\AndroidStudio*"),
            ("Gradle", "USERPROFILE", ".gradle"),
            ("SDK / AVD", "USERPROFILE", ".android"),
        ],
        caches: &[("Gradle Caches", "USERPROFILE", ".gradle\\caches")],
    },
    Spec {
        name: "IntelliJ IDEA",
        roots: &[("Data", "LOCALAPPDATA", "JetBrains")],
        caches: &[("Caches", "LOCALAPPDATA", "JetBrains\\IntelliJIdea*\\caches")],
    },
    Spec {
        name: "Adobe Creative Cloud",
        roots: &[
            ("Installation", "ProgramFiles", "Adobe"),
            ("Local Data", "LOCALAPPDATA", "Adobe"),
        ],
        caches: &[
            ("Media Cache Files", "APPDATA", "Adobe\\Common\\Media Cache Files"),
            ("Media Cache", "APPDATA", "Adobe\\Common\\Media Cache"),
        ],
    },
];

fn resolve(env: &str, rel: &str) -> Vec<PathBuf> {
    let Some(base) = env_path(env) else {
        return Vec::new();
    };
    let full = base.join(rel);
    if rel.contains('*') {
        expand_glob(&full)
    } else if full.exists() {
        vec![full]
    } else {
        Vec::new()
    }
}

pub fn analyze() -> Vec<AppReport> {
    SPECS
        .par_iter()
        .filter_map(|spec| {
            let roots: Vec<(String, PathBuf)> = spec
                .roots
                .iter()
                .flat_map(|(label, env, rel)| {
                    resolve(env, rel)
                        .into_iter()
                        .map(move |p| (label.to_string(), p))
                })
                .collect();
            if roots.is_empty() {
                return None;
            }

            let caches: Vec<(String, PathBuf)> = spec
                .caches
                .iter()
                .flat_map(|(label, env, rel)| {
                    resolve(env, rel)
                        .into_iter()
                        .map(move |p| (label.to_string(), p))
                })
                .collect();

            let mut components = Vec::new();
            let mut total = 0u64;
            for (label, path) in &roots {
                let bytes = dir_size(path);
                total += bytes;
                components.push(AppComponent {
                    label: label.clone(),
                    path: path.to_string_lossy().into_owned(),
                    bytes,
                    recoverable: false,
                });
            }

            let mut recoverable = 0u64;
            for (label, path) in &caches {
                let bytes = dir_size(path);
                recoverable += bytes;
                // Caches outside every root are not yet part of the total.
                if !roots.iter().any(|(_, root)| path.starts_with(root)) {
                    total += bytes;
                }
                components.push(AppComponent {
                    label: label.clone(),
                    path: path.to_string_lossy().into_owned(),
                    bytes,
                    recoverable: true,
                });
            }

            Some(AppReport {
                name: spec.name.to_string(),
                total_bytes: total,
                recoverable_bytes: recoverable.min(total),
                status: "analyzed".to_string(),
                components,
            })
        })
        .collect()
}
