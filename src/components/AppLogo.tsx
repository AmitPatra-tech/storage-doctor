import { useEffect, useState } from "react";
import { AppWindow } from "lucide-react";

/**
 * An installed application's logo, extracted from its executable. Falls back
 * to the app's initial on a neutral tile when Windows has no icon for it, so
 * every row keeps the same visual weight.
 */
export function AppLogo({
  name,
  src,
  size = 28,
}: {
  name: string;
  src?: string;
  size?: number;
}) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);

  const initial = name.trim().charAt(0).toUpperCase();

  return (
    <div
      className="flex shrink-0 items-center justify-center overflow-hidden rounded-md border border-border bg-surface-hover"
      style={{ width: size, height: size }}
    >
      {src && !failed ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          className="h-full w-full object-contain"
          onError={() => setFailed(true)}
        />
      ) : initial ? (
        <span className="text-xs font-semibold text-muted">{initial}</span>
      ) : (
        <AppWindow className="h-4 w-4 text-muted" />
      )}
    </div>
  );
}
