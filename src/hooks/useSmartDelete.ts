import { useCallback, useState } from "react";
import { backend } from "@/lib/backend";

/** Deletion with graceful failure handling, an admin-elevated retry, and a
 *  force-delete last resort for items still locked open even as
 *  administrator. Used by every delete flow so these cases are handled
 *  consistently. */
export function useSmartDelete() {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string[]>([]);
  /** Set only by `forceDelete`: a restart removal was scheduled as a last
   *  resort (unreliable — never promised). */
  const [scheduledForReboot, setScheduledForReboot] = useState<string[]>([]);
  /** Programs still holding an item open after every attempt — what the user
   *  must close. When set, a restart will not help. */
  const [blockedBy, setBlockedBy] = useState<string[]>([]);
  const [freed, setFreed] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const reset = useCallback(() => {
    setBusy(false);
    setFailed([]);
    setScheduledForReboot([]);
    setBlockedBy([]);
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
        setBlockedBy(res.blockedBy);
        return res;
      } catch (e) {
        setError(String(e));
        return { freedBytes: 0, removed: [], scheduledForReboot: [], failed: paths, blockedBy: [] };
      } finally {
        setBusy(false);
      }
    },
    []
  );

  return { busy, failed, scheduledForReboot, blockedBy, freed, error, run, forceDelete, reset };
}
