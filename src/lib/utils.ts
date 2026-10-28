import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const i = Math.min(Math.floor(Math.log2(bytes) / 10), UNITS.length - 1);
  const value = bytes / 2 ** (10 * i);
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${UNITS[i]}`;
}

/** The notice shown after a delete, worded for what actually happened to
 *  the space: permanently deleted items free it; items in the Recycle Bin
 *  still use it until the bin is emptied. */
export function removalNotice(bytes: number, permanent: boolean, notRemoved = 0): string {
  const main =
    bytes <= 0
      ? "No data was removed."
      : permanent
        ? `Freed ${formatBytes(bytes)} — permanently deleted.`
        : `Moved ${formatBytes(bytes)} to the Recycle Bin — empty the Recycle Bin to free the space.`;
  return notRemoved > 0 ? `${main} ${notRemoved} item(s) were not removed.` : main;
}

export function formatPercent(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  return `${((part / whole) * 100).toFixed(1)}%`;
}

/** "just now" / "3 hours ago" / "2 days ago" — used to date scan results so a
 *  stored size is never mistaken for a live one. */
export function formatRelativeTime(iso: string): string {
  // SQLite's datetime('now') has no timezone marker but is always UTC.
  const stamp = /[Zz]|[+-]\d{2}:?\d{2}$/.test(iso) ? iso : `${iso.replace(" ", "T")}Z`;
  const then = new Date(stamp).getTime();
  if (Number.isNaN(then)) return "recently";

  const minutes = Math.round((Date.now() - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}
