# RAM Benchmark Results (quick mode)

- **Date:** 2026-11-27T01:28:54.789Z
- **Host:** Windows_NT 10.0.26300 (x64) · AMD Ryzen 5 40 with Radeon Graphics             (8 threads) · 15.3 GB RAM
- **Workload:** 15 real tabs (`bench/urls.json`)
- **Metric:** Total **Private Memory (MB)** across the entire OS process tree (browser + GPU + network/utility + all renderers). Working Set shown in parentheses.

| Browser | 1 Tab Baseline | 15 Tabs Loaded | Post-Sleep / Steady | Processes (End) |
|---|---:|---:|---:|---:|
| Seu Navegador | **179.8 MB** (529.9 WS) | **1240.3 MB** (2649 WS) | **704.4 MB** (1347.3 WS) | 13 |
| Chrome | **468.2 MB** (832.6 WS) | **1489.6 MB** (2597.3 WS) | **1466.7 MB** (2540.9 WS) | 34 |

> Skipped (not installed on this machine): firefox
