# ADR 002 — Chromium / CEF Memory Flags and Multi-Process RAM Measurement

- **Status:** Accepted (Phase 3)
- **Date:** 2026-11-24

## Context
Seu Navegador aims for real, measurable RAM savings over Chrome/Firefox on Windows x64 — especially on multi-tab workloads (10–20+ tabs) — without breaking web compatibility (YouTube, SPAs, WebGL, OAuth, etc.).

## Decision

### 1. Enabled Chromium / CEF Command-Line Flags
Applied in `SeuCefApp::on_before_command_line_processing`:
- `--disable-background-networking`: eliminates Chromium background prefetch/update/safe-browsing network workers we do not use.
- `--disable-component-update`: disables background component updater threads/downloads.
- `--disable-domain-reliability`: disables telemetry reporting buffers.
- `--disable-sync`: disables Chromium sync engine allocations.
- `--disable-breakpad` + `--metrics-recording-only`: avoids crashpad/UMA telemetry overhead.
- `--disable-features=BackForwardCache`: prevents Chromium from pinning entire closed DOM + V8 JS heaps in RAM across navigations; our `core` preserves the tab Back/Forward stack cleanly even across tab sleep/wake.

### 2. Flags Deliberately NOT Enabled (and Why)
- `--renderer-process-limit=1` / `--single-process`: NOT forced by default. Collapsing all tabs into 1 renderer weakens Site Isolation security and causes a single heavy tab crash to kill every tab. Furthermore, shared renderer processes prevent OS Private Bytes from dropping when one tab sleeps if another live tab remains in that renderer.
- `--disable-gpu`: NOT enabled. Disabling GPU forces software rasterization, degrading YouTube 1080p/4K video playback and increasing CPU/RAM pressure on video buffers.

### 3. Honest Multi-Process RAM Measurement & Shared-Renderer Accounting
- `crates/engine/src/metrics.rs` enumerates the **entire OS process tree** rooted at the main `seu-navegador.exe` PID via Win32 `CreateToolhelp32Snapshot` + `GetProcessMemoryInfo` (`PROCESS_MEMORY_COUNTERS_EX`), summing `PrivateUsage` (Private Bytes) and `WorkingSetSize` across Browser + GPU + Utility/Network + every Renderer process.
- When a tab sleeps, Seu Navegador captures a compact `240x150` JPEG thumbnail directly to disk (`%LOCALAPPDATA%\SeuNavegador\thumbs\`, 0 bytes retained in RAM), closes the tab's CEF browser, and measures the real OS `PrivateUsage` delta ~1.5s later.
- If the OS process count does not drop and `delta < 5 MB`, `shared_renderer_detected` is flagged honestly in the Memory Popover rather than inflating fake "saved MB" numbers.
