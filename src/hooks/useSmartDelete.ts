import { useCallback, useState } from "react";
import { backend } from "@/lib/backend";
import type { ForceDeleteResult, ItemNote } from "@/lib/types";

/** Deletion with graceful failure handling, an admin-elevated retry, and a
 *  force-delete last resort for items still locked open even as
 *  administrator. Used by every delete flow so these cases are handled
 *  consistently. */
export function useSmartDelete() {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string[]>([]);
  /** The full result of the last force delete — null until one has run
   *  successfully, so an error is never mistaken for "nothing left". */
  const [forceResult, setForceResult] = useState<ForceDeleteResult | null>(null);
  /** Items this app will never delete, with the reason — not retried. */
  const [refused, setRefused] = useState<ItemNote[]>([]);
  const [freed, setFreed] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const reset = useCallback(() => {
    setBusy(false);
    setFailed([]);
    setRefused([]);
    setForceResult(null);
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
        setRefused((prev) => [
          ...prev,
          ...res.refused.filter((r) => !prev.some((p) => p.path === r.path)),
        ]);
        return res;
      } catch (e) {
        setError(String(e));
        return { freedBytes: 0, failed: paths, refused: [] as ItemNote[] };
      } finally {
        setBusy(false);
      }
    },
    []
  );

  /** Offered only once `run(paths, true, ...)` has already left failures.
   *  Returns null when the force delete itself could not run (e.g. the
   *  administrator prompt was declined); `failed` is left unchanged then. */
  const forceDelete = useCallback(
    async (paths: string[], approved: string[], source?: string, permanent = false) => {
      setBusy(true);
      setError(null);
      try {
        const res = await backend.forceDeletePaths(paths, source, permanent, approved);
        setFreed((f) => f + res.freedBytes);
        setForceResult(res);
        setFailed([
          ...res.blocked.map((b) => b.path),
          ...res.scheduledForReboot,
          ...res.failed.map((f) => f.path),
        ]);
        return res;
      } catch (e) {
        setError(String(e));
        return null;
      } finally {
        setBusy(false);
      }
    },
    []
  );

  return { busy, failed, refused, forceResult, freed, error, run, forceDelete, reset };
}
