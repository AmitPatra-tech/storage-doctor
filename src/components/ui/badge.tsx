import type { HTMLAttributes } from "react";
import { cn } from "@/lib/utils";
import type { Classification, RiskLevel, Safety } from "@/lib/types";

const riskStyles: Record<RiskLevel, string> = {
  low: "bg-success/15 text-success",
  medium: "bg-warning/15 text-warning",
  high: "bg-danger/15 text-danger",
};

export function RiskBadge({ risk }: { risk: RiskLevel }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium capitalize",
        riskStyles[risk]
      )}
    >
      {risk}
    </span>
  );
}

const safetyStyles: Record<Safety, { label: string; className: string }> = {
  safe: { label: "Safe to clear", className: "bg-success/15 text-success" },
  review: { label: "Review", className: "bg-warning/15 text-warning" },
  personal: { label: "Your files", className: "bg-primary/15 text-primary" },
  apps: { label: "App files", className: "bg-surface-hover text-muted" },
  system: { label: "System", className: "bg-danger/15 text-danger" },
};

/** Tags an entry with what it is and whether deleting it is safe.
 *  The full explanation is shown on hover. */
export function SafetyBadge({ classification }: { classification?: Classification | null }) {
  if (!classification) return null;
  const style = safetyStyles[classification.safety] ?? safetyStyles.apps;
  return (
    <span
      title={`${classification.category} — ${classification.explanation}`}
      className={cn(
        "inline-flex shrink-0 cursor-help items-center rounded-full px-2 py-0.5 text-[11px] font-medium",
        style.className
      )}
    >
      {style.label}
    </span>
  );
}

export function Badge({
  className,
  ...props
}: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full bg-surface-hover px-2.5 py-0.5 text-xs font-medium",
        className
      )}
      {...props}
    />
  );
}
