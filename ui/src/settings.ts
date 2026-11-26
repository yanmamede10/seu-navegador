import "./icons";
import "./settings.css";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type BlockerStats,
  type CoreSettings,
  type DataSummaryPayload,
  type DetectedBrowserProfile,
  type ImportReport,
  type LoadedTheme,
  type MemoryPolicy,
  type MemoryStats,
  type QuickShortcut,
  type SitePermissionItem,
  type ThemeSummary,
  type WindowSnapshot,
} from "./bridge";
import { setLocale } from "./i18n";

const root = document.getElementById("settings-root") as HTMLDivElement;
const windowId = getWindowIdFromQuery();

type SettingsTab = "shortcuts" | "skins" | "memory" | "blocker" | "session" | "import" | "quick";

let activeTab: SettingsTab = "shortcuts";
let currentSettings: CoreSettings = {
  locale: "pt-br",
  search_engine: "duck-duck-go",
  active_theme_id: "y2k-maximal",
  game_mode_enabled: false,
  shortcuts: {},
  quick_shortcuts: [],
  memory_policy: {
    sleep_timeout_secs: 600,
    max_live_tabs: null,
    never_sleep_pinned: true,
    never_sleep_domains: ["meet.google.com", "discord.com", "web.whatsapp.com"],
  },
};
let currentMemory: MemoryStats = {
  total_private_mb: 0,
  total_working_set_mb: 0,
  process_count: 0,
  live_tabs_count: 1,
  sleeping_tabs_count: 0,
  saved_mb: 0,
  shared_renderer_detected: false,
};
let currentBlocker: BlockerStats = {
  enabled: true,
  total_blocked: 0,
  allowlisted_domains: [],
  recent_blocked: [],
  lists_loaded: 1,
  last_updated_secs: null,
};
let detectedProfiles: DetectedBrowserProfile[] = [];
let selectedProfileId = "";
let importBookmarks = true;
let importHistory = true;
let importQuickShortcuts = false;
let importing = false;
let statusMessage: { text: string; isError: boolean } | null = null;
let dataSummary: DataSummaryPayload = { bookmarks: [], history: [] };
let savedPermissions: SitePermissionItem[] = [];
let availableThemes: ThemeSummary[] = [];
let pendingScriptOptInThemeId: string | null = null;
let recordingActionId: string | null = null;

interface ShortcutMeta {
  id: string;
  group: "tabs" | "nav" | "windows" | "ui";
  pt: string;
  en: string;
}

const SHORTCUT_CATALOG: ShortcutMeta[] = [
  { id: "tab.new", group: "tabs", pt: "Nova aba", en: "New tab" },
  { id: "tab.close", group: "tabs", pt: "Fechar aba atual", en: "Close active tab" },
  { id: "tab.reopenClosed", group: "tabs", pt: "Reabrir última aba fechada", en: "Reopen last closed tab" },
  { id: "tab.next", group: "tabs", pt: "Próxima aba", en: "Next tab" },
  { id: "tab.prev", group: "tabs", pt: "Aba anterior", en: "Previous tab" },
  { id: "tab.duplicate", group: "tabs", pt: "Duplicar aba atual", en: "Duplicate active tab" },
  { id: "tab.pin", group: "tabs", pt: "Fixar / desfixar aba", en: "Pin / unpin tab" },
  { id: "tab.switch1", group: "tabs", pt: "Ir para aba 1", en: "Switch to tab 1" },
  { id: "tab.switch2", group: "tabs", pt: "Ir para aba 2", en: "Switch to tab 2" },
  { id: "tab.switch3", group: "tabs", pt: "Ir para aba 3", en: "Switch to tab 3" },
  { id: "tab.switch4", group: "tabs", pt: "Ir para aba 4", en: "Switch to tab 4" },
  { id: "tab.switch5", group: "tabs", pt: "Ir para aba 5", en: "Switch to tab 5" },
  { id: "tab.switch6", group: "tabs", pt: "Ir para aba 6", en: "Switch to tab 6" },
  { id: "tab.switch7", group: "tabs", pt: "Ir para aba 7", en: "Switch to tab 7" },
  { id: "tab.switch8", group: "tabs", pt: "Ir para aba 8", en: "Switch to tab 8" },
  { id: "tab.switchLast", group: "tabs", pt: "Ir para última aba", en: "Switch to last tab" },

  { id: "nav.focusOmnibox", group: "nav", pt: "Focar barra de endereço", en: "Focus address bar" },
  { id: "nav.back", group: "nav", pt: "Voltar página", en: "Go back" },
  { id: "nav.forward", group: "nav", pt: "Avançar página", en: "Go forward" },
  { id: "tab.reload", group: "nav", pt: "Recarregar página", en: "Reload page" },
  { id: "tab.hardReload", group: "nav", pt: "Recarregar sem cache", en: "Hard reload (ignore cache)" },
  { id: "nav.openSettings", group: "nav", pt: "Abrir configurações", en: "Open settings" },

  { id: "window.new", group: "windows", pt: "Nova janela", en: "New window" },
  { id: "window.newIncognito", group: "windows", pt: "Nova janela anônima", en: "New incognito window" },

  { id: "zoom.in", group: "ui", pt: "Aumentar zoom", en: "Zoom in" },
  { id: "zoom.out", group: "ui", pt: "Diminuir zoom", en: "Zoom out" },
  { id: "zoom.reset", group: "ui", pt: "Resetar zoom (100%)", en: "Reset zoom (100%)" },
  { id: "overlay.toggleMenu", group: "ui", pt: "Abrir / fechar menu", en: "Toggle menu" },
  { id: "devtools.toggle", group: "ui", pt: "DevTools acoplado (F12)", en: "Docked DevTools (F12)" },
  { id: "split.toggle", group: "ui", pt: "Split View (duas abas lado a lado)", en: "Split View (two tabs side by side)" },
  { id: "studio.toggle", group: "ui", pt: "Abrir / fechar Studio IDE (Split)", en: "Toggle Studio IDE (Split)" },
  { id: "bookmarks.toggle", group: "nav", pt: "Favoritar página atual", en: "Bookmark current page" },
  { id: "nav.openHistory", group: "nav", pt: "Abrir Histórico", en: "Open History" },
  { id: "nav.openDownloads", group: "nav", pt: "Abrir Downloads", en: "Open Downloads" },
];

function isEn(): boolean {
  return currentSettings.locale === "en";
}

function l(pt: string, en: string): string {
  return isEn() ? en : pt;
}

