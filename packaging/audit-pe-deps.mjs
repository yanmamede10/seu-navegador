#!/usr/bin/env node
/**
 * PE Import Table Auditor for Windows x64 Distribution (SPEC §1, §3, §12)
 *
 * Verifies that `seu-navegador.exe` and every bundled DLL inside the distribution
 * directory have ZERO unbundled Microsoft runtime dependencies (no VCRUNTIME140.dll,
 * no MSVCP140.dll, no WebView2Loader.dll, no .NET runtime).
 *
 * Usage:
 *   node packaging/audit-pe-deps.mjs dist/seu-navegador-win64
 */

import fs from "node:fs";
import path from "node:path";

// Built-in Windows 10/11 system DLLs guaranteed to exist on every clean Windows x64 install.
const WINDOWS_OS_DLLS = new Set([
  "advapi32.dll",
  "bcrypt.dll",
  "bcryptprimitives.dll",
  "cfgmgr32.dll",
  "comctl32.dll",
  "comdlg32.dll",
  "crypt32.dll",
  "d3d11.dll",
  "d3d9.dll",
  "dbghelp.dll",
  "dhcpcsvc.dll",
  "dnsapi.dll",
  "dwmapi.dll",
  "dwrite.dll",
  "dxgi.dll",
  "gdi32.dll",
  "hid.dll",
  "imm32.dll",
  "iphlpapi.dll",
  "kernel32.dll",
  "kernelbase.dll",
  "msimg32.dll",
  "ncrypt.dll",
  "netapi32.dll",
  "ntdll.dll",
  "ole32.dll",
  "oleacc.dll",
  "oleaut32.dll",
  "powrprof.dll",
  "propsys.dll",
  "psapi.dll",
  "rpcrt4.dll",
  "secur32.dll",
  "setupapi.dll",
  "shcore.dll",
  "shell32.dll",
  "shlwapi.dll",
  "ucrtbase.dll",
  "uiautomationcore.dll",
  "urlmon.dll",
  "user32.dll",
  "userenv.dll",
  "usp10.dll",
  "uxtheme.dll",
  "version.dll",
  "wevtapi.dll",
  "winhttp.dll",
  "wininet.dll",
  "winmm.dll",
  "winspool.drv",
  "wintrust.dll",
  "wtsapi32.dll",
  "ws2_32.dll",
]);

// Explicitly forbidden external Microsoft runtimes (SPEC §1 Anti-goals).
const FORBIDDEN_PREFIXES = [
  "vcruntime",
  "msvcp",
  "msvcr",
  "mfc",
  "vcomp",
  "concrt",
  "webview2",
  "mscoree",
  "clr",
];

function isWindowsOsDll(lowerName) {
  if (WINDOWS_OS_DLLS.has(lowerName)) return true;
  if (lowerName.startsWith("api-ms-win-")) return true;
  if (lowerName.startsWith("ext-ms-win-")) return true;
  return false;
}

function readPeImports(filePath) {
  const buf = fs.readFileSync(filePath);
  if (buf.length < 0x40 || buf.readUInt16LE(0) !== 0x5a4d) {
    throw new Error(`Not a valid PE binary: ${filePath}`);
  }
  const peOff = buf.readUInt32LE(0x3c);
  if (buf.readUInt32LE(peOff) !== 0x00004550) {
    throw new Error(`Invalid PE signature: ${filePath}`);
  }
  const numSections = buf.readUInt16LE(peOff + 6);
  const optHeaderSize = buf.readUInt16LE(peOff + 20);
  const optOff = peOff + 24;
  const magic = buf.readUInt16LE(optOff);
  const dataDirOff = optOff + (magic === 0x20b ? 112 : 96);
  const importRva = buf.readUInt32LE(dataDirOff + 8);
  if (importRva === 0) return [];

  const sections = [];
  const secOff = optOff + optHeaderSize;
  for (let i = 0; i < numSections; i++) {
    const o = secOff + i * 40;
    sections.push({
      va: buf.readUInt32LE(o + 12),
      vs: buf.readUInt32LE(o + 8),
      raw: buf.readUInt32LE(o + 20),
      rs: buf.readUInt32LE(o + 16),
    });
  }

  const rvaToOff = (rva) => {
    for (const s of sections) {
      const span = Math.max(s.vs, s.rs);
      if (rva >= s.va && rva < s.va + span) {
        return s.raw + (rva - s.va);
      }
    }
    return -1;
  };

  const imports = [];
  let descOff = rvaToOff(importRva);
  while (descOff >= 0 && descOff + 20 <= buf.length) {
    const nameRva = buf.readUInt32LE(descOff + 12);
    if (nameRva === 0) break;
    const strOff = rvaToOff(nameRva);
    if (strOff < 0) break;
    let end = strOff;
    while (end < buf.length && buf[end] !== 0) end++;
    imports.push(buf.toString("ascii", strOff, end));
    descOff += 20;
  }
  return imports;
}

function main() {
  const distDir = path.resolve(process.argv[2] || "dist/seu-navegador-win64");
  if (!fs.existsSync(distDir)) {
    console.error(`Distribution directory not found: ${distDir}`);
    process.exit(1);
  }

  const entries = fs.readdirSync(distDir);
  const bundledFiles = new Set(entries.map((f) => f.toLowerCase()));
  const peTargets = entries.filter((f) => {
    const low = f.toLowerCase();
    return low.endsWith(".exe") || low.endsWith(".dll");
  });

  if (!bundledFiles.has("seu-navegador.exe")) {
    console.error(`Missing seu-navegador.exe in ${distDir}`);
    process.exit(1);
  }

  const violations = [];
  for (const file of peTargets) {
    const full = path.join(distDir, file);
    const imports = readPeImports(full);
    for (const imp of imports) {
      const low = imp.toLowerCase();
      if (FORBIDDEN_PREFIXES.some((p) => low.startsWith(p))) {
        violations.push(`${file} imports forbidden runtime DLL: ${imp}`);
        continue;
      }
      if (!isWindowsOsDll(low) && !bundledFiles.has(low)) {
        violations.push(`${file} imports missing non-OS DLL: ${imp}`);
      }
    }
  }

  const exeImports = readPeImports(path.join(distDir, "seu-navegador.exe"));
  console.log(`Audited ${peTargets.length} PE binaries in ${distDir}`);
  console.log(`seu-navegador.exe imports (${exeImports.length}): ${exeImports.join(", ")}`);

  if (violations.length > 0) {
    console.error("\nPE DEPENDENCY AUDIT FAILED:");
    for (const v of violations) console.error(`  - ${v}`);
    process.exit(1);
  }

  console.log("\nPE DEPENDENCY AUDIT PASSED: 0 unbundled Microsoft runtime dependencies!");
}

main();
