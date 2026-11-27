#!/usr/bin/env node
/**
 * Reproducible 15-Tab Multi-Process RAM Benchmark (SPEC §12)
 * Compares Seu Navegador vs. Google Chrome vs. Mozilla Firefox on Windows x64.
 *
 * Usage:
 *   node bench/ram-benchmark.mjs --mode quick           # Fast verification (baseline, 15 tabs, post-sleep)
 *   node bench/ram-benchmark.mjs --mode full            # Exact SPEC §12 timeline (0, 5, 15 min)
 *   node bench/ram-benchmark.mjs --browsers seu,chrome  # Limit browsers tested
 *
 * Isolation guarantee:
 *   Chrome and Firefox are launched with fresh temporary user-data profiles so existing
 *   user sessions are never modified or terminated, and ONLY the spawned PID tree is measured.
 */

import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const URLS = JSON.parse(fs.readFileSync(path.join(__dirname, "urls.json"), "utf8")).urls;
const CDP_PORT = 9222;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function parseArgs() {
  const args = process.argv.slice(2);
  let mode = "quick";
  let browsers = ["seu", "chrome", "firefox"];
  let seuExe = path.join(ROOT, "target", "phase1-dist", "app", "seu-navegador.exe");
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--mode" && args[i + 1]) mode = args[++i];
    else if (args[i] === "--browsers" && args[i + 1])
      browsers = args[++i].split(",").map((s) => s.trim().toLowerCase());
    else if (args[i] === "--seu-exe" && args[i + 1]) seuExe = path.resolve(args[++i]);
  }
  return { mode, browsers, seuExe };
}

function findFirstExisting(paths) {
  return paths.find((p) => p && fs.existsSync(p)) || null;
}

function detectBrowsers(seuExe) {
  const localAppData = process.env.LOCALAPPDATA || "";
  return {
    seu: findFirstExisting([seuExe]),
    chrome: findFirstExisting([
      "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
      "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
      path.join(localAppData, "Google", "Chrome", "Application", "chrome.exe"),
    ]),
    firefox: findFirstExisting([
      "C:\\Program Files\\Mozilla Firefox\\firefox.exe",
      "C:\\Program Files (x86)\\Mozilla Firefox\\firefox.exe",
    ]),
  };
}

/**
 * Measures Private Memory (MB), Working Set (MB), and Process Count for the ENTIRE
 * process tree rooted at `rootPid` (plus any processes matching `extraMatch` like
 * an isolated profile path, in case a browser launcher spawns a detached broker).
 */
function measureProcessTree(rootPid, profileMarker = "") {
  const psScript = `
    $root = ${Number(rootPid)};
    $marker = ${JSON.stringify(profileMarker)};
    $procs = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, CommandLine;
    $owned = New-Object 'System.Collections.Generic.HashSet[int]';
    [void]$owned.Add($root);
    if ($marker -ne '') {
      foreach ($p in $procs) {
        if ($p.CommandLine -and $p.CommandLine.IndexOf($marker, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
          [void]$owned.Add([int]$p.ProcessId);
        }
      }
    }
    do {
      $added = $false;
      foreach ($p in $procs) {
        if ($owned.Contains([int]$p.ParentProcessId) -and $owned.Add([int]$p.ProcessId)) {
          $added = $true;
        }
      }
    } while ($added);
    $live = Get-Process -ErrorAction SilentlyContinue | Where-Object { $owned.Contains($_.Id) };
    $priv = 0; $ws = 0; $cnt = 0; $pids = @();
    foreach ($l in $live) {
      $priv += [int64]$l.PrivateMemorySize64;
      $ws += [int64]$l.WorkingSet64;
      $cnt += 1;
      $pids += $l.Id;
    }
    @{
      private_mb = [math]::Round($priv / 1MB, 1);
      working_set_mb = [math]::Round($ws / 1MB, 1);
      process_count = $cnt;
      pids = $pids
    } | ConvertTo-Json -Compress
  `;
  const out = execFileSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", psScript],
    { encoding: "utf8" },
  ).trim();
  return JSON.parse(out);
}

