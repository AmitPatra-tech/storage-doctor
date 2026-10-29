import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { EyeOff, FolderOpen, Loader2, Trash2 } from "lucide-react";
import { backend } from "@/lib/backend";
import { refreshAfterCleanup } from "@/lib/refresh";
import { formatBytes, removalNotice } from "@/lib/utils";
import { RiskBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";
import { ConfirmDeleteModal, type DeleteItem } from "@/components/ConfirmDeleteModal";
import type { Recommendation } from "@/lib/types";

export function Recommendations() {
  const queryClient = useQueryClient();
  const [target, setTarget] = useState<Recommendation | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** The recommendation just cleaned, while every recommendation is being
   *  re-measured — that takes a while and must be visible. */
  const [updating, setUpdating] = useState<string | null>(null);

  const { data: recommendations, isLoading } = useQuery({
    queryKey: ["recommendations"],
    queryFn: backend.getRecommendations,
  });

  const ignoreMutation = useMutation({
    mutationFn: (id: string) => backend.setRecommendationIgnored(id, true),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["recommendations"] }),
  });

  const totalRecoverable =
    recommendations?.reduce((sum, r) => sum + r.recoverableBytes, 0) ?? 0;

  // Each recommendation's paths become delete items; risk maps to a safety
  // label so the confirmation dialog reads consistently.
  const targetItems: DeleteItem[] = (target?.paths ?? []).map((path) => ({
    path,
    sizeBytes: 0,
    classification: {
      category: target!.name,
      safety: target!.risk === "low" ? "safe" : "review",
      explanation: target!.description,
    },
  }));

  const afterDelete = async (bytes: number, remaining: string[]) => {
    setNotice(removalNotice(bytes, true, remaining.length));
    setUpdating(target?.id ?? null);
    setTarget(null);
    try {
      // Re-measure so cleaned recommendations drop off / update their size.
      await backend.regenerateRecommendations();
      await queryClient.invalidateQueries({ queryKey: ["recommendations"] });
      refreshAfterCleanup(queryClient);
    } finally {
      setUpdating(null);
    }
  };

  return (
    <div>
      <PageHeader
        title="Recommendations"
        description={`Safe cleanup suggestions. Up to ${formatBytes(totalRecoverable)} recoverable.`}
      />
      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}
      {updating && (
        <div className="mb-3 rounded-md border border-primary/30 bg-primary/10 px-4 py-2.5 text-sm">
          <div className="mb-1.5 flex items-center gap-2 text-primary">
            <Loader2 className="h-4 w-4 animate-spin" />
            Updating recommendations — re-measuring what is left…
          </div>
          <div className="h-1.5 overflow-hidden rounded-full bg-surface-hover">
            <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
          </div>
        </div>
      )}
      {isLoading && <p className="text-sm text-muted">Loading recommendations…</p>}
      {recommendations?.length === 0 && (
        <Card>
          <CardContent className="p-5 text-sm text-muted">
            No recommendations yet — run a scan from the Dashboard first.
          </CardContent>
        </Card>
      )}
      <div className="flex flex-col gap-3">
        {recommendations?.map((rec) => (
          <Card key={rec.id}>
            <CardContent className="flex items-center gap-4 p-5">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2.5">
                  <span className="text-sm font-medium">{rec.name}</span>
                  <RiskBadge risk={rec.risk} />
                  {rec.recommended && (
                    <span className="text-xs text-success">Recommended</span>
                  )}
                </div>
                <p className="mt-1 text-sm text-muted">{rec.description}</p>
                {rec.paths[0] && (
                  <p className="mt-1 truncate text-xs text-muted/70" title={rec.paths.join("\n")}>
                    {rec.paths[0]}
                    {rec.paths.length > 1 && ` (+${rec.paths.length - 1} more)`}
                  </p>
                )}
              </div>
              <div className="shrink-0 text-right">
                {updating === rec.id ? (
                  <div className="flex items-center gap-1.5 text-xs text-primary">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" /> Updating…
                  </div>
                ) : (
                  <>
                    <div className="text-sm font-semibold text-success">
                      {formatBytes(rec.recoverableBytes)}
                    </div>
                    <div className="text-xs text-muted">recoverable</div>
                  </>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                {rec.paths[0] && (
                  <Button
                    variant="ghost"
                    size="icon"
                    title="Open location"
                    onClick={() => backend.revealInExplorer(rec.paths[0])}
                  >
                    <FolderOpen className="h-4 w-4" />
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  title="Ignore this recommendation"
                  disabled={ignoreMutation.isPending}
                  onClick={() => ignoreMutation.mutate(rec.id)}
                >
                  <EyeOff className="h-4 w-4" />
                </Button>
                {rec.paths.length > 0 && (
                  <Button
                    variant="danger"
                    size="sm"
                    title="Delete permanently to free the space (asks for administrator rights only if needed)"
                    disabled={updating !== null}
                    onClick={() => setTarget(rec)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    Clean
                  </Button>
                )}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      <ConfirmDeleteModal
        open={target !== null}
        title={target ? `Clean “${target.name}”?` : ""}
        items={targetItems}
        knownTotalBytes={target?.recoverableBytes}
        source={target?.name}
        permanent
        allowForce={false}
        onDone={afterDelete}
        onClose={() => setTarget(null)}
      />
    </div>
  );
}
