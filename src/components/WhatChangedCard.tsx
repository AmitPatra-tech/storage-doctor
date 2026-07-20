import { useQuery } from "@tanstack/react-query";
import { TrendingDown, TrendingUp } from "lucide-react";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

function formatDelta(bytes: number): string {
  return `${bytes >= 0 ? "+" : "−"}${formatBytes(Math.abs(bytes))}`;
}

function timeAgo(iso: string): string {
  const ms = Date.now() - new Date(iso.endsWith("Z") ? iso : iso + "Z").getTime();
  const hours = Math.round(ms / 3_600_000);
  if (hours < 1) return "less than an hour ago";
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

/** Answers "why did my storage increase?" by comparing the last two scans. */
export function WhatChangedCard() {
  const { data: comparison } = useQuery({
    queryKey: ["scanComparison"],
    queryFn: backend.getScanComparison,
  });

  if (!comparison || comparison.changes.length === 0) return null;

  const grew = comparison.deltaBytes >= 0;

  return (
    <Card>
      <CardHeader>
        <CardTitle>
          What changed since the last scan ({timeAgo(comparison.previousAt)})
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex items-center gap-2 text-sm">
          {grew ? (
            <TrendingUp className="h-4 w-4 text-warning" />
          ) : (
            <TrendingDown className="h-4 w-4 text-success" />
          )}
          <span className={grew ? "text-warning" : "text-success"}>
            {formatDelta(comparison.deltaBytes)} used space
          </span>
        </div>
        {comparison.changes.map((change) => (
          <div
            key={change.path}
            className="flex items-center justify-between gap-4 text-sm"
          >
            <div className="min-w-0">
              <span className="font-medium">{change.name}</span>
              {change.isNew && (
                <span className="ml-2 text-xs text-primary">new</span>
              )}
              <div className="truncate text-xs text-muted">{change.path}</div>
            </div>
            <span
              className={`shrink-0 ${
                change.deltaBytes >= 0 ? "text-warning" : "text-success"
              }`}
            >
              {formatDelta(change.deltaBytes)}
            </span>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}