function killProcessTree(rootPid, profileMarker = "") {
  try {
    const snap = measureProcessTree(rootPid, profileMarker);
    const pids = Array.isArray(snap.pids) ? snap.pids : snap.pids ? [snap.pids] : [];
    for (const pid of pids) {
      try {
        process.kill(pid, "SIGKILL");
      } catch {}
    }
  } catch {}
}

async function waitForCdpShell(timeoutMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
      const shell = list.find((t) => t.url.startsWith("seu://shell/"));
      if (shell) return shell;
    } catch {}
    await sleep(250);
  }
  throw new Error("Timed out waiting for Seu Navegador CDP shell");
}

function connectCdp(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let nextId = 1;
    const pending = new Map();
    ws.onopen = () => {
      const send = (method, params = {}) =>
        new Promise((res, rej) => {
          const id = nextId++;
          pending.set(id, { res, rej });
          ws.send(JSON.stringify({ id, method, params }));
        });
      resolve({
        send,
        bridge: async (method, params) => {
          const payload = params === undefined ? { method } : { method, params };
          const r = await send("Runtime.evaluate", {
            expression: `new Promise((ok, err) => window.cefQuery({
              request: ${JSON.stringify(JSON.stringify(payload))},
              persistent: false,
              onSuccess: ok,
              onFailure: (c, m) => err(new Error(c + ': ' + m))
            }))`,
            awaitPromise: true,
            returnByValue: true,
          });
          const parsed = JSON.parse(r.result.value);
          return "data" in parsed ? parsed.data : parsed;
        },
        close: () => ws.close(),
      });
    };
    ws.onmessage = (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && pending.has(msg.id)) {
        const { res, rej } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) rej(new Error(msg.error.message));
        else res(msg.result);
      }
    };
    ws.onerror = reject;
  });
}

async function benchmarkSeu(exePath, mode) {
  console.log(`\n[Seu Navegador] Launching ${exePath} (mode=${mode})...`);
  const child = spawn(exePath, [`--remote-debugging-port=${CDP_PORT}`], {
    cwd: path.dirname(exePath),
    stdio: "ignore",
  });

  try {
    const shellTarget = await waitForCdpShell();
    const cdp = await connectCdp(shellTarget.webSocketDebuggerUrl);
    await sleep(2000);

    // Ensure clean 1-tab baseline (keep only 1 newtab).
    const initSnap = await cdp.bridge("windows.getSnapshot", { window_id: 1 });
    const nt = await cdp.bridge("tabs.create", {
      window_id: 1,
      url: "seu://newtab/",
      activate: true,
    });
    for (const t of initSnap.tabs || []) {
      if (t.id !== nt.tab_id) {
        await cdp.bridge("tabs.close", { tab_id: t.id }).catch(() => {});
      }
    }
    await sleep(2000);

    const baseline = measureProcessTree(child.pid);
    console.log(
      `  baseline (1 tab): ${baseline.private_mb} MB private / ${baseline.working_set_mb} MB WS (${baseline.process_count} procs)`,
    );

    // Open all 15 URLs
    const openedIds = [];
    for (let i = 0; i < URLS.length; i++) {
      const created = await cdp.bridge("tabs.create", {
        window_id: 1,
        url: URLS[i],
        activate: i === URLS.length - 1,
      });
      openedIds.push(created.tab_id);
      await sleep(350);
    }
    await cdp.bridge("tabs.close", { tab_id: nt.tab_id }).catch(() => {});

    // Wait for pages to finish initial load & settle
    console.log("  waiting 15s for 15 tabs to settle...");
    await sleep(15000);
    const t0 = measureProcessTree(child.pid);
    console.log(
      `  t=0m (15 tabs live): ${t0.private_mb} MB private / ${t0.working_set_mb} MB WS (${t0.process_count} procs)`,
    );

    let t5 = null;
    let t15 = null;
    if (mode === "full") {
      console.log("  [full mode] waiting 5 min for t=5m sample...");
      await sleep(5 * 60 * 1000);
      t5 = measureProcessTree(child.pid);
      console.log(
        `  t=5m: ${t5.private_mb} MB private / ${t5.working_set_mb} MB WS (${t5.process_count} procs)`,
      );

      console.log("  [full mode] waiting 10 more min for t=15m sample (post auto-sleep)...");
      await sleep(10 * 60 * 1000);
      t15 = measureProcessTree(child.pid);
      console.log(
        `  t=15m: ${t15.private_mb} MB private / ${t15.working_set_mb} MB WS (${t15.process_count} procs)`,
      );
    } else {
      // Quick mode: trigger Seu Navegador's real sleepInactive (same path as 10m idle timeout / Game Mode)
      await cdp.bridge("memory.sleepInactive");
      console.log("  waiting 6s after sleeping inactive tabs...");
      await sleep(6000);
      const afterSleep = measureProcessTree(child.pid);
      const memStats = await cdp.bridge("memory.getStats");
      console.log(
        `  post-sleep (${memStats.sleeping_tabs_count} sleeping, ${memStats.live_tabs_count} live): ${afterSleep.private_mb} MB private / ${afterSleep.working_set_mb} MB WS (${afterSleep.process_count} procs)`,
      );
      t5 = afterSleep;
    }

    // Restore clean single newtab before closing so user's saved session stays neat
    const finalNt = await cdp.bridge("tabs.create", {
      window_id: 1,
      url: "seu://newtab/",
      activate: true,
    });
    for (const tid of openedIds) {
      await cdp.bridge("tabs.close", { tab_id: tid }).catch(() => {});
    }
    await sleep(600);
    cdp.close();
    return {
      browser: "Seu Navegador",
      baseline_1_tab: baseline,
      t0_15_tabs: t0,
      t5_or_post_sleep: t5,
      t15_full: t15,
    };
  } finally {
    killProcessTree(child.pid);
    await sleep(800);
  }
}