function applyThemeTokens(theme: LoadedTheme): void {
  let styleEl = document.getElementById("seu-tokens-css") as HTMLStyleElement | null;
  if (!styleEl) {
    styleEl = document.createElement("style");
    styleEl.id = "seu-tokens-css";
    document.head.appendChild(styleEl);
  }
  styleEl.textContent = theme.tokens_css;
}

function browserIconClass(name: string): string {
  const lower = name.toLowerCase();
  if (lower.includes("chrome")) return "fa-brands fa-chrome";
  if (lower.includes("edge")) return "fa-brands fa-edge";
  if (lower.includes("firefox")) return "fa-brands fa-firefox-browser";
  if (lower.includes("opera")) return "fa-brands fa-opera";
  if (lower.includes("brave")) return "fa-solid fa-shield-halved";
  return "fa-solid fa-globe";
}

function formatKeyEventToCombo(ev: KeyboardEvent): string | null {
  const rawKey = ev.key;
  if (
    rawKey === "Control" ||
    rawKey === "Shift" ||
    rawKey === "Alt" ||
    rawKey === "Meta"
  ) {
    return null;
  }

  let keyToken = "";
  if (rawKey.length === 1) {
    if (rawKey === "+" || rawKey === "=") keyToken = "=";
    else if (rawKey === "-") keyToken = "-";
    else if (rawKey === ",") keyToken = ",";
    else if (rawKey === ".") keyToken = ".";
    else if (rawKey === "/") keyToken = "/";
    else if (/[a-z0-9]/i.test(rawKey)) keyToken = rawKey.toUpperCase();
    else return null;
  } else if (
    [
      "Tab",
      "Enter",
      "PageUp",
      "PageDown",
      "Home",
      "End",
      "ArrowLeft",
      "ArrowRight",
      "ArrowUp",
      "ArrowDown",
      "Delete",
    ].includes(rawKey) ||
    /^F([1-9]|1[0-2])$/.test(rawKey)
  ) {
    keyToken = rawKey;
  } else {
    return null;
  }

  const isFn = /^F([1-9]|1[0-2])$/.test(keyToken);
  if (!ev.ctrlKey && !ev.altKey && !isFn) {
    return null;
  }

  const parts: string[] = [];
  if (ev.ctrlKey) parts.push("Ctrl");
  if (ev.altKey) parts.push("Alt");
  if (ev.shiftKey) parts.push("Shift");
  parts.push(keyToken);
  return parts.join("+");
}

window.addEventListener("keydown", (ev) => {
  if (!recordingActionId) return;
  ev.preventDefault();
  ev.stopPropagation();

  if (ev.key === "Escape") {
    recordingActionId = null;
    render();
    return;
  }

  const combo = formatKeyEventToCombo(ev);
  if (!combo) {
    return;
  }

  const actionToSave = recordingActionId;
  recordingActionId = null;
  void callBridge<CoreSettings>("shortcuts.set", {
    action: actionToSave,
    combo,
  })
    .then((updated) => {
      if (updated) {
        currentSettings = updated;
        statusMessage = {
          text: l(
            `Atalho atualizado para ${combo}.`,
            `Shortcut updated to ${combo}.`,
          ),
          isError: false,
        };
      }
      render();
    })
    .catch((err: unknown) => {
      statusMessage = {
        text: err instanceof Error ? err.message : String(err),
        isError: true,
      };
      render();
    });
});

async function refreshProfilesAndData(): Promise<void> {
  const [profiles, summary] = await Promise.all([
    callBridge<DetectedBrowserProfile[]>("import.listProfiles").catch(() => []),
    callBridge<DataSummaryPayload>("data.summary").catch(() => ({
      bookmarks: [],
      history: [],
    })),
  ]);
  detectedProfiles = profiles ?? [];
  if (!selectedProfileId && detectedProfiles.length > 0) {
    selectedProfileId = detectedProfiles[0]?.id ?? "";
  }
  if (summary) {
    dataSummary = summary;
  }
  render();
}

async function navigateInActiveTab(url: string): Promise<void> {
  const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
    window_id: windowId,
  }).catch(() => undefined);
  if (snap?.active_tab_id) {
    await callBridge("tabs.navigate", { tab_id: snap.active_tab_id, input: url });
  }
}

function render(): void {
  root.innerHTML = "";
  const shell = document.createElement("div");
  shell.className = "settings-shell";

  const header = document.createElement("header");
  header.className = "settings-header";

  const h1 = document.createElement("h1");
  h1.className = "settings-title";
  h1.innerHTML = `<i class="fa-solid fa-sliders" aria-hidden="true"></i><span>${l("Configurações do Seu Navegador", "Seu Navegador Settings")}</span>`;
  header.appendChild(h1);

  const tabs = document.createElement("div");
  tabs.className = "settings-tabs";
  const tabDefs: [SettingsTab, string, string][] = [
    ["shortcuts", "fa-solid fa-keyboard", l("Atalhos", "Shortcuts")],
    ["skins", "fa-solid fa-wand-magic-sparkles", l("Skins de Sites", "Site Skins")],
    ["memory", "fa-solid fa-microchip", l("Memória", "Memory")],
    ["blocker", "fa-solid fa-shield-halved", l("Bloqueador", "Blocker")],
    ["session", "fa-solid fa-lock", l("Sessão & Permissões", "Session & Permissions")],
    ["import", "fa-solid fa-file-import", l("Importar", "Import")],
    ["quick", "fa-solid fa-compact-disc", l("CDs", "CDs")],
  ];
  for (const [id, icon, label] of tabDefs) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `tab-btn${activeTab === id ? " active" : ""}`;
    btn.innerHTML = `<i class="${icon}" aria-hidden="true"></i><span>${label}</span>`;
    btn.addEventListener("click", () => {
      activeTab = id;
      recordingActionId = null;
      statusMessage = null;
      render();
    });
    tabs.appendChild(btn);
  }
  header.appendChild(tabs);
  shell.appendChild(header);

  if (statusMessage) {
    const banner = document.createElement("div");
    banner.className = `status-banner${statusMessage.isError ? " error" : ""}`;
    banner.innerHTML = `<i class="fa-solid ${statusMessage.isError ? "fa-triangle-exclamation" : "fa-circle-check"}" aria-hidden="true"></i><span></span>`;
    const span = banner.querySelector("span");
    if (span) span.textContent = statusMessage.text;
    shell.appendChild(banner);
  }

  if (activeTab === "shortcuts") {
    shell.appendChild(renderShortcutsPanel());
  } else if (activeTab === "skins") {
    shell.appendChild(renderSiteSkinsPanel());
  } else if (activeTab === "memory") {
    shell.appendChild(renderMemoryPanel());
  } else if (activeTab === "blocker") {
    shell.appendChild(renderBlockerPanel());
  } else if (activeTab === "session") {
    shell.appendChild(renderSessionPermissionsPanel());
  } else if (activeTab === "import") {
    shell.appendChild(renderImportPanel());
  } else {
    shell.appendChild(renderQuickShortcutsPanel());
  }

  root.appendChild(shell);
}

