import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AppWindow,
  File,
  Folder,
  FolderOpen,
  HardDrive,
  Loader2,
  Search,
  Sparkles,
  Trash2,
} from "lucide-react";
import { useNavigate } from "react-router-dom";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/utils";
import type { SearchResultItem } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";
import { ConfirmDeleteModal, type DeleteItem } from "@/components/ConfirmDeleteModal";
import { useRunningTasks } from "@/components/RunningTasksProvider";

const KIND_META = {
  folder: { label: "Folders", icon: Folder },
  file: { label: "Files", icon: File },
  recommendation: { label: "Recommendations", icon: Sparkles },
  app: { label: "Applications", icon: AppWindow },
} as const;

function ResultRow({
  result,
  onOpen,
  onDelete,
  checked,
  onToggle,
}: {
  result: SearchResultItem;
  onOpen: (r: SearchResultItem) => void;
  onDelete?: (r: SearchResultItem) => void;
  checked?: boolean;
  onToggle?: (path: string) => void;
}) {
  const Icon = KIND_META[result.kind].icon;
  const deletable = onDelete && (result.kind === "file" || result.kind === "folder");
  return (
    <div className="flex items-center gap-3 px-5 py-3">
      {onToggle && (
        <input
          type="checkbox"
          checked={checked}
          onChange={() => onToggle(result.path)}
          className="h-3.5 w-3.5 shrink-0 accent-[hsl(199_89%_48%)]"
        />
      )}
      <Icon className="h-4 w-4 shrink-0 text-primary" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{result.name}</div>
        {result.path && <div className="truncate text-xs text-muted">{result.path}</div>}
      </div>
      <span className="shrink-0 text-sm text-muted">
        {result.sizeBytes > 0 ? formatBytes(result.sizeBytes) : ""}
      </span>
      <Button
        variant="ghost"
        size="icon"
        title={
          result.kind === "recommendation" || result.kind === "app"
            ? "Go to page"
            : "Open location"
        }
        onClick={() => onOpen(result)}
      >
        <FolderOpen className="h-4 w-4" />
      </Button>
      {deletable && (
        <Button
          variant="ghost"
          size="icon"
          title="Delete to Recycle Bin"
          onClick={() => onDelete!(result)}
        >
          <Trash2 className="h-4 w-4 text-danger" />
        </Button>
      )}
    </div>
  );
}