async function benchmarkExternalBrowser(name, exePath, mode) {
  const tmpProfile = fs.mkdtempSync(path.join(os.tmpdir(), `seu-bench-${name.toLowerCase()}-`));
  console.log(`\n[${name}] Launching isolated profile ${tmpProfile} (mode=${mode})...`);

  const baseArgs =
    name === "Chrome"
      ? [
          `--user-data-dir=${tmpProfile}`,
          "--no-first-run",
          "--no-default-browser-check",
          "--disable-sync",
          "--window-size=1280,800",
        ]
      : ["-no-remote", "-profile", tmpProfile];

  // Phase A: 1-tab baseline
  const baseProc = spawn(exePath, [...baseArgs, "about:blank"], { stdio: "ignore" });
  await sleep(6000);
  const baseline = measureProcessTree(baseProc.pid, tmpProfile);
  console.log(
    `  baseline (1 tab): ${baseline.private_mb} MB private / ${baseline.working_set_mb} MB WS (${baseline.process_count} procs)`,
  );
  killProcessTree(baseProc.pid, tmpProfile);
  await sleep(1200);

  // Phase B: 15 tabs loaded simultaneously
  const proc = spawn(exePath, [...baseArgs, ...URLS], { stdio: "ignore" });
  try {
    console.log("  waiting 20s for 15 tabs to settle...");
    await sleep(20000);
    const t0 = measureProcessTree(proc.pid, tmpProfile);
    console.log(
      `  t=0m (15 tabs live): ${t0.private_mb} MB private / ${t0.working_set_mb} MB WS (${t0.process_count} procs)`,
    );

    let t5 = null;
    let t15 = null;
    if (mode === "full") {
      console.log("  [full mode] waiting 5 min for t=5m sample...");
      await sleep(5 * 60 * 1000);
      t5 = measureProcessTree(proc.pid, tmpProfile);
      console.log(
        `  t=5m: ${t5.private_mb} MB private / ${t5.working_set_mb} MB WS (${t5.process_count} procs)`,
      );

      console.log("  [full mode] waiting 10 more min for t=15m sample...");
      await sleep(10 * 60 * 1000);
      t15 = measureProcessTree(proc.pid, tmpProfile);
      console.log(
        `  t=15m: ${t15.private_mb} MB private / ${t15.working_set_mb} MB WS (${t15.process_count} procs)`,
      );
    } else {
      await sleep(6000);
      t5 = measureProcessTree(proc.pid, tmpProfile);
      console.log(
        `  steady (+6s): ${t5.private_mb} MB private / ${t5.working_set_mb} MB WS (${t5.process_count} procs)`,
      );
    }

    return {
      browser: name,
      baseline_1_tab: baseline,
      t0_15_tabs: t0,
      t5_or_post_sleep: t5,
      t15_full: t15,
    };
  } finally {
    killProcessTree(proc.pid, tmpProfile);
    await sleep(1000);
    try {
      fs.rmSync(tmpProfile, { recursive: true, force: true });
    } catch {}
  }
}

