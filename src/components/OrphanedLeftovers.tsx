import { useState } from "react";
import { AlertTriangle, FolderX, Loader2, Search, Trash2 } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { backend } from "@/lib/backend";
import { refreshAfterCleanup } from "@/lib/refresh";
import { formatBytes, removalNotice } from "@/lib/utils";
import type { Orphan } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ConfirmDeleteModal } from "@/components/ConfirmDeleteModal";

const RECENT_DAYS = 30;

function lastChanged(unix: number): { text: string; recent: boolean } {
  if (!unix) return { text: "last change unknown", recent: false };
  const days = Math.floor((Date.now() / 1000 - unix) / 86400);
  const text =
    days < 1
      ? "changed today"
      : days < 60
        ? `last changed ${days} day(s) ago`
        : `last changed ${Math.floor(days / 30)} month(s) ago`;
  return { text, recent: days < RECENT_DAYS };
}

/** Folders left behind by apps that are no longer installed. Found on
 *  request (it reads every installed program and Store app first), never
 *  pre-selected, and deleted to the Recycle Bin. */
export function OrphanedLeftovers() {
  const queryClient = useQueryClient();
  const [items, setItems] = useState<Orphan[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const scan = async () => {
    setScanning(true);
    setError(null);
    setNotice(null);
    try {
      setItems(await backend.findOrphanedLeftovers());
      setSelected(new Set());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  };

  const toggle = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const chosen = (items ?? []).filter((i) => selected.has(i.path));
  const chosenBytes = chosen.reduce((s, i) => s + i.sizeBytes, 0);

  const afterDelete = (bytes: number, remaining: string[]) => {
    setConfirmOpen(false);
    const gone = new Set(chosen.map((c) => c.path).filter((p) => !remaining.includes(p)));
    setItems((prev) => prev?.filter((i) => !gone.has(i.path)) ?? null);
    setSelected(new Set(remaining));
    setNotice(removalNotice(bytes, false, remaining.length));
    refreshAfterCleanup(queryClient);
  };

  return (
    <div className="mt-6">
      <div className="mb-3 flex items-end justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold">Leftovers from removed apps</h2>
          <p className="text-xs text-muted">
            Folders in AppData, ProgramData and Program Files that no installed program or Store
            app uses — usually left behind after an uninstall. Check each one before deleting.
          </p>
        </div>
        <Button size="sm" variant="secondary" className="shrink-0" disabled={scanning} onClick={scan}>
          {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Search className="h-3.5 w-3.5" />}
          {scanning ? "Looking…" : items ? "Look again" : "Find leftovers"}
        </Button>
      </div>

      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}
      {error && <p className="mb-3 text-sm text-danger">{error}</p>}

      {items && (
        <Card>
          <CardContent className="p-0">
            {items.length === 0 ? (
              <p className="px-5 py-6 text-sm text-muted">
                No leftovers found — every folder belongs to something still installed.
              </p>
            ) : (
              <>
                <div className="flex items-center gap-3 border-b border-border px-5 py-3 text-sm">
                  <span className="flex-1 text-muted">
                    {items.length} folder(s) · {formatBytes(items.reduce((s, i) => s + i.sizeBytes, 0))}
                    {selected.size > 0 && ` · ${selected.size} selected (${formatBytes(chosenBytes)})`}
                  </span>
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={selected.size === 0}
                    onClick={() => setConfirmOpen(true)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    Delete selected
                  </Button>
                </div>
                <div className="divide-y divide-border">
                  {items.map((item) => {
                    const changed = lastChanged(item.modifiedUnix);
                    return (
                      <label
                        key={item.path}
                        className="flex cursor-pointer items-start gap-3 px-5 py-3 text-sm hover:bg-surface-hover/50"
                      >
                        <input
                          type="checkbox"
                          className="mt-1"
                          checked={selected.has(item.path)}
                          onChange={() => toggle(item.path)}
                        />
                        <FolderX className="mt-0.5 h-4 w-4 shrink-0 text-muted" />
                        <div className="min-w-0 flex-1">
                          <div className="font-medium">{item.name}</div>
                          <div className="truncate text-xs text-muted" title={item.path}>
                            {item.location} · {item.path}
                          </div>
                          <div className="text-xs text-muted">
                            No installed program or Store app uses this folder · {changed.text}
                          </div>
                          {changed.recent && (
                            <div className="mt-0.5 flex items-center gap-1.5 text-xs text-warning">
                              <AlertTriangle className="h-3 w-3" />
                              Changed in the last {RECENT_DAYS} days — something may still be using it.
                            </div>
                          )}
                        </div>
                        <span className="shrink-0 text-xs">{formatBytes(item.sizeBytes)}</span>
                      </label>
                    );
                  })}
                </div>
              </>
            )}
          </CardContent>
        </Card>
      )}

      <ConfirmDeleteModal
        open={confirmOpen}
        title="Delete these leftovers?"
        items={chosen.map((c) => ({ path: c.path, sizeBytes: c.sizeBytes }))}
        source="App leftovers"
        onDone={afterDelete}
        onClose={() => setConfirmOpen(false)}
      />
    </div>
  );
}
