# Reproducible 15-Tab RAM Benchmark (`bench/`)

This directory contains the reproducible Windows x64 multi-process memory benchmark specified in [docs/SPEC.md](../docs/SPEC.md) §12.

## Methodology

- **Whole process-tree accounting:** Every measurement sums **Private Bytes** (`PrivateMemorySize64`) and **Working Set** (`WorkingSet64`) across the **entire process tree** of each browser (browser + GPU + network/utility + every renderer process), never cherry-picking a single PID.
- **Profile isolation:** Chrome and Firefox are launched with fresh temporary profile directories (`--user-data-dir` / `-profile`), so your personal browser session is never touched or counted.
- **Same 15-tab workload:** All browsers load the exact same 15 URLs from [`urls.json`](./urls.json) (GitHub, docs.rs, MDN, Wikipedia, Hacker News, YouTube, crates.io, Godot, SQLite, TypeScript docs, Vite, etc.).
- **Honest reporting:** Both live 15-tab RAM (before sleeping) and post-sleep RAM (after idle background tabs sleep / Game Mode) are reported transparently alongside process counts and hardware specs.

## How to Run

### Quick verification (~90 seconds)
```powershell
node bench/ram-benchmark.mjs --mode quick
```
Measures:
1. **1-tab baseline**
2. **15 live tabs loaded**
3. **Post-sleep** (`Seu Navegador` sleeps eligible background tabs via the exact same lifecycle engine as the 10-minute idle timer / Game Mode, while Chrome/Firefox stay steady).

### Full SPEC §12 timeline (0, 5, and 15 minutes)
```powershell
node bench/ram-benchmark.mjs --mode full
```
Measures each browser at **1-tab baseline**, **t = 0 min**, **t = 5 min**, and **t = 15 min** (allowing Seu Navegador's default 10-minute auto-sleep policy to trigger naturally without manual calls).

### Options
- `--browsers seu,chrome,firefox` — select specific browsers
- `--seu-exe path\to\seu-navegador.exe` — point to a custom build

Results are written to `bench/results/latest.json` and `bench/results/latest.md`.
