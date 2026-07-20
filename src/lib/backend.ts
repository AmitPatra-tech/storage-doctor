import type {
  AppUsage,
  BrowseEntry,
  DeleteResult,
  DriveInfo,
  DupeGroup,
  DupeProgress,
  InstalledApp,
  Leftover,
  License,
  OperationRecord,
  Recommendation,
  SafeItem,
  ScanComparison,
  ScanComplete,
  ScanProgress,
  ScanSummary,
  SearchProgress,
  SearchResultItem,
} from "./types";

// In-browser dev (vite without Tauri) falls back to mock data so the UI
// remains workable; inside the Tauri webview `__TAURI_INTERNALS__` exists.
const isTauri = "__TAURI_INTERNALS__" in window;

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

const mockDrives: DriveInfo[] = [
  {
    letter: "C:",
    label: "Local Disk",
    totalBytes: 512 * 2 ** 30,
    usedBytes: 391 * 2 ** 30,
    freeBytes: 121 * 2 ** 30,
    isRemovable: false,
  },
  {
    letter: "D:",
    label: "Data",
    totalBytes: 1024 * 2 ** 30,
    usedBytes: 610 * 2 ** 30,
    freeBytes: 414 * 2 ** 30,
    isRemovable: false,
  },
];

const mockScan: ScanSummary = {
  scanId: 1,
  startedAt: new Date().toISOString(),
  durationMs: 74_000,
  drives: mockDrives,
  largestFolders: [
    { path: "C:\\Users", name: "Users", sizeBytes: 182 * 2 ** 30, fileCount: 412_331 },
    { path: "C:\\Program Files", name: "Program Files", sizeBytes: 96 * 2 ** 30, fileCount: 158_204 },
    { path: "C:\\Windows", name: "Windows", sizeBytes: 41 * 2 ** 30, fileCount: 210_887 },
    { path: "C:\\ProgramData", name: "ProgramData", sizeBytes: 22 * 2 ** 30, fileCount: 64_112 },
    { path: "C:\\Users\\me\\Downloads", name: "Downloads", sizeBytes: 19 * 2 ** 30, fileCount: 1_204 },
  ],
  largestFiles: [
    {
      path: "C:\\Users\\me\\Downloads\\Windows11.iso",
      name: "Windows11.iso",
      extension: "iso",
      sizeBytes: 5.8 * 2 ** 30,
      modifiedAt: "2025-01-12T10:00:00Z",
      classification: {
        category: "Downloads",
        safety: "review",
        explanation: "Files you downloaded. Old installers, archives and ISO images here are usually safe to delete.",
      },
    },
    {
      path: "C:\\Users\\me\\Videos\\project-final.mp4",
      name: "project-final.mp4",
      extension: "mp4",
      sizeBytes: 3.2 * 2 ** 30,
      modifiedAt: "2025-11-03T18:30:00Z",
      classification: {
        category: "Personal files",
        safety: "personal",
        explanation: "Your personal files. They cannot be recreated — review carefully before deleting.",
      },
    },
  ],
  recoverableBytes: 34 * 2 ** 30,
};

const mockRecommendations: Recommendation[] = [
  {
    id: "nvidia-shader-cache",
    name: "NVIDIA Shader Cache",
    description: "Graphics shader cache that Windows recreates automatically.",
    recoverableBytes: 4 * 2 ** 30,
    risk: "low",
    recommended: true,
    paths: ["C:\\Users\\me\\AppData\\Local\\NVIDIA\\DXCache"],
  },
  {
    id: "npm-cache",
    name: "npm Cache",
    description: "Package cache used by npm. Re-downloaded on demand.",
    recoverableBytes: 2.4 * 2 ** 30,
    risk: "low",
    recommended: true,
    paths: ["C:\\Users\\me\\AppData\\Local\\npm-cache"],
  },
  {
    id: "old-installers",
    name: "Old Installers in Downloads",
    description: "Setup files not opened in over 6 months.",
    recoverableBytes: 8.1 * 2 ** 30,
    risk: "medium",
    recommended: false,
    paths: ["C:\\Users\\me\\Downloads\\Windows11.iso"],
  },
];