export function SearchPage() {
  const navigate = useNavigate();
  const [input, setInput] = useState("");
  const [query, setQuery] = useState("");

  // Deep filesystem search runs in the app-root provider so navigating away
  // does not stop it; results persist and reappear when you return.
  const {
    deepQuery,
    deepSearching: deepBusy,
    deepFound,
    deepResults,
    setDeepResults,
    runDeepSearch,
  } = useRunningTasks();
  const [deepSelected, setDeepSelected] = useState<Set<string>>(new Set());
  const [bulkConfirm, setBulkConfirm] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<SearchResultItem | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    const t = setTimeout(() => setQuery(input.trim()), 300);
    return () => clearTimeout(t);
  }, [input]);

  // Show deep results only for the current query; results for a previous query
  // remain in the provider but the search button reappears for the new one.
  const deepForQuery = deepResults !== null && deepQuery === query;

  useEffect(() => {
    setDeepSelected(new Set());
  }, [query]);

  const { data: results, isFetching } = useQuery({
    queryKey: ["search", query],
    queryFn: () => backend.search(query),
    enabled: query.length >= 2,
  });

  const grouped = (["folder", "file", "app", "recommendation"] as const)
    .map((kind) => ({ kind, items: (results ?? []).filter((r) => r.kind === kind) }))
    .filter((g) => g.items.length > 0);

  const open = (result: SearchResultItem) => {
    if (result.kind === "recommendation") navigate("/recommendations");
    else if (result.kind === "app") navigate("/applications");
    else if (result.path) backend.revealInExplorer(result.path);
  };

  const afterDelete = (freed: number) => {
    const removed = deleteTarget?.path;
    setDeleteTarget(null);
    setNotice(`Freed ${formatBytes(freed)} — item moved to the Recycle Bin.`);
    if (removed) {
      setDeepResults((prev) => prev?.filter((r) => r.path !== removed) ?? null);
    }
  };

  const startDeepSearch = () => {
    setDeepSelected(new Set());
    runDeepSearch(query);
  };

  const toggleDeep = (path: string) => {
    const next = new Set(deepSelected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setDeepSelected(next);
  };

  const selectedDeepItems = (deepResults ?? []).filter((r) => deepSelected.has(r.path));

  const afterBulkDelete = (freed: number) => {
    const removed = new Set(selectedDeepItems.map((r) => r.path));
    setBulkConfirm(false);
    setDeepSelected(new Set());
    setNotice(`Freed ${formatBytes(freed)} — items are in the Recycle Bin.`);
    setDeepResults((prev) => prev?.filter((r) => !removed.has(r.path)) ?? null);
  };

  return (
    <div>
      <PageHeader
        title="Search"
        description="Search scanned folders, large files, recommendations and installed applications."
      />

      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}

      <div className="relative mb-5">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
        <input
          autoFocus
          type="text"
          placeholder="Search for folders, files, extensions, apps…"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          className="h-11 w-full rounded-md border border-border bg-surface pl-10 pr-4 text-sm placeholder:text-muted focus:outline-none focus:ring-2 focus:ring-primary"
        />
      </div>

      <div className="flex flex-col gap-4">
        {grouped.map(({ kind, items }) => (
          <div key={kind}>
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
              {KIND_META[kind].label} ({items.length})
            </h2>
            <Card>
              <CardContent className="divide-y divide-border p-0">
                {items.map((result) => (
                  <ResultRow
                    key={`${result.kind}-${result.path}-${result.name}`}
                    result={result}
                    onOpen={open}
                    onDelete={setDeleteTarget}
                  />
                ))}
              </CardContent>
            </Card>
          </div>
        ))}

        {/* Deep filesystem search — finds small files the scan never indexed. */}
        {query.length >= 2 && (
          <div>
            <div className="mb-2 flex items-center justify-between gap-3">
              <h2 className="text-xs font-semibold uppercase tracking-wide text-muted">
                Files on this PC{deepForQuery ? ` (${deepResults!.length})` : ""}
              </h2>
              {!deepForQuery && !deepBusy && (
                <Button variant="secondary" size="sm" onClick={startDeepSearch}>
                  <HardDrive className="h-3.5 w-3.5" />
                  Search all files for "{query}"
                </Button>
              )}
              {deepForQuery && deepResults!.length > 0 && (
                <div className="flex items-center gap-2">
                  <label className="flex cursor-pointer items-center gap-1.5 text-xs text-muted">
                    <input
                      type="checkbox"
                      checked={deepSelected.size === deepResults!.length}
                      onChange={(e) =>
                        setDeepSelected(
                          e.target.checked
                            ? new Set(deepResults!.map((r) => r.path))
                            : new Set()
                        )
                      }
                      className="h-3.5 w-3.5 accent-[hsl(199_89%_48%)]"
                    />
                    Select all
                  </label>
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={deepSelected.size === 0}
                    onClick={() => setBulkConfirm(true)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    Delete selected ({deepSelected.size})
                  </Button>
                </div>
              )}
            </div>
            {deepBusy && (
              <Card>
                <CardContent className="flex flex-col gap-3 p-5">
                  <div className="flex items-center gap-3 text-sm text-muted">
                    <Loader2 className="h-4 w-4 animate-spin text-primary" />
                    Scanning every drive for matching files — {deepFound.toLocaleString()} found
                    so far.
                  </div>
                  <div className="h-2 overflow-hidden rounded-full bg-surface-hover">
                    <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
                  </div>
                </CardContent>
              </Card>
            )}
            {deepForQuery && !deepBusy && (
              <Card>
                <CardContent className="divide-y divide-border p-0">
                  {deepResults!.length === 0 && (
                    <p className="p-5 text-sm text-muted">
                      No files or folders named like "{query}" were found on your drives.
                    </p>
                  )}
                  {deepResults!.map((result) => (
                    <ResultRow
                      key={result.path}
                      result={result}
                      onOpen={open}
                      onDelete={setDeleteTarget}
                      checked={deepSelected.has(result.path)}
                      onToggle={toggleDeep}
                    />
                  ))}
                </CardContent>
              </Card>
            )}
          </div>
        )}

        {query.length >= 2 &&
          !isFetching &&
          (results?.length ?? 0) === 0 &&
          !deepForQuery &&
          !deepBusy && (
            <p className="text-sm text-muted">
              No indexed matches for "{query}". Use "Search all files" above to look on disk.
            </p>
          )}
      </div>

      <ConfirmDeleteModal
        open={deleteTarget !== null}
        title="Delete this item?"
        items={
          deleteTarget
            ? ([{ path: deleteTarget.path, sizeBytes: deleteTarget.sizeBytes }] as DeleteItem[])
            : []
        }
        source="Search"
        onDone={afterDelete}
        onClose={() => setDeleteTarget(null)}
      />

      <ConfirmDeleteModal
        open={bulkConfirm}
        title="Delete selected files?"
        items={
          selectedDeepItems.map((r) => ({
            path: r.path,
            sizeBytes: r.sizeBytes,
          })) as DeleteItem[]
        }
        source="Search"
        onDone={afterBulkDelete}
        onClose={() => setBulkConfirm(false)}
      />
    </div>
  );
}
