import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, Download, Loader2, Moon, RefreshCw, Sun } from "lucide-react";
import { backend } from "@/lib/backend";
import { useAppUpdate } from "@/hooks/useAppUpdate";
import {
  applyTheme,
  loadSettings,
  saveSettings,
  type AppSettings,
} from "@/lib/settings";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { useLicense } from "@/components/LicenseProvider";
import { CONFIG, dodoCheckoutUrl } from "@/lib/config";

function SettingRow({
  label,
  description,
  control,
}: {
  label: string;
  description: string;
  control: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 py-3.5">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <div className="text-xs text-muted">{description}</div>
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}


export function Settings() {
  const [settings, setSettings] = useState<AppSettings>(loadSettings);
  const { data: drives } = useQuery({ queryKey: ["drives"], queryFn: backend.getDrives });
  const { isPro, license, activate, deactivate } = useLicense();
  const { status: updateStatus, check: checkUpdate, install: installUpdate } = useAppUpdate();
  const [licenseKey, setLicenseKey] = useState("");
  const [licenseBusy, setLicenseBusy] = useState(false);
  const [licenseError, setLicenseError] = useState<string | null>(null);

  const activateLicense = async () => {
    setLicenseBusy(true);
    setLicenseError(null);
    try {
      await activate(licenseKey);
      setLicenseKey("");
    } catch (e) {
      setLicenseError(e instanceof Error ? e.message : String(e));
    } finally {
      setLicenseBusy(false);
    }
  };

  const update = (patch: Partial<AppSettings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    saveSettings(next);
    if (patch.theme) applyTheme(patch.theme);
  };

  const toggleDrive = (letter: string) => {
    const has = settings.scanDrives.includes(letter);
    update({
      scanDrives: has
        ? settings.scanDrives.filter((l) => l !== letter)
        : [...settings.scanDrives, letter],
    });
  };

  return (
    <div>
      <PageHeader title="Settings" />
      <div className="flex flex-col gap-4">
        <Card>
          <CardHeader>
            <CardTitle>General</CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <SettingRow
              label="Theme"
              description="Appearance of the application"
              control={
                <div className="flex gap-1.5">
                  <Button
                    size="sm"
                    variant={settings.theme === "dark" ? "default" : "secondary"}
                    onClick={() => update({ theme: "dark" })}
                  >
                    <Moon className="h-3.5 w-3.5" />
                    Dark
                  </Button>
                  <Button
                    size="sm"
                    variant={settings.theme === "light" ? "default" : "secondary"}
                    onClick={() => update({ theme: "light" })}
                  >
                    <Sun className="h-3.5 w-3.5" />
                    Light
                  </Button>
                </div>
              }
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Default scan location</CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <p className="mb-3 text-xs text-muted">
              Choose which drives the Scan button covers. Select none to scan all local
              drives.
            </p>
            <div className="flex flex-wrap gap-2">
              {(drives ?? []).map((drive) => {
                const active =
                  settings.scanDrives.length === 0 ||
                  settings.scanDrives.includes(drive.letter);
                const explicit = settings.scanDrives.includes(drive.letter);
                return (
                  <button
                    key={drive.letter}
                    onClick={() => toggleDrive(drive.letter)}
                    className={`flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm transition-colors ${
                      explicit
                        ? "border-primary bg-primary/15 text-primary"
                        : "border-border text-muted hover:bg-surface-hover"
                    }`}
                    title={active ? "Included in scans" : "Excluded"}
                  >
                    {explicit && <Check className="h-3.5 w-3.5" />}
                    {drive.label} ({drive.letter})
                  </button>
                );
              })}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>License</CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            {isPro ? (
              <div className="flex items-center justify-between">
                <div>
                  <div className="text-sm font-medium text-primary">Storage Doctor Pro</div>
                  <div className="text-xs text-muted">
                    Active{license?.email ? ` · ${license.email}` : ""}
                    {license?.expiresAt
                      ? ` · renews ${new Date(license.expiresAt).toLocaleDateString()}`
                      : " · lifetime"}
                  </div>
                </div>
                <Button variant="secondary" onClick={deactivate}>
                  Deactivate
                </Button>
              </div>
            ) : (
              <div className="flex flex-col gap-3">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-sm font-medium">Free version</div>
                    <div className="text-xs text-muted">
                      Unlock the duplicate finder and exportable PDF reports.
                    </div>
                  </div>
                  <Button onClick={() => backend.openExternal(dodoCheckoutUrl)}>
                    Upgrade to Pro — {CONFIG.proPriceLabel}
                  </Button>
                </div>
                <p className="rounded-md border border-border bg-surface-hover/50 px-3 py-2 text-xs text-muted">
                  After completing the payment, send the invoice to{" "}
                  <span className="font-medium text-foreground">{CONFIG.invoiceEmail}</span> —
                  we will respond within 5 minutes.
                </p>
                <div className="flex gap-2">
                  <input
                    type="text"
                    placeholder="Enter license key"
                    value={licenseKey}
                    onChange={(e) => setLicenseKey(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && activateLicense()}
                    className="h-9 flex-1 rounded-md border border-border bg-surface px-3 text-sm placeholder:text-muted focus:outline-none focus:ring-2 focus:ring-primary"
                  />
                  <Button
                    variant="secondary"
                    disabled={licenseBusy || !licenseKey.trim()}
                    onClick={activateLicense}
                  >
                    {licenseBusy ? "Activating…" : "Activate"}
                  </Button>
                </div>
                {licenseError && <p className="text-xs text-danger">{licenseError}</p>}
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>About</CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <p className="text-sm text-muted">
              Storage Doctor 1.0.0 — by HutZon. No file contents ever leave your
              computer.
            </p>
            <div className="mt-3 flex items-center gap-3">
              {updateStatus.state === "available" ? (
                <Button size="sm" onClick={installUpdate}>
                  <Download className="h-3.5 w-3.5" />
                  Update to {updateStatus.version}
                </Button>
              ) : (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={
                    updateStatus.state === "checking" ||
                    updateStatus.state === "downloading"
                  }
                  onClick={() => checkUpdate()}
                >
                  {updateStatus.state === "checking" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" />
                  )}
                  Check for updates
                </Button>
              )}
              <span className="text-xs text-muted">
                {updateStatus.state === "checking" && "Checking…"}
                {updateStatus.state === "uptodate" && "You're on the latest version."}
                {updateStatus.state === "downloading" &&
                  `Downloading… ${updateStatus.percent}%`}
                {updateStatus.state === "ready" && "Installed — restarting…"}
                {updateStatus.state === "error" && `Couldn't check: ${updateStatus.message}`}
              </span>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