const mockApps: AppUsage[] = [
  {
    name: "Google Chrome",
    totalBytes: 8.4 * 2 ** 30,
    recoverableBytes: 3.1 * 2 ** 30,
    status: "analyzed",
    components: [
      { label: "Data", path: "C:\\Users\\me\\AppData\\Local\\Google\\Chrome", bytes: 8.4 * 2 ** 30, recoverable: false },
      { label: "Cache", path: "C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache", bytes: 2.1 * 2 ** 30, recoverable: true },
      { label: "Code Cache", path: "C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Code Cache", bytes: 1 * 2 ** 30, recoverable: true },
    ],
  },
  {
    name: "VS Code",
    totalBytes: 2.9 * 2 ** 30,
    recoverableBytes: 1.2 * 2 ** 30,
    status: "analyzed",
    components: [
      { label: "Installation", path: "C:\\Users\\me\\AppData\\Local\\Programs\\Microsoft VS Code", bytes: 1.7 * 2 ** 30, recoverable: false },
      { label: "Cached Data", path: "C:\\Users\\me\\AppData\\Roaming\\Code\\CachedData", bytes: 1.2 * 2 ** 30, recoverable: true },
    ],
  },
  {
    name: "Docker",
    totalBytes: 24.6 * 2 ** 30,
    recoverableBytes: 0,
    status: "analyzed",
    components: [
      { label: "Data", path: "C:\\Users\\me\\AppData\\Local\\Docker", bytes: 24.6 * 2 ** 30, recoverable: false },
    ],
  },
];

export type Unsubscribe = () => void;

// Mock event plumbing so the scan flow is testable in a plain browser.
const mockProgressListeners = new Set<(p: ScanProgress) => void>();
const mockCompleteListeners = new Set<(c: ScanComplete) => void>();

function runMockScan() {
  let files = 0;
  let bytes = 0;
  const paths = [
    "C:\\Users\\me\\AppData\\Local",
    "C:\\Program Files\\Common Files",
    "C:\\Windows\\WinSxS",
    "C:\\Users\\me\\Downloads",
  ];
  const started = Date.now();
  const timer = setInterval(() => {
    files += 40_000 + Math.floor(Math.random() * 20_000);
    bytes += 12 * 2 ** 30;
    const progress: ScanProgress = {
      scanId: mockScan.scanId,
      filesScanned: files,
      bytesScanned: bytes,
      currentPath: paths[Math.floor(Math.random() * paths.length)],
    };
    mockProgressListeners.forEach((cb) => cb(progress));
    if (Date.now() - started > 3_000) {
      clearInterval(timer);
      mockCompleteListeners.forEach((cb) =>
        cb({
          scanId: mockScan.scanId,
          durationMs: Date.now() - started,
          filesScanned: files,
          bytesScanned: bytes,
          errors: 12,
        })
      );
    }
  }, 250);
}

async function listenTauri<T>(
  event: string,
  cb: (payload: T) => void
): Promise<Unsubscribe> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => cb(e.payload));
}