function renderShortcutsPanel(): HTMLElement {
  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";

  const title = document.createElement("h2");
  title.innerHTML = `<i class="fa-solid fa-keyboard" aria-hidden="true"></i><span>${l("Atalhos de teclado (padrão Google Chrome)", "Keyboard shortcuts (Chrome defaults)")}</span>`;
  head.appendChild(title);

  const resetBtn = document.createElement("button");
  resetBtn.type = "button";
  resetBtn.className = "btn";
  resetBtn.innerHTML = `<i class="fa-solid fa-rotate-left" aria-hidden="true"></i><span>${l("Restaurar padrão Chrome", "Reset to Chrome defaults")}</span>`;
  resetBtn.addEventListener("click", () => {
    void callBridge<CoreSettings>("shortcuts.reset").then((updated) => {
      if (updated) {
        currentSettings = updated;
        statusMessage = {
          text: l(
            "Atalhos restaurados para o padrão do Google Chrome.",
            "Shortcuts restored to Google Chrome defaults.",
          ),
          isError: false,
        };
      }
      render();
    });
  });
  head.appendChild(resetBtn);
  card.appendChild(head);

  const hint = document.createElement("p");
  hint.className = "muted";
  hint.textContent = l(
    "Clique em qualquer atalho e pressione a nova combinação (ex: Ctrl+Q, Alt+1, F2). Pressione Esc para cancelar. Aliases do Chrome como F5 e Alt+D também funcionam.",
    "Click any shortcut and press a new key combination (e.g. Ctrl+Q, Alt+1, F2). Press Esc to cancel. Chrome aliases like F5 and Alt+D also work.",
  );
  card.appendChild(hint);

  const groups: ["tabs" | "nav" | "windows" | "ui", string][] = [
    ["tabs", l("Abas", "Tabs")],
    ["nav", l("Navegação", "Navigation")],
    ["windows", l("Janelas", "Windows")],
    ["ui", l("Zoom e interface", "Zoom & interface")],
  ];

  const boundMap = currentSettings.shortcuts ?? {};

  for (const [groupKey, groupLabel] of groups) {
    const gt = document.createElement("div");
    gt.className = "shortcut-group-title";
    gt.textContent = groupLabel;
    card.appendChild(gt);

    const list = document.createElement("div");
    list.className = "shortcut-list";

    for (const meta of SHORTCUT_CATALOG.filter((s) => s.group === groupKey)) {
      const row = document.createElement("div");
      row.className = "shortcut-row";

      const name = document.createElement("span");
      name.className = "shortcut-name";
      name.textContent = isEn() ? meta.en : meta.pt;
      row.appendChild(name);

      const isRec = recordingActionId === meta.id;
      const kbd = document.createElement("button");
      kbd.type = "button";
      kbd.className = `kbd-btn${isRec ? " recording" : ""}`;
      kbd.textContent = isRec
        ? l("Pressione teclas...", "Press keys...")
        : boundMap[meta.id] || l("Sem atalho", "Unbound");
      kbd.addEventListener("click", () => {
        recordingActionId = isRec ? null : meta.id;
        render();
      });
      row.appendChild(kbd);

      list.appendChild(row);
    }

    card.appendChild(list);
  }

  return card;
}

