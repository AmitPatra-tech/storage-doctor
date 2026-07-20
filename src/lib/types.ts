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
  classification?: Classification | null;
}

export interface BrowseEntry {
  path: string;
  name: string;
  isDir: boolean;
  sizeBytes: number;
  fileCount: number;
  classification?: Classification | null;
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

export interface SearchProgress {
  found: number;
  done: boolean;
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
