import { useCallback, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";

// Type-only import above is erased at build time, so this adds no runtime
// dependency in the browser (where the updater plugin isn't available).
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type UpdateStatus =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "available"; version: string; notes?: string }
  | { state: "downloading"; percent: number }
  | { state: "ready" }
  | { state: "uptodate" }
  | { state: "error"; message: string };

/** Wraps tauri-plugin-updater: checks for a newer release, downloads + installs
 *  the signed package, then relaunches into the new version. No-ops in the
 *  browser (dev) where the plugin isn't present. */
export function useAppUpdate() {
  const [status, setStatus] = useState<UpdateStatus>({ state: "idle" });
  const [pending, setPending] = useState<Update | null>(null);

  const check = useCallback(async (opts?: { silent?: boolean }) => {
    if (!isTauri) {
      if (!opts?.silent) setStatus({ state: "uptodate" });
      return;
    }
    setStatus({ state: "checking" });
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      if (update) {
        setPending(update);
        setStatus({ state: "available", version: update.version, notes: update.body });
      } else {
        setStatus({ state: "uptodate" });
      }
    } catch (e) {
      setStatus({ state: "error", message: e instanceof Error ? e.message : String(e) });
    }
  }, []);

  const install = useCallback(async () => {
    if (!pending) return;
    try {
      let total = 0;
      let downloaded = 0;
      setStatus({ state: "downloading", percent: 0 });
      await pending.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            setStatus({
              state: "downloading",
              percent: total ? Math.round((downloaded / total) * 100) : 0,
            });
            break;
          case "Finished":
            setStatus({ state: "ready" });
            break;
        }
      });
      // Restart into the freshly installed version.
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (e) {
      setStatus({ state: "error", message: e instanceof Error ? e.message : String(e) });
    }
  }, [pending]);

  const dismiss = useCallback(() => setStatus({ state: "idle" }), []);

  return { status, check, install, dismiss };
}
