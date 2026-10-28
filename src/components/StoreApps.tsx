import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Check, Loader2, Lock, Package, Trash2, X } from "lucide-react";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/utils";
import type { StoreApp, StoreRemoveResult } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Modal } from "@/components/ui/modal";

/** Built-in and Microsoft Store apps. These are not classic programs — they
 *  have no uninstaller and never appear in the list above — so they are
 *  removed as packages, for every account on the PC. */
export function StoreApps() {
  const queryClient = useQueryClient();
  const { data: apps, isLoading, error } = useQuery({
    queryKey: ["storeApps"],
    queryFn: backend.listStoreApps,
  });
  const [filter, setFilter] = useState("");
  const [target, setTarget] = useState<StoreApp | null>(null);
  const [busy, setBusy] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, StoreRemoveResult>>({});

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return (apps ?? []).filter(
      (a) =>
        !q ||
        a.displayName.toLowerCase().includes(q) ||
        a.publisher.toLowerCase().includes(q) ||
        a.name.toLowerCase().includes(q)
    );
  }, [apps, filter]);

  const remove = async () => {
    if (!target) return;
    setBusy(true);
    setRemoveError(null);
    try {
      const res = await backend.removeStoreApp(target.name);
      setResults((r) => ({ ...r, [target.name]: res }));
      setTarget(null);
      if (res.removed) queryClient.invalidateQueries({ queryKey: ["storeApps"] });
    } catch (e) {
      setRemoveError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mt-6">
      <div className="mb-3 flex items-end justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold">Built-in &amp; Store apps</h2>
          <p className="text-xs text-muted">
            Apps that came with Windows or your PC, or from the Microsoft Store. Removing one
            removes it for every account on this PC; most can be reinstalled from the
            Microsoft Store.
          </p>
        </div>
        <input
          type="text"
          placeholder="Filter apps"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="h-8 w-48 shrink-0 rounded-md border border-border bg-surface px-3 text-sm placeholder:text-muted focus:outline-none focus:ring-2 focus:ring-primary"
        />
      </div>

      <Card>
        <CardContent className="divide-y divide-border p-0">
          {isLoading && (
            <div className="flex items-center gap-2 px-5 py-6 text-sm text-muted">
              <Loader2 className="h-4 w-4 animate-spin" /> Reading built-in and Store apps…
            </div>
          )}
          {error && (
            <p className="px-5 py-4 text-sm text-danger">Could not list the apps: {String(error)}</p>
          )}
          {!isLoading && !error && shown.length === 0 && (
            <p className="px-5 py-6 text-sm text-muted">No apps match.</p>
          )}
          {shown.map((app) => {
            const result = results[app.name];
            return (
              <div key={app.name} className="flex items-start gap-3 px-5 py-3 text-sm">
                <Package className="mt-0.5 h-4 w-4 shrink-0 text-muted" />
                <div className="min-w-0 flex-1">
                  <div className="font-medium">{app.displayName}</div>
                  <div className="truncate text-xs text-muted">
                    {app.publisher} · {app.version} · {app.name}
                  </div>
                  {app.caution && !result && (
                    <div className="mt-1 flex items-start gap-1.5 text-xs text-warning">
                      <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                      {app.caution}
                    </div>
                  )}
                  {result && (
                    <div className="mt-1 text-xs">
                      {result.removed ? (
                        <span className="flex items-center gap-1.5 text-success">
                          <Check className="h-3.5 w-3.5" /> Removed for every account.
                        </span>
                      ) : (
                        <span className="flex items-center gap-1.5 text-danger">
                          <X className="h-3.5 w-3.5" /> Not removed — still installed for{" "}
                          {result.remainingInstalls} account(s).
                        </span>
                      )}
                      {result.notes.map((n) => (
                        <div key={n} className="mt-0.5 text-muted">
                          {n}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
                <span className="shrink-0 text-xs text-muted">
                  {app.sizeBytes > 0 ? formatBytes(app.sizeBytes) : ""}
                </span>
                {app.nonRemovable ? (
                  <span
                    className="flex shrink-0 items-center gap-1 text-xs text-muted"
                    title="Windows marks this app as part of the system and does not allow removing it."
                  >
                    <Lock className="h-3.5 w-3.5" /> Part of Windows
                  </span>
                ) : (
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy || result?.removed}
                    onClick={() => {
                      setRemoveError(null);
                      setTarget(app);
                    }}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    Remove
                  </Button>
                )}
              </div>
            );
          })}
        </CardContent>
      </Card>

      <Modal
        open={target !== null}
        onClose={busy ? () => {} : () => setTarget(null)}
        title={target ? `Remove ${target.displayName}?` : ""}
        footer={
          <>
            <Button variant="secondary" size="sm" disabled={busy} onClick={() => setTarget(null)}>
              Cancel
            </Button>
            <Button variant="danger" size="sm" disabled={busy} onClick={remove}>
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
              {busy ? "Removing…" : "Remove for every account"}
            </Button>
          </>
        }
      >
        {target && (
          <div className="flex flex-col gap-2 text-sm text-muted">
            <p>
              Removes <span className="font-medium text-foreground">{target.displayName}</span> for
              every account on this PC and stops Windows adding it to new accounts. Its app data
              is removed with it.{" "}
              {target.name === "Microsoft.WindowsStore"
                ? "To get the Microsoft Store back later, run “wsreset -i” from the Start menu."
                : "You can reinstall it from the Microsoft Store."}
            </p>
            {target.caution && (
              <p className="flex items-start gap-1.5 text-warning">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                {target.caution}
              </p>
            )}
            <p className="text-xs">Needs administrator permission.</p>
            {removeError && <p className="text-xs text-danger">{removeError}</p>}
          </div>
        )}
      </Modal>
    </div>
  );
}
