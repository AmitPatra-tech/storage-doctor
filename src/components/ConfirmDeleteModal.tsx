import { useEffect, useState } from "react";
import { Clock, ShieldAlert, Trash2, Zap } from "lucide-react";
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

/** Which retry, if any, has already been attempted — drives which failure
 *  stage is shown, since an administrator retry and a force-delete retry
 *  need different explanations and actions. */
type Attempt = "none" | "elevated" | "force";

/** Confirmation dialog that owns the deletion: moves items to the Recycle
 *  Bin, and if any fail offers first an administrator-elevated retry, then —
 *  for whatever is still locked open by something else even as
 *  administrator — a force-delete retry that closes whatever has it open, or
 *  schedules it for removal on the next restart. Calls `onDone(freed)` when
 *  finished. */
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
  const { busy, failed, scheduledForReboot, freed, error, run, forceDelete, reset } =
    useSmartDelete();
  const [attempt, setAttempt] = useState<Attempt>("none");

  useEffect(() => {
    if (open) {
      reset();
      setAttempt("none");
    }
  }, [open, reset]);

  const total = knownTotalBytes ?? items.reduce((sum, i) => sum + i.sizeBytes, 0);
  const hasFailed = failed.length > 0;

  const deleteAll = async () => {
    const res = await run(items.map((i) => i.path), false, source, permanent);
    if (res.failed.length === 0) onDone(freed + res.freedBytes);
  };

  const retryElevated = async () => {
    const res = await run(failed, true, source, permanent);
    setAttempt("elevated");
    if (res.failed.length === 0) onDone(freed + res.freedBytes);
    // Otherwise stay open: something is still locked open, which elevation
    // alone cannot fix — offer Force Delete instead of silently giving up.
  };

  const retryForce = async () => {
    await forceDelete(failed, source, permanent);
    setAttempt("force");
    // Stay open regardless of outcome — a scheduled-for-reboot result is
    // meaningfully different from "freed now" and deserves to be shown
    // rather than folded silently into a single freed-bytes number.
  };

  const finish = () => onDone(freed);

  return (
    <Modal
      open={open}
      onClose={busy ? () => {} : onClose}
      title={title}
      footer={
        attempt === "force" ? (
          <Button variant="secondary" size="sm" onClick={finish}>
            Done
          </Button>
        ) : hasFailed ? (
          <>
            <Button variant="secondary" size="sm" disabled={busy} onClick={finish}>
              Done
            </Button>
            {attempt === "none" ? (
              <Button variant="danger" size="sm" disabled={busy} onClick={retryElevated}>
                <ShieldAlert className="h-3.5 w-3.5" />
                {busy ? "Requesting admin…" : `Retry ${failed.length} as administrator`}
              </Button>
            ) : (
              <Button variant="danger" size="sm" disabled={busy} onClick={retryForce}>
                <Zap className="h-3.5 w-3.5" />
                {busy ? "Closing programs…" : `Force Delete ${failed.length} item(s)`}
              </Button>
            )}
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
      {attempt === "none" && !hasFailed && (
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

      {attempt === "none" && hasFailed && (
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

      {attempt === "elevated" && hasFailed && (
        <>
          <div className="mb-3 flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2.5 text-sm text-warning">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              {failed.length} item(s) could not be removed even as administrator — something
              else has them open. {freed > 0 && `Freed ${formatBytes(freed)} so far. `}
              Force Delete closes whatever is using them and removes them; if that is still
              not possible, they are scheduled to be removed automatically the next time you
              restart your PC.
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

      {attempt === "force" && (
        <div className="flex flex-col gap-3">
          {freed > 0 && (
            <p className="text-sm text-success">Freed {formatBytes(freed)} so far.</p>
          )}
          {scheduledForReboot.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-primary/30 bg-primary/10 px-3 py-2.5 text-sm">
              <Clock className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
              <div className="min-w-0 flex-1">
                <p>
                  {scheduledForReboot.length} item(s) are still open in something that could not
                  be closed — Windows will remove them automatically the next time you restart
                  your PC.
                </p>
                <div className="mt-2 flex flex-col divide-y divide-border">
                  {scheduledForReboot.map((path) => (
                    <div key={path} className="truncate py-1 text-xs text-muted">
                      {path}
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
          {failed.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-danger/40 bg-danger/10 px-3 py-2.5 text-sm text-danger">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0 flex-1">
                <p>{failed.length} item(s) genuinely could not be removed at all.</p>
                <div className="mt-2 flex flex-col divide-y divide-border">
                  {failed.map((path) => (
                    <div key={path} className="truncate py-1 text-xs">
                      {path}
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
          {scheduledForReboot.length === 0 && failed.length === 0 && (
            <p className="text-sm text-success">Everything was removed.</p>
          )}
        </div>
      )}

      {error && <p className="mt-3 text-xs text-danger">{error}</p>}
    </Modal>
  );
}
