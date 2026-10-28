import { useEffect, useState } from "react";
import { CheckCircle2, Clock, ShieldAlert, Trash2, XCircle, Zap } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Button } from "@/components/ui/button";
import { SafetyBadge } from "@/components/ui/badge";
import { formatBytes } from "@/lib/utils";
import { useSmartDelete } from "@/hooks/useSmartDelete";
import type { Classification, ForcePreviewItem, ItemNote } from "@/lib/types";
import { backend } from "@/lib/backend";

export interface DeleteItem {
  path: string;
  sizeBytes: number;
  classification?: Classification | null;
}

/** Which retry, if any, has already been attempted — drives which failure
 *  stage is shown, since an administrator retry and a force-delete retry
 *  need different explanations and actions. */
type Attempt = "none" | "elevated" | "preview" | "force";

/** What has been removed so far, worded for what actually happened to the
 *  space: permanently deleted items free it; Recycle Bin items do not until
 *  the bin is emptied. */
function progressNote(bytes: number, permanent: boolean): string {
  if (bytes <= 0) return "";
  return permanent
    ? `${formatBytes(bytes)} deleted so far.`
    : `${formatBytes(bytes)} moved to the Recycle Bin so far.`;
}

function PathList({ paths }: { paths: string[] }) {
  return (
    <div className="mt-2 flex flex-col divide-y divide-border">
      {paths.map((path) => (
        <div key={path} className="truncate py-1 text-xs text-muted" title={path}>
          {path}
        </div>
      ))}
    </div>
  );
}

