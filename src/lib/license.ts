import { dodoApiBase } from "./config";
import type { License } from "./types";

// Offline master/dev key — only available in dev builds (stripped by Vite in production).
const MASTER_KEY = import.meta.env.DEV ? "321-123" : "";

// SHA-256 hashes of manually-issued activation codes. These are sent by hand
// (via email, after the buyer forwards their Dodo invoice) instead of relying
// on Dodo's auto-issued license keys. Only the hash is stored here — the
// plaintext codes never appear in the app, so they can't be read out of the
// built bundle, only verified against.
const MANUAL_CODE_HASHES = new Set([
  "fd89e7a035c74c4e412b78a8670ee64931404b7e3519663975333f7d040fc447",
  "0e019abb16740f338fa4336fa083894912e94cfff97ac79703fdc69a4711b517",
  "b0cab3380940a2b5ed64a42bf42e6e3a9815708db74294bbad4586305537d9a0",
  "e4a546ba2419e2025c3349f8b522b30c31553308f902159280d660f5663eef1e",
  "dfecb859477c4e8aab88a80edece92d43985014cb6294077b0b79b77b1b9e723",
  "ba75e3aed190a146f4641b67d1e82c06f65b3539c79a5f7a4a9810b5e199fb8a",
  "758d906ec72a4da3c89ea3d6d1056a9909ee2da7e0a8e89780553918cec5d408",
  "e49e2385e2b0654bdf0a9a42737e2e9719c288383b677c2f438be56d7e37a4ae",
  "ff2026e843d332a5f83899803055025ee90489b24d1f5aa281467e4689dcf0c8",
  "18a84b79cc44e9e67575e2eb5abb6cf5ea55352fa3cae65897f227f8b9cd735d",
  "62b47b6731b71ea04fb8e4177b5617a158b1a796478e19200c0b51a88f92cb8e",
  "5352bb92caf602a0a6a2f1307b50e5d0133a89de00304b092caff4db6683e816",
  "f2993b60e72f762c50a6c9fe93619f0d597c6ec309f74b59e7b0cc5d76ed3a25",
  "a6a94f1bf7bc277d64f5b6dc7e0155311e38568eded7b074ccb164916ac447e7",
  "a4f02ab6193c5ad040b4574b633a0b57d7bac92dbe89c1526a46c72e23fc2932",
  "04657c6ea581f9c496cf500108f15f3406ebe8642f49b205dcf23d34c6ec62a9",
  "a44041453c75b0c9a055310b5045f80f3ed2226eed288275a16cbae0b6ad7f7a",
  "36f9c9355860cafcecffc088b5018c74f96a8c8fdfc05ec58063dc02fe25abf9",
  "ff01e74e3807b4839609b614eeed81c6be77879c11ee9732f0f9e6329594a0c3",
  "c28b6ba6dee0a0bdc92ce06bb5402f894e8cc432c328d452b4d5be0feb5291a6",
]);

async function sha256Hex(text: string): Promise<string> {
  const bytes = new TextEncoder().encode(text);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

async function isManualCode(key: string): Promise<boolean> {
  return MANUAL_CODE_HASHES.has(await sha256Hex(key));
}

/** Calls Dodo's public license validate endpoint. Returns whether the key is
 *  currently valid (active, not expired, not revoked). */
async function validateWithDodo(licenseKey: string): Promise<boolean> {
  const res = await fetch(`${dodoApiBase}/licenses/validate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ license_key: licenseKey }),
  });
  if (!res.ok) {
    // 4xx typically means the key does not exist / is not valid.
    if (res.status >= 400 && res.status < 500) return false;
    throw new Error(`License server returned ${res.status}.`);
  }
  const data = await res.json();
  return data.valid === true;
}

/** Validates a license key against Dodo and returns a License on success.
 *  In the browser (no Tauri) the demo key still works for previewing Pro. */
export async function verifyLicenseKey(key: string): Promise<License> {
  const trimmed = key.trim();
  if (!trimmed) throw new Error("Enter a license key.");

  if (trimmed === MASTER_KEY || (await isManualCode(trimmed))) {
    return { key: trimmed, plan: "pro", activatedAt: new Date().toISOString(), expiresAt: null };
  }

  let valid: boolean;
  try {
    valid = await validateWithDodo(trimmed);
  } catch {
    throw new Error("Could not reach the license server. Check your connection.");
  }
  if (!valid) throw new Error("This license key is not valid.");

  return { key: trimmed, plan: "pro", activatedAt: new Date().toISOString(), expiresAt: null };
}

/** Re-checks a stored key with Dodo. Returns:
 *  - true  → still valid
 *  - false → definitively invalid (revoked / expired)
 *  - null  → could not verify (offline) — caller should keep a grace period. */
export async function revalidateLicense(license: License): Promise<boolean | null> {
  if (license.key === MASTER_KEY || (await isManualCode(license.key))) return true;
  try {
    return await validateWithDodo(license.key);
  } catch {
    return null;
  }
}

export function isLicenseActive(license: License | null): boolean {
  if (!license) return false;
  if (license.expiresAt && new Date(license.expiresAt) < new Date()) return false;
  return true;
}
