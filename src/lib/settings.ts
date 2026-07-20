export type Theme = "dark" | "light";

export interface AppSettings {
  theme: Theme;
  /** Drive letters to scan by default; empty means all local drives. */
  scanDrives: string[];
}

const KEY = "storage-doctor:settings";

const DEFAULTS: AppSettings = {
  theme: "dark",
  scanDrives: [],
};

export function loadSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULTS };
    return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    return { ...DEFAULTS };
  }
}

export function saveSettings(settings: AppSettings) {
  localStorage.setItem(KEY, JSON.stringify(settings));
}

/** Applies the theme by toggling the `light` class on <html>; the CSS
 *  variables in index.css switch accordingly. */
export function applyTheme(theme: Theme) {
  document.documentElement.classList.toggle("light", theme === "light");
}
