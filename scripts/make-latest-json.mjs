// Generates the `latest.json` update manifest that tauri-plugin-updater fetches
// from the release endpoint. Run this AFTER `npm run tauri build`.
//
//   node scripts/make-latest-json.mjs "Release notes for this version"
//
// It reads the version from tauri.conf.json, locates the signed NSIS installer
// and its `.sig` in the bundle output, and writes `latest.json` next to them.
// Upload BOTH the `-setup.exe` and `latest.json` to the GitHub release whose
// tag matches the version (e.g. v1.1.0), so the download URL below resolves.
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const notes = process.argv[2] ?? "";

// GitHub owner/repo the updater endpoint points at — keep in sync with the
// `plugins.updater.endpoints` URL in tauri.conf.json.
const REPO = "AmitPatra-tech/storage-doctor";

const conf = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const version = conf.version;
const productName = conf.productName; // "Storage Doctor"

const bundleDir = join(root, "src-tauri/target/release/bundle/nsis");
const setupName = `${productName}_${version}_x64-setup.exe`;
const setupPath = join(bundleDir, setupName);
const sigPath = `${setupPath}.sig`;

if (!existsSync(setupPath)) {
  console.error(`Installer not found: ${setupPath}\nRun \`npm run tauri build\` first.`);
  process.exit(1);
}
if (!existsSync(sigPath)) {
  console.error(
    `Signature not found: ${sigPath}\n` +
      "The build must sign updater artifacts — set TAURI_SIGNING_PRIVATE_KEY " +
      "(and TAURI_SIGNING_PRIVATE_KEY_PASSWORD) before building."
  );
  process.exit(1);
}

const signature = readFileSync(sigPath, "utf8").trim();
const tag = `v${version}`;
const url = `https://github.com/${REPO}/releases/download/${tag}/${encodeURIComponent(setupName)}`;

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": { signature, url },
  },
};

const outPath = join(bundleDir, "latest.json");
writeFileSync(outPath, JSON.stringify(manifest, null, 2));
console.log(`Wrote ${outPath}`);
console.log(`\nUpload to the ${tag} GitHub release:\n  - ${setupName}\n  - latest.json`);
