import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { EyeOff, FolderOpen, Trash2 } from "lucide-react";
import { backend } from "@/lib/backend";
import { refreshAfterCleanup } from "@/lib/refresh";
import { formatBytes } from "@/lib/utils";
import type { LargeFile } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { SafetyBadge } from "@/components/ui/badge";
import { PageHeader } from "@/components/PageHeader";
import { ConfirmDeleteModal } from "@/components/ConfirmDeleteModal";

const SIZE_FILTERS = [
  { label: "100 MB+", bytes: 100 * 2 ** 20 },
  { label: "500 MB+", bytes: 500 * 2 ** 20 },
  { label: "1 GB+", bytes: 2 ** 30 },
  { label: "5 GB+", bytes: 5 * 2 ** 30 },
  { label: "10 GB+", bytes: 10 * 2 ** 30 },
];

const TYPE_FILTERS: { label: string; extensions: string[] | null }[] = [
  { label: "All types", extensions: null },
  { label: "Disc images (ISO, VHD, VMDK)", extensions: ["iso", "vhd", "vhdx", "vmdk"] },
  { label: "Archives (ZIP, RAR, 7Z)", extensions: ["zip", "rar", "7z"] },
  { label: "Video (MP4, MKV, MOV)", extensions: ["mp4", "mkv", "mov", "avi"] },
  { label: "Design (PSD, AI)", extensions: ["psd", "ai"] },
  { label: "Installers (EXE, MSI)", extensions: ["exe", "msi"] },
];

const IGNORED_KEY = "storage-doctor:ignored-files";

function loadIgnored(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(IGNORED_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

const selectClass =
  "h-9 rounded-md border border-border bg-surface px-3 text-sm focus:outline-none focus:ring-2 focus:ring-primary";

export function LargeFiles() {
  const queryClient = useQueryClient();
  const [minBytes, setMinBytes] = useState(SIZE_FILTERS[0].bytes);
  const [typeIndex, setTypeIndex] = useState(0);
  const [ignored, setIgnored] = useState(loadIgnored);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const { data: scan } = useQuery({
    queryKey: ["lastScan"],
    queryFn: backend.getLastScan,
  });

  const files = useMemo(() => {
    const extensions = TYPE_FILTERS[typeIndex].extensions;
    return (scan?.largestFiles ?? []).filter(
      (f) =>
        f.sizeBytes >= minBytes &&
        !ignored.has(f.path) &&
        (!extensions || extensions.includes(f.extension))
    );
  }, [scan, minBytes, typeIndex, ignored]);

  const ignoreFile = (path: string) => {
    const next = new Set(ignored);
    next.add(path);
    localStorage.setItem(IGNORED_KEY, JSON.stringify([...next]));
    setIgnored(next);
  };

  const toggleSelect = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const selectedFiles = files.filter((f) => selected.has(f.path));
  const selectSafe = () => {
    setSelected(
      new Set(files.filter((f) => f.classification?.safety === "safe").map((f) => f.path))
    );
  };

  const afterDelete = (freed: number) => {
    setConfirmOpen(false);
    setSelected(new Set());
    setNotice(`Freed ${formatBytes(freed)} — files permanently deleted.`);
    refreshAfterCleanup(queryClient);
  };

  const totalBytes = files.reduce((sum, f) => sum + f.sizeBytes, 0);
  const safeCount = files.filter((f) => f.classification?.safety === "safe").length;

  return (
    <div>
      <PageHeader
        title="Large Files"
        description={`${files.length} files · ${formatBytes(totalBytes)}. Badges show which are safe to delete.`}
        actions={
          <div className="flex gap-2">
            <select
              className={selectClass}
              value={minBytes}
              onChange={(e) => setMinBytes(Number(e.target.value))}
            >
              {SIZE_FILTERS.map((f) => (
                <option key={f.bytes} value={f.bytes}>
                  {f.label}
                </option>
              ))}
            </select>
            <select
              className={selectClass}
              value={typeIndex}
              onChange={(e) => setTypeIndex(Number(e.target.value))}
            >
              {TYPE_FILTERS.map((f, i) => (
                <option key={f.label} value={i}>
                  {f.label}
                </option>
              ))}
            </select>
          </div>
        }
      />

      <div className="mb-3 flex items-center gap-3 rounded-md border border-border bg-surface px-4 py-2.5 text-sm">
        <span>
          {selected.size > 0
            ? `${selected.size} selected · ${formatBytes(
                selectedFiles.reduce((s, f) => s + f.sizeBytes, 0)
              )}`
            : safeCount > 0
              ? `${safeCount} file(s) here are marked safe to delete`
              : "Select files to delete them together"}
        </span>
        {safeCount > 0 && (
          <Button variant="secondary" size="sm" onClick={selectSafe}>
            Select all safe
          </Button>
        )}
        <Button
          variant="danger"
          size="sm"
          disabled={selected.size === 0}
          onClick={() => setConfirmOpen(true)}
        >
          <Trash2 className="h-3.5 w-3.5" />
          Delete selected
        </Button>
        {selected.size > 0 && (
          <Button variant="ghost" size="sm" onClick={() => setSelected(new Set())}>
            Clear
          </Button>
        )}
      </div>

      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}

      <Card>
        <CardContent className="divide-y divide-border p-0">
          {files.length === 0 && (
            <p className="p-5 text-sm text-muted">
              No files match the current filters. Run a scan first if you have not yet.
            </p>
          )}
          {files.map((file: LargeFile) => (
            <div key={file.path} className="flex items-center gap-3 px-5 py-3">
              <input
                type="checkbox"
                checked={selected.has(file.path)}
                onChange={() => toggleSelect(file.path)}
                className="h-3.5 w-3.5 shrink-0 accent-[hsl(199_89%_48%)]"
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium">{file.name}</span>
                  <SafetyBadge classification={file.classification} />
                </div>
                <div className="truncate text-xs text-muted">{file.path}</div>
              </div>
              <div className="shrink-0 text-right text-sm">{formatBytes(file.sizeBytes)}</div>
              <div className="flex shrink-0 items-center gap-1.5">
                <Button
                  variant="ghost"
                  size="icon"
                  title="Open location"
                  onClick={() => backend.revealInExplorer(file.path)}
                >
                  <FolderOpen className="h-4 w-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  title="Ignore"
                  onClick={() => ignoreFile(file.path)}
                >
                  <EyeOff className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <ConfirmDeleteModal
        open={confirmOpen}
        title="Delete selected files?"
        items={selectedFiles.map((f) => ({
          path: f.path,
          sizeBytes: f.sizeBytes,
          classification: f.classification,
        }))}
        source="Large files"
        permanent
        onDone={afterDelete}
        onClose={() => setConfirmOpen(false)}
      />
    </div>
  );
}