function renderImportPanel(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.style.display = "flex";
  wrap.style.flexDirection = "column";
  wrap.style.gap = "20px";

  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";

  const title = document.createElement("h2");
  title.innerHTML = `<i class="fa-solid fa-file-import" aria-hidden="true"></i><span>${l("Importar dados de outro navegador", "Import data from another browser")}</span>`;
  head.appendChild(title);

  const rescanBtn = document.createElement("button");
  rescanBtn.type = "button";
  rescanBtn.className = "btn";
  rescanBtn.innerHTML = `<i class="fa-solid fa-arrows-rotate" aria-hidden="true"></i><span>${l("Detectar novamente", "Rescan browsers")}</span>`;
  rescanBtn.addEventListener("click", () => {
    void refreshProfilesAndData();
  });
  head.appendChild(rescanBtn);
  card.appendChild(head);

  const desc = document.createElement("p");
  desc.className = "muted";
  desc.textContent = l(
    "Detecta perfis locais do Google Chrome, Microsoft Edge, Brave, Opera, Opera GX, Vivaldi e Mozilla Firefox. Funciona mesmo com o outro navegador aberto (importa Favoritos, Histórico e opcionalmente substitui os CDs da Nova Aba pelos sites mais acessados).",
    "Detects local profiles for Google Chrome, Microsoft Edge, Brave, Opera, Opera GX, Vivaldi, and Mozilla Firefox. Safe to run while the other browser is open (imports Bookmarks, History, and optionally updates New Tab CD shortcuts with your top sites).",
  );
  card.appendChild(desc);

  if (detectedProfiles.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = l(
      "Nenhum perfil de outro navegador foi encontrado neste PC.",
      "No other browser profiles were found on this PC.",
    );
    card.appendChild(empty);
  } else {
    const grid = document.createElement("div");
    grid.className = "profile-grid";

    for (const prof of detectedProfiles) {
      const pCard = document.createElement("div");
      pCard.className = `profile-card${selectedProfileId === prof.id ? " selected" : ""}`;
      pCard.innerHTML = `
        <div class="profile-top">
          <i class="${browserIconClass(prof.browser_name)}" aria-hidden="true"></i>
          <span></span>
        </div>
        <div class="profile-meta">
          <span><i class="fa-solid fa-user" aria-hidden="true"></i> ${escapeHtml(prof.profile_name)}</span>
          ${prof.has_bookmarks ? `<span><i class="fa-solid fa-bookmark" aria-hidden="true"></i> ${l("Favoritos", "Bookmarks")}</span>` : ""}
          ${prof.has_history ? `<span><i class="fa-solid fa-clock-rotate-left" aria-hidden="true"></i> ${l("Histórico", "History")}</span>` : ""}
        </div>
      `;
      const nameSpan = pCard.querySelector(".profile-top span");
      if (nameSpan) nameSpan.textContent = prof.browser_name;
      pCard.addEventListener("click", () => {
        selectedProfileId = prof.id;
        render();
      });
      grid.appendChild(pCard);
    }
    card.appendChild(grid);

    const checks = document.createElement("div");
    checks.className = "checks-row";

    checks.appendChild(
      makeCheckbox(l("Favoritos", "Bookmarks"), importBookmarks, (v) => {
        importBookmarks = v;
      }),
    );
    checks.appendChild(
      makeCheckbox(l("Histórico de navegação", "Browsing history"), importHistory, (v) => {
        importHistory = v;
      }),
    );
    checks.appendChild(
      makeCheckbox(
        l("Atalhos rápidos da Nova Aba (Top 6 sites)", "New Tab quick shortcuts (Top 6 sites)"),
        importQuickShortcuts,
        (v) => {
          importQuickShortcuts = v;
        },
      ),
    );
    card.appendChild(checks);

    const actions = document.createElement("div");
    const importBtn = document.createElement("button");
    importBtn.type = "button";
    importBtn.className = "btn btn-primary";
    importBtn.disabled = importing || !selectedProfileId;
    importBtn.innerHTML = `<i class="fa-solid fa-download" aria-hidden="true"></i><span>${importing ? l("Importando...", "Importing...") : l("Importar dados selecionados", "Import selected data")}</span>`;
    importBtn.addEventListener("click", () => {
      if (!selectedProfileId || importing) return;
      importing = true;
      statusMessage = null;
      render();
      void callBridge<ImportReport>("import.run", {
        profile_id: selectedProfileId,
        import_bookmarks: importBookmarks,
        import_history: importHistory,
        import_quick_shortcuts: importQuickShortcuts,
      })
        .then(async (report) => {
          importing = false;
          if (report) {
            statusMessage = {
              text: l(
                `Importado de ${report.browser_name} (${report.profile_name}): ${report.bookmarks_imported} favoritos, ${report.history_imported} itens de histórico${report.quick_shortcuts.length > 0 ? `, ${report.quick_shortcuts.length} atalhos na Nova Aba` : ""}.`,
                `Imported from ${report.browser_name} (${report.profile_name}): ${report.bookmarks_imported} bookmarks, ${report.history_imported} history items${report.quick_shortcuts.length > 0 ? `, ${report.quick_shortcuts.length} New Tab shortcuts` : ""}.`,
              ),
              isError: false,
            };
          }
          await refreshProfilesAndData();
        })
        .catch((err: unknown) => {
          importing = false;
          statusMessage = {
            text: err instanceof Error ? err.message : String(err),
            isError: true,
          };
          render();
        });
    });
    actions.appendChild(importBtn);
    card.appendChild(actions);
  }

  wrap.appendChild(card);

  // Preview of imported bookmarks & history in Seu Navegador's database.
  const dataCard = document.createElement("section");
  dataCard.className = "panel-card";
  dataCard.innerHTML = `
    <div class="panel-head">
      <h2><i class="fa-solid fa-database" aria-hidden="true"></i><span>${l("Dados salvos no Seu Navegador", "Saved data in Seu Navegador")}</span></h2>
    </div>
  `;

  const cols = document.createElement("div");
  cols.className = "data-columns";

  cols.appendChild(
    renderLinkCollection(
      `${l("Favoritos", "Bookmarks")} (${dataSummary.bookmarks.length})`,
      "fa-solid fa-bookmark",
      dataSummary.bookmarks.map((b) => ({ title: b.title, url: b.url })),
      l("Nenhum favorito importado ainda.", "No bookmarks imported yet."),
    ),
  );
  cols.appendChild(
    renderLinkCollection(
      `${l("Histórico recente", "Recent history")} (${dataSummary.history.length})`,
      "fa-solid fa-clock-rotate-left",
      dataSummary.history.map((h) => ({ title: h.title, url: h.url })),
      l("Nenhum histórico importado ainda.", "No history imported yet."),
    ),
  );
  dataCard.appendChild(cols);
  wrap.appendChild(dataCard);

  return wrap;
}

function renderLinkCollection(
  heading: string,
  iconClass: string,
  items: { title: string; url: string }[],
  emptyText: string,
): HTMLElement {
  const box = document.createElement("div");
  box.style.display = "flex";
  box.style.flexDirection = "column";
  box.style.gap = "8px";

  const h = document.createElement("div");
  h.className = "shortcut-group-title";
  h.innerHTML = `<i class="${iconClass}" aria-hidden="true"></i> ${escapeHtml(heading)}`;
  box.appendChild(h);

  const list = document.createElement("div");
  list.className = "data-list";

  if (items.length === 0) {
    const p = document.createElement("p");
    p.className = "muted";
    p.textContent = emptyText;
    list.appendChild(p);
  } else {
    for (const item of items) {
      const a = document.createElement("a");
      a.className = "data-item";
      a.href = item.url;
      a.addEventListener("click", (ev) => {
        if (!ev.ctrlKey && !ev.metaKey) {
          ev.preventDefault();
          void navigateInActiveTab(item.url);
        }
      });
      const tEl = document.createElement("span");
      tEl.className = "data-item-title";
      tEl.textContent = item.title;
      const uEl = document.createElement("span");
      uEl.className = "data-item-url";
      uEl.textContent = item.url;
      a.appendChild(tEl);
      a.appendChild(uEl);
      list.appendChild(a);
    }
  }
  box.appendChild(list);
  return box;
}

