import { Loader2 } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { formatBytes } from "@/lib/utils";
import type { ScanProgress } from "@/lib/types";

export function ScanProgressCard({
  progress,
  totalBytes,
}: {
  progress: ScanProgress | null;
  totalBytes: number;
}) {
  // Progress is estimated from bytes scanned vs. total used space on the
  // drives being scanned. Capped at 99% until the completion event arrives.
  const percent =
    totalBytes > 0 && progress
      ? Math.min(99, Math.round((progress.bytesScanned / totalBytes) * 100))
      : null;

  return (
    <Card className="border-primary/40">
      <CardContent className="flex flex-col gap-3 p-5">
        <div className="flex items-center gap-4">
          <Loader2 className="h-6 w-6 shrink-0 animate-spin text-primary" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium">
              Scanning drives{percent !== null ? ` — ${percent}%` : "…"}
            </div>
            <div className="mt-0.5 truncate text-xs text-muted">
              {progress?.currentPath ?? "Preparing scan"}
            </div>
          </div>
          <div className="shrink-0 text-right text-sm">
            <div>{(progress?.filesScanned ?? 0).toLocaleString()} files</div>
            <div className="text-xs text-muted">
              {formatBytes(progress?.bytesScanned ?? 0)} analyzed
            </div>
          </div>
        </div>
        <div className="h-2 overflow-hidden rounded-full bg-surface-hover">
          <div
            className={`h-full rounded-full bg-primary transition-all duration-300 ${
              percent === null ? "animate-pulse" : ""
            }`}
            style={{ width: `${percent ?? 15}%` }}
          />
        </div>
      </CardContent>
    </Card>
  );
}
