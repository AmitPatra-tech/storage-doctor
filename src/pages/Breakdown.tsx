import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  Info,
  Loader2,
  Sparkles,
  Trash2,
} from "lucide-react";
import { backend } from "@/lib/backend";
import { refreshAfterCleanup } from "@/lib/refresh";
import { formatBytes, formatPercent, formatRelativeTime } from "@/lib/utils";
import type { Classification, SafeItem } from "@/lib/types";
import { SafetyBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Modal } from "@/components/ui/modal";
import { PageHeader } from "@/components/PageHeader";
import { ConfirmDeleteModal, type DeleteItem } from "@/components/ConfirmDeleteModal";

interface Crumb {
  path: string;
  name: string;
  sizeBytes: number;
}

interface Row {
  path: string;
  name: string;
  isDir: boolean;
  sizeBytes: number;
  fileCount: number;
  /** `0` = nothing to clear, `null` = this scan never measured it. Neither
   *  gets a cleanup action; the banner explains the `null` case. */
  recoverableBytes: number | null;
  classification?: Classification | null;
}

/** Dialog for the per-folder Clean action: lists everything safe to delete
 *  inside the folder, however deep, and deletes the checked items. */
function SafeCleanModal({
  folderPath,
  onClose,
  onDeleted,
}: {
  folderPath: string;
  onClose: () => void;
  onDeleted: (freed: number) => void;
}) {
  const [selected, setSelected] = useState<Set<string> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const { data: items, isLoading } = useQuery({
    queryKey: ["safeCleanup", folderPath],
    queryFn: () => backend.findSafeCleanup(folderPath),
    staleTime: 30_000,
  });

  // Default: everything checked.
  const checked = selected ?? new Set((items ?? []).map((i) => i.path));
  const toggle = (path: string) => {
    const next = new Set(checked);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const selectedItems = (items ?? []).filter((i) => checked.has(i.path));
  const totalBytes = selectedItems.reduce((s, i) => s + i.sizeBytes, 0);

  const clean = async () => {
    setBusy(true);
    setError(null);
    try {
      // Safe caches/temp are permanently removed so the space is actually
      // reclaimed (they are recreated automatically).
      const res = await backend.deletePaths(
        selectedItems.map((i) => i.path),
        "Safe cleanup",
        true
      );
      let freed = res.freedBytes;
      // Auto-retry any that need admin rights (one UAC prompt).
      if (res.failed.length > 0) {
        const elevated = await backend.deletePathsElevated(res.failed, "Safe cleanup", true);
        freed += elevated.freedBytes;
        if (elevated.failed.length > 0) {
          setError(`${elevated.failed.length} item(s) could not be removed.`);
        }
      }
      onDeleted(freed);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={busy ? () => {} : onClose}
      title={`Safe cleanup — ${folderPath}`}
      footer={
        <>
          <Button variant="secondary" size="sm" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            size="sm"
            disabled={busy || selectedItems.length === 0}
            onClick={clean}
          >
            <Trash2 className="h-3.5 w-3.5" />
            {busy
              ? "Cleaning…"
              : `Clean ${selectedItems.length} item(s) (${formatBytes(totalBytes)})`}
          </Button>
        </>
      }
    >
      {isLoading && (
        <div className="flex items-center gap-3 py-4 text-sm text-muted">
          <Loader2 className="h-4 w-4 animate-spin text-primary" />
          Searching this folder for content that is safe to delete…
        </div>
      )}
      {items && items.length === 0 && (
        <p className="py-4 text-sm text-muted">
          Nothing here is confidently safe to delete automatically. Drill into the folder
          to review its contents manually.
        </p>
      )}
      {items && items.length > 0 && (
        <>
          <p className="mb-3 text-sm text-muted">
            Everything below was identified as safe to delete — caches, temporary files
            and other content that applications recreate automatically. Files go to the
            Recycle Bin.
          </p>
          <div className="flex flex-col divide-y divide-border">
            {items.map((item: SafeItem) => (
              <label key={item.path} className="flex cursor-pointer items-start gap-3 py-2.5">
                <input
                  type="checkbox"
                  checked={checked.has(item.path)}
                  onChange={() => toggle(item.path)}
                  className="mt-0.5 h-3.5 w-3.5 accent-[hsl(199_89%_48%)]"
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 text-sm">
                    <span className="truncate">{item.path}</span>
                  </div>
                  <div className="text-xs text-muted">
                    {item.category} · {item.fileCount.toLocaleString()} files —{" "}
                    {item.explanation}
                  </div>
                </div>
                <span className="shrink-0 text-sm">{formatBytes(item.sizeBytes)}</span>
              </label>
            ))}
          </div>
        </>
      )}
      {error && <p className="mt-3 text-xs text-danger">{error}</p>}
    </Modal>
  );
}

export function Breakdown() {
  const queryClient = useQueryClient();
  const [trail, setTrail] = useState<Crumb[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [cleanTarget, setCleanTarget] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const current = trail[trail.length - 1] ?? null;

  const { data: scan } = useQuery({
    queryKey: ["lastScan"],
    queryFn: backend.getLastScan,
  });

  const { data: browsed, isFetching, dataUpdatedAt } = useQuery({
    queryKey: ["browse", current?.path],
    queryFn: () => backend.browseFolder(current!.path),
    enabled: current !== null,
    // Re-measured on every visit: the sizes here are read live from disk and
    // must not be served from cache after something was deleted.
    staleTime: 0,
    gcTime: 0,
  });

  // Measuring a folder also corrects its recorded size in the stored scan, so
  // pick the corrected numbers up for the list behind this one.
  useEffect(() => {
    if (browsed) queryClient.invalidateQueries({ queryKey: ["lastScan"] });
  }, [dataUpdatedAt, browsed, queryClient]);

  // The top-level list covers every scanned drive, so percentages are shares
  // of everything scanned rather than of one drive.
  const scannedUsedBytes = (scan?.drives ?? []).reduce((s, d) => s + d.usedBytes, 0);

  // Folders the last scan never measured. Rather than making the user run
  // another scan or open each one, measure them here and fill the figures in
  // as they arrive.
  const [measured, setMeasured] = useState<Record<string, number>>({});
  const [measuring, setMeasuring] = useState(false);
  /** Paths already handed to the backend, so a refetch mid-run never starts a
   *  second walk over the same folders. */
  const requested = useRef<Set<string>>(new Set());

  // Kept separate from the trigger below and mounted once: results stream in
  // over the whole run, and re-subscribing on every refetch would drop them.
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    backend
      .onFolderMeasured((m) => {
        if (live) setMeasured((prev) => ({ ...prev, [m.path]: m.recoverableBytes }));
      })
      .then((unsubscribe) => {
        if (live) stop = unsubscribe;
        else unsubscribe();
      });
    return () => {
      live = false;
      stop?.();
    };
  }, []);

  const scanFolders = scan?.largestFolders;
  useEffect(() => {
    const missing = (scanFolders ?? [])
      .filter((f) => f.recoverableBytes === null && !requested.current.has(f.path))
      .map((f) => f.path);
    if (missing.length === 0) return;
    missing.forEach((path) => requested.current.add(path));

    setMeasuring(true);
    backend.measureRecoverable(missing).finally(() => {
      setMeasuring(false);
      // The measurements were stored too, so pick them up from the scan.
      queryClient.invalidateQueries({ queryKey: ["lastScan"] });
    });
  }, [scanFolders, queryClient]);

  // Inside a folder, percentages are shares of what it actually holds now —
  // not of the size the last scan recorded for it, which may be long stale.
  const liveBytes = (browsed ?? []).reduce((s, e) => s + e.sizeBytes, 0);
  const parentBytes = current === null ? scannedUsedBytes : liveBytes;

  // The list you clicked through showed a scanned size; this folder was just
  // measured live. When they disagree the user deserves to be told why rather
  // than left staring at "330 MB" turning into "129 B".
  const recordedBytes = current?.sizeBytes ?? 0;
  const shrankSinceScan =
    browsed !== undefined &&
    browsed.length < 300 &&
    recordedBytes > 0 &&
    recordedBytes - liveBytes > 4 * 2 ** 20 &&
    liveBytes < recordedBytes * 0.9;

  const rows: Row[] | undefined =
    current === null
      ? scan?.largestFolders
          // A folder the scan recorded can be gone or empty by now; the stored
          // size is updated on delete, so drop anything that reached zero.
          .filter((f) => f.sizeBytes > 0)
          .map((f) => ({
            path: f.path,
            name: f.name,
            isDir: true,
            sizeBytes: f.sizeBytes,
            fileCount: f.fileCount,
            recoverableBytes: f.recoverableBytes ?? measured[f.path] ?? null,
            classification: f.classification,
          }))
      : browsed;

  const navigate = (next: Crumb[]) => {
    setTrail(next);
    setSelected(new Set());
    setNotice(null);
  };

  const toggleSelect = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const selectedRows: DeleteItem[] = (rows ?? [])
    .filter((r) => selected.has(r.path))
    .map((r) => ({ path: r.path, sizeBytes: r.sizeBytes, classification: r.classification }));
  const selectedBytes = selectedRows.reduce((s, r) => s + r.sizeBytes, 0);

  const finishDelete = (freed: number, message: string) => {
    setConfirmOpen(false);
    setCleanTarget(null);
    setSelected(new Set());
    setNotice(`Freed ${formatBytes(freed)} — ${message}`);
    refreshAfterCleanup(queryClient);
  };
  const afterDelete = (freed: number) =>
    finishDelete(freed, "items moved to the Recycle Bin.");
  const afterSafeClean = (freed: number) =>
    finishDelete(freed, "caches permanently removed to reclaim space.");

  return (
    <div>
      <PageHeader
        title="Storage Breakdown"
        description="Drill into any folder. Badges tell you what each item is and whether it is safe to delete."
        actions={
          current && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => backend.revealInExplorer(current.path)}
            >
              <FolderOpen className="h-4 w-4" />
              Open in Explorer
            </Button>
          )
        }
      />

      <div className="mb-4 flex flex-wrap items-center gap-1 text-sm">
        <button
          className={`rounded px-2 py-1 transition-colors hover:bg-surface-hover ${
            current === null ? "font-medium text-foreground" : "text-muted"
          }`}
          onClick={() => navigate([])}
        >
          All drives
        </button>
        {trail.map((crumb, i) => (
          <span key={crumb.path} className="flex items-center gap-1">
            <ChevronRight className="h-3.5 w-3.5 text-muted" />
            <button
              className={`rounded px-2 py-1 transition-colors hover:bg-surface-hover ${
                i === trail.length - 1 ? "font-medium text-foreground" : "text-muted"
              }`}
              onClick={() => navigate(trail.slice(0, i + 1))}
            >
              {crumb.name}
            </button>
          </span>
        ))}
      </div>

      {selected.size > 0 && (
        <div className="mb-3 flex items-center gap-3 rounded-md border border-border bg-surface px-4 py-2.5 text-sm">
          <span>
            {selected.size} selected · {formatBytes(selectedBytes)}
          </span>
          <Button variant="danger" size="sm" onClick={() => setConfirmOpen(true)}>
            <Trash2 className="h-3.5 w-3.5" />
            Delete selected
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setSelected(new Set())}>
            Clear selection
          </Button>
        </div>
      )}

      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}

      {current === null && scan && (
        <p className="mb-3 flex items-start gap-2 text-xs text-muted">
          <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            Sizes here were measured during your last scan (
            {formatRelativeTime(scan.startedAt)}). Open any folder to measure it live —
            the list updates with what is found.
            {measuring && (
              <>
                {" "}
                <span className="text-foreground">
                  Working out how much can be cleared from each folder — figures appear
                  as they are measured.
                </span>
              </>
            )}
          </span>
        </p>
      )}

      {shrankSinceScan && current && (
        <div className="mb-3 flex items-start gap-2 rounded-md border border-border bg-surface px-4 py-2.5 text-sm">
          <Info className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
          <span className="text-muted">
            <span className="text-foreground">{current.name}</span> holds{" "}
            <span className="font-medium text-foreground">{formatBytes(liveBytes)}</span> right
            now. The previous screen showed{" "}
            <span className="font-medium text-foreground">{formatBytes(recordedBytes)}</span>{" "}
            because that is what your last scan recorded — its contents have been removed
            since. The list has been corrected.
          </span>
        </div>
      )}

      <Card>
        <CardContent className="divide-y divide-border p-0">
          {isFetching && (
            <div className="flex items-center gap-3 p-5 text-sm text-muted">
              <Loader2 className="h-4 w-4 animate-spin text-primary" />
              Measuring folder contents…
            </div>
          )}
          {!isFetching && (!rows || rows.length === 0) && (
            <p className="p-5 text-sm text-muted">
              {current
                ? "This folder is empty or could not be read (it may need administrator rights)."
                : "Run a scan to see the breakdown."}
            </p>
          )}
          {!isFetching &&
            rows?.map((entry) => {
              const isSystem = entry.classification?.safety === "system";
              return (
                <div
                  key={entry.path}
                  className={`flex items-center gap-3 px-5 py-3 transition-colors ${
                    entry.isDir ? "cursor-pointer hover:bg-surface-hover" : ""
                  }`}
                  onClick={() =>
                    entry.isDir &&
                    navigate([
                      ...trail,
                      { path: entry.path, name: entry.name, sizeBytes: entry.sizeBytes },
                    ])
                  }
                >
                  <input
                    type="checkbox"
                    checked={selected.has(entry.path)}
                    disabled={isSystem}
                    title={isSystem ? "System items cannot be selected for deletion" : "Select"}
                    onClick={(e) => e.stopPropagation()}
                    onChange={() => toggleSelect(entry.path)}
                    className="h-3.5 w-3.5 shrink-0 accent-[hsl(199_89%_48%)] disabled:opacity-30"
                  />
                  {entry.isDir ? (
                    <Folder className="h-4 w-4 shrink-0 text-primary" />
                  ) : (
                    <File className="h-4 w-4 shrink-0 text-muted" />
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium">{entry.name}</span>
                      <SafetyBadge classification={entry.classification} />
                    </div>
                    {entry.classification && (
                      <div className="truncate text-xs text-muted">
                        {entry.classification.explanation}
                      </div>
                    )}
                  </div>
                  <div className="shrink-0 text-right">
                    <div className="text-sm">{formatBytes(entry.sizeBytes)}</div>
                    <div className="text-xs text-muted">
                      {formatPercent(entry.sizeBytes, parentBytes)}
                      {entry.isDir && ` · ${entry.fileCount.toLocaleString()} files`}
                    </div>
                  </div>
                  {/* Only offered where there is genuinely something to
                      reclaim, labelled with how much — the row's own size is
                      what the folder holds, not what cleanup can free. */}
                  {entry.isDir && !isSystem && (entry.recoverableBytes ?? 0) > 0 && (
                    <Button
                      variant="ghost"
                      size="sm"
                      title={`Recoverable space — ${formatBytes(
                        entry.recoverableBytes!
                      )} of caches, temporary files and logs can be cleared from this folder`}
                      onClick={(e) => {
                        e.stopPropagation();
                        setCleanTarget(entry.path);
                      }}
                    >
                      <Sparkles className="h-4 w-4 shrink-0 text-success" />
                      <span className="text-xs text-success">
                        {formatBytes(entry.recoverableBytes!)}
                      </span>
                    </Button>
                  )}
                  {entry.isDir && (
                    <ChevronRight className="h-4 w-4 shrink-0 text-muted" />
                  )}
                </div>
              );
            })}
        </CardContent>
      </Card>

      <ConfirmDeleteModal
        open={confirmOpen}
        title="Delete selected items?"
        items={selectedRows}
        source="Storage breakdown"
        onDone={afterDelete}
        onClose={() => setConfirmOpen(false)}
      />

      {cleanTarget && (
        <SafeCleanModal
          folderPath={cleanTarget}
          onClose={() => setCleanTarget(null)}
          onDeleted={afterSafeClean}
        />
      )}
    </div>
  );
}
