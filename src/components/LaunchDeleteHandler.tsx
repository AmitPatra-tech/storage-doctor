import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { backend } from "@/lib/backend";
import { refreshAfterCleanup } from "@/lib/refresh";
import { ConfirmDeleteModal, type DeleteItem } from "@/components/ConfirmDeleteModal";

/** Bridges the Explorer right-click "Force delete with Storage Doctor" verb
 *  into the normal delete flow: it opens the same confirm-and-escalate modal
 *  (Recycle Bin → administrator → force) for the path the shell handed us,
 *  whether the app was launched by the verb or was already running when it
 *  fired. Renders nothing until there is a target. */
export function LaunchDeleteHandler() {
  const queryClient = useQueryClient();
  const [target, setTarget] = useState<string | null>(null);

  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    let disposed = false;

    // Launched by the verb: pick up the path captured at startup.
    backend.takeLaunchDeletePath().then((path) => {
      if (!disposed && path) setTarget(path);
    });

    // Already running when a later right-click fires: the path arrives here.
    backend.onForceDeleteRequest((path) => setTarget(path)).then((off) => {
      if (disposed) off();
      else unsubscribe = off;
    });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, []);

  if (!target) return null;

  const items: DeleteItem[] = [{ path: target, sizeBytes: 0 }];
  return (
    <ConfirmDeleteModal
      open
      title="Force delete this item?"
      items={items}
      source="Right-click force delete"
      onDone={() => {
        setTarget(null);
        refreshAfterCleanup(queryClient);
      }}
      onClose={() => setTarget(null)}
    />
  );
}
