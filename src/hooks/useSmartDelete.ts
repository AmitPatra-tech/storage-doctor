import { useCallback, useState } from "react";
import { backend } from "@/lib/backend";

/** Deletion with graceful failure handling and an admin-elevated retry.
 *  Used by every delete flow so the "permission denied / operation aborted"
 *  case is handled consistently. */
export function useSmartDelete() {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string[]>([]);
  const [freed, setFreed] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const reset = useCallback(() => {
    setBusy(false);
    setFailed([]);
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

  return { busy, failed, freed, error, run, reset };
}