function renderQuickShortcutsPanel(): HTMLElement {
  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";
  head.innerHTML = `<h2><i class="fa-solid fa-compact-disc" aria-hidden="true"></i><span>${l("Atalhos rápidos da Nova Aba (CDs)", "New Tab Quick Shortcuts (CDs)")}</span></h2>`;
  card.appendChild(head);

  const form = document.createElement("form");
  form.className = "form-inline";

  const nameInput = document.createElement("input");
  nameInput.className = "text-input";
  nameInput.placeholder = l("Nome (ex: Twitch)", "Name (e.g. Twitch)");

  const urlInput = document.createElement("input");
  urlInput.className = "text-input";
  urlInput.placeholder = "https://...";

  const addBtn = document.createElement("button");
  addBtn.type = "submit";
  addBtn.className = "btn btn-primary";
  addBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i><span>${l("Adicionar", "Add")}</span>`;

  form.appendChild(nameInput);
  form.appendChild(urlInput);
  form.appendChild(addBtn);

  form.addEventListener("submit", (ev) => {
    ev.preventDefault();
    const name = nameInput.value.trim();
    let url = urlInput.value.trim();
    if (!name || !url) return;
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      url = `https://${url}`;
    }
    const code = name.replace(/[^a-z0-9]/gi, "").slice(0, 2).toUpperCase() || "WW";
    const next: QuickShortcut[] = [
      ...(currentSettings.quick_shortcuts ?? []),
      {
        id: `custom-${Date.now()}`,
        code,
        name,
        url,
      },
    ];
    void callBridge<CoreSettings>("quickShortcuts.set", { items: next }).then((updated) => {
      if (updated) {
        currentSettings = updated;
        statusMessage = {
          text: l(`Atalho "${name}" adicionado à Nova Aba.`, `Shortcut "${name}" added to New Tab.`),
          isError: false,
        };
      }
      render();
    });
  });
  card.appendChild(form);

  const list = document.createElement("div");
  list.className = "shortcut-list";
  const items = currentSettings.quick_shortcuts ?? [];

  for (const item of items) {
    const row = document.createElement("div");
    row.className = "shortcut-row";

    const left = document.createElement("div");
    left.innerHTML = `<strong>${escapeHtml(item.code)}</strong> — ${escapeHtml(item.name)} <span class="muted">(${escapeHtml(item.url)})</span>`;
    row.appendChild(left);

    const delBtn = document.createElement("button");
    delBtn.type = "button";
    delBtn.className = "btn btn-danger";
    delBtn.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
    delBtn.addEventListener("click", () => {
      const next = items.filter((x) => x.id !== item.id);
      void callBridge<CoreSettings>("quickShortcuts.set", { items: next }).then((updated) => {
        if (updated) {
          currentSettings = updated;
        }
        render();
      });
    });
    row.appendChild(delBtn);
    list.appendChild(row);
  }
  card.appendChild(list);

  return card;
}

function getPolicy(): MemoryPolicy {
  return (
    currentSettings.memory_policy ?? {
      sleep_timeout_secs: 600,
      max_live_tabs: null,
      never_sleep_pinned: true,
      never_sleep_domains: ["meet.google.com", "discord.com", "web.whatsapp.com"],
    }
  );
}

function saveMemoryPolicy(next: MemoryPolicy): void {
  void callBridge<CoreSettings>("memory.setPolicy", { policy: next }).then((updated) => {
    if (updated) {
      currentSettings = updated;
      statusMessage = {
        text: l("Política de memória salva.", "Memory policy saved."),
        isError: false,
      };
    }
    render();
  });
}