function NoteList({ notes }: { notes: ItemNote[] }) {
  return (
    <div className="mt-2 flex flex-col divide-y divide-border">
      {notes.map((n) => (
        <div key={n.path} className="py-1.5">
          <div className="truncate text-xs font-medium text-foreground" title={n.path}>
            {n.path}
          </div>
          <ul className="mt-0.5 list-disc pl-5 text-xs text-muted">
            {n.reasons.map((r) => (
              <li key={r}>{r}</li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

/** Confirmation dialog that owns the deletion: moves items to the Recycle
 *  Bin (or deletes them permanently), and if any fail offers first an
 *  administrator retry, then a force delete that closes the programs holding
 *  them. Every result is reported per item, exactly as it happened. Calls
 *  `onDone(bytes)` with the size of what was removed. */
export function ConfirmDeleteModal({
  open,
  title,
  items,
  knownTotalBytes,
  source,
  permanent = false,
  allowForce = true,
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
  /** Offer Force Delete (closing programs) for items still in use. Off for
   *  cache and Temp cleanups: files in use there belong to running programs
   *  and are simply skipped. */
  allowForce?: boolean;
  /** `removedBytes`: size of what is gone. `remaining`: paths still there. */
  onDone: (removedBytes: number, remaining: string[]) => void;
  onClose: () => void;
}) {
  const { busy, failed, refused, forceResult, freed, error, run, forceDelete, reset } =
    useSmartDelete();
  const [attempt, setAttempt] = useState<Attempt>("none");
  /** The first delete has run — the item list gives way to results. */
  const [started, setStarted] = useState(false);
  /** What holds each still-there item open, for consent before closing. */
  const [preview, setPreview] = useState<ForcePreviewItem[] | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  /** Programs (process names) the user ticked to be closed. */
  const [approved, setApproved] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (open) {
      reset();
      setAttempt("none");
      setStarted(false);
      setPreview(null);
      setApproved(new Set());
      setPreviewError(null);
    }
  }, [open, reset]);

  const total = knownTotalBytes ?? items.reduce((sum, i) => sum + i.sizeBytes, 0);
  const hasFailed = failed.length > 0;
  const verb = permanent ? "deleted" : "moved to the Recycle Bin";

  const deleteAll = async () => {
    const res = await run(items.map((i) => i.path), false, source, permanent);
    setStarted(true);
    // Close straight away only when everything is gone; anything refused or
    // failed stays on screen with its reason.
    if (res.failed.length === 0 && res.refused.length === 0) onDone(freed + res.freedBytes, []);
  };

  const retryElevated = async () => {
    const res = await run(failed, true, source, permanent);
    setAttempt("elevated");
    // Cache cleanups never force anything — but say exactly what is using the
    // items rather than assume it (read-only lookup, nothing is closed).
    if (!allowForce && res.failed.length > 0) {
      try {
        setPreview(await backend.forceDeletePreview(res.failed));
      } catch {
        setPreview(null);
      }
    }
    if (res.failed.length === 0 && res.refused.length === 0 && refused.length === 0) {
      onDone(freed + res.freedBytes, []);
    }
  };

  /** Step 1 of force delete: show who holds the items. Nothing is closed. */
  const showPreview = async () => {
    setPreviewBusy(true);
    setPreviewError(null);
    try {
      setPreview(await backend.forceDeletePreview(failed));
      setApproved(new Set());
      setAttempt("preview");
    } catch (e) {
      setPreviewError(e instanceof Error ? e.message : String(e));
    } finally {
      setPreviewBusy(false);
    }
  };

  // One entry per program across all items, since consent is per program.
  const programs = (() => {
    const map = new Map<string, { exe: string; display: string; closable: boolean; reason: string | null; items: string[] }>();
    for (const item of preview ?? []) {
      for (const h of item.holders) {
        const key = h.closable ? h.exe.toLowerCase() : `${h.exe.toLowerCase()}|${h.reason ?? ""}`;
        const entry = map.get(key) ?? { ...h, items: [] };
        if (!entry.items.includes(item.path)) entry.items.push(item.path);
        map.set(key, entry);
      }
    }
    return [...map.values()];
  })();
  const closable = programs.filter((p) => p.closable);
  const keptOpen = programs.filter((p) => !p.closable);

  const toggleApproved = (exe: string) => {
    const next = new Set(approved);
    if (next.has(exe)) next.delete(exe);
    else next.add(exe);
    setApproved(next);
  };

  /** Step 2: force delete, closing only the ticked programs. */
  const retryForce = async () => {
    const res = await forceDelete(failed, [...approved], source, permanent);
    // Only switch to the result view when the force delete actually ran; if
    // it could not (e.g. the administrator prompt was declined) the error is
    // shown and the items stay offered for another try.
    if (res) setAttempt("force");
  };

  const finish = () => onDone(freed, [...failed, ...refused.map((r) => r.path)]);

  return (
    <Modal
      open={open}
      onClose={busy ? () => {} : onClose}
      title={title}
      footer={
        attempt === "force" || (started && !hasFailed) ? (
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
            ) : attempt === "elevated" ? (
              allowForce && (
                <Button variant="danger" size="sm" disabled={previewBusy} onClick={showPreview}>
                  <Zap className="h-3.5 w-3.5" />
                  {previewBusy ? "Checking what uses them…" : `Force Delete ${failed.length} item(s)…`}
                </Button>
              )
            ) : (
              <Button variant="danger" size="sm" disabled={busy} onClick={retryForce}>
                <Zap className="h-3.5 w-3.5" />
                {busy
                  ? "Working…"
                  : approved.size > 0
                    ? `Close ${approved.size} program(s) and delete`
                    : "Delete without closing anything"}
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
      {!started && (
        <>
          <p className="mb-3 text-sm text-muted">
            {permanent
              ? "These items will be permanently deleted — the space is reclaimed immediately, and this cannot be undone."
              : "Items are moved to the Recycle Bin, so you can restore them if needed. They keep using disk space until the Recycle Bin is emptied."}
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
              {failed.length} item(s) could not be {verb} — Windows refused. That happens when
              your account lacks permission for them, or when a program has them open.{" "}
              {progressNote(freed, permanent)} Retrying as administrator solves the permission
              case.
            </span>
          </div>
          <PathList paths={failed} />
        </>
      )}

      {attempt === "elevated" && hasFailed && (
        <>
          <div className="mb-3 flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2.5 text-sm text-warning">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              {failed.length} item(s) could not be {verb} even as administrator.{" "}
              {progressNote(freed, permanent)}{" "}
              {allowForce
                ? "Force Delete first shows you which programs have them open; only the ones you tick are closed."
                : preview === null
                  ? "They were skipped. Nothing is force-closed for a cache cleanup."
                  : programs.length > 0
                    ? `They are in use by: ${programs.map((p) => p.display).join(", ")}. They were skipped — nothing is force-closed for a cache cleanup. Close those programs (or restart) and clean again.`
                    : "No running program was found using them, so Windows refused for another reason (such as a protected file). They were skipped."}
            </span>
          </div>
          <PathList paths={failed} />
        </>
      )}

      {attempt === "elevated" && previewError && (
        <p className="mt-2 text-xs text-danger">{previewError}</p>
      )}

      {attempt === "preview" && preview && (
        <div className="flex flex-col gap-3 text-sm">
          {closable.length > 0 && (
            <div className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2.5">
              <p className="text-warning">
                These programs have the items open. Tick the ones that may be closed —
                unsaved work in them is lost. Unticked programs are left running.
              </p>
              <div className="mt-2 flex flex-col gap-1.5">
                {closable.map((p) => (
                  <label key={p.exe} className="flex cursor-pointer items-start gap-2">
                    <input
                      type="checkbox"
                      className="mt-1"
                      checked={approved.has(p.exe)}
                      onChange={() => toggleApproved(p.exe)}
                    />
                    <span className="min-w-0">
                      <span className="font-medium text-foreground">{p.display}</span>
                      <span className="block truncate text-xs text-muted" title={p.items.join("\n")}>
                        uses {p.items.length === 1 ? p.items[0] : `${p.items.length} of these items`}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
            </div>
          )}
          {keptOpen.length > 0 && (
            <div className="rounded-md border border-border px-3 py-2.5">
              <p>Also using them — these are never closed:</p>
              <ul className="mt-1 list-disc pl-5 text-xs text-muted">
                {keptOpen.map((p) => (
                  <li key={`${p.exe}|${p.reason}`}>{p.reason ?? p.display}</li>
                ))}
              </ul>
            </div>
          )}
          {programs.length === 0 && (
            <p className="text-muted">
              No running program was found holding these items, so nothing will be closed. They
              will be retried with administrator rights, taking ownership if needed.
            </p>
          )}
          <PathList paths={failed} />
        </div>
      )}

      {started && refused.length > 0 && (
        <div className="mb-3 flex items-start gap-2 rounded-md border border-danger/40 bg-danger/10 px-3 py-2.5 text-sm text-danger">
          <XCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0 flex-1">
            <p>{refused.length} item(s) not deleted — protected:</p>
            <NoteList notes={refused} />
          </div>
        </div>
      )}

      {started && !hasFailed && attempt !== "force" && (
        <p className="text-sm text-muted">
          {freed > 0
            ? permanent
              ? `${formatBytes(freed)} deleted.`
              : `${formatBytes(freed)} moved to the Recycle Bin.`
            : "Nothing was removed."}
        </p>
      )}

      {attempt === "force" && forceResult && (
        <div className="flex flex-col gap-3">
          {forceResult.removed.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-success/40 bg-success/10 px-3 py-2.5 text-sm text-success">
              <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0 flex-1">
                <p>
                  {forceResult.removed.length} item(s){" "}
                  {permanent
                    ? `deleted (${formatBytes(forceResult.freedBytes)} freed).`
                    : `removed (${formatBytes(forceResult.freedBytes)}) — sent to the Recycle Bin, or deleted permanently if too large for it. Space in the Recycle Bin is freed when you empty it.`}
                </p>
                <PathList paths={forceResult.removed} />
              </div>
            </div>
          )}

          {forceResult.closedPrograms.length > 0 && (
            <div className="rounded-md border border-border px-3 py-2.5 text-sm">
              <p>Closed to free the items:</p>
              <ul className="mt-1 list-disc pl-5 text-xs text-muted">
                {forceResult.closedPrograms.map((p) => (
                  <li key={p}>{p}</li>
                ))}
              </ul>
            </div>
          )}

          {forceResult.blocked.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2.5 text-sm text-warning">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0 flex-1">
                <p>
                  {forceResult.blocked.length} item(s) not deleted — still in use by something
                  this app does not close. Nothing was scheduled for a restart: whatever holds
                  them would hold them again.
                </p>
                <NoteList notes={forceResult.blocked} />
              </div>
            </div>
          )}

          {forceResult.scheduledForReboot.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-primary/30 bg-primary/10 px-3 py-2.5 text-sm">
              <Clock className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
              <div className="min-w-0 flex-1">
                <p>
                  {forceResult.scheduledForReboot.length} item(s) not deleted yet. No program
                  has them open, but Windows refused to delete them now. They are scheduled to
                  be deleted <span className="font-medium">permanently</span> (not to the
                  Recycle Bin) during the next restart — use Start ▸ Power ▸ Restart, not Shut
                  down. If a program creates them again when Windows starts, they will be
                  back.
                </p>
                <PathList paths={forceResult.scheduledForReboot} />
              </div>
            </div>
          )}

          {forceResult.failed.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-danger/40 bg-danger/10 px-3 py-2.5 text-sm text-danger">
              <XCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0 flex-1">
                <p>{forceResult.failed.length} item(s) not deleted:</p>
                <NoteList notes={forceResult.failed} />
              </div>
            </div>
          )}

          {forceResult.blocked.length === 0 &&
            forceResult.scheduledForReboot.length === 0 &&
            forceResult.failed.length === 0 && (
              <p className="text-sm text-success">All items were removed.</p>
            )}
        </div>
      )}

      {error && <p className="mt-3 text-xs text-danger">{error}</p>}
    </Modal>
  );
}
