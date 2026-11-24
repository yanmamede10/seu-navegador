import "./icons";
import "./settings.css";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type DataSummaryPayload,
  type DetectedBrowserProfile,
  type ImportReport,
  type LoadedTheme,
  type QuickShortcut,
  type WindowSnapshot,
} from "./bridge";
import { setLocale } from "./i18n";

const root = document.getElementById("settings-root") as HTMLDivElement;
const windowId = getWindowIdFromQuery();

type SettingsTab = "shortcuts" | "import" | "quick";

let activeTab: SettingsTab = "shortcuts";
let currentSettings: CoreSettings = {
  locale: "pt-br",
  search_engine: "duck-duck-go",
  active_theme_id: "y2k-maximal",
  game_mode_enabled: false,
  shortcuts: {},
  quick_shortcuts: [],
};
let detectedProfiles: DetectedBrowserProfile[] = [];
let selectedProfileId = "";
let importBookmarks = true;
let importHistory = true;
let importQuickShortcuts = false;
let importing = false;
let statusMessage: { text: string; isError: boolean } | null = null;
let dataSummary: DataSummaryPayload = { bookmarks: [], history: [] };
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
    ["shortcuts", "fa-solid fa-keyboard", l("Atalhos de teclado", "Keyboard shortcuts")],
    ["import", "fa-solid fa-file-import", l("Importar de outro navegador", "Import from browser")],
    ["quick", "fa-solid fa-compact-disc", l("Atalhos da Nova Aba", "New Tab shortcuts")],
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
  }
});

async function bootstrap(): Promise<void> {
  const [settings, theme] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
  ]);
  if (settings) {
    currentSettings = settings;
    setLocale(settings.locale);
  }
  if (theme) {
    applyThemeTokens(theme);
  }
  await refreshProfilesAndData();
}

void bootstrap();