function renderMemoryPanel(): HTMLElement {
  const policy = getPolicy();
  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";
  head.innerHTML = `<h2><i class="fa-solid fa-microchip" aria-hidden="true"></i><span>${l("Gerenciador de Memória e Tab Sleeping", "Memory Manager & Tab Sleeping")}</span></h2>`;

  const sleepBtn = document.createElement("button");
  sleepBtn.type = "button";
  sleepBtn.className = "btn btn-primary";
  sleepBtn.innerHTML = `<i class="fa-solid fa-moon" aria-hidden="true"></i><span>${l("Dormir abas inativas agora", "Sleep inactive tabs now")}</span>`;
  sleepBtn.addEventListener("click", () => {
    void callBridge<{ count: number }>("memory.sleepInactive").then((res) => {
      const n = res?.count ?? 0;
      statusMessage = {
        text: l(
          `${n} aba(s) elegível(is) colocada(s) para dormir. Medindo delta real de RAM...`,
          `${n} eligible tab(s) put to sleep. Measuring real OS RAM delta...`,
        ),
        isError: false,
      };
      render();
    });
  });
  head.appendChild(sleepBtn);
  card.appendChild(head);

  const summary = document.createElement("p");
  summary.className = "muted";
  summary.textContent = l(
    `Uso real atual (todos processos): ${currentMemory.total_private_mb} MB Private / ${currentMemory.total_working_set_mb} MB Working Set (${currentMemory.process_count} processos) — ${currentMemory.live_tabs_count} vivas, ${currentMemory.sleeping_tabs_count} dormindo, ${currentMemory.saved_mb} MB devolvidos ao PC.`,
    `Current real usage (all processes): ${currentMemory.total_private_mb} MB Private / ${currentMemory.total_working_set_mb} MB Working Set (${currentMemory.process_count} processes) — ${currentMemory.live_tabs_count} live, ${currentMemory.sleeping_tabs_count} sleeping, ${currentMemory.saved_mb} MB returned to PC.`,
  );
  card.appendChild(summary);

  const timeoutRow = document.createElement("div");
  timeoutRow.className = "shortcut-row";
  timeoutRow.innerHTML = `<span class="shortcut-name">${l("Dormir abas em segundo plano após", "Sleep background tabs after")}</span>`;
  const timeoutSel = document.createElement("select");
  timeoutSel.className = "text-input";
  timeoutSel.style.maxWidth = "200px";
  for (const [secs, label] of [
    [0, l("Desligado (manual)", "Off (manual only)")],
    [60, l("1 minuto", "1 minute")],
    [300, l("5 minutos", "5 minutes")],
    [600, l("10 minutos (padrão)", "10 minutes (default)")],
    [900, l("15 minutos", "15 minutes")],
    [1800, l("30 minutos", "30 minutes")],
    [3600, l("60 minutos", "60 minutes")],
  ] as const) {
    const opt = document.createElement("option");
    opt.value = String(secs);
    opt.textContent = label;
    if (policy.sleep_timeout_secs === secs) opt.selected = true;
    timeoutSel.appendChild(opt);
  }
  timeoutSel.addEventListener("change", () => {
    saveMemoryPolicy({ ...policy, sleep_timeout_secs: Number(timeoutSel.value) });
  });
  timeoutRow.appendChild(timeoutSel);
  card.appendChild(timeoutRow);

  const capRow = document.createElement("div");
  capRow.className = "shortcut-row";
  capRow.innerHTML = `<span class="shortcut-name">${l("Limite máximo de abas vivas (dorme a mais antiga)", "Max live tabs cap (sleeps LRU)")}</span>`;
  const capSel = document.createElement("select");
  capSel.className = "text-input";
  capSel.style.maxWidth = "200px";
  for (const [val, label] of [
    ["", l("Sem limite", "Unlimited")],
    ["3", "3"],
    ["5", "5"],
    ["8", "8"],
    ["12", "12"],
    ["20", "20"],
  ] as const) {
    const opt = document.createElement("option");
    opt.value = val;
    opt.textContent = label;
    if ((policy.max_live_tabs === null && val === "") || String(policy.max_live_tabs) === val) {
      opt.selected = true;
    }
    capSel.appendChild(opt);
  }
  capSel.addEventListener("change", () => {
    saveMemoryPolicy({
      ...policy,
      max_live_tabs: capSel.value ? Number(capSel.value) : null,
    });
  });
  capRow.appendChild(capSel);
  card.appendChild(capRow);

  const checks = document.createElement("div");
  checks.className = "checks-row";
  checks.appendChild(
    makeCheckbox(
      l("Nunca dormir abas fixadas", "Never sleep pinned tabs"),
      policy.never_sleep_pinned,
      (v) => saveMemoryPolicy({ ...policy, never_sleep_pinned: v }),
    ),
  );
  card.appendChild(checks);

  const domTitle = document.createElement("div");
  domTitle.className = "shortcut-group-title";
  domTitle.textContent = l(
    "Domínios que nunca dormem (ex: meet.google.com, discord.com)",
    "Never-sleep domains (e.g. meet.google.com, discord.com)",
  );
  card.appendChild(domTitle);

  const form = document.createElement("form");
  form.className = "form-inline";
  const domInput = document.createElement("input");
  domInput.className = "text-input";
  domInput.placeholder = "ex: web.whatsapp.com";
  const addBtn = document.createElement("button");
  addBtn.type = "submit";
  addBtn.className = "btn btn-primary";
  addBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i><span>${l("Adicionar domínio", "Add domain")}</span>`;
  form.appendChild(domInput);
  form.appendChild(addBtn);
  form.addEventListener("submit", (ev) => {
    ev.preventDefault();
    const cleaned = domInput.value
      .trim()
      .toLowerCase()
      .replace(/^https?:\/\//, "")
      .split("/")[0];
    if (!cleaned) return;
    if (!policy.never_sleep_domains.includes(cleaned)) {
      saveMemoryPolicy({
        ...policy,
        never_sleep_domains: [...policy.never_sleep_domains, cleaned],
      });
    }
  });
  card.appendChild(form);

  const domList = document.createElement("div");
  domList.className = "shortcut-list";
  for (const dom of policy.never_sleep_domains) {
    const row = document.createElement("div");
    row.className = "shortcut-row";
    const span = document.createElement("span");
    span.className = "shortcut-name";
    span.textContent = dom;
    row.appendChild(span);
    const del = document.createElement("button");
    del.type = "button";
    del.className = "btn btn-danger";
    del.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
    del.addEventListener("click", () => {
      saveMemoryPolicy({
        ...policy,
        never_sleep_domains: policy.never_sleep_domains.filter((d) => d !== dom),
      });
    });
    row.appendChild(del);
    domList.appendChild(row);
  }
  card.appendChild(domList);

  return card;
}

function renderBlockerPanel(): HTMLElement {
  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";
  head.innerHTML = `<h2><i class="fa-solid fa-shield-halved" aria-hidden="true"></i><span>${l("Bloqueador de Anúncios e Rastreadores", "Ad & Tracker Blocker")}</span></h2>`;

  const updateBtn = document.createElement("button");
  updateBtn.type = "button";
  updateBtn.className = "btn btn-primary";
  updateBtn.innerHTML = `<i class="fa-solid fa-arrows-rotate" aria-hidden="true"></i><span>${l("Atualizar listas (EasyList / uBlock)", "Update lists (EasyList / uBlock)")}</span>`;
  updateBtn.addEventListener("click", () => {
    void callBridge("blocker.updateLists").then(() => {
      statusMessage = {
        text: l(
          "Atualização em segundo plano iniciada (EasyList + EasyPrivacy + uBlock Filters).",
          "Background list update started (EasyList + EasyPrivacy + uBlock Filters).",
        ),
        isError: false,
      };
      render();
    });
  });
  head.appendChild(updateBtn);
  card.appendChild(head);

  const summary = document.createElement("p");
  summary.className = "muted";
  summary.textContent = l(
    `Motor Brave adblock com filtros de rede, cosméticos e scriptlets (YouTube). Bloqueados nesta sessão: ${currentBlocker.total_blocked} | Listas ativas: ${currentBlocker.lists_loaded}.`,
    `Brave adblock engine with network, cosmetic & scriptlet filters (YouTube). Blocked this session: ${currentBlocker.total_blocked} | Active lists: ${currentBlocker.lists_loaded}.`,
  );
  card.appendChild(summary);

  const checks = document.createElement("div");
  checks.className = "checks-row";
  checks.appendChild(
    makeCheckbox(
      l("Ativar bloqueador de anúncios e rastreadores", "Enable ad & tracker blocker"),
      currentBlocker.enabled,
      (v) => {
        void callBridge<BlockerStats>("blocker.setEnabled", { enabled: v }).then((stats) => {
          if (stats) currentBlocker = stats;
          render();
        });
      },
    ),
  );
  card.appendChild(checks);

  const allowTitle = document.createElement("div");
  allowTitle.className = "shortcut-group-title";
  allowTitle.textContent = l(
    "Sites na allowlist (bloqueador pausado)",
    "Allowlisted sites (blocker paused)",
  );
  card.appendChild(allowTitle);

  const form = document.createElement("form");
  form.className = "form-inline";
  const domInput = document.createElement("input");
  domInput.className = "text-input";
  domInput.placeholder = "ex: exemplo.com";
  const addBtn = document.createElement("button");
  addBtn.type = "submit";
  addBtn.className = "btn btn-primary";
  addBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i><span>${l("Adicionar à allowlist", "Add to allowlist")}</span>`;
  form.appendChild(domInput);
  form.appendChild(addBtn);
  form.addEventListener("submit", (ev) => {
    ev.preventDefault();
    const cleaned = domInput.value
      .trim()
      .toLowerCase()
      .replace(/^https?:\/\//, "")
      .replace(/^www\./, "")
      .split("/")[0];
    if (!cleaned) return;
    if (!currentBlocker.allowlisted_domains.includes(cleaned)) {
      void callBridge<BlockerStats>("blocker.toggleSite", { domain: cleaned }).then((stats) => {
        if (stats) currentBlocker = stats;
        render();
      });
    }
  });
  card.appendChild(form);

  const list = document.createElement("div");
  list.className = "shortcut-list";
  for (const dom of currentBlocker.allowlisted_domains) {
    const row = document.createElement("div");
    row.className = "shortcut-row";
    const span = document.createElement("span");
    span.className = "shortcut-name";
    span.textContent = dom;
    row.appendChild(span);
    const del = document.createElement("button");
    del.type = "button";
    del.className = "btn btn-danger";
    del.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
    del.addEventListener("click", () => {
      void callBridge<BlockerStats>("blocker.toggleSite", { domain: dom }).then((stats) => {
        if (stats) currentBlocker = stats;
        render();
      });
    });
    row.appendChild(del);
    list.appendChild(row);
  }
  card.appendChild(list);

  return card;
}

function renderSiteSkinsPanel(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.style.display = "flex";
  wrap.style.flexDirection = "column";
  wrap.style.gap = "20px";

  const globalCard = document.createElement("section");
  globalCard.className = "panel-card";
  globalCard.innerHTML = `<div class="panel-head"><h2><i class="fa-solid fa-wand-magic-sparkles" aria-hidden="true"></i><span>${l("Skins por Site (CSS + JS isolado)", "Per-Site Skins (CSS + isolated JS)")}</span></h2></div>`;

  const desc = document.createElement("p");
  desc.className = "muted";
  desc.textContent = l(
    "Temas podem estilizar sites específicos (ex: skin do YouTube no Y2K Maximal) injetando CSS no início do carregamento e JS em contexto isolado, sem acesso à API window.seu.",
    "Themes can restyle specific sites (e.g. YouTube skin in Y2K Maximal) by injecting anti-flash CSS at load start and JS in an isolated context with zero access to window.seu.",
  );
  globalCard.appendChild(desc);

  const skinsDisabled = currentSettings.site_skins_enabled === false;
  const checks = document.createElement("div");
  checks.className = "checks-row";
  const toggleRow = makeCheckbox(
    l(
      "Desativar skins de sites (interruptor global — remove CSS e JS de skins em todas as abas)",
      "Disable site skins (global kill-switch — removes skin CSS & JS across all tabs)",
    ),
    skinsDisabled,
    (disabledChecked) => {
      void callBridge<CoreSettings>("settings.setSiteSkinsEnabled", {
        enabled: !disabledChecked,
      }).then((updated) => {
        if (updated) currentSettings = updated;
        statusMessage = {
          text: disabledChecked
            ? l("Skins de sites desativadas em todas as abas.", "Site skins disabled across all tabs.")
            : l("Skins de sites ativadas.", "Site skins enabled."),
          isError: false,
        };
        render();
      });
    },
  );
  toggleRow.dataset.testId = "disable-site-skins-toggle";
  checks.appendChild(toggleRow);
  globalCard.appendChild(checks);

  const activeTheme = availableThemes.find((t) => t.id === currentSettings.active_theme_id);
  const activeSites = activeTheme?.site_skin_patterns ?? [];
  const activeInfo = document.createElement("p");
  activeInfo.className = "muted";
  activeInfo.style.marginTop = "12px";
  activeInfo.textContent =
    activeSites.length > 0
      ? l(
          `Tema ativo (${activeTheme?.name ?? currentSettings.active_theme_id}) inclui skins para: ${activeSites.join(", ")}.`,
          `Active theme (${activeTheme?.name ?? currentSettings.active_theme_id}) includes skins for: ${activeSites.join(", ")}.`,
        )
      : l(
          `O tema ativo (${activeTheme?.name ?? currentSettings.active_theme_id}) não declara skins de sites.`,
          `Active theme (${activeTheme?.name ?? currentSettings.active_theme_id}) does not declare site skins.`,
        );
  globalCard.appendChild(activeInfo);
  wrap.appendChild(globalCard);

  // Security opt-in section for user/imported themes containing site scripts.
  const secCard = document.createElement("section");
  secCard.className = "panel-card";
  secCard.innerHTML = `<div class="panel-head"><h2><i class="fa-solid fa-shield-halved" aria-hidden="true"></i><span>${l("Segurança de Scripts em Temas Importados", "Imported Theme Script Security")}</span></h2></div>`;

  const secDesc = document.createElement("p");
  secDesc.className = "muted";
  secDesc.textContent = l(
    "CSS de skins é sempre permitido quando as skins estão ligadas. Já o JS de temas importados/usuário vem DESLIGADO por padrão e exige sua autorização explícita abaixo. Scripts de skin nunca acessam window.seu.",
    "Skin CSS is always allowed when site skins are on. Site JS from imported/user themes is DISABLED by default and requires explicit opt-in below. Skin scripts never access window.seu.",
  );
  secCard.appendChild(secDesc);

  const allowedSet = new Set(currentSettings.allowed_site_script_themes ?? []);
  const customWithJs = availableThemes.filter((t) => !t.builtin && t.has_site_scripts);

  const list = document.createElement("div");
  list.className = "shortcut-list";
  if (customWithJs.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = l(
      "Nenhum tema importado com scripts de site instalado no momento.",
      "No imported themes containing site scripts are currently installed.",
    );
    list.appendChild(empty);
  } else {
    for (const item of customWithJs) {
      const allowed = allowedSet.has(item.id);
      const row = document.createElement("div");
      row.className = "shortcut-row";
      row.style.flexWrap = "wrap";
      row.style.gap = "8px";

      const info = document.createElement("span");
      info.className = "shortcut-name";
      info.textContent = `${item.name} (${item.id}) — ${(item.site_skin_patterns ?? []).join(", ")} · ${
        allowed ? l("JS PERMITIDO", "JS ALLOWED") : l("JS BLOQUEADO", "JS BLOCKED")
      }`;
      row.appendChild(info);

      if (allowed) {
        const revokeBtn = document.createElement("button");
        revokeBtn.type = "button";
        revokeBtn.className = "btn btn-danger";
        revokeBtn.innerHTML = `<i class="fa-solid fa-ban" aria-hidden="true"></i><span>${l("Revogar JS", "Revoke JS")}</span>`;
        revokeBtn.addEventListener("click", () => {
          pendingScriptOptInThemeId = null;
          void callBridge<CoreSettings>("themes.setSiteScriptsAllowed", {
            id: item.id,
            allowed: false,
          }).then((updated) => {
            if (updated) currentSettings = updated;
            statusMessage = {
              text: l(`Scripts de site revogados para "${item.name}".`, `Site scripts revoked for "${item.name}".`),
              isError: false,
            };
            render();
          });
        });
        row.appendChild(revokeBtn);
      } else if (pendingScriptOptInThemeId === item.id) {
        const confirmBtn = document.createElement("button");
        confirmBtn.type = "button";
        confirmBtn.className = "btn btn-primary";
        confirmBtn.innerHTML = `<i class="fa-solid fa-triangle-exclamation" aria-hidden="true"></i><span>${l("Confirmar: entendo os riscos e permito JS", "Confirm: I trust this theme's site JS")}</span>`;
        confirmBtn.addEventListener("click", () => {
          pendingScriptOptInThemeId = null;
          void callBridge<CoreSettings>("themes.setSiteScriptsAllowed", {
            id: item.id,
            allowed: true,
          }).then((updated) => {
            if (updated) currentSettings = updated;
            statusMessage = {
              text: l(`Scripts de site permitidos para "${item.name}".`, `Site scripts enabled for "${item.name}".`),
              isError: false,
            };
            render();
          });
        });
        const cancelBtn = document.createElement("button");
        cancelBtn.type = "button";
        cancelBtn.className = "btn";
        cancelBtn.textContent = l("Cancelar", "Cancel");
        cancelBtn.addEventListener("click", () => {
          pendingScriptOptInThemeId = null;
          render();
        });
        row.appendChild(confirmBtn);
        row.appendChild(cancelBtn);
      } else {
        const allowBtn = document.createElement("button");
        allowBtn.type = "button";
        allowBtn.className = "btn";
        allowBtn.innerHTML = `<i class="fa-solid fa-shield" aria-hidden="true"></i><span>${l("Permitir JS de sites...", "Allow site JS...")}</span>`;
        allowBtn.addEventListener("click", () => {
          pendingScriptOptInThemeId = item.id;
          render();
        });
        row.appendChild(allowBtn);
      }

      list.appendChild(row);
    }
  }
  secCard.appendChild(list);
  wrap.appendChild(secCard);

  return wrap;
}

function renderSessionPermissionsPanel(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.style.display = "flex";
  wrap.style.flexDirection = "column";
  wrap.style.gap = "20px";

  const sessCard = document.createElement("section");
  sessCard.className = "panel-card";
  sessCard.innerHTML = `<div class="panel-head"><h2><i class="fa-solid fa-rotate-left" aria-hidden="true"></i><span>${l("Restaurar Sessão e Páginas Internas", "Session Restore & Internal Pages")}</span></h2></div>`;

  const checks = document.createElement("div");
  checks.className = "checks-row";
  checks.appendChild(
    makeCheckbox(
      l(
        "Restaurar janelas, abas e grupos ao abrir (abas inativas iniciam dormindo para economizar RAM)",
        "Restore windows, tabs and groups on startup (inactive tabs start sleeping to save RAM)",
      ),
      currentSettings.restore_session_on_startup !== false,
      (v) => {
        void callBridge<CoreSettings>("settings.setRestoreSession", { enabled: v }).then(
          (updated) => {
            if (updated) currentSettings = updated;
            statusMessage = {
              text: l("Preferência de sessão salva.", "Session preference saved."),
              isError: false,
            };
            render();
          },
        );
      },
    ),
  );
  sessCard.appendChild(checks);

  const navRow = document.createElement("div");
  navRow.className = "settings-tabs";
  for (const [href, ic, lbl] of [
    ["seu://history/", "fa-solid fa-clock-rotate-left", l("Abrir Histórico", "Open History")],
    ["seu://bookmarks/", "fa-solid fa-bookmark", l("Abrir Favoritos", "Open Bookmarks")],
    ["seu://downloads/", "fa-solid fa-download", l("Abrir Downloads", "Open Downloads")],
  ] as const) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "btn";
    b.innerHTML = `<i class="${ic}" aria-hidden="true"></i><span>${lbl}</span>`;
    b.addEventListener("click", () => void navigateInActiveTab(href));
    navRow.appendChild(b);
  }
  sessCard.appendChild(navRow);
  wrap.appendChild(sessCard);

  const permCard = document.createElement("section");
  permCard.className = "panel-card";
  permCard.innerHTML = `<div class="panel-head"><h2><i class="fa-solid fa-lock" aria-hidden="true"></i><span>${l("Permissões de sites salvas", "Saved site permissions")} (${savedPermissions.length})</span></h2></div>`;

  const list = document.createElement("div");
  list.className = "shortcut-list";
  if (savedPermissions.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = l(
      "Nenhuma permissão salva ainda. Quando um site pedir câmera, microfone, notificações ou localização, sua decisão aparecerá aqui.",
      "No saved permissions yet. When a site requests camera, mic, notifications, or location, your decision appears here.",
    );
    list.appendChild(empty);
  } else {
    for (const p of savedPermissions) {
      const row = document.createElement("div");
      row.className = "shortcut-row";
      const lbl = document.createElement("span");
      lbl.className = "shortcut-name";
      lbl.textContent = `${p.origin} — ${p.permission} (${p.decision.toUpperCase()})`;
      row.appendChild(lbl);

      const del = document.createElement("button");
      del.type = "button";
      del.className = "btn btn-danger";
      del.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
      del.addEventListener("click", () => {
        void callBridge<SitePermissionItem[]>("permissions.delete", {
          origin: p.origin,
          permission: p.permission,
        }).then((items) => {
          savedPermissions = items ?? [];
          render();
        });
      });
      row.appendChild(del);
      list.appendChild(row);
    }
  }
  permCard.appendChild(list);
  wrap.appendChild(permCard);

  return wrap;
}

