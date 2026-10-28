import { PageHeader } from "@/components/PageHeader";
import { InstalledApps } from "@/components/InstalledApps";
import { StoreApps } from "@/components/StoreApps";
import { OrphanedLeftovers } from "@/components/OrphanedLeftovers";

export function Applications() {
  return (
    <div>
      <PageHeader
        title="Applications"
        description="Every installed application with its storage, recoverable cache size, and a guided uninstall that also removes leftovers."
      />
      <InstalledApps />
      <StoreApps />
      <OrphanedLeftovers />
    </div>
  );
}
