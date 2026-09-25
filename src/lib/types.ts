export interface DriveInfo {
  letter: string;
  label: string;
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
  isRemovable: boolean;
}

export type Safety = "safe" | "review" | "personal" | "apps" | "system";

export interface Classification {
  category: string;
  safety: Safety;
  explanation: string;
}

export interface FolderEntry {
  path: string;
  name: string;
  sizeBytes: number;
  fileCount: number;
  /** How much of sizeBytes is safe to clear — 0 means the cleanup action has
   *  nothing to offer for this row. `null` means a scan taken before this was
   *  measured, which is not the same as zero. */
  recoverableBytes: number | null;
  classification?: Classification | null;
}

export interface BrowseEntry {
  path: string;
  name: string;
  isDir: boolean;
  sizeBytes: number;
  fileCount: number;
  recoverableBytes: number;
  classification?: Classification | null;
}

/** A folder measured on demand, filling a gap the last scan left. */
export interface FolderMeasurement {
  path: string;
  sizeBytes: number;
  fileCount: number;
  recoverableBytes: number;
}

export interface FolderDelta {
  path: string;
  name: string;
  deltaBytes: number;
  isNew: boolean;
}

export interface ScanComparison {
  previousAt: string;
  currentAt: string;
  deltaBytes: number;
  changes: FolderDelta[];
}

export interface LargeFile {
  path: string;
  name: string;
  extension: string;
  sizeBytes: number;
  modifiedAt: string;
  classification?: Classification | null;
}

export interface SafeItem {
  path: string;
  sizeBytes: number;
  fileCount: number;
  category: string;
  explanation: string;
}

export interface DupeFile {
  path: string;
  modifiedAt: string;
}

export interface DupeGroup {
  hash: string;
  sizeBytes: number;
  files: DupeFile[];
}

export interface DupeProgress {
  phase: "collecting" | "hashing" | "done";
  filesProcessed: number;
  totalFiles: number;
  bytesHashed: number;
}

export interface DeleteResult {
  freedBytes: number;
  failed: string[];
}

export interface ForceDeleteResult {
  freedBytes: number;
  /** Removed immediately, once whatever had them open was closed. */
  removed: string[];
  /** Not free yet — Windows will remove these automatically the next time
   *  the PC restarts, because nothing running right now could be made to
   *  let go of them. */
  scheduledForReboot: string[];
  /** Genuinely could not be handled at all. */
  failed: string[];
}

export interface ForceUninstallStep {
  label: string;
  ok: boolean;
}

export interface ForceUninstallReport {
  steps: ForceUninstallStep[];
  /** No matching registry entry and (if it had one) no install folder left —
   *  the practical definition of "actually uninstalled now". */
  complete: boolean;
}

export interface SearchProgress {
  /** The query these results belong to — late events from a superseded
   *  search are ignored rather than mixed into the current result list. */
  query: string;
  found: number;
  done: boolean;
  /** Matches found since the previous event, so results appear while the
   *  drives are still being walked. */
  items: SearchResultItem[];
}

export interface License {
  key: string;
  plan: "pro";
  activatedAt: string;
  expiresAt?: string | null;
  email?: string | null;
}

export interface OperationRecord {
  id: number;
  performedAt: string;
  source: string;
  itemCount: number;
  freedBytes: number;
  method: string;
}

export interface SearchResultItem {
  kind: "folder" | "file" | "recommendation" | "app";
  name: string;
  path: string;
  sizeBytes: number;
}

export type RiskLevel = "low" | "medium" | "high";

export interface Recommendation {
  id: string;
  name: string;
  description: string;
  recoverableBytes: number;
  risk: RiskLevel;
  recommended: boolean;
  paths: string[];
}

export interface AppComponent {
  label: string;
  path: string;
  bytes: number;
  recoverable: boolean;
}

export interface AppUsage {
  name: string;
  totalBytes: number;
  recoverableBytes: number;
  status: "analyzed" | "pending";
  components: AppComponent[];
}

export interface ScanProgress {
  scanId: number;
  filesScanned: number;
  bytesScanned: number;
  currentPath: string;
}

export interface ScanComplete {
  scanId: number;
  durationMs: number;
  filesScanned: number;
  bytesScanned: number;
  errors: number;
}

export interface InstalledApp {
  name: string;
  version: string;
  publisher: string;
  installLocation: string | null;
  estimatedBytes: number;
  uninstallString: string | null;
  displayIcon: string | null;
}

export interface Leftover {
  path: string;
  sizeBytes: number;
}

export interface ScanSummary {
  scanId: number;
  startedAt: string;
  durationMs: number;
  drives: DriveInfo[];
  largestFolders: FolderEntry[];
  largestFiles: LargeFile[];
  recoverableBytes: number;
}
