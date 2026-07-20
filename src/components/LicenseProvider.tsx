import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { backend } from "@/lib/backend";
import { isLicenseActive, revalidateLicense, verifyLicenseKey } from "@/lib/license";
import type { License } from "@/lib/types";

interface LicenseContextValue {
  license: License | null;
  isPro: boolean;
  loading: boolean;
  activate: (key: string) => Promise<void>;
  deactivate: () => Promise<void>;
}

const LicenseContext = createContext<LicenseContextValue | null>(null);

export function LicenseProvider({ children }: { children: ReactNode }) {
  const [license, setLicense] = useState<License | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    (async () => {
      const stored = await backend.getLicense();
      if (!isLicenseActive(stored)) {
        setLicense(null);
        setLoading(false);
        return;
      }
      setLicense(stored);
      setLoading(false);
      // Re-verify with Dodo in the background. Only downgrade on a definitive
      // "invalid"; keep Pro when offline (null) so a flaky connection never
      // locks a paying user out.
      const result = await revalidateLicense(stored!);
      if (result === false) {
        await backend.clearLicense();
        setLicense(null);
      }
    })();
  }, []);

  const activate = async (key: string) => {
    const lic = await verifyLicenseKey(key);
    await backend.setLicense(lic);
    setLicense(lic);
  };

  const deactivate = async () => {
    await backend.clearLicense();
    setLicense(null);
  };

  return (
    <LicenseContext.Provider
      value={{ license, isPro: isLicenseActive(license), loading, activate, deactivate }}
    >
      {children}
    </LicenseContext.Provider>
  );
}

export function useLicense(): LicenseContextValue {
  const ctx = useContext(LicenseContext);
  if (!ctx) throw new Error("useLicense must be used within LicenseProvider");
  return ctx;
}
