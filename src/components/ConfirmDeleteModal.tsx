import { useEffect } from "react";
import { ShieldAlert, Trash2 } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Button } from "@/components/ui/button";
import { SafetyBadge } from "@/components/ui/badge";
import { formatBytes } from "@/lib/utils";
import { useSmartDelete } from "@/hooks/useSmartDelete";
import type { Classification } from "@/lib/types";

export interface DeleteItem {
  path: string;
  sizeBytes: number;
  classification?: Classification | null;
}

/** Confirmation dialog that owns the deletion: moves items to the Recycle
 *  Bin, and if any fail (permission denied / locked) offers an
 *  administrator-elevated retry. Calls `onDone(freed)` when finished. */
export function ConfirmDeleteModal({
  open,
  title,
  items,
  knownTotalBytes,
  source,
  permanent = false,
  onDone,
  onClose,
}: {
  open: boolean;
  title: string;
  items: DeleteItem[];
  /** Used for the total when per-item sizes aren't known (e.g. recommendations). */
  knownTotalBytes?: number;
  /** Label recorded in the cleanup journal. */
  source?: string;
  /** Permanently delete (frees space) instead of moving to the Recycle Bin. */
  permanent?: boolean;
  onDone: (freedBytes: number) => void;
  onClose: () => void;
}) {
  const { busy, failed, freed, error, run, reset } = useSmartDelete();

  useEffect(() => {
    if (open) reset();
  }, [open, reset]);

  const total = knownTotalBytes ?? items.reduce((sum, i) => sum + i.sizeBytes, 0);
  const hasFailed = failed.length > 0;

  const deleteAll = async () => {
    const res = await run(items.map((i) => i.path), false, source, permanent);
    if (res.failed.length === 0) onDone(freed + res.freedBytes);
  };

  const retryElevated = async () => {
    const res = await run(failed, true, source, permanent);
    // Always close after the elevated attempt — remaining failures are locked
    // files or directories that Windows recreated instantly.
    onDone(freed + res.freedBytes);
  };

  return (
    <Modal
      open={open}
      onClose={busy ? () => {} : onClose}
      title={title}
      footer={
        hasFailed ? (
          <>
            <Button variant="secondary" size="sm" disabled={busy} onClick={() => onDone(freed)}>
              Done
            </Button>
            <Button variant="danger" size="sm" disabled={busy} onClick={retryElevated}>
              <ShieldAlert className="h-3.5 w-3.5" />
              {busy ? "Requesting admin…" : `Retry ${failed.length} as administrator`}
            </Button>
          </>
        ) : (
          <>
            <Button variant="secondary" size="sm" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button
              variant="danger"
              size="sm"
              disabled={busy || items.length === 0}
              onClick={deleteAll}
            >
              <Trash2 className="h-3.5 w-3.5" />
              {busy
                ? permanent
                  ? "Deleting…"
                  : "Moving to Recycle Bin…"
                : permanent
                  ? `Delete ${items.length} item(s) (${formatBytes(total)})`
                  : `Move ${items.length} item(s) to Recycle Bin (${formatBytes(total)})`}
            </Button>
          </>
        )
      }
    >
      {!hasFailed && (
        <>
          <p className="mb-3 text-sm text-muted">
            {permanent
              ? "These items will be permanently deleted — the space is reclaimed immediately, and this cannot be undone."
              : "Items are moved to the Recycle Bin, so you can restore them if needed."}
          </p>
          {busy && (
            <div className="mb-3">
              <div className="mb-1.5 text-xs text-muted">
                {permanent ? "Deleting…" : "Moving to Recycle Bin…"} This can take a
                while for large folders.
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-surface-hover">
                <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
              </div>
            </div>
          )}
          <div className="flex flex-col divide-y divide-border">
            {items.map((item) => (
              <div key={item.path} className="flex items-center gap-3 py-2 text-sm">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-xs text-muted">{item.path}</span>
                    <SafetyBadge classification={item.classification} />
                  </div>
                </div>
                {item.sizeBytes > 0 && (
                  <span className="shrink-0 text-xs">{formatBytes(item.sizeBytes)}</span>
                )}
              </div>
            ))}
          </div>
        </>
      )}

      {hasFailed && (
        <>
          <div className="mb-3 flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2.5 text-sm text-warning">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              {failed.length} item(s) could not be deleted — they likely need administrator
              rights or are in use. {freed > 0 && `Freed ${formatBytes(freed)} so far. `}
              Retry with administrator to {permanent ? "delete them" : "move them to the Recycle Bin"}.
            </span>
          </div>
          <div className="flex flex-col divide-y divide-border">
            {failed.map((path) => (
              <div key={path} className="truncate py-1.5 text-xs text-muted">
                {path}
              </div>
            ))}
          </div>
        </>
      )}

      {error && <p className="mt-3 text-xs text-danger">{error}</p>}
    </Modal>
  );
}
