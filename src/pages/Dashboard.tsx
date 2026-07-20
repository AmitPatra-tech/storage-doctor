import { useQuery } from "@tanstack/react-query";
import { HardDrive, ScanSearch } from "lucide-react";
import { backend } from "@/lib/backend";
import { formatBytes, formatPercent } from "@/lib/utils";
import { useRunningTasks } from "@/components/RunningTasksProvider";
import { loadSettings } from "@/lib/settings";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";
import { ScanProgressCard } from "@/components/ScanProgressCard";
import { WhatChangedCard } from "@/components/WhatChangedCard";

function StatCard({ title, value, accent }: { title: string; value: string; accent?: string }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className={`text-2xl font-semibold ${accent ?? ""}`}>{value}</div>
      </CardContent>
    </Card>
  );
}

export function Dashboard() {
  const {
    scanning,
    scanProgress: progress,
    scanResult: lastResult,
    scanError: error,
    startScan,
  } = useRunningTasks();
  const { data: scan, isLoading } = useQuery({
    queryKey: ["lastScan"],
    queryFn: backend.getLastScan,
  });

  const primaryDrive = scan?.drives[0];
  const totalUsedBytes = (scan?.drives ?? []).reduce((s, d) => s + d.usedBytes, 0);

  return (
    <div>
      <PageHeader
        title="Dashboard"
        description="Understand what is using your storage and why."
        actions={
          <Button
            size="lg"
            disabled={scanning}
            onClick={() => startScan(loadSettings().scanDrives)}
          >
            <ScanSearch className="h-4 w-4" />
            {scanning ? "Scanning…" : "Scan Drives"}
          </Button>
        }
      />

      {scanning && (
        <div className="mb-6">
          <ScanProgressCard progress={progress} totalBytes={totalUsedBytes} />
        </div>
      )}

      {error && (
        <div className="mb-6 rounded-md border border-danger/40 bg-danger/10 px-4 py-3 text-sm text-danger">
          Scan failed: {error}
        </div>
      )}

      {lastResult && !scanning && (
        <div className="mb-6 rounded-md border border-success/30 bg-success/10 px-4 py-3 text-sm text-success">
          Scan finished in {(lastResult.durationMs / 1000).toFixed(1)}s —{" "}
          {lastResult.filesScanned.toLocaleString()} files (
          {formatBytes(lastResult.bytesScanned)}) analyzed
          {lastResult.errors > 0 && `, ${lastResult.errors} folders skipped`}.
        </div>
      )}

      {isLoading && <p className="text-sm text-muted">Loading last scan…</p>}

      {!isLoading && !scan && !scanning && (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-16 text-center">
            <HardDrive className="h-10 w-10 text-muted" />
            <p className="text-sm font-medium">No scans yet</p>
            <p className="max-w-sm text-sm text-muted">
              Run your first scan to see what is using your storage.
            </p>
          </CardContent>
        </Card>
      )}

      {scan && primaryDrive && (
        <>
          <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
            <StatCard title="Disk Size" value={formatBytes(primaryDrive.totalBytes)} />
            <StatCard title="Used" value={formatBytes(primaryDrive.usedBytes)} />
            <StatCard title="Free" value={formatBytes(primaryDrive.freeBytes)} />
            <StatCard
              title="Recoverable"
              value={formatBytes(scan.recoverableBytes)}
              accent="text-success"
            />
          </div>

          <div className="mt-6">
            <WhatChangedCard />
          </div>

          <div className="mt-6 grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle>Drives</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-4">
                {scan.drives.map((drive) => {
                  const usedPct = (drive.usedBytes / drive.totalBytes) * 100;
                  return (
                    <div key={drive.letter}>
                      <div className="mb-1.5 flex items-center justify-between text-sm">
                        <span className="flex items-center gap-2">
                          <HardDrive className="h-4 w-4 text-muted" />
                          {drive.label} ({drive.letter})
                        </span>
                        <span className="text-muted">
                          {formatBytes(drive.freeBytes)} free of {formatBytes(drive.totalBytes)}
                        </span>
                      </div>
                      <div className="h-2 overflow-hidden rounded-full bg-surface-hover">
                        <div
                          className={`h-full rounded-full ${usedPct > 90 ? "bg-danger" : "bg-primary"}`}
                          style={{ width: `${usedPct}%` }}
                        />
                      </div>
                    </div>
                  );
                })}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Top Storage Consumers</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-3">
                {scan.largestFolders.slice(0, 5).map((folder) => (
                  <div key={folder.path} className="flex items-center justify-between text-sm">
                    <span className="truncate" title={folder.path}>
                      {folder.name}
                    </span>
                    <span className="ml-4 shrink-0 text-muted">
                      {formatBytes(folder.sizeBytes)}
                      <span className="ml-2 text-xs">
                        {formatPercent(folder.sizeBytes, primaryDrive.usedBytes)}
                      </span>
                    </span>
                  </div>
                ))}
              </CardContent>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}
