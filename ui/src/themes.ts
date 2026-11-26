import "./icons";
import "./themes.css";
import {
  callBridge,
  getTabIdFromQuery,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type ThemeExportPayload,
  type ThemeSummary,
  type WindowSnapshot,
} from "./bridge";

const root = document.getElementById("themes-root") as HTMLDivElement;
const windowId = getWindowIdFromQuery();
const selfTabId = getTabIdFromQuery();

type FilterKind = "all" | "dark" | "light" | "custom";

let locale: "pt-br" | "en" = "pt-br";
let themes: ThemeSummary[] = [];
let activeThemeId = "y2k-maximal";
let allowedScriptThemes: string[] = [];
let pendingOptInThemeId: string | null = null;
let filter: FilterKind = "all";
let searchQuery = "";
let bannerText = "";

function l(pt: string, en: string): string {
  return locale === "en" ? en : pt;
}

function applyThemeTokens(theme: LoadedTheme): void {
  activeThemeId = theme.manifest.id;
  let styleEl = document.getElementById("seu-tokens-css") as HTMLStyleElement | null;
  if (!styleEl) {
    styleEl = document.createElement("style");
    styleEl.id = "seu-tokens-css";
    document.head.appendChild(styleEl);
  }
  styleEl.textContent = theme.tokens_css;
}

