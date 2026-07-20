import { useState } from "react";
import { Crown, KeyRound, Loader2 } from "lucide-react";
import { backend } from "@/lib/backend";
import { CONFIG, dodoCheckoutUrl } from "@/lib/config";
import { useLicense } from "@/components/LicenseProvider";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/** Inline upgrade panel: opens Dodo checkout and accepts a license key.
 *  Shown wherever a Pro feature is gated. */
export function UpgradePanel({ feature }: { feature: string }) {
  const { activate } = useLicense();
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const activateKey = async () => {
    setBusy(true);
    setError(null);
    try {
      await activate(key);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card className="border-primary/40">
      <CardContent className="flex flex-col items-center gap-4 px-6 py-10 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/15">
          <Crown className="h-6 w-6 text-primary" />
        </div>
        <div>
          <h3 className="text-base font-semibold">{feature} is a Pro feature</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-muted">
            Upgrade to Storage Doctor Pro to unlock the duplicate finder and exportable
            PDF cleanup reports — a {CONFIG.proPriceLabel} that keeps future premium
            updates.
          </p>
        </div>

        <Button size="lg" onClick={() => backend.openExternal(dodoCheckoutUrl)}>
          <Crown className="h-4 w-4" />
          Upgrade to Pro — {CONFIG.proPriceLabel}
        </Button>

        <div className="mt-2 w-full max-w-sm">
          <p className="mb-2 rounded-md border border-border bg-surface-hover/50 px-3 py-2 text-xs text-muted">
            After completing the payment, send the invoice to{" "}
            <span className="font-medium text-foreground">{CONFIG.invoiceEmail}</span> — we
            will respond within 5 minutes.
          </p>
          <div className="mb-1.5 flex items-center gap-2 text-xs text-muted">
            <KeyRound className="h-3.5 w-3.5" />
            Already purchased? Enter your license key
          </div>
          <div className="flex gap-2">
            <input
              type="text"
              placeholder="License key"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && activateKey()}
              className="h-9 flex-1 rounded-md border border-border bg-surface px-3 text-sm placeholder:text-muted focus:outline-none focus:ring-2 focus:ring-primary"
            />
            <Button variant="secondary" size="sm" disabled={busy || !key.trim()} onClick={activateKey}>
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Activate"}
            </Button>
          </div>
          {error && <p className="mt-2 text-left text-xs text-danger">{error}</p>}
        </div>
      </CardContent>
    </Card>
  );
}
