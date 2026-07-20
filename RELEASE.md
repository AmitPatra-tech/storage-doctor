# Storage Doctor 1.0.0 — Release Notes

First public release.

## Highlights

- Parallel drive scanner with live progress and scan-over-scan "what changed".
- Storage breakdown with safe/review/system classification and per-folder
  one-click safe cleanup.
- Large file finder (size + type filters) with permanent delete.
- Installed-app inventory with recoverable cache sizes and guided uninstall +
  leftover removal.
- Recommendations engine (caches, temp, logs, crash dumps, Windows Update,
  Recycle Bin, Delivery Optimization, Windows.old, dev caches, and more).
- Duplicate finder (SHA-256) and full-disk search.
- Cleanup journal with PDF export.
- Dodo Payments Pro licensing.

## Cleanup behaviour

- **Caches / temp / logs** are permanently deleted so the space is actually
  reclaimed (they are recreated automatically).
- **User files** (large files, breakdown selections, duplicates, search) —
  large files are permanently deleted; others go to the Recycle Bin.
- The Recycle Bin recommendation uses `Clear-RecycleBin` (never deletes the
  protected `$Recycle.Bin` folder).
- Deletions that need admin rights offer a one-click elevated retry.
- Every deletion is recorded in Reports.

## Manual QA checklist

Run these in `npm run tauri dev` (or the installed build):

- [ ] Scan completes; Dashboard shows drives, top consumers, recoverable.
- [ ] Second scan enables the "What changed" card.
- [ ] Breakdown drills into folders; badges + explanations show; Clean button
      finds safe items and removes them; list refreshes.
- [ ] Large Files filters work; select + permanent delete frees space.
- [ ] Applications lists installed apps; expand shows caches; Clean recovers
      space; Uninstall launches the real uninstaller; leftover scan works.
- [ ] Recommendations show real sizes; Clean permanently removes and the item
      updates/drops; Recycle Bin recommendation empties the bin without hanging.
- [ ] Duplicate finder (Pro) scans chosen folders; deletes extra copies.
- [ ] Search: instant results + full-disk search with progress; multi-select
      delete.
- [ ] Reports lists operations; Export PDF saves a file.
- [ ] Starting a scan/search/duplicate scan and switching pages does NOT stop
      it; results persist on return.
- [ ] Settings: theme switches and persists; default scan drives respected.
- [ ] License: `321-123` unlocks Pro; Deactivate returns to Free.

## Build the installer

The build must be able to sign the updater artifacts, so set the update-signing
key first (this is the minisign key generated at setup, NOT a code-signing cert):

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/storage-doctor.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""   # empty — the key has no password
npm run tauri build
```

Output (with `createUpdaterArtifacts: true`):
- `src-tauri/target/release/bundle/nsis/Storage Doctor_<version>_x64-setup.exe`
- `…_x64-setup.exe.sig`  ← the update signature

## Auto-update (tauri-plugin-updater)

The app checks for updates on launch and from **Settings → About → Check for
updates**. It compares its version against a `latest.json` manifest served from
the endpoint in `src-tauri/tauri.conf.json`:

```
https://github.com/AmitPatra-tech/storage-doctor/releases/latest/download/latest.json
```

> This must stay in sync between `tauri.conf.json` →
> `plugins.updater.endpoints` and `scripts/make-latest-json.mjs` → `REPO`.

**Signing keys** live outside the repo:
- Private (secret, never commit): `~/.tauri/storage-doctor.key`
- Public (embedded in `tauri.conf.json` → `plugins.updater.pubkey`)

If the private key is lost, existing installs can no longer be updated — back it
up somewhere safe (password manager).

## Cutting a new release

1. Bump `version` in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and
   `package.json` (all three must match).
2. Build with the signing env vars set (see above).
3. Generate the manifest from the build output:
   ```sh
   npm run release:manifest -- "What changed in this version"
   ```
   This writes `latest.json` next to the installer.
4. Create a GitHub Release tagged `v<version>` and upload **both** the
   `-setup.exe` and `latest.json` as assets.
5. Existing users get prompted to update on next launch; new users download the
   `-setup.exe` directly.

Running the new installer over an existing install upgrades in place — users do
**not** need to uninstall first, and their data in `%LOCALAPPDATA%` is preserved.

## Publishing (first release)

1. Test the installer on a clean Windows machine (no dev tools).
2. Optionally code-sign the installer (`signCommand` in `tauri.conf.json`) with a
   real code-signing certificate to avoid SmartScreen warnings. (This is separate
   from the updater signing key above.)
3. `dodoMode` is `"live"` in `src/lib/config.ts` for the public build; ensure the
   Dodo product has the License Key entitlement enabled.
4. Distribute the `-setup.exe` (GitHub Releases, website, etc.).
