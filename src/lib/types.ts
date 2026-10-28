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
  /** Windows refused — can be retried as administrator. */
  failed: string[];
  /** Never deleted by this app (a drive, a core folder…), with the reason. */
  refused: ItemNote[];
}

/** An item that is still there, with the exact reason(s). */
export interface ItemNote {
  path: string;
  reasons: string[];
}

export interface ForceDeleteResult {
  /** Size of the items now gone. Not free disk space while they sit in the
   *  Recycle Bin. */
  freedBytes: number;
  /** Gone from their location. */
  removed: string[];
  /** Programs that were closed to free the items (unsaved work lost). */
  closedPrograms: string[];
  /** Still there — held open by something this app does not close. */
  blocked: ItemNote[];
  /** Still there; nothing holds it; set to be deleted permanently during the
   *  next restart. Not guaranteed if a program recreates it at startup. */
  scheduledForReboot: string[];
  /** Still there and could not be scheduled — with the reason. */
  failed: ItemNote[];
  /** The administrator step stopped before finishing. */
  elevationDeclined: boolean;
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
  /** The registry entry is gone, but Windows never recorded where the
   *  program's files are, so their removal can't be confirmed. */
  filesUnknown: boolean;
  /** Plain-language explanations of anything that did not work. */
  notes: string[];
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

/** A built-in or Microsoft Store app (AppX package). */
export interface StoreApp {
  displayName: string;
  /** Package name — what removal targets, e.g. "Microsoft.BingNews". */
  name: string;
  version: string;
  publisher: string;
  installLocation: string;
  /** Windows refuses to remove this one (a system component). */
  nonRemovable: boolean;
  /** What removing it breaks, when that matters. */
  caution: string | null;
  sizeBytes: number;
}

export interface StoreRemoveResult {
  /** Gone for every user account on this PC. */
  removed: boolean;
  /** Accounts it is still installed for. */
  remainingInstalls: number;
  /** Windows will still add it to new accounts. */
  stillProvisioned: boolean;
  notes: string[];
}

/** A folder left behind by an app that is no longer installed. */
export interface Orphan {
  path: string;
  sizeBytes: number;
  /** Unix seconds anything inside last changed (0 = unknown). */
  modifiedUnix: number;
  name: string;
  /** Where it was found, e.g. "AppData (Roaming)". */
  location: string;
}
