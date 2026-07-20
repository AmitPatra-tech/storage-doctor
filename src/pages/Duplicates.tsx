import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CopyCheck, FolderOpen, FolderSearch, Loader2, Trash2 } from "lucide-react";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/PageHeader";
import { ConfirmDeleteModal } from "@/components/ConfirmDeleteModal";
import { UpgradePanel } from "@/components/ProGate";
import { useLicense } from "@/components/LicenseProvider";
import { useRunningTasks } from "@/components/RunningTasksProvider";

function formatDate(iso: string): string {
  if (!iso) return "unknown date";
  return new Date(iso).toLocaleDateString();
}

export function Duplicates() {
  const queryClient = useQueryClient();
  const { isPro } = useLicense();
  // Scan state lives in the app-root provider, so navigating away and back
  // never cancels or loses an in-progress duplicate scan.
  const {
    dupeScanning: scanning,
    dupeProgress: progress,
    dupeGroups: groups,
    setDupeGroups: setGroups,
    runDupeScan,
  } = useRunningTasks();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  // Preselect every copy except the oldest ("original") whenever results change.
  useEffect(() => {
    if (!groups) return;
    const preselect = new Set<string>();
    for (const group of groups) {
      for (const file of group.files.slice(1)) preselect.add(file.path);
    }
    setSelected(preselect);
  }, [groups]);

  const scan = async (roots: string[] = []) => {
    setNotice(null);
    setSelected(new Set());
    await runDupeScan(roots);
  };

  const scanChosenFolders = async () => {
    const folders = await backend.pickFolders();
    if (folders.length > 0) scan(folders);
  };

  const toggle = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const { selectedItems, wastedBytes } = useMemo(() => {
    const items: { path: string; sizeBytes: number }[] = [];
    let wasted = 0;
    for (const group of groups ?? []) {
      wasted += group.sizeBytes * (group.files.length - 1);
      for (const file of group.files) {
        if (selected.has(file.path)) {
          items.push({ path: file.path, sizeBytes: group.sizeBytes });
        }
      }
    }
    return { selectedItems: items, wastedBytes: wasted };
  }, [groups, selected]);

  const afterDelete = (freed: number) => {
    setConfirmOpen(false);
    setGroups(
      (prev) =>
        prev
          ?.map((g) => ({
            ...g,
            files: g.files.filter((f) => !selected.has(f.path)),
          }))
          .filter((g) => g.files.length > 1) ?? null
    );
    setSelected(new Set());
    setNotice(`Freed ${formatBytes(freed)} — files are in the Recycle Bin.`);
    queryClient.invalidateQueries({ queryKey: ["lastScan"] });
  };

  const selectedBytes = selectedItems.reduce((s, i) => s + i.sizeBytes, 0);

  if (!isPro) {
    return (
      <div>
        <PageHeader
          title="Duplicate Files"
          description="Identical files found by content (SHA-256), not by name."
        />
        <UpgradePanel feature="Duplicate detection" />
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Duplicate Files"
        description="Identical files found by content (SHA-256), not by name."
        actions={
          <div className="flex gap-2">
            <Button variant="secondary" size="lg" disabled={scanning} onClick={scanChosenFolders}>
              <FolderSearch className="h-4 w-4" />
              Choose Folders…
            </Button>
            <Button size="lg" disabled={scanning} onClick={() => scan()}>
              <CopyCheck className="h-4 w-4" />
              {scanning ? "Scanning…" : "Scan Common Folders"}
            </Button>
          </div>
        }
      />

      {scanning && (
        <Card className="mb-4 border-primary/40">
          <CardContent className="flex items-center gap-4 p-5">
            <Loader2 className="h-6 w-6 shrink-0 animate-spin text-primary" />
            <div className="flex-1 text-sm">
              {progress?.phase === "hashing"
                ? `Comparing file contents — ${progress.filesProcessed.toLocaleString()} of ${progress.totalFiles.toLocaleString()} candidates, ${formatBytes(progress.bytesHashed)} hashed`
                : "Collecting files…"}
            </div>
          </CardContent>
        </Card>
      )}

      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}

      {groups && groups.length === 0 && (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-16 text-center">
            <CopyCheck className="h-10 w-10 text-success" />
            <p className="text-sm font-medium">No duplicates found</p>
            <p className="max-w-sm text-sm text-muted">
              Your user folders contain no identical files larger than 1 MB.
            </p>
          </CardContent>
        </Card>
      )}

      {groups && groups.length > 0 && (
        <>
          <div className="mb-3 flex items-center gap-3 rounded-md border border-border bg-surface px-4 py-2.5 text-sm">
            <span>
              {groups.length} duplicate group(s) · {formatBytes(wastedBytes)} wasted ·{" "}
              {selectedItems.length} copies selected ({formatBytes(selectedBytes)})
            </span>
            <Button
              variant="danger"
              size="sm"
              disabled={selectedItems.length === 0}
              onClick={() => setConfirmOpen(true)}
            >
              <Trash2 className="h-3.5 w-3.5" />
              Delete selected copies
            </Button>
          </div>

          <div className="flex flex-col gap-3">
            {groups.map((group) => (
              <Card key={group.hash}>
                <CardContent className="p-4">
                  <div className="mb-2 flex items-center gap-3 text-sm">
                    <Badge>{formatBytes(group.sizeBytes)} each</Badge>
                    <span className="text-muted">
                      {group.files.length} identical copies ·{" "}
                      {formatBytes(group.sizeBytes * (group.files.length - 1))} reclaimable
                    </span>
                  </div>
                  <div className="flex flex-col gap-1.5">
                    {group.files.map((file, index) => (
                      <div
                        key={file.path}
                        className="flex items-center gap-2.5 text-sm"
                      >
                        <input
                          type="checkbox"
                          checked={selected.has(file.path)}
                          onChange={() => toggle(file.path)}
                          className="h-3.5 w-3.5 shrink-0 accent-[hsl(199_89%_48%)]"
                        />
                        <span className="min-w-0 flex-1 truncate text-muted">{file.path}</span>
                        <span className="shrink-0 text-xs text-muted">
                          {formatDate(file.modifiedAt)}
                        </span>
                        {index === 0 && (
                          <span className="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-[11px] text-primary">
                            original
                          </span>
                        )}
                        <Button
                          variant="ghost"
                          size="icon"
                          title="View file in Explorer"
                          onClick={() => backend.revealInExplorer(file.path)}
                        >
                          <FolderOpen className="h-4 w-4" />
                        </Button>
                      </div>
                    ))}
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        </>
      )}

      {!groups && !scanning && (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-16 text-center">
            <CopyCheck className="h-10 w-10 text-muted" />
            <p className="text-sm font-medium">Find identical files wasting space</p>
            <p className="max-w-sm text-sm text-muted">
              Files are compared by content hash, so renamed copies are found too. The
              oldest copy is treated as the original and kept by default.
            </p>
          </CardContent>
        </Card>
      )}

      <ConfirmDeleteModal
        open={confirmOpen}
        title="Delete duplicate copies?"
        items={selectedItems}
        source="Duplicate cleanup"
        onDone={afterDelete}
        onClose={() => setConfirmOpen(false)}
      />
    </div>
  );
}