function makeCheckbox(
  label: string,
  checked: boolean,
  onChange: (val: boolean) => void,
): HTMLElement {
  const el = document.createElement("label");
  el.className = "check-label";
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = checked;
  input.addEventListener("change", () => onChange(input.checked));
  el.appendChild(input);
  el.appendChild(document.createTextNode(label));
  return el;
}

function escapeHtml(raw: string): string {
  return raw
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

onBridgeEvent((event) => {
  if (event.type === "settings") {
    currentSettings = event.settings;
    setLocale(event.settings.locale);
    render();
  } else if (event.type === "theme") {
    applyThemeTokens(event.theme);
  } else if (event.type === "memory") {
    currentMemory = event.stats;
    if (activeTab === "memory") render();
  } else if (event.type === "blocker") {
    currentBlocker = event.stats;
    if (activeTab === "blocker") render();
  }
});

async function bootstrap(): Promise<void> {
  const [settings, theme, mem, blk, perms, themeList] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
    callBridge<MemoryStats>("memory.getStats").catch(() => undefined),
    callBridge<BlockerStats>("blocker.getStats").catch(() => undefined),
    callBridge<SitePermissionItem[]>("permissions.list").catch(() => []),
    callBridge<ThemeSummary[]>("themes.list").catch(() => []),
  ]);
  if (settings) {
    currentSettings = settings;
    setLocale(settings.locale);
  }
  if (theme) {
    applyThemeTokens(theme);
  }
  if (mem) {
    currentMemory = mem;
  }
  if (blk) {
    currentBlocker = blk;
  }
  savedPermissions = perms ?? [];
  availableThemes = Array.isArray(themeList) ? themeList : [];
  await refreshProfilesAndData();
}

void bootstrap();