export const backend = {
  async getDrives(): Promise<DriveInfo[]> {
    if (!isTauri) return mockDrives;
    return invoke<DriveInfo[]>("get_drives");
  },

  async getLastScan(): Promise<ScanSummary | null> {
    if (!isTauri) return mockScan;
    return invoke<ScanSummary | null>("get_last_scan");
  },

  async startScan(driveLetters: string[]): Promise<number> {
    if (!isTauri) {
      runMockScan();
      return mockScan.scanId;
    }
    return invoke<number>("start_scan", { driveLetters });
  },

  async onScanProgress(cb: (p: ScanProgress) => void): Promise<Unsubscribe> {
    if (!isTauri) {
      mockProgressListeners.add(cb);
      return () => mockProgressListeners.delete(cb);
    }
    return listenTauri<ScanProgress>("scan-progress", cb);
  },

  async onScanComplete(cb: (c: ScanComplete) => void): Promise<Unsubscribe> {
    if (!isTauri) {
      mockCompleteListeners.add(cb);
      return () => mockCompleteListeners.delete(cb);
    }
    return listenTauri<ScanComplete>("scan-complete", cb);
  },

  async onScanError(cb: (message: string) => void): Promise<Unsubscribe> {
    if (!isTauri) return () => {};
    return listenTauri<string>("scan-error", cb);
  },

  async getRecommendations(): Promise<Recommendation[]> {
    if (!isTauri) return mockRecommendations;
    return invoke<Recommendation[]>("get_recommendations");
  },

  async setRecommendationIgnored(id: string, ignored: boolean): Promise<void> {
    if (!isTauri) return;
    return invoke("set_recommendation_ignored", { id, ignored });
  },

  async browseFolder(path: string): Promise<BrowseEntry[]> {
    if (!isTauri) {
      return [
        {
          path: `${path}\\Cache`,
          name: "Cache",
          isDir: true,
          sizeBytes: 3.2 * 2 ** 30,
          fileCount: 12_400,
          classification: {
            category: "Cache",
            safety: "safe",
            explanation: "Safe to delete — applications rebuild caches automatically.",
          },
        },
        {
          path: `${path}\\Documents`,
          name: "Documents",
          isDir: true,
          sizeBytes: 1.1 * 2 ** 30,
          fileCount: 840,
          classification: {
            category: "Personal files",
            safety: "personal",
            explanation: "Your personal files. They cannot be recreated.",
          },
        },
        {
          path: `${path}\\setup.exe`,
          name: "setup.exe",
          isDir: false,
          sizeBytes: 0.4 * 2 ** 30,
          fileCount: 1,
          classification: null,
        },
      ];
    }
    return invoke<BrowseEntry[]>("browse_folder", { path });
  },

  async getScanComparison(): Promise<ScanComparison | null> {
    if (!isTauri) {
      return {
        previousAt: new Date(Date.now() - 2 * 86_400_000).toISOString(),
        currentAt: new Date().toISOString(),
        deltaBytes: 12.4 * 2 ** 30,
        changes: [
          { path: "C:\\Users\\me\\AppData\\Local\\Docker", name: "Docker", deltaBytes: 8 * 2 ** 30, isNew: false },
          { path: "C:\\Users\\me\\Downloads", name: "Downloads", deltaBytes: 3.1 * 2 ** 30, isNew: false },
          { path: "C:\\Windows\\SoftwareDistribution", name: "SoftwareDistribution", deltaBytes: -1.2 * 2 ** 30, isNew: false },
        ],
      };
    }
    return invoke<ScanComparison | null>("get_scan_comparison");
  },

  async getAppUsage(): Promise<AppUsage[]> {
    if (!isTauri) return mockApps;
    return invoke<AppUsage[]>("get_app_usage");
  },

  async getInstalledApps(): Promise<InstalledApp[]> {
    if (!isTauri) {
      return [
        {
          name: "Google Chrome",
          version: "126.0",
          publisher: "Google LLC",
          installLocation: "C:\\Program Files\\Google\\Chrome\\Application",
          estimatedBytes: 650 * 2 ** 20,
          uninstallString: "mock",
        },
        {
          name: "7-Zip 23.01 (x64)",
          version: "23.01",
          publisher: "Igor Pavlov",
          installLocation: "C:\\Program Files\\7-Zip",
          estimatedBytes: 5 * 2 ** 20,
          uninstallString: "mock",
        },
      ];
    }
    return invoke<InstalledApp[]>("get_installed_apps");
  },

  async launchUninstaller(uninstallString: string): Promise<void> {
    if (!isTauri) return;
    return invoke("launch_uninstaller", { uninstallString });
  },

  async findAppLeftovers(app: InstalledApp): Promise<Leftover[]> {
    if (!isTauri) {
      return [
        { path: "C:\\Users\\me\\AppData\\Local\\Google\\Chrome", sizeBytes: 900 * 2 ** 20 },
        { path: "C:\\Users\\me\\AppData\\Roaming\\Google\\Chrome", sizeBytes: 40 * 2 ** 20 },
      ];
    }
    return invoke<Leftover[]>("find_app_leftovers", {
      name: app.name,
      publisher: app.publisher,
      installLocation: app.installLocation,
    });
  },

  /** Deletes the given paths. `permanent` removes them outright (for caches —
   *  frees space, they're auto-recreated); otherwise they go to the Recycle
   *  Bin. Failures are returned rather than aborting the whole batch. */
  async deletePaths(
    paths: string[],
    source?: string,
    permanent = false
  ): Promise<DeleteResult> {
    if (!isTauri) return { freedBytes: paths.length * 2 ** 20, failed: [] };
    return invoke<DeleteResult>("delete_paths", { paths, source, permanent });
  },

  /** Retries deletion with administrator rights (one UAC prompt). */
  async deletePathsElevated(
    paths: string[],
    source?: string,
    permanent = false
  ): Promise<DeleteResult> {
    if (!isTauri) return { freedBytes: paths.length * 2 ** 20, failed: [] };
    return invoke<DeleteResult>("delete_paths_elevated", { paths, source, permanent });
  },

  async regenerateRecommendations(): Promise<void> {
    if (!isTauri) return;
    return invoke("regenerate_recommendations");
  },

  async getOperations(): Promise<OperationRecord[]> {
    if (!isTauri) {
      return [
        { id: 3, performedAt: new Date().toISOString(), source: "Duplicate cleanup", itemCount: 4, freedBytes: 18.3 * 2 ** 20, method: "recycle" },
        { id: 2, performedAt: new Date(Date.now() - 86400000).toISOString(), source: "npm Cache", itemCount: 1, freedBytes: 2.4 * 2 ** 30, method: "recycle" },
        { id: 1, performedAt: new Date(Date.now() - 172800000).toISOString(), source: "App leftovers", itemCount: 2, freedBytes: 2.5 * 2 ** 20, method: "recycle-admin" },
      ];
    }
    return invoke<OperationRecord[]>("get_operations");
  },

  async getLicense(): Promise<License | null> {
    if (!isTauri) {
      const raw = localStorage.getItem("storage-doctor:license");
      return raw ? (JSON.parse(raw) as License) : null;
    }
    const json = await invoke<string | null>("get_license");
    return json ? (JSON.parse(json) as License) : null;
  },

  async setLicense(license: License): Promise<void> {
    if (!isTauri) {
      localStorage.setItem("storage-doctor:license", JSON.stringify(license));
      return;
    }
    return invoke("set_license", { license: JSON.stringify(license) });
  },

  async clearLicense(): Promise<void> {
    if (!isTauri) {
      localStorage.removeItem("storage-doctor:license");
      return;
    }
    return invoke("clear_license");
  },

  async openExternal(url: string): Promise<void> {
    if (!isTauri) {
      window.open(url, "_blank");
      return;
    }
    return invoke("open_external", { url });
  },

  /** Saves PDF bytes via a native save dialog. Returns the path, or null if
   *  cancelled. In the browser, falls back to a normal download. */
  async savePdf(bytes: Uint8Array, defaultName: string): Promise<string | null> {
    if (!isTauri) {
      const blob = new Blob([bytes as unknown as BlobPart], { type: "application/pdf" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = defaultName;
      a.click();
      URL.revokeObjectURL(url);
      return defaultName;
    }
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      defaultPath: defaultName,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!path) return null;
    await invoke("write_bytes", { path, bytes: Array.from(bytes) });
    return path;
  },

  async findSafeCleanup(path: string): Promise<SafeItem[]> {
    if (!isTauri) {
      return [
        {
          path: `${path}\\AppData\\Local\\Temp`,
          sizeBytes: 1.2 * 2 ** 30,
          fileCount: 4_210,
          category: "Temporary files",
          explanation: "Safe to delete — programs recreate temporary files whenever they need them.",
        },
        {
          path: `${path}\\project\\node_modules`,
          sizeBytes: 0.8 * 2 ** 30,
          fileCount: 51_000,
          category: "Project dependencies",
          explanation: "Safe to delete for projects you are not working on — `npm install` recreates it.",
        },
      ];
    }
    return invoke<SafeItem[]>("find_safe_cleanup", { path });
  },

  async findDuplicates(roots: string[] = []): Promise<DupeGroup[]> {
    if (!isTauri) {
      await new Promise((r) => setTimeout(r, 1200));
      return [
        {
          hash: "a1b2c3d4e5f60718",
          sizeBytes: 350 * 2 ** 20,
          files: [
            { path: "C:\\Users\\me\\Videos\\holiday.mp4", modifiedAt: "2025-06-01T10:00:00Z" },
            { path: "C:\\Users\\me\\Downloads\\holiday (1).mp4", modifiedAt: "2025-08-11T09:00:00Z" },
          ],
        },
        {
          hash: "0f9e8d7c6b5a4938",
          sizeBytes: 120 * 2 ** 20,
          files: [
            { path: "C:\\Users\\me\\Documents\\report-final.pdf", modifiedAt: "2025-04-20T14:00:00Z" },
            { path: "C:\\Users\\me\\Desktop\\report-final.pdf", modifiedAt: "2025-04-21T08:00:00Z" },
            { path: "C:\\Users\\me\\Downloads\\report-final (2).pdf", modifiedAt: "2025-05-02T16:00:00Z" },
          ],
        },
      ];
    }
    return invoke<DupeGroup[]>("find_duplicates", { roots });
  },

  async onDupeProgress(cb: (p: DupeProgress) => void): Promise<Unsubscribe> {
    if (!isTauri) return () => {};
    return listenTauri<DupeProgress>("dupe-progress", cb);
  },

  async search(query: string): Promise<SearchResultItem[]> {
    if (!isTauri) {
      const q = query.toLowerCase();
      return [
        { kind: "folder" as const, name: "Downloads", path: "C:\\Users\\me\\Downloads", sizeBytes: 19 * 2 ** 30 },
        { kind: "file" as const, name: "Windows11.iso", path: "C:\\Users\\me\\Downloads\\Windows11.iso", sizeBytes: 5.8 * 2 ** 30 },
        { kind: "app" as const, name: "Google Chrome", path: "C:\\Program Files\\Google\\Chrome", sizeBytes: 650 * 2 ** 20 },
      ].filter((r) => r.name.toLowerCase().includes(q));
    }
    return invoke<SearchResultItem[]>("search", { query });
  },

  async onSearchProgress(cb: (p: SearchProgress) => void): Promise<Unsubscribe> {
    if (!isTauri) return () => {};
    return listenTauri<SearchProgress>("search-progress", cb);
  },

  /** Opens a native folder picker; returns selected directory paths. */
  async pickFolders(): Promise<string[]> {
    if (!isTauri) return ["D:\\Finished Songs"];
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({ directory: true, multiple: true });
    if (!result) return [];
    return Array.isArray(result) ? result : [result];
  },

  /** Live filesystem search across all fixed drives (finds files of any size). */
  async searchFiles(query: string): Promise<SearchResultItem[]> {
    if (!isTauri) {
      await new Promise((r) => setTimeout(r, 800));
      const q = query.toLowerCase();
      return [
        { kind: "file" as const, name: "logo.png", path: "D:\\App\\assets\\logo.png", sizeBytes: 240 * 2 ** 10 },
        { kind: "file" as const, name: "logo.png", path: "C:\\Users\\me\\Pictures\\logo.png", sizeBytes: 512 * 2 ** 10 },
      ].filter((r) => r.name.toLowerCase().includes(q));
    }
    return invoke<SearchResultItem[]>("search_files", { query });
  },

  async revealInExplorer(path: string): Promise<void> {
    if (!isTauri) return;
    return invoke("reveal_in_explorer", { path });
  },

  /** Moves the file to the Recycle Bin (never a permanent delete). */
  async deleteFile(path: string): Promise<void> {
    if (!isTauri) return;
    return invoke("delete_file", { path });
  },
};
