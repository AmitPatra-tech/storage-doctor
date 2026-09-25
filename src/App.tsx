import { HashRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Layout } from "@/components/Layout";
import { Dashboard } from "@/pages/Dashboard";
import { Breakdown } from "@/pages/Breakdown";
import { LargeFiles } from "@/pages/LargeFiles";
import { Applications } from "@/pages/Applications";
import { Recommendations } from "@/pages/Recommendations";
import { Duplicates } from "@/pages/Duplicates";
import { SearchPage } from "@/pages/SearchPage";
import { Reports } from "@/pages/Reports";
import { Settings } from "@/pages/Settings";
import { LicenseProvider } from "@/components/LicenseProvider";
import { RunningTasksProvider } from "@/components/RunningTasksProvider";
import { UpdateBanner } from "@/components/UpdateBanner";
import { LaunchDeleteHandler } from "@/components/LaunchDeleteHandler";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 1,
    },
  },
});

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <LicenseProvider>
        <RunningTasksProvider>
        <HashRouter>
          <Routes>
            <Route element={<Layout />}>
              <Route index element={<Dashboard />} />
              <Route path="breakdown" element={<Breakdown />} />
              <Route path="large-files" element={<LargeFiles />} />
              <Route path="applications" element={<Applications />} />
              <Route path="recommendations" element={<Recommendations />} />
              <Route path="duplicates" element={<Duplicates />} />
              <Route path="search" element={<SearchPage />} />
              <Route path="reports" element={<Reports />} />
              <Route path="settings" element={<Settings />} />
            </Route>
          </Routes>
          <UpdateBanner />
          <LaunchDeleteHandler />
        </HashRouter>
        </RunningTasksProvider>
      </LicenseProvider>
    </QueryClientProvider>
  );
}
