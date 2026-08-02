import type { QueryClient } from "@tanstack/react-query";

/** Every query whose answer changes once files are deleted.
 *
 *  Pages used to invalidate an ad-hoc subset each — which is why freeing 30 GB
 *  left the breakdown, the dashboard and the reports showing the old numbers
 *  until the next full scan. Deletions now go through one list. */
const CLEANUP_SENSITIVE = [
  "lastScan",
  "drives",
  "browse",
  "safeCleanup",
  "recommendations",
  "scanComparison",
  "operations",
  "appUsage",
  "search",
];

export function refreshAfterCleanup(queryClient: QueryClient) {
  for (const key of CLEANUP_SENSITIVE) {
    queryClient.invalidateQueries({ queryKey: [key] });
  }
}
