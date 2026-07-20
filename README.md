# Storage Doctor

**Version 1.0.0 — by HutZon**

A Windows desktop app that explains **why** your storage is filling up — what
is using space, why the files exist, whether they are safe to remove, and how
much can be recovered.

Built with Tauri 2, React, TypeScript, Tailwind CSS 4, and Rust with SQLite.

## Features

- **Drive scan** — fast parallel scan of all local drives with live progress.
- **Storage breakdown** — drill into any folder to any depth; every item is
  labelled with what it is and whether it is safe to delete.
- **Large files** — filter by size and type; permanent delete to reclaim space.
- **Applications** — every installed app with storage, recoverable cache size,
  and a guided uninstall that also finds leftovers.
- **Recommendations** — safe cleanup for caches, temp, logs, dumps, Windows
  Update, Recycle Bin and more, with plain-English risk and consequences.
- **Duplicate finder** (Pro) — content-hash (SHA-256) duplicate detection over
  chosen folders.
- **Search** — instant index search plus a full-disk file search.
- **Reports** (Pro) — a cleanup journal exportable to PDF.
- **What changed** — compares scans to explain why storage grew.

Cleanup is honest about outcomes: caches/temp are permanently removed to
actually free space (they are recreated automatically); user files go to the
Recycle Bin. Every deletion is logged to the Reports journal.

## Prerequisites

- Node.js 18+
- Rust toolchain (`rustup` with the MSVC target) — required for the desktop
  build. Install from https://rustup.rs
- Visual Studio Build Tools with the "Desktop development with C++" workload
  (WebView2 is preinstalled on Windows 11)

## Development

```sh
npm install
npm run dev          # frontend only in a browser (uses mock data)
npm run tauri dev    # full desktop app (requires Rust)
```

The frontend detects whether it is running inside the Tauri webview. In a
plain browser it serves mock data from `src/lib/backend.ts` so UI work does
not require the Rust toolchain.

## Building the installer

```sh
npm run tauri build
```

Produces an NSIS installer under `src-tauri/target/release/bundle/nsis`.

## Project layout

- `src/` — React frontend (pages, components, IPC wrapper in `src/lib/backend.ts`)
- `src-tauri/src/commands.rs` — Tauri commands exposed to the frontend
- `src-tauri/src/db.rs` — SQLite schema and connection (stored in
  `%LOCALAPPDATA%\StorageDoctor`)

## Licensing (Pro)

Pro unlocks the duplicate finder and PDF report export. Payments run through
[Dodo Payments](https://dodopayments.com); license keys are validated directly
against Dodo's public license API (no server or secret key in the client).
Configure the product/mode in `src/lib/config.ts`. The dev master key
`321-123` unlocks Pro offline for testing.

## Milestone status

- [x] M1 — project setup, Tauri config, React UI shell, SQLite integration
- [x] M2 — drive scanning engine, folder analysis, dashboard wiring
- [x] M3 — large file finder, application detection, recommendations engine
- [x] M4 — duplicate detection, cleanup actions, search
- [x] M5 — Dodo Payments integration, license activation, Pro gating
- [x] M6 — QA, performance optimization, packaging, release
