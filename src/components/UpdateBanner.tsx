import { useEffect } from "react";
import { Download, Loader2, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAppUpdate } from "@/hooks/useAppUpdate";

/** Silently checks for a newer version on startup and, if one is found, shows
 *  a dismissible banner offering a one-click download + install + relaunch. */
export function UpdateBanner() {
  const { status, check, install, dismiss } = useAppUpdate();

  useEffect(() => {
    check({ silent: true });
  }, [check]);

  // Nothing to show for idle / checking / up-to-date / error during the
  // background check — stay out of the way unless there's an actual update.
  const show =
    status.state === "available" ||
    status.state === "downloading" ||
    status.state === "ready";
  if (!show) return null;

  return (
    <div className="fixed bottom-4 left-1/2 z-50 w-[min(92vw,520px)] -translate-x-1/2 rounded-lg border border-primary/40 bg-surface shadow-lg">
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/15">
          {status.state === "downloading" ? (
            <Loader2 className="h-4 w-4 animate-spin text-primary" />
          ) : (
            <RefreshCw className="h-4 w-4 text-primary" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          {status.state === "available" && (
            <>
              <div className="text-sm font-medium">
                Update available — version {status.version}
              </div>
              <div className="truncate text-xs text-muted">
                {status.notes?.trim() || "Install the latest Storage Doctor."}
              </div>
            </>
          )}
          {status.state === "downloading" && (
            <>
              <div className="text-sm font-medium">Downloading update… {status.percent}%</div>
              <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-surface-hover">
                <div
                  className="h-full rounded-full bg-primary transition-all"
                  style={{ width: `${status.percent}%` }}
                />
              </div>
            </>
          )}
          {status.state === "ready" && (
            <div className="text-sm font-medium">Update installed — restarting…</div>
          )}
        </div>
        {status.state === "available" && (
          <div className="flex shrink-0 items-center gap-1.5">
            <Button size="sm" onClick={install}>
              <Download className="h-3.5 w-3.5" />
              Update now
            </Button>
            <Button variant="ghost" size="icon" title="Later" onClick={dismiss}>
              <X className="h-4 w-4" />
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