function isDarkHex(hex: string): boolean {
  const clean = hex.trim().replace(/^#/, "");
  if (clean.length !== 6) return true;
  const r = parseInt(clean.slice(0, 2), 16);
  const g = parseInt(clean.slice(2, 4), 16);
  const b = parseInt(clean.slice(4, 6), 16);
  if ([r, g, b].some((v) => Number.isNaN(v))) return true;
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum < 0.5;
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

async function exportTheme(id: string): Promise<void> {
  try {
    const exp = await callBridge<ThemeExportPayload>("themes.export", { id });
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
    bannerText = l(
      `Exportado: ${exp.filename} (${Math.round(exp.size_bytes / 1024)} KB)`,
      `Exported: ${exp.filename} (${Math.round(exp.size_bytes / 1024)} KB)`,
    );
    render();
  } catch (err) {
    bannerText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function importThemeFile(file: File): Promise<void> {
  try {
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    let binary = "";
    for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    const base64 = btoa(binary);
    const imported = await callBridge<LoadedTheme>("themes.import", { base64 });
    const list = await callBridge<ThemeSummary[]>("themes.list").catch(() => themes);
    if (Array.isArray(list)) themes = list;
    if (imported) {
      bannerText = l(
        `Tema "${imported.manifest.name}" importado com segurança (JS de sites desativado por padrão).`,
        `Theme "${imported.manifest.name}" imported safely (site scripts disabled by default).`,
      );
    }
    render();
  } catch (err) {
    bannerText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function deleteUserTheme(id: string): Promise<void> {
  try {
    const list = await callBridge<ThemeSummary[]>("themes.delete", { id });
    if (Array.isArray(list)) themes = list;
    bannerText = l(`Tema "${id}" removido.`, `Theme "${id}" deleted.`);
    render();
  } catch (err) {
    bannerText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function activateTheme(id: string): Promise<void> {
  try {
    const loaded = await callBridge<LoadedTheme>("themes.setActive", { id });
    if (loaded) {
      applyThemeTokens(loaded);
      bannerText = l(
        `Tema "${loaded.manifest.name}" aplicado instantaneamente em todo o navegador, DevTools e Studio.`,
        `Theme "${loaded.manifest.name}" applied live across Browser, DevTools, and Studio.`,
      );
      render();
    }
  } catch (err) {
    bannerText = err instanceof Error ? err.message : String(err);
    render();
  }
}

function renderPreview(item: ThemeSummary): HTMLElement {
  const p = item.preview ?? {
    bg: "#070B24",
    surface: "#0B1240",
    surface2: "#0E1650",
    text: "#E6E9FF",
    accent: "#FF2E93",
    border: "#2A3680",
    radius: "12px",
  };
  const r = p.radius === "0px" ? "0px" : "8px";

  const box = document.createElement("div");
  box.className = "tg-preview";
  box.style.background = p.bg;

  const bar = document.createElement("div");
  bar.className = "tg-mini-bar";
  bar.style.background = p.surface;
  bar.style.borderColor = p.border;
  bar.style.borderRadius = r;

  const tab1 = document.createElement("span");
  tab1.className = "tg-mini-tab";
  tab1.style.background = p.accent;
  tab1.style.borderColor = p.border;
  tab1.style.borderRadius = r;

  const tab2 = document.createElement("span");
  tab2.className = "tg-mini-tab ghost";
  tab2.style.background = p.surface2;
  tab2.style.borderColor = p.border;
  tab2.style.borderRadius = r;

  const dots = document.createElement("span");
  dots.className = "tg-mini-dots";
  for (const col of [p.border, p.text, p.accent]) {
    const d = document.createElement("span");
    d.className = "tg-mini-dot";
    d.style.background = col;
    dots.appendChild(d);
  }

  bar.appendChild(tab1);
  bar.appendChild(tab2);
  bar.appendChild(dots);
  box.appendChild(bar);

  const body = document.createElement("div");
  body.className = "tg-mini-body";

  // Left mini browser pane
  const paneLeft = document.createElement("div");
  paneLeft.className = "tg-mini-pane";
  paneLeft.style.background = p.surface;
  paneLeft.style.borderColor = p.border;
  paneLeft.style.borderRadius = r;

  const l1 = document.createElement("div");
  l1.className = "tg-mini-line";
  l1.style.background = p.text;
  const l2 = document.createElement("div");
  l2.className = "tg-mini-line short";
  l2.style.background = p.text;
  const pill = document.createElement("div");
  pill.className = "tg-mini-pill";
  pill.style.background = p.accent;
  pill.style.borderRadius = r;
  paneLeft.appendChild(l1);
  paneLeft.appendChild(l2);
  paneLeft.appendChild(pill);

  // Right mini Studio/code pane
  const paneRight = document.createElement("div");
  paneRight.className = "tg-mini-pane";
  paneRight.style.background = p.surface2;
  paneRight.style.borderColor = p.border;
  paneRight.style.borderRadius = r;

  for (const [w, col] of [
    ["80%", p.accent],
    ["65%", p.text],
    ["90%", p.text],
    ["50%", p.accent],
  ] as const) {
    const ln = document.createElement("div");
    ln.className = "tg-mini-line";
    ln.style.width = w;
    ln.style.background = col;
    paneRight.appendChild(ln);
  }

  body.appendChild(paneLeft);
  body.appendChild(paneRight);
  box.appendChild(body);

  return box;
}

function render(): void {
  root.innerHTML = "";

  const wrap = document.createElement("div");
  wrap.className = "tg-wrap";

  const header = document.createElement("header");
  header.className = "tg-header";

  const titleCol = document.createElement("div");
  titleCol.className = "tg-title-col";
  const h1 = document.createElement("h1");
  h1.textContent = l("Galeria de Temas", "Theme Gallery");
  const sub = document.createElement("p");
  sub.className = "tg-subtitle";
  sub.textContent = l(
    `${themes.length} interfaces oficiais com troca instantânea no Navegador, DevTools e Studio.`,
    `${themes.length} official interfaces with instant hot-swap across Browser, DevTools, and Studio.`,
  );
  titleCol.appendChild(h1);
  titleCol.appendChild(sub);

  const actions = document.createElement("div");
  actions.className = "tg-actions";

  const createBtn = document.createElement("button");
  createBtn.type = "button";
  createBtn.className = "tg-btn primary";
  createBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i><span>${l("Criar tema", "Create theme")}</span>`;
  createBtn.addEventListener("click", () => {
    void navigateSelf(`seu://themes/editor?base=${encodeURIComponent(activeThemeId)}`);
  });

  const importInput = document.createElement("input");
  importInput.type = "file";
  importInput.accept = ".seutheme,.zip";
  importInput.style.display = "none";
  importInput.addEventListener("change", () => {
    const file = importInput.files?.[0];
    if (file) void importThemeFile(file);
  });

  const importBtn = document.createElement("button");
  importBtn.type = "button";
  importBtn.className = "tg-btn";
  importBtn.innerHTML = `<i class="fa-solid fa-file-import" aria-hidden="true"></i><span>${l("Importar .seutheme", "Import .seutheme")}</span>`;
  importBtn.addEventListener("click", () => importInput.click());

  const studioBtn = document.createElement("button");
  studioBtn.type = "button";
  studioBtn.className = "tg-btn";
  studioBtn.innerHTML = `<i class="fa-solid fa-table-columns" aria-hidden="true"></i><span>${l("Testar no Split + Studio", "Preview in Split + Studio")}</span>`;
  studioBtn.addEventListener("click", () => {
    void callBridge("studio.setMode", { window_id: windowId, mode: "split" });
  });

  const newTabBtn = document.createElement("button");
  newTabBtn.type = "button";
  newTabBtn.className = "tg-btn";
  newTabBtn.innerHTML = `<i class="fa-solid fa-house" aria-hidden="true"></i><span>${l("Ver Nova Aba", "Open New Tab")}</span>`;
  newTabBtn.addEventListener("click", () => {
    void navigateSelf("seu://newtab/");
  });

  actions.appendChild(createBtn);
  actions.appendChild(importInput);
  actions.appendChild(importBtn);
  actions.appendChild(studioBtn);
  actions.appendChild(newTabBtn);
  header.appendChild(titleCol);
  header.appendChild(actions);
  wrap.appendChild(header);

  if (bannerText) {
    const banner = document.createElement("div");
    banner.className = "tg-banner";
    banner.textContent = bannerText;
    wrap.appendChild(banner);
  }

  const toolbar = document.createElement("div");
  toolbar.className = "tg-toolbar";

  const customCount = themes.filter((t) => !t.builtin).length;
  const filters = document.createElement("div");
  filters.className = "tg-filters";
  for (const [id, label] of [
    ["all", l(`Todos (${themes.length})`, `All (${themes.length})`)],
    ["dark", l("Escuros", "Dark")],
    ["light", l("Claros", "Light")],
    ["custom", l(`Meus temas (${customCount})`, `My themes (${customCount})`)],
  ] as const) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `tg-chip${filter === id ? " active" : ""}`;
    btn.textContent = label;
    btn.addEventListener("click", () => {
      filter = id;
      render();
    });
    filters.appendChild(btn);
  }

  const searchBox = document.createElement("div");
  searchBox.className = "tg-search-box";
  searchBox.innerHTML = `<i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i>`;
  const searchInput = document.createElement("input");
  searchInput.type = "text";
  searchInput.value = searchQuery;
  searchInput.placeholder = l("Filtrar temas...", "Filter themes...");
  searchInput.addEventListener("input", () => {
    searchQuery = searchInput.value;
    renderCardsOnly(grid);
  });
  searchBox.appendChild(searchInput);

  toolbar.appendChild(filters);
  toolbar.appendChild(searchBox);
  wrap.appendChild(toolbar);

  const grid = document.createElement("div");
  grid.className = "tg-grid";
  renderCardsOnly(grid);
  wrap.appendChild(grid);

  root.appendChild(wrap);
}

function renderCardsOnly(grid: HTMLElement): void {
  grid.innerHTML = "";
  const q = searchQuery.trim().toLowerCase();

  const visible = themes.filter((item) => {
    if (filter === "custom" && item.builtin) return false;
    if ((filter === "dark" || filter === "light") && item.preview) {
      const dark = isDarkHex(item.preview.bg);
      if (filter === "dark" && !dark) return false;
      if (filter === "light" && dark) return false;
    }
    if (q) {
      const hay = `${item.name} ${item.id} ${item.description}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });

  for (const item of visible) {
    const isActive = item.id === activeThemeId;
    const card = document.createElement("article");
    card.className = `tg-card${isActive ? " active" : ""}`;
    card.dataset.themeId = item.id;

    card.appendChild(renderPreview(item));

    const body = document.createElement("div");
    body.className = "tg-card-body";

    const top = document.createElement("div");
    top.className = "tg-card-top";
    const name = document.createElement("h2");
    name.className = "tg-card-name";
    name.textContent = item.name;
    const tag = document.createElement("span");
    tag.className = `tg-tag${isActive ? " active" : ""}`;
    tag.textContent = isActive
      ? l("Ativo", "Active")
      : item.builtin
        ? l("Oficial", "Official")
        : l("Usuário", "Custom");
    top.appendChild(name);
    top.appendChild(tag);
    body.appendChild(top);

    const desc = document.createElement("p");
    desc.className = "tg-desc";
    desc.textContent = item.description;
    body.appendChild(desc);

    const patterns = item.site_skin_patterns ?? [];
    if (patterns.length > 0) {
      const skinBar = document.createElement("div");
      skinBar.style.display = "flex";
      skinBar.style.flexWrap = "wrap";
      skinBar.style.alignItems = "center";
      skinBar.style.gap = "6px";
      skinBar.style.marginBottom = "8px";

      const skinPill = document.createElement("span");
      skinPill.className = "tg-tag";
      skinPill.innerHTML = `<i class="fa-solid fa-wand-magic-sparkles" aria-hidden="true"></i> ${patterns.join(", ")}`;
      skinBar.appendChild(skinPill);

      if (!item.builtin && item.has_site_scripts) {
        const jsAllowed = allowedScriptThemes.includes(item.id);
        if (jsAllowed) {
          const revokeBtn = document.createElement("button");
          revokeBtn.type = "button";
          revokeBtn.className = "tg-apply-btn";
          revokeBtn.style.fontSize = "11px";
          revokeBtn.innerHTML = `<i class="fa-solid fa-shield-halved" aria-hidden="true"></i><span>${l("JS ativo (Revogar)", "JS active (Revoke)")}</span>`;
          revokeBtn.addEventListener("click", () => {
            pendingOptInThemeId = null;
            void callBridge<CoreSettings>("themes.setSiteScriptsAllowed", {
              id: item.id,
              allowed: false,
            }).then((s) => {
              if (s) allowedScriptThemes = s.allowed_site_script_themes ?? [];
              bannerText = l(
                `JS de sites desativado para "${item.name}".`,
                `Site JS disabled for "${item.name}".`,
              );
              render();
            });
          });
          skinBar.appendChild(revokeBtn);
        } else if (pendingOptInThemeId === item.id) {
          const warnBox = document.createElement("div");
          warnBox.style.width = "100%";
          warnBox.style.padding = "8px";
          warnBox.style.borderRadius = "8px";
          warnBox.style.border = "1px solid var(--seu-danger)";
          warnBox.style.fontSize = "11px";
          warnBox.style.display = "flex";
          warnBox.style.flexDirection = "column";
          warnBox.style.gap = "6px";
          const warnText = document.createElement("span");
          warnText.textContent = l(
            `Aviso: este tema importado contém JS para ${patterns.join(", ")}. Só ative se confiar no autor (nunca acessa window.seu).`,
            `Warning: this imported theme includes JS for ${patterns.join(", ")}. Only enable if you trust the author (never accesses window.seu).`,
          );
          const warnBtns = document.createElement("div");
          warnBtns.style.display = "flex";
          warnBtns.style.gap = "6px";
          const okBtn = document.createElement("button");
          okBtn.type = "button";
          okBtn.className = "tg-apply-btn active";
          okBtn.textContent = l("Confirmar e permitir JS", "Confirm & allow JS");
          okBtn.addEventListener("click", () => {
            pendingOptInThemeId = null;
            void callBridge<CoreSettings>("themes.setSiteScriptsAllowed", {
              id: item.id,
              allowed: true,
            }).then((s) => {
              if (s) allowedScriptThemes = s.allowed_site_script_themes ?? [];
              bannerText = l(
                `JS de sites permitido para "${item.name}".`,
                `Site JS allowed for "${item.name}".`,
              );
              render();
            });
          });
          const cancelBtn = document.createElement("button");
          cancelBtn.type = "button";
          cancelBtn.className = "tg-apply-btn";
          cancelBtn.textContent = l("Cancelar", "Cancel");
          cancelBtn.addEventListener("click", () => {
            pendingOptInThemeId = null;
            render();
          });
          warnBtns.appendChild(okBtn);
          warnBtns.appendChild(cancelBtn);
          warnBox.appendChild(warnText);
          warnBox.appendChild(warnBtns);
          skinBar.appendChild(warnBox);
        } else {
          const allowBtn = document.createElement("button");
          allowBtn.type = "button";
          allowBtn.className = "tg-apply-btn";
          allowBtn.style.fontSize = "11px";
          allowBtn.innerHTML = `<i class="fa-solid fa-lock" aria-hidden="true"></i><span>${l("JS bloqueado · Permitir", "JS blocked · Allow")}</span>`;
          allowBtn.addEventListener("click", () => {
            pendingOptInThemeId = item.id;
            render();
          });
          skinBar.appendChild(allowBtn);
        }
      }

      body.appendChild(skinBar);
    }

    const foot = document.createElement("div");
    foot.className = "tg-card-foot";

    const swatches = document.createElement("div");
    swatches.className = "tg-swatches";
    if (item.preview) {
      for (const col of [
        item.preview.bg,
        item.preview.surface,
        item.preview.accent,
        item.preview.text,
      ]) {
        const sw = document.createElement("span");
        sw.className = "tg-swatch";
        sw.style.background = col;
        swatches.appendChild(sw);
      }
    }

    const btnGroup = document.createElement("div");
    btnGroup.style.display = "flex";
    btnGroup.style.gap = "6px";
    btnGroup.style.flexWrap = "wrap";

    const editBtn = document.createElement("button");
    editBtn.type = "button";
    editBtn.className = "tg-apply-btn";
    editBtn.title = item.builtin ? l("Personalizar cópia", "Customize copy") : l("Editar tema", "Edit theme");
    editBtn.innerHTML = `<i class="fa-solid fa-pen" aria-hidden="true"></i>`;
    editBtn.addEventListener("click", () => {
      const param = item.builtin ? `base=${encodeURIComponent(item.id)}` : `edit=${encodeURIComponent(item.id)}`;
      void navigateSelf(`seu://themes/editor?${param}`);
    });

    const expBtn = document.createElement("button");
    expBtn.type = "button";
    expBtn.className = "tg-apply-btn";
    expBtn.title = l("Exportar .seutheme", "Export .seutheme");
    expBtn.innerHTML = `<i class="fa-solid fa-file-export" aria-hidden="true"></i>`;
    expBtn.addEventListener("click", () => void exportTheme(item.id));

    btnGroup.appendChild(editBtn);
    btnGroup.appendChild(expBtn);

    if (!item.builtin) {
      const delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.className = "tg-apply-btn";
      delBtn.title = l("Excluir tema", "Delete theme");
      delBtn.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
      delBtn.addEventListener("click", () => void deleteUserTheme(item.id));
      btnGroup.appendChild(delBtn);
    }

    const applyBtn = document.createElement("button");
    applyBtn.type = "button";
    applyBtn.className = `tg-apply-btn${isActive ? " active" : ""}`;
    applyBtn.innerHTML = isActive
      ? `<i class="fa-solid fa-check" aria-hidden="true"></i><span>${l("Em uso", "In use")}</span>`
      : `<i class="fa-solid fa-wand-magic-sparkles" aria-hidden="true"></i><span>${l("Aplicar", "Apply")}</span>`;
    applyBtn.addEventListener("click", () => {
      void activateTheme(item.id);
    });
    btnGroup.appendChild(applyBtn);

    foot.appendChild(swatches);
    foot.appendChild(btnGroup);
    body.appendChild(foot);
    card.appendChild(body);
    grid.appendChild(card);
  }
}

onBridgeEvent((event) => {
  if (event.type === "theme") {
    applyThemeTokens(event.theme);
    render();
  } else if (event.type === "settings") {
    locale = event.settings.locale;
    allowedScriptThemes = event.settings.allowed_site_script_themes ?? [];
    render();
  }
});

async function bootstrap(): Promise<void> {
  const [settings, active, list] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
    callBridge<ThemeSummary[]>("themes.list").catch(() => []),
  ]);
  if (settings) {
    locale = settings.locale;
    allowedScriptThemes = settings.allowed_site_script_themes ?? [];
  }
  if (active) applyThemeTokens(active);
  if (Array.isArray(list)) themes = list;
  render();
}

void bootstrap();
