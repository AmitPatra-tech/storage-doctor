import { NavLink, Outlet } from "react-router-dom";
import {
  AppWindow,
  CopyCheck,
  FileSearch,
  FileText,
  FolderTree,
  HardDrive,
  LayoutDashboard,
  Loader2,
  Search,
  Settings,
  Sparkles,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useLicense } from "@/components/LicenseProvider";
import { useRunningTasks } from "@/components/RunningTasksProvider";

const navItems = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard },
  { to: "/breakdown", label: "Storage Breakdown", icon: FolderTree },
  { to: "/large-files", label: "Large Files", icon: FileSearch },
  { to: "/applications", label: "Applications", icon: AppWindow },
  { to: "/recommendations", label: "Recommendations", icon: Sparkles },
  { to: "/duplicates", label: "Duplicate Files", icon: CopyCheck },
  { to: "/search", label: "Search", icon: Search },
  { to: "/reports", label: "Reports", icon: FileText },
  { to: "/settings", label: "Settings", icon: Settings },
];

export function Layout() {
  const { isPro } = useLicense();
  const { scanning, dupeScanning, deepSearching } = useRunningTasks();

  // These keep running while you are on another page, so the sidebar says so —
  // otherwise leaving the tab looks indistinguishable from cancelling.
  const busy: Record<string, boolean> = {
    "/": scanning,
    "/duplicates": dupeScanning,
    "/search": deepSearching,
  };

  return (
    <div className="flex h-full">
      <aside className="flex w-60 shrink-0 flex-col border-r border-border bg-surface">
        <div className="flex items-center gap-2.5 px-5 py-5">
          <HardDrive className="h-6 w-6 text-primary" />
          <div>
            <div className="text-sm font-semibold leading-tight">Storage Doctor</div>
            <div className="text-[11px] text-muted">by HutZon</div>
          </div>
        </div>
        <nav className="flex flex-1 flex-col gap-1 px-3">
          {navItems.map(({ to, label, icon: Icon }) => (
            <NavLink
              key={to}
              to={to}
              end={to === "/"}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-sm transition-colors",
                  isActive
                    ? "bg-primary/15 font-medium text-primary"
                    : "text-muted hover:bg-surface-hover hover:text-foreground"
                )
              }
            >
              <Icon className="h-4 w-4 shrink-0" />
              <span className="flex-1">{label}</span>
              {busy[to] && (
                <Loader2
                  className="h-3.5 w-3.5 shrink-0 animate-spin text-primary"
                  aria-label="Still running"
                />
              )}
            </NavLink>
          ))}
        </nav>
        <div className="px-5 py-4 text-[11px]">
          {isPro ? (
            <span className="font-medium text-primary">Pro</span>
          ) : (
            <span className="text-muted">Free version</span>
          )}
        </div>
      </aside>
      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl p-8">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
