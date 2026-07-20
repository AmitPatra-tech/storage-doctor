import { Fragment, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, Loader2, Search, Sparkles, Trash2 } from "lucide-react";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/utils";
import type { AppUsage, InstalledApp, Leftover } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ConfirmDeleteModal, type DeleteItem } from "@/components/ConfirmDeleteModal";

function UninstallPanel({ app, onClose }: { app: InstalledApp; onClose: () => void }) {
  const queryClient = useQueryClient();
  const [launched, setLaunched] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [leftovers, setLeftovers] = useState<Leftover[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [freed, setFreed] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const launch = async () => {
    setError(null);
    try {
      await backend.launchUninstaller(app.uninstallString!);
      setLaunched(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const scan = async () => {
    setScanning(true);
    setError(null);
    try {
      const found = await backend.findAppLeftovers(app);
      setLeftovers(found);
      setSelected(new Set(found.map((l) => l.path)));
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  };

  const removeSelected = async () => {
    setDeleting(true);
    setError(null);
    try {
      const label = `${app.name} leftovers`;
      const res = await backend.deletePaths([...selected], label);
      let bytes = res.freedBytes;
      let failed = res.failed;
      // App leftovers often live in ProgramData / Program Files, which need
      // admin rights — retry the failures elevated (one UAC prompt).
      if (failed.length > 0) {
        const elevated = await backend.deletePathsElevated(failed, label);
        bytes += elevated.freedBytes;
        failed = elevated.failed;
      }
      const removed = new Set([...selected].filter((p) => !failed.includes(p)));
      setFreed(bytes);
      setLeftovers((prev) => prev?.filter((l) => !removed.has(l.path)) ?? null);
      setSelected(new Set(failed));
      if (failed.length > 0) {
        setError(`${failed.length} item(s) could not be removed even with admin rights.`);
      }
      queryClient.invalidateQueries({ queryKey: ["installedApps"] });
    } catch (e) {
      setError(String(e));
    } finally {
      setDeleting(false);
    }
  };

  const toggle = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const selectedBytes =
    leftovers?.filter((l) => selected.has(l.path)).reduce((s, l) => s + l.sizeBytes, 0) ?? 0;

  return (
    <div className="border-t border-border bg-background/40 px-6 py-4">
      <div className="flex flex-col gap-3 text-sm">
        <div className="flex items-center gap-3">
          <span className="flex h-5 w-5 items-center justify-center rounded-full bg-surface-hover text-xs">
            1
          </span>
          <span className="flex-1">
            Run the app's own uninstaller{app.uninstallString ? "" : " (not available for this app)"}.
          </span>
          <Button size="sm" disabled={!app.uninstallString || launched} onClick={launch}>
            <ExternalLink className="h-3.5 w-3.5" />
            {launched ? "Launched" : "Launch Uninstaller"}
          </Button>
        </div>
        {launched && (
          <p className="ml-8 text-xs text-muted">
            Complete the uninstaller window that just opened (it may ask for administrator
            permission), then continue with step 2.
          </p>
        )}

        <div className="flex items-center gap-3">
          <span className="flex h-5 w-5 items-center justify-center rounded-full bg-surface-hover text-xs">
            2
          </span>
          <span className="flex-1">
            Scan for leftover files the uninstaller did not remove.
          </span>
          <Button size="sm" variant="secondary" disabled={scanning} onClick={scan}>
            {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Search className="h-3.5 w-3.5" />}
            {scanning ? "Scanning…" : "Scan for Leftovers"}
          </Button>
        </div>

        {leftovers !== null && (
          <div className="ml-8 flex flex-col gap-2">
            {leftovers.length === 0 && (
              <p className="text-xs text-success">
                No leftovers found — this app cleaned up after itself.
              </p>
            )}
            {leftovers.map((leftover) => (
              <label
                key={leftover.path}
                className="flex cursor-pointer items-center gap-2.5 text-xs"
              >
                <input
                  type="checkbox"
                  checked={selected.has(leftover.path)}
                  onChange={() => toggle(leftover.path)}
                  className="h-3.5 w-3.5 accent-[hsl(199_89%_48%)]"
                />
                <span className="min-w-0 flex-1 truncate text-muted">{leftover.path}</span>
                <span className="shrink-0">{formatBytes(leftover.sizeBytes)}</span>
              </label>
            ))}
            {leftovers.length > 0 && (
              <div className="mt-1 flex items-center gap-3">
                <Button
                  size="sm"
                  variant="danger"
                  disabled={selected.size === 0 || deleting}
                  onClick={removeSelected}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {deleting
                    ? "Moving to Recycle Bin…"
                    : `Move ${selected.size} item(s) to Recycle Bin (${formatBytes(selectedBytes)})`}
                </Button>
                <span className="text-xs text-muted">
                  Review each path before deleting — recoverable from the Recycle Bin.
                </span>
              </div>
            )}
            {freed !== null && (
              <p className="text-xs text-success">
                Freed {formatBytes(freed)} — files are in the Recycle Bin.
              </p>
            )}
          </div>
        )}

        {error && <p className="ml-8 text-xs text-danger">{error}</p>}

        <div>
          <button className="text-xs text-muted hover:text-foreground" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

// Maps recognized deep-analysis app names to substrings of the normalized
// registry name. Longest match wins so "Visual Studio" doesn't steal "VS Code".
const RECOGNIZED_ALIASES: Record<string, string[]> = {
  "Google Chrome": ["googlechrome"],
  "Microsoft Edge": ["microsoftedge"],
  "Mozilla Firefox": ["mozillafirefox", "firefox"],
  Discord: ["discord"],
  Spotify: ["spotify"],
  Steam: ["steam"],
  "Epic Games Launcher": ["epicgameslauncher", "epicgames"],
  "VS Code": ["visualstudiocode", "vscode"],
  "Visual Studio": ["visualstudio"],
  Docker: ["docker"],
  "Android Studio": ["androidstudio"],
  "IntelliJ IDEA": ["intellijidea", "intellij"],
  "Adobe Creative Cloud": ["adobecreativecloud", "creativecloud"],
};

const normalize = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

interface MergedApp extends InstalledApp {
  storageBytes: number;
  recoverableBytes: number | null;
  components: AppUsage["components"] | null;
}

export function InstalledApps() {
  const queryClient = useQueryClient();
  const [filter, setFilter] = useState("");
  const [openApp, setOpenApp] = useState<string | null>(null);
  const [cleanTarget, setCleanTarget] = useState<{
    name: string;
    items: DeleteItem[];
    total: number;
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const { data: apps, isLoading } = useQuery({
    queryKey: ["installedApps"],
    queryFn: backend.getInstalledApps,
    staleTime: 60_000,
  });
  const { data: usage } = useQuery({
    queryKey: ["appUsage"],
    queryFn: backend.getAppUsage,
    staleTime: 5 * 60_000,
  });

  const merged: MergedApp[] = useMemo(() => {
    const byName = new Map((usage ?? []).map((u) => [u.name, u]));
    return (apps ?? []).map((app) => {
      const norm = normalize(app.name);
      // Find the recognized app whose alias is the longest substring match.
      let best: { name: string; len: number } | null = null;
      for (const [recName, aliases] of Object.entries(RECOGNIZED_ALIASES)) {
        if (!byName.has(recName)) continue;
        for (const alias of aliases) {
          if (norm.includes(alias) && (!best || alias.length > best.len)) {
            best = { name: recName, len: alias.length };
          }
        }
      }
      const analysis = best ? byName.get(best.name) : undefined;
      return {
        ...app,
        storageBytes: analysis?.totalBytes ?? app.estimatedBytes,
        recoverableBytes: analysis ? analysis.recoverableBytes : null,
        components: analysis ? analysis.components : null,
      };
    });
  }, [apps, usage]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const list = q
      ? merged.filter(
          (a) =>
            a.name.toLowerCase().includes(q) || a.publisher.toLowerCase().includes(q)
        )
      : merged;
    return [...list].sort((a, b) => b.storageBytes - a.storageBytes);
  }, [merged, filter]);

  return (
    <div>
      <div className="mb-3 flex items-center justify-between gap-4">
        <h2 className="text-sm font-semibold">
          Installed Applications{apps ? ` (${apps.length})` : ""}
        </h2>
        <input
          type="text"
          placeholder="Search applications…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="h-9 w-64 rounded-md border border-border bg-surface px-3 text-sm placeholder:text-muted focus:outline-none focus:ring-2 focus:ring-primary"
        />
      </div>
      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}
      <Card>
        <CardContent className="p-0">
          {isLoading && (
            <p className="p-5 text-sm text-muted">Reading installed applications…</p>
          )}
          <table className="w-full text-sm">
            {!isLoading && (
              <thead>
                <tr className="border-b border-border text-left text-xs text-muted">
                  <th className="px-5 py-3 font-medium">Application</th>
                  <th className="px-5 py-3 text-right font-medium">Storage</th>
                  <th className="px-5 py-3 text-right font-medium">Recoverable</th>
                  <th className="px-5 py-3 text-right font-medium">Action</th>
                </tr>
              </thead>
            )}
            <tbody className="divide-y divide-border">
              {filtered.map((app) => {
                const isOpen = openApp === app.name;
                return (
                  <Fragment key={app.name}>
                    <tr
                      className="cursor-pointer transition-colors hover:bg-surface-hover"
                      onClick={() => setOpenApp(isOpen ? null : app.name)}
                    >
                      <td className="px-5 py-3">
                        <div className="font-medium">{app.name}</div>
                        <div className="text-xs text-muted">
                          {[app.publisher, app.version].filter(Boolean).join(" · ")}
                        </div>
                      </td>
                      <td className="px-5 py-3 text-right">
                        {app.storageBytes > 0 ? formatBytes(app.storageBytes) : "—"}
                      </td>
                      <td className="px-5 py-3 text-right text-success">
                        {app.recoverableBytes && app.recoverableBytes > 0
                          ? formatBytes(app.recoverableBytes)
                          : "—"}
                      </td>
                      <td className="px-5 py-3 text-right">
                        {app.uninstallString ? (
                          <Button
                            size="sm"
                            variant={isOpen ? "secondary" : "ghost"}
                            onClick={(e) => {
                              e.stopPropagation();
                              setOpenApp(isOpen ? null : app.name);
                            }}
                          >
                            <Trash2 className="h-3.5 w-3.5 text-danger" />
                            Uninstall
                          </Button>
                        ) : (
                          <span className="text-xs text-muted">—</span>
                        )}
                      </td>
                    </tr>
                    {isOpen && (
                      <tr>
                        <td colSpan={4} className="p-0">
                          {app.components && app.components.length > 0 && (
                            <div className="border-t border-border bg-background/40 px-6 py-3">
                              <div className="mb-1.5 flex items-center justify-between">
                                <span className="text-xs font-medium text-muted">
                                  Storage breakdown
                                </span>
                                {app.recoverableBytes && app.recoverableBytes > 0 && (
                                  <Button
                                    size="sm"
                                    variant="secondary"
                                    onClick={() => {
                                      const recoverables = (app.components ?? []).filter(
                                        (c) => c.recoverable && c.bytes > 0
                                      );
                                      setCleanTarget({
                                        name: app.name,
                                        total: recoverables.reduce((s, c) => s + c.bytes, 0),
                                        items: recoverables.map((c) => ({
                                          path: c.path,
                                          sizeBytes: c.bytes,
                                          classification: {
                                            category: `${app.name} — ${c.label}`,
                                            safety: "safe",
                                            explanation:
                                              "Rebuildable cache — the app recreates it automatically.",
                                          },
                                        })),
                                      });
                                    }}
                                  >
                                    <Sparkles className="h-3.5 w-3.5 text-success" />
                                    Clean {formatBytes(app.recoverableBytes)}
                                  </Button>
                                )}
                              </div>
                              <div className="flex flex-col gap-1">
                                {app.components.map((c) => (
                                  <div
                                    key={c.path}
                                    className="flex items-center justify-between text-xs"
                                  >
                                    <span className="text-muted" title={c.path}>
                                      {c.label}
                                      {c.recoverable && (
                                        <span className="ml-2 text-success">recoverable</span>
                                      )}
                                    </span>
                                    <span>{formatBytes(c.bytes)}</span>
                                  </div>
                                ))}
                              </div>
                            </div>
                          )}
                          {app.uninstallString && (
                            <UninstallPanel app={app} onClose={() => setOpenApp(null)} />
                          )}
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
              {!isLoading && filtered.length === 0 && (
                <tr>
                  <td colSpan={4} className="p-5 text-sm text-muted">
                    No applications match "{filter}".
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </CardContent>
      </Card>

      <ConfirmDeleteModal
        open={cleanTarget !== null}
        title={cleanTarget ? `Clean ${cleanTarget.name} caches?` : ""}
        items={cleanTarget?.items ?? []}
        knownTotalBytes={cleanTarget?.total}
        source={cleanTarget ? `${cleanTarget.name} caches` : undefined}
        permanent
        onDone={(freed) => {
          setNotice(`Freed ${formatBytes(freed)} — caches permanently removed to reclaim space.`);
          setCleanTarget(null);
          queryClient.invalidateQueries({ queryKey: ["appUsage"] });
        }}
        onClose={() => setCleanTarget(null)}
      />
    </div>
  );
}
