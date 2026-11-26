import "./icons";
import "./theme-editor.css";
import {
  callBridge,
  getTabIdFromQuery,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type ImportedFontResult,
  type LoadedTheme,
  type SemanticTokens,
  type ThemeExportPayload,
  type ThemeSummary,
  type ThemeTokens,
  type WindowSnapshot,
} from "./bridge";

const root = document.getElementById("editor-root") as HTMLDivElement;
const windowId = getWindowIdFromQuery();
const selfTabId = getTabIdFromQuery();
const urlParams = new URLSearchParams(window.location.search);

let locale: "pt-br" | "en" = "pt-br";
let allThemes: ThemeSummary[] = [];
let userDirPath = "";
let statusMsg: { text: string; isError: boolean } | null = null;

let draftId = "meu-tema";
let draftName = "Meu Tema";
let draftAuthor = "Yan";
let draftDescription = "Interface personalizada criada no Editor Visual.";
let baseThemeId = urlParams.get("edit") || urlParams.get("base") || "y2k-maximal";
let draftTokens: ThemeTokens = {
  semantic: {
    bg: "#070B24",
    surface: "#0B1240",
    surface2: "#0E1650",
    text: "#E6E9FF",
    text_muted: "#8F9BD9",
    accent: "#FF2E93",
    accent_text: "#070B24",
    border: "#2A3680",
    danger: "#FF3B5C",
    incognito_bg: "#15072E",
    incognito_accent: "#8A7CFF",
    font_display: "system-ui, sans-serif",
    font_body: "system-ui, sans-serif",
    font_mono: "Consolas, monospace",
    radius: "14px",
    shadow: "6px 6px 0 #FF2E93",
  },
  custom: {},
};
let extraShellCss = "";
let extraNewtabCss = "";
let importedFonts: ImportedFontResult[] = [];

function l(pt: string, en: string): string {
  return locale === "en" ? en : pt;
}

function slugify(raw: string): string {
  return raw
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 44);
}

function applyPageThemeTokens(theme: LoadedTheme): void {
  let styleEl = document.getElementById("seu-tokens-css") as HTMLStyleElement | null;
  if (!styleEl) {
    styleEl = document.createElement("style");
    styleEl.id = "seu-tokens-css";
    document.head.appendChild(styleEl);
  }
  styleEl.textContent = theme.tokens_css;
}

