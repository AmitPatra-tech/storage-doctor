import { useCallback, useState } from "react";
import { backend } from "@/lib/backend";

/** Deletion with graceful failure handling, an admin-elevated retry, and a
 *  force-delete last resort for items still locked open even as
 *  administrator. Used by every delete flow so these cases are handled
 *  consistently. */
export function useSmartDelete() {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string[]>([]);
  /** Not free yet — set only by `forceDelete`, for items Windows will
   *  remove automatically the next time the PC restarts. */
  const [scheduledForReboot, setScheduledForReboot] = useState<string[]>([]);
  const [freed, setFreed] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const reset = useCallback(() => {
    setBusy(false);
    setFailed([]);
    setScheduledForReboot([]);
    setFreed(0);
    setError(null);
  }, []);

  const run = useCallback(
    async (paths: string[], elevated = false, source?: string, permanent = false) => {
      setBusy(true);
      setError(null);
      try {
        const res = elevated
          ? await backend.deletePathsElevated(paths, source, permanent)
          : await backend.deletePaths(paths, source, permanent);
        setFreed((f) => f + res.freedBytes);
        setFailed(res.failed);
        return res;
      } catch (e) {
        setError(String(e));
        return { freedBytes: 0, failed: paths };
      } finally {
        setBusy(false);
      }
    },
    []
  );

  /** Offered only once `run(paths, true, ...)` has already left failures —
   *  that is the signal an item is genuinely open somewhere, not merely
   *  permission-denied, which elevation alone cannot fix. */
  const forceDelete = useCallback(
    async (paths: string[], source?: string, permanent = false) => {
      setBusy(true);
      setError(null);
      try {
        const res = await backend.forceDeletePaths(paths, source, permanent);
        setFreed((f) => f + res.freedBytes);
        setFailed(res.failed);
        setScheduledForReboot(res.scheduledForReboot);
        return res;
      } catch (e) {
        setError(String(e));
        return { freedBytes: 0, removed: [], scheduledForReboot: [], failed: paths };
      } finally {
        setBusy(false);
      }
    },
    []
  );

  return { busy, failed, scheduledForReboot, freed, error, run, forceDelete, reset };
}
