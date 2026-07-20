import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Download, FileText } from "lucide-react";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/utils";
import type { OperationRecord } from "@/lib/types";
import { useLicense } from "@/components/LicenseProvider";
import { UpgradePanel } from "@/components/ProGate";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";

function formatDate(iso: string): string {
  const d = new Date(iso.endsWith("Z") ? iso : iso + "Z");
  return d.toLocaleString();
}

async function buildPdf(ops: OperationRecord[]): Promise<Uint8Array> {
  const { jsPDF } = await import("jspdf");
  const doc = new jsPDF();
  const total = ops.reduce((s, o) => s + o.freedBytes, 0);

  doc.setFontSize(18);
  doc.text("Storage Doctor — Cleanup Report", 14, 20);
  doc.setFontSize(10);
  doc.setTextColor(120);
  doc.text(`Generated ${new Date().toLocaleString()}`, 14, 27);
  doc.text(`by HutZon`, 14, 32);
  doc.setTextColor(0);

  doc.setFontSize(12);
  doc.text(`Total space reclaimed: ${formatBytes(total)}`, 14, 42);
  doc.text(`Operations recorded: ${ops.length}`, 14, 48);

  let y = 60;
  doc.setFontSize(10);
  doc.setTextColor(120);
  doc.text("Date", 14, y);
  doc.text("Action", 70, y);
  doc.text("Items", 140, y);
  doc.text("Freed", 165, y);
  doc.setTextColor(0);
  doc.line(14, y + 1.5, 196, y + 1.5);
  y += 7;

  for (const op of ops) {
    if (y > 280) {
      doc.addPage();
      y = 20;
    }
    doc.text(formatDate(op.performedAt).slice(0, 22), 14, y);
    doc.text(op.source.slice(0, 34), 70, y);
    doc.text(String(op.itemCount), 140, y);
    doc.text(formatBytes(op.freedBytes), 165, y);
    y += 6.5;
  }

  return new Uint8Array(doc.output("arraybuffer"));
}

export function Reports() {
  const { isPro } = useLicense();
  const [notice, setNotice] = useState<string | null>(null);

  const { data: operations } = useQuery({
    queryKey: ["operations"],
    queryFn: backend.getOperations,
  });

  const totalFreed = (operations ?? []).reduce((s, o) => s + o.freedBytes, 0);

  const exportPdf = async () => {
    if (!operations || operations.length === 0) return;
    const bytes = await buildPdf(operations);
    const saved = await backend.savePdf(
      bytes,
      `storage-doctor-report-${new Date().toISOString().slice(0, 10)}.pdf`
    );
    if (saved) setNotice(`Report saved to ${saved}`);
  };

  return (
    <div>
      <PageHeader
        title="Reports"
        description="A journal of every cleanup Storage Doctor has performed, exportable as a PDF."
        actions={
          isPro && (
            <Button
              size="lg"
              disabled={!operations || operations.length === 0}
              onClick={exportPdf}
            >
              <Download className="h-4 w-4" />
              Export PDF
            </Button>
          )
        }
      />

      {notice && (
        <div className="mb-3 rounded-md border border-success/30 bg-success/10 px-4 py-2.5 text-sm text-success">
          {notice}
        </div>
      )}

      <Card className="mb-4">
        <CardContent className="flex items-center justify-between p-5">
          <div>
            <div className="text-sm text-muted">Total space reclaimed</div>
            <div className="text-2xl font-semibold text-success">{formatBytes(totalFreed)}</div>
          </div>
          <div className="text-right">
            <div className="text-sm text-muted">Operations recorded</div>
            <div className="text-2xl font-semibold">{operations?.length ?? 0}</div>
          </div>
        </CardContent>
      </Card>

      {!isPro && (
        <div className="mb-4">
          <UpgradePanel feature="PDF report export" />
        </div>
      )}

      <Card>
        <CardContent className="divide-y divide-border p-0">
          {(operations ?? []).length === 0 && (
            <div className="flex flex-col items-center gap-2 py-14 text-center">
              <FileText className="h-9 w-9 text-muted" />
              <p className="text-sm text-muted">
                No cleanups recorded yet. Deletions you perform will appear here.
              </p>
            </div>
          )}
          {operations?.map((op) => (
            <div key={op.id} className="flex items-center gap-3 px-5 py-3 text-sm">
              <div className="min-w-0 flex-1">
                <div className="font-medium">{op.source}</div>
                <div className="text-xs text-muted">
                  {formatDate(op.performedAt)} · {op.itemCount} item(s)
                  {op.method === "recycle-admin" && " · admin"}
                </div>
              </div>
              <div className="shrink-0 text-success">{formatBytes(op.freedBytes)}</div>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