// Local WCAG 2.1 contrast calculation for instant 60fps feedback while dragging color pickers
function parseHex(raw: string): [number, number, number] | null {
  const h = raw.trim().replace(/^#/, "");
  if (h.length === 3) {
    const r = parseInt(h[0] + h[0], 16);
    const g = parseInt(h[1] + h[1], 16);
    const b = parseInt(h[2] + h[2], 16);
    return [r, g, b].some(Number.isNaN) ? null : [r, g, b];
  }
  if (h.length === 6) {
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    return [r, g, b].some(Number.isNaN) ? null : [r, g, b];
  }
  return null;
}

function relLum([r, g, b]: [number, number, number]): number {
  const lin = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrastRatio(fg: string, bg: string): number {
  const a = parseHex(fg);
  const b = parseHex(bg);
  if (!a || !b) return 1;
  const l1 = relLum(a);
  const l2 = relLum(b);
  const hi = Math.max(l1, l2);
  const lo = Math.min(l1, l2);
  return Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100;
}

function normalizeColorForPicker(val: string): string {
  const p = parseHex(val);
  if (!p) return "#000000";
  return `#${p.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

async function navigateSelf(url: string): Promise<void> {
  if (selfTabId !== null) {
    await callBridge("tabs.navigate", { tab_id: selfTabId, input: url });
    return;
  }
  const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
    window_id: windowId,
  }).catch(() => undefined);
  if (snap?.active_tab_id) {
    await callBridge("tabs.navigate", { tab_id: snap.active_tab_id, input: url });
  }
}

async function loadThemeIntoDraft(id: string, isEditingExistingUser: boolean): Promise<void> {
  const loaded = await callBridge<LoadedTheme>("themes.getById", { id }).catch(() => undefined);
  if (!loaded) return;
  baseThemeId = loaded.manifest.base || loaded.manifest.id;
  draftTokens = {
    semantic: { ...loaded.tokens.semantic },
    custom: { ...(loaded.tokens.custom ?? {}) },
  };
  if (isEditingExistingUser && !loaded.builtin) {
    draftId = loaded.manifest.id;
    draftName = loaded.manifest.name;
    draftAuthor = loaded.manifest.author;
    draftDescription = loaded.manifest.description;
  } else {
    draftId = `${loaded.manifest.id}-custom`;
    draftName = `${loaded.manifest.name} Custom`;
    draftAuthor = "Yan";
    draftDescription = l(
      `Tema copy-on-write baseado em ${loaded.manifest.name}.`,
      `Copy-on-write theme based on ${loaded.manifest.name}.`,
    );
  }
  render();
}

async function saveDraft(activate: boolean): Promise<LoadedTheme | null> {
  const cleanId = slugify(draftId || draftName);
  if (cleanId.length < 2) {
    statusMsg = {
      text: l("Informe um ID válido (mínimo 2 letras minúsculas/hífen).", "Enter a valid slug ID."),
      isError: true,
    };
    render();
    return null;
  }
  draftId = cleanId;
  try {
    const saved = await callBridge<LoadedTheme>("themes.save", {
      id: cleanId,
      name: draftName.trim() || cleanId,
      author: draftAuthor.trim() || "User",
      description: draftDescription.trim(),
      base_id: baseThemeId,
      tokens: draftTokens,
      extra_shell_css: extraShellCss.trim() ? extraShellCss : null,
      extra_newtab_css: extraNewtabCss.trim() ? extraNewtabCss : null,
      activate,
    });
    if (saved) {
      if (activate) applyPageThemeTokens(saved);
      const list = await callBridge<ThemeSummary[]>("themes.list").catch(() => allThemes);
      if (Array.isArray(list)) allThemes = list;
      statusMsg = {
        text: activate
          ? l(
              `Tema "${saved.manifest.name}" salvo em disco e aplicado ao vivo!`,
              `Theme "${saved.manifest.name}" saved to disk and applied live!`,
            )
          : l(`Tema "${saved.manifest.name}" salvo.`, `Theme "${saved.manifest.name}" saved.`),
        isError: false,
      };
      render();
      return saved;
    }
  } catch (err) {
    statusMsg = { text: err instanceof Error ? err.message : String(err), isError: true };
    render();
  }
  return null;
}

async function exportCurrentTheme(): Promise<void> {
  const saved = await saveDraft(false);
  if (!saved) return;
  try {
    const exp = await callBridge<ThemeExportPayload>("themes.export", { id: saved.manifest.id });
    if (!exp) return;
    const bin = atob(exp.base64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const blob = new Blob([bytes], { type: "application/zip" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = exp.filename;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1500);
    statusMsg = {
      text: l(
        `Pacote ${exp.filename} exportado (${Math.round(exp.size_bytes / 1024)} KB)!`,
        `Exported ${exp.filename} (${Math.round(exp.size_bytes / 1024)} KB)!`,
      ),
      isError: false,
    };
    render();
  } catch (err) {
    statusMsg = { text: err instanceof Error ? err.message : String(err), isError: true };
    render();
  }
}

async function handleFontFile(file: File): Promise<void> {
  const cleanId = slugify(draftId || draftName);
  if (cleanId.length < 2) {
    statusMsg = { text: l("Defina um ID de tema antes de importar fonte.", "Set a theme ID first."), isError: true };
    render();
    return;
  }
  const saved = await saveDraft(false);
  if (!saved) return;
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  const base64 = btoa(binary);
  try {
    const res = await callBridge<ImportedFontResult>("themes.importFont", {
      theme_id: saved.manifest.id,
      filename: file.name,
      base64,
    });
    if (res) {
      importedFonts = [res, ...importedFonts.filter((f) => f.filename !== res.filename)];
      statusMsg = {
        text: l(
          `Fonte "${res.family}" importada em assets/fonts/${res.filename}!`,
          `Font "${res.family}" imported to assets/fonts/${res.filename}!`,
        ),
        isError: false,
      };
      render();
    }
  } catch (err) {
    statusMsg = { text: err instanceof Error ? err.message : String(err), isError: true };
    render();
  }
}

function updateLivePreviewVars(): void {
  const stage = root.querySelector(".te-live-stage") as HTMLElement | null;
  if (!stage) return;
  const s = draftTokens.semantic;
  stage.style.setProperty("--p-bg", s.bg);
  stage.style.setProperty("--p-surface", s.surface);
  stage.style.setProperty("--p-surface2", s.surface2);
  stage.style.setProperty("--p-text", s.text);
  stage.style.setProperty("--p-text-muted", s.text_muted);
  stage.style.setProperty("--p-accent", s.accent);
  stage.style.setProperty("--p-accent-text", s.accent_text);
  stage.style.setProperty("--p-border", s.border);
  stage.style.setProperty("--p-danger", s.danger);
  stage.style.setProperty("--p-incog-bg", s.incognito_bg);
  stage.style.setProperty("--p-incog-accent", s.incognito_accent);
  stage.style.setProperty("--p-font-display", s.font_display);
  stage.style.setProperty("--p-font-body", s.font_body);
  stage.style.setProperty("--p-font-mono", s.font_mono);
  stage.style.setProperty("--p-radius", s.radius);
  stage.style.setProperty("--p-shadow", s.shadow);

  const wcagHost = root.querySelector(".te-wcag-list");
  if (wcagHost) {
    renderWcagChecksInto(wcagHost as HTMLElement);
  }
}

function renderWcagChecksInto(host: HTMLElement): void {
  host.innerHTML = "";
  const s = draftTokens.semantic;
  const pairs: [string, string, string][] = [
    ["text / bg", s.text, s.bg],
    ["text / surface", s.text, s.surface],
    ["text_muted / surface", s.text_muted, s.surface],
    ["accent_text / accent", s.accent_text, s.accent],
    ["text / surface2", s.text, s.surface2],
  ];
  for (const [label, fg, bg] of pairs) {
    const ratio = contrastRatio(fg, bg);
    const item = document.createElement("div");
    item.className = "te-wcag-item";
    const name = document.createElement("span");
    name.textContent = label;
    const badge = document.createElement("span");
    const cls = ratio >= 4.5 ? "pass" : ratio >= 3.0 ? "warn" : "fail";
    const tag = ratio >= 7.0 ? "AAA" : ratio >= 4.5 ? "AA" : ratio >= 3.0 ? "AA L" : "FAIL";
    badge.className = `te-wcag-badge ${cls}`;
    badge.textContent = `${tag} ${ratio.toFixed(1)}:1`;
    item.appendChild(name);
    item.appendChild(badge);
    host.appendChild(item);
  }
}

function addColorField(
  grid: HTMLElement,
  key: keyof SemanticTokens,
  labelText: string,
): void {
  const field = document.createElement("div");
  field.className = "te-field";
  const lbl = document.createElement("label");
  lbl.textContent = labelText;
  const row = document.createElement("div");
  row.className = "te-color-row";

  const picker = document.createElement("input");
  picker.type = "color";
  picker.value = normalizeColorForPicker(draftTokens.semantic[key]);

  const hexIn = document.createElement("input");
  hexIn.className = "te-input";
  hexIn.value = draftTokens.semantic[key];
  hexIn.dataset.tokenKey = key;

  picker.addEventListener("input", () => {
    draftTokens.semantic[key] = picker.value.toUpperCase();
    hexIn.value = draftTokens.semantic[key];
    updateLivePreviewVars();
  });
  hexIn.addEventListener("input", () => {
    draftTokens.semantic[key] = hexIn.value.trim();
    if (parseHex(hexIn.value)) {
      picker.value = normalizeColorForPicker(hexIn.value);
    }
    updateLivePreviewVars();
  });

  row.appendChild(picker);
  row.appendChild(hexIn);
  field.appendChild(lbl);
  field.appendChild(row);
  grid.appendChild(field);
}

function render(): void {
  root.innerHTML = "";
  const page = document.createElement("div");
  page.className = "te-page";

  // Topbar
  const top = document.createElement("header");
  top.className = "te-topbar";

  const tGroup = document.createElement("div");
  tGroup.className = "te-title-group";
  const backBtn = document.createElement("button");
  backBtn.type = "button";
  backBtn.className = "te-btn";
  backBtn.innerHTML = `<i class="fa-solid fa-arrow-left" aria-hidden="true"></i><span>${l("Galeria", "Gallery")}</span>`;
  backBtn.addEventListener("click", () => void navigateSelf("seu://themes/"));
  const h1 = document.createElement("h1");
  h1.textContent = l("Editor Visual de Temas", "Visual Theme Editor");
  tGroup.appendChild(backBtn);
  tGroup.appendChild(h1);

  const actions = document.createElement("div");
  actions.className = "te-actions";

  const resetBtn = document.createElement("button");
  resetBtn.type = "button";
  resetBtn.className = "te-btn";
  resetBtn.innerHTML = `<i class="fa-solid fa-rotate-left" aria-hidden="true"></i><span>${l("Resetar", "Reset")}</span>`;
  resetBtn.addEventListener("click", () => void loadThemeIntoDraft(baseThemeId, false));

  const expBtn = document.createElement("button");
  expBtn.type = "button";
  expBtn.className = "te-btn";
  expBtn.innerHTML = `<i class="fa-solid fa-file-export" aria-hidden="true"></i><span>${l("Exportar .seutheme", "Export .seutheme")}</span>`;
  expBtn.addEventListener("click", () => void exportCurrentTheme());

  const saveBtn = document.createElement("button");
  saveBtn.type = "button";
  saveBtn.className = "te-btn primary";
  saveBtn.dataset.action = "save-theme";
  saveBtn.innerHTML = `<i class="fa-solid fa-floppy-disk" aria-hidden="true"></i><span>${l("Salvar & Aplicar", "Save & Apply")}</span>`;
  saveBtn.addEventListener("click", () => void saveDraft(true));

  actions.appendChild(resetBtn);
  actions.appendChild(expBtn);
  actions.appendChild(saveBtn);
  top.appendChild(tGroup);
  top.appendChild(actions);
  page.appendChild(top);

  if (statusMsg) {
    const st = document.createElement("div");
    st.className = `te-status${statusMsg.isError ? " err" : ""}`;
    st.textContent = statusMsg.text;
    page.appendChild(st);
  }

  const layout = document.createElement("div");
  layout.className = "te-layout";

  // LEFT COLUMN: Controls
  const controls = document.createElement("div");
  controls.className = "te-controls";

  // 1. Metadata & Copy-on-Write Base
  const metaCard = document.createElement("section");
  metaCard.className = "te-card";
  metaCard.innerHTML = `<h2><i class="fa-solid fa-sliders" aria-hidden="true"></i><span>${l("Identidade & Base (Copy-on-Write)", "Identity & Base (Copy-on-Write)")}</span></h2>`;

  const baseField = document.createElement("div");
  baseField.className = "te-field";
  const baseLbl = document.createElement("label");
  baseLbl.textContent = l("Tema base (copia layout e fontes)", "Base theme (copies layout & fonts)");
  const baseSel = document.createElement("select");
  baseSel.className = "te-select";
  for (const t of allThemes) {
    const opt = document.createElement("option");
    opt.value = t.id;
    opt.textContent = `${t.name} (${t.id})`;
    if (t.id === baseThemeId) opt.selected = true;
    baseSel.appendChild(opt);
  }
  baseSel.addEventListener("change", () => {
    void loadThemeIntoDraft(baseSel.value, false);
  });
  baseField.appendChild(baseLbl);
  baseField.appendChild(baseSel);
  metaCard.appendChild(baseField);

  const metaGrid = document.createElement("div");
  metaGrid.className = "te-grid-2";

  const nameF = document.createElement("div");
  nameF.className = "te-field";
  nameF.innerHTML = `<label>${l("Nome do tema", "Theme name")}</label>`;
  const nameIn = document.createElement("input");
  nameIn.className = "te-input";
  nameIn.value = draftName;
  nameIn.addEventListener("input", () => {
    draftName = nameIn.value;
    const autoSlug = slugify(draftName);
    if (autoSlug) {
      draftId = autoSlug;
      idIn.value = draftId;
    }
  });
  nameF.appendChild(nameIn);

  const idF = document.createElement("div");
  idF.className = "te-field";
  idF.innerHTML = `<label>ID (slug)</label>`;
  const idIn = document.createElement("input");
  idIn.className = "te-input";
  idIn.value = draftId;
  idIn.addEventListener("input", () => {
    draftId = slugify(idIn.value);
  });
  idF.appendChild(idIn);

  metaGrid.appendChild(nameF);
  metaGrid.appendChild(idF);
  metaCard.appendChild(metaGrid);

  const descF = document.createElement("div");
  descF.className = "te-field";
  descF.innerHTML = `<label>${l("Descrição", "Description")}</label>`;
  const descIn = document.createElement("input");
  descIn.className = "te-input";
  descIn.value = draftDescription;
  descIn.addEventListener("input", () => {
    draftDescription = descIn.value;
  });
  descF.appendChild(descIn);
  metaCard.appendChild(descF);
  controls.appendChild(metaCard);

  // 2. Semantic Color Tokens
  const colorsCard = document.createElement("section");
  colorsCard.className = "te-card";
  colorsCard.innerHTML = `<h2><i class="fa-solid fa-palette" aria-hidden="true"></i><span>${l("Tokens Semânticos de Cor", "Semantic Color Tokens")}</span></h2>`;
  const cGrid = document.createElement("div");
  cGrid.className = "te-grid-2";
  addColorField(cGrid, "bg", "bg (Fundo)");
  addColorField(cGrid, "surface", "surface (Superfície)");
  addColorField(cGrid, "surface2", "surface2 (Elevada)");
  addColorField(cGrid, "border", "border (Borda)");
  addColorField(cGrid, "text", "text (Texto)");
  addColorField(cGrid, "text_muted", "text_muted (Secundário)");
  addColorField(cGrid, "accent", "accent (Destaque)");
  addColorField(cGrid, "accent_text", "accent_text (Sobre destaque)");
  addColorField(cGrid, "danger", "danger (Alerta)");
  addColorField(cGrid, "incognito_bg", "incognito_bg (Anônimo)");
  addColorField(cGrid, "incognito_accent", "incognito_accent");
  colorsCard.appendChild(cGrid);
  controls.appendChild(colorsCard);

  // 3. Typography, Shape & Font Importer
  const typoCard = document.createElement("section");
  typoCard.className = "te-card";
  typoCard.innerHTML = `<h2><i class="fa-solid fa-font" aria-hidden="true"></i><span>${l("Tipografia, Forma & Fontes", "Typography, Shape & Fonts")}</span></h2>`;

  const tGrid = document.createElement("div");
  tGrid.className = "te-grid-2";
  for (const [k, lbl] of [
    ["font_display", "font_display"],
    ["font_body", "font_body"],
    ["font_mono", "font_mono"],
    ["radius", "radius (ex: 14px, 0px)"],
    ["shadow", "shadow"],
  ] as [keyof SemanticTokens, string][]) {
    const f = document.createElement("div");
    f.className = "te-field";
    const la = document.createElement("label");
    la.textContent = lbl;
    const inp = document.createElement("input");
    inp.className = "te-input";
    inp.value = draftTokens.semantic[k];
    inp.addEventListener("input", () => {
      draftTokens.semantic[k] = inp.value;
      updateLivePreviewVars();
    });
    f.appendChild(la);
    f.appendChild(inp);
    tGrid.appendChild(f);
  }
  typoCard.appendChild(tGrid);

  const fontRow = document.createElement("div");
  fontRow.style.display = "flex";
  fontRow.style.flexWrap = "wrap";
  fontRow.style.gap = "8px";
  fontRow.style.alignItems = "center";

  const fontFileIn = document.createElement("input");
  fontFileIn.type = "file";
  fontFileIn.accept = ".woff2,.ttf";
  fontFileIn.style.display = "none";
  fontFileIn.addEventListener("change", () => {
    const file = fontFileIn.files?.[0];
    if (file) void handleFontFile(file);
  });

  const fontBtn = document.createElement("button");
  fontBtn.type = "button";
  fontBtn.className = "te-btn";
  fontBtn.innerHTML = `<i class="fa-solid fa-upload" aria-hidden="true"></i><span>${l("Importar fonte (.woff2 / .ttf)", "Import font (.woff2 / .ttf)")}</span>`;
  fontBtn.addEventListener("click", () => fontFileIn.click());
  fontRow.appendChild(fontFileIn);
  fontRow.appendChild(fontBtn);
  typoCard.appendChild(fontRow);

  for (const f of importedFonts) {
    const row = document.createElement("div");
    row.style.display = "flex";
    row.style.alignItems = "center";
    row.style.justifyContent = "space-between";
    row.style.gap = "6px";
    row.style.fontSize = "12px";
    row.innerHTML = `<span>${f.family} (${f.filename})</span>`;
    const useBtn = document.createElement("button");
    useBtn.type = "button";
    useBtn.className = "te-btn";
    useBtn.textContent = l("Usar no Display", "Use as Display");
    useBtn.addEventListener("click", () => {
      draftTokens.semantic.font_display = `'${f.family}', sans-serif`;
      render();
    });
    row.appendChild(useBtn);
    typoCard.appendChild(row);
  }
  controls.appendChild(typoCard);

  // 4. Custom Tokens + Extra CSS + Hot-Reload Folder
  const customCard = document.createElement("section");
  customCard.className = "te-card";
  customCard.innerHTML = `<h2><i class="fa-solid fa-code" aria-hidden="true"></i><span>${l("Tokens Extras, CSS & Hot-Reload", "Custom Tokens, CSS & Hot-Reload")}</span></h2>`;

  for (const [ck, cv] of Object.entries(draftTokens.custom)) {
    const row = document.createElement("div");
    row.className = "te-custom-row";
    const kIn = document.createElement("input");
    kIn.className = "te-input";
    kIn.value = ck;
    kIn.disabled = true;
    const vIn = document.createElement("input");
    vIn.className = "te-input";
    vIn.value = cv;
    vIn.addEventListener("input", () => {
      draftTokens.custom[ck] = vIn.value;
    });
    const delB = document.createElement("button");
    delB.type = "button";
    delB.className = "te-btn";
    delB.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
    delB.addEventListener("click", () => {
      delete draftTokens.custom[ck];
      render();
    });
    row.appendChild(kIn);
    row.appendChild(vIn);
    row.appendChild(delB);
    customCard.appendChild(row);
  }

  const addCustomRow = document.createElement("div");
  addCustomRow.className = "te-custom-row";
  const newK = document.createElement("input");
  newK.className = "te-input";
  newK.placeholder = "nomeToken";
  const newV = document.createElement("input");
  newV.className = "te-input";
  newV.placeholder = "#4FD8FF";
  const addBtn = document.createElement("button");
  addBtn.type = "button";
  addBtn.className = "te-btn";
  addBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i>`;
  addBtn.addEventListener("click", () => {
    const k = newK.value.trim().replace(/[^a-zA-Z0-9_-]/g, "");
    if (!k) return;
    draftTokens.custom[k] = newV.value.trim() || "#FFFFFF";
    render();
  });
  addCustomRow.appendChild(newK);
  addCustomRow.appendChild(newV);
  addCustomRow.appendChild(addBtn);
  customCard.appendChild(addCustomRow);

  const shCssF = document.createElement("div");
  shCssF.className = "te-field";
  shCssF.innerHTML = `<label>CSS extra da Shell (opcional)</label>`;
  const shCssTa = document.createElement("textarea");
  shCssTa.className = "te-textarea";
  shCssTa.value = extraShellCss;
  shCssTa.placeholder = "/* ex: .y2k-navbar { border-bottom-width: 3px; } */";
  shCssTa.addEventListener("input", () => {
    extraShellCss = shCssTa.value;
  });
  shCssF.appendChild(shCssTa);
  customCard.appendChild(shCssF);

  const ntCssF = document.createElement("div");
  ntCssF.className = "te-field";
  ntCssF.innerHTML = `<label>CSS extra da Nova Aba (opcional)</label>`;
  const ntCssTa = document.createElement("textarea");
  ntCssTa.className = "te-textarea";
  ntCssTa.value = extraNewtabCss;
  ntCssTa.placeholder = "/* ex: .y2k-hero-title { letter-spacing: 0px; } */";
  ntCssTa.addEventListener("input", () => {
    extraNewtabCss = ntCssTa.value;
  });
  ntCssF.appendChild(ntCssTa);
  customCard.appendChild(ntCssF);

  if (userDirPath) {
    const hrHint = document.createElement("p");
    hrHint.className = "te-hint";
    hrHint.textContent = l(
      `Hot-reload ativo na pasta: ${userDirPath} — edite qualquer arquivo HTML/CSS lá e o navegador recarrega automaticamente!`,
      `Hot-reload watching: ${userDirPath} — edit any HTML/CSS file there and the browser updates live!`,
    );
    customCard.appendChild(hrHint);

    const openStudioBtn = document.createElement("button");
    openStudioBtn.type = "button";
    openStudioBtn.className = "te-btn";
    openStudioBtn.innerHTML = `<i class="fa-solid fa-code" aria-hidden="true"></i><span>${l("Abrir pasta de temas no Seu Studio", "Open themes folder in Seu Studio")}</span>`;
    openStudioBtn.addEventListener("click", () => {
      void (async () => {
        await callBridge("studio.openWorkspace", { path: userDirPath }).catch(() => {});
        await callBridge("studio.setMode", { window_id: windowId, mode: "split" });
      })();
    });
    customCard.appendChild(openStudioBtn);
  }
  controls.appendChild(customCard);

  // RIGHT COLUMN: WCAG AA Checker + Live Side-by-Side Preview
  const previewCol = document.createElement("div");
  previewCol.className = "te-preview-col";

  const wcagCard = document.createElement("section");
  wcagCard.className = "te-card";
  wcagCard.innerHTML = `<h2><i class="fa-solid fa-universal-access" aria-hidden="true"></i><span>${l("Contraste WCAG AA (Tempo Real)", "WCAG AA Contrast (Real-Time)")}</span></h2>`;
  const wcagList = document.createElement("div");
  wcagList.className = "te-wcag-list";
  renderWcagChecksInto(wcagList);
  wcagCard.appendChild(wcagList);
  previewCol.appendChild(wcagCard);

  const pvCard = document.createElement("section");
  pvCard.className = "te-card";
  pvCard.innerHTML = `<h2><i class="fa-solid fa-eye" aria-hidden="true"></i><span>${l("Preview ao Vivo — Shell + Nova Aba + Studio", "Live Preview — Shell + New Tab + Studio")}</span></h2>`;

  const stage = document.createElement("div");
  stage.className = "te-live-stage";
  stage.innerHTML = `
    <div class="te-pv-shell">
      <div class="te-pv-tabs">
        <span class="te-pv-tab active">Início</span>
        <span class="te-pv-tab">GitHub</span>
        <span class="te-pv-tab sleep">docs.rs zzz</span>
      </div>
      <div class="te-pv-nav">
        <div class="te-pv-omni">https://seu-navegador.dev</div>
        <span class="te-pv-pill">RAM 165 MB</span>
        <span class="te-pv-pill">128 blk</span>
      </div>
    </div>
    <div class="te-pv-split">
      <div class="te-pv-newtab">
        <h3 class="te-pv-hero">SEU NAVEGADOR</h3>
        <div class="te-pv-search">
          <span class="te-pv-search-txt">${l("Pesquise qualquer coisa...", "Search anything...")}</span>
          <span class="te-pv-search-btn">${l("BUSCAR", "SEARCH")}</span>
        </div>
        <div class="te-pv-tiles">
          <div class="te-pv-tile">YouTube</div>
          <div class="te-pv-tile">GitHub</div>
          <div class="te-pv-tile">docs.rs</div>
        </div>
      </div>
      <div class="te-pv-studio">
        <div class="te-pv-st-head">
          <span>SEU STUDIO</span>
          <span>main (0)</span>
        </div>
        <div class="te-pv-code"><span class="te-pv-accent">pub fn</span> theme_ready() -&gt; bool {
  <span class="te-pv-muted">// 100% seu jeito</span>
  <span class="te-pv-accent">let</span> ram_mb = 165;
  <span class="te-pv-danger">if</span> ram_mb &gt; 500 { <span class="te-pv-accent">return</span> false; }
  true
}</div>
      </div>
    </div>
    <div class="te-pv-incog-strip">
      <span>${l("Modo Anônimo", "Incognito Mode")}</span>
      <span class="te-pv-incog-pill">ANÔNIMO</span>
    </div>
  `;
  pvCard.appendChild(stage);
  previewCol.appendChild(pvCard);

  layout.appendChild(controls);
  layout.appendChild(previewCol);
  page.appendChild(layout);
  root.appendChild(page);
  updateLivePreviewVars();
}

onBridgeEvent((event) => {
  if (event.type === "theme") {
    applyPageThemeTokens(event.theme);
  } else if (event.type === "settings") {
    locale = event.settings.locale;
    render();
  }
});

async function bootstrap(): Promise<void> {
  const [settings, active, list, dir] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
    callBridge<ThemeSummary[]>("themes.list").catch(() => []),
    callBridge<{ path: string }>("themes.getUserDir").catch(() => undefined),
  ]);
  if (settings) locale = settings.locale;
  if (active) applyPageThemeTokens(active);
  if (Array.isArray(list)) allThemes = list;
  if (dir?.path) userDirPath = dir.path;

  const editId = urlParams.get("edit");
  const baseId = urlParams.get("base") || active?.manifest.id || "y2k-maximal";
  await loadThemeIntoDraft(editId || baseId, Boolean(editId));
}

void bootstrap();