function renderMarkdown(report) {
  const isFull = report.mode === "full";
  const lines = [
    `# RAM Benchmark Results (${report.mode} mode)`,
    "",
    `- **Date:** ${report.timestamp}`,
    `- **Host:** ${report.host.os} · ${report.host.cpu} (${report.host.cores} threads) · ${report.host.total_ram_gb} GB RAM`,
    `- **Workload:** ${report.url_count} real tabs (\`bench/urls.json\`)`,
    `- **Metric:** Total **Private Memory (MB)** across the entire OS process tree (browser + GPU + network/utility + all renderers). Working Set shown in parentheses.`,
    "",
    isFull
      ? "| Browser | 1 Tab Baseline | 15 Tabs (0 min) | 15 Tabs (5 min) | 15 Tabs (15 min) |"
      : "| Browser | 1 Tab Baseline | 15 Tabs Loaded | Post-Sleep / Steady | Processes (End) |",
    isFull
      ? "|---|---:|---:|---:|---:|"
      : "|---|---:|---:|---:|---:|",
  ];

  for (const r of report.results) {
    const fmt = (m) => (m ? `**${m.private_mb} MB** (${m.working_set_mb} WS)` : "—");
    if (isFull) {
      lines.push(
        `| ${r.browser} | ${fmt(r.baseline_1_tab)} | ${fmt(r.t0_15_tabs)} | ${fmt(r.t5_or_post_sleep)} | ${fmt(r.t15_full)} |`,
      );
    } else {
      lines.push(
        `| ${r.browser} | ${fmt(r.baseline_1_tab)} | ${fmt(r.t0_15_tabs)} | ${fmt(r.t5_or_post_sleep)} | ${r.t5_or_post_sleep?.process_count ?? "—"} |`,
      );
    }
  }

  if (report.skipped.length > 0) {
    lines.push("", `> Skipped (not installed on this machine): ${report.skipped.join(", ")}`);
  }
  lines.push("");
  return lines.join("\n");
}

async function main() {
  const { mode, browsers, seuExe } = parseArgs();
  const detected = detectBrowsers(seuExe);
  const results = [];
  const skipped = [];

  for (const id of browsers) {
    const exe = detected[id];
    if (!exe) {
      console.log(`[skip] ${id} not found on this machine`);
      skipped.push(id);
      continue;
    }
    if (id === "seu") {
      results.push(await benchmarkSeu(exe, mode));
    } else if (id === "chrome") {
      results.push(await benchmarkExternalBrowser("Chrome", exe, mode));
    } else if (id === "firefox") {
      results.push(await benchmarkExternalBrowser("Firefox", exe, mode));
    }
  }

  const report = {
    timestamp: new Date().toISOString(),
    mode,
    url_count: URLS.length,
    host: {
      os: `${os.type()} ${os.release()} (${os.arch()})`,
      cpu: os.cpus()[0]?.model || "unknown",
      cores: os.cpus().length,
      total_ram_gb: Math.round((os.totalmem() / 1024 ** 3) * 10) / 10,
    },
    results,
    skipped,
  };

  const outDir = path.join(__dirname, "results");
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(path.join(outDir, "latest.json"), JSON.stringify(report, null, 2));
  const md = renderMarkdown(report);
  fs.writeFileSync(path.join(outDir, "latest.md"), md);

  console.log("\n" + md);
}

main().catch((err) => {
  console.error("BENCHMARK FAILED:", err);
  process.exit(1);
});
