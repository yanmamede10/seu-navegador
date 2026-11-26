import "./icons";
import "./themes.css";
import {
  callBridge,
  getTabIdFromQuery,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type ThemeSummary,
  type WindowSnapshot,
} from "./bridge";

const root = document.getElementById("themes-root") as HTMLDivElement;
const windowId = getWindowIdFromQuery();
const selfTabId = getTabIdFromQuery();

type FilterKind = "all" | "dark" | "light";

let locale: "pt-br" | "en" = "pt-br";
let themes: ThemeSummary[] = [];
let activeThemeId = "y2k-maximal";
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

  const studioBtn = document.createElement("button");
  studioBtn.type = "button";
  studioBtn.className = "tg-btn";
  studioBtn.innerHTML = `<i class="fa-solid fa-table-columns" aria-hidden="true"></i><span>${l("Testar no Split + Studio", "Preview in Split + Studio")}</span>`;
  studioBtn.addEventListener("click", () => {
    void callBridge("studio.setMode", { window_id: windowId, mode: "split" });
  });

  const newTabBtn = document.createElement("button");
  newTabBtn.type = "button";
  newTabBtn.className = "tg-btn primary";
  newTabBtn.innerHTML = `<i class="fa-solid fa-house" aria-hidden="true"></i><span>${l("Ver Nova Aba", "Open New Tab")}</span>`;
  newTabBtn.addEventListener("click", () => {
    void navigateSelf("seu://newtab/");
  });

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

  const filters = document.createElement("div");
  filters.className = "tg-filters";
  for (const [id, label] of [
    ["all", l("Todos (9)", "All (9)")],
    ["dark", l("Escuros", "Dark")],
    ["light", l("Claros", "Light")],
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
    if (filter !== "all" && item.preview) {
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

    const applyBtn = document.createElement("button");
    applyBtn.type = "button";
    applyBtn.className = `tg-apply-btn${isActive ? " active" : ""}`;
    applyBtn.innerHTML = isActive
      ? `<i class="fa-solid fa-check" aria-hidden="true"></i><span>${l("Em uso", "In use")}</span>`
      : `<i class="fa-solid fa-wand-magic-sparkles" aria-hidden="true"></i><span>${l("Aplicar", "Apply")}</span>`;
    applyBtn.addEventListener("click", () => {
      void activateTheme(item.id);
    });

    foot.appendChild(swatches);
    foot.appendChild(applyBtn);
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
    render();
  }
});

async function bootstrap(): Promise<void> {
  const [settings, active, list] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
    callBridge<ThemeSummary[]>("themes.list").catch(() => []),
  ]);
  if (settings) locale = settings.locale;
  if (active) applyThemeTokens(active);
  if (Array.isArray(list)) themes = list;
  render();
}

void bootstrap();
