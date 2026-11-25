import "./icons";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type MemoryStats,
  type WindowSnapshot,
} from "./bridge";
import { getLocale, setLocale, t } from "./i18n";

const windowId = getWindowIdFromQuery();
const menuList = document.getElementById("menu-list") as HTMLDivElement;

let openedAt = 0;
let currentPanel: "menu" | "memory" = "menu";
let currentSnapshot: WindowSnapshot | null = null;
let currentMemory: MemoryStats = {
  total_private_mb: 0,
  total_working_set_mb: 0,
  process_count: 0,
  live_tabs_count: 1,
  sleeping_tabs_count: 0,
  saved_mb: 0,
  shared_renderer_detected: false,
};
let currentSettings: CoreSettings = {
  locale: "pt-br",
  search_engine: "duck-duck-go",
  active_theme_id: "y2k-maximal",
  game_mode_enabled: false,
};

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

function closeOverlay(): void {
  void callBridge("overlay.close", { window_id: windowId });
}

function addActionButton(iconClass: string, label: string, onClick: () => void): void {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "menu-btn";

  const icon = document.createElement("i");
  icon.className = iconClass;
  icon.setAttribute("aria-hidden", "true");
  btn.appendChild(icon);

  const span = document.createElement("span");
  span.textContent = label;
  btn.appendChild(span);

  btn.addEventListener("click", () => {
    onClick();
    closeOverlay();
  });
  menuList.appendChild(btn);
}

function renderActivePanel(): void {
  if (currentPanel === "memory") {
    renderMemoryPopover();
  } else {
    renderMenu();
  }
}

function renderMenu(): void {
  menuList.innerHTML = "";

  addActionButton("fa-solid fa-window-maximize", t("menu.newWindow"), () => {
    void callBridge("windows.create", { incognito: false });
  });

  addActionButton("fa-solid fa-user-secret", t("menu.newIncognito"), () => {
    void callBridge("windows.create", { incognito: true });
  });

  addActionButton("fa-solid fa-layer-group", t("menu.newGroup"), () => {
    void (async () => {
      const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      });
      if (snap?.active_tab_id) {
        const colors = ["#FF2E93", "#4FD8FF", "#B6FF3B", "#8A7CFF"];
        const color = colors[snap.groups.length % colors.length] ?? "#4FD8FF";
        await callBridge("tabGroups.create", {
          window_id: windowId,
          title: `Grupo ${snap.groups.length + 1}`,
          color,
          tab_ids: [snap.active_tab_id],
        });
      }
    })();
  });

  addActionButton("fa-solid fa-rotate-left", t("menu.reopenClosed"), () => {
    void callBridge("tabs.reopenClosed", { window_id: windowId });
  });

  addActionButton("fa-solid fa-moon", t("menu.sleepInactive"), () => {
    void callBridge("memory.sleepInactive");
  });

  addActionButton("fa-solid fa-sliders", t("menu.settings"), () => {
    void (async () => {
      const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      });
      if (snap?.active_tab_id) {
        await callBridge("tabs.navigate", {
          tab_id: snap.active_tab_id,
          input: "seu://settings/",
        });
      }
    })();
  });

  const sep = document.createElement("div");
  sep.className = "menu-sep";
  menuList.appendChild(sep);

  const langLabel = document.createElement("div");
  langLabel.className = "menu-label";
  langLabel.textContent = t("menu.language");
  menuList.appendChild(langLabel);

  const langRow = document.createElement("div");
  langRow.className = "pill-row";
  for (const [code, label] of [
    ["pt-BR", "Português"],
    ["en", "English"],
  ] as const) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "pill-btn";
    if (getLocale() === code.toLowerCase()) {
      btn.classList.add("active");
    }
    btn.textContent = label;
    btn.addEventListener("click", () => {
      void callBridge<CoreSettings>("settings.setLocale", { locale: code }).then(
        (updated) => {
          if (updated) {
            currentSettings = updated;
            setLocale(updated.locale);
            renderActivePanel();
          }
        },
      );
    });
    langRow.appendChild(btn);
  }
  menuList.appendChild(langRow);

  const engineLabel = document.createElement("div");
  engineLabel.className = "menu-label";
  engineLabel.textContent = t("menu.searchEngine");
  menuList.appendChild(engineLabel);

  const engineRow = document.createElement("div");
  engineRow.className = "pill-row";
  for (const [engine, label] of [
    ["duck-duck-go", "DDG"],
    ["google", "Google"],
    ["brave", "Brave"],
  ] as const) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "pill-btn";
    if (currentSettings.search_engine === engine) {
      btn.classList.add("active");
    }
    btn.textContent = label;
    btn.addEventListener("click", () => {
      void callBridge<CoreSettings>("settings.setSearchEngine", { engine }).then(
        (updated) => {
          if (updated) {
            currentSettings = updated;
            renderActivePanel();
          }
        },
      );
    });
    engineRow.appendChild(btn);
  }
  menuList.appendChild(engineRow);
}

function renderMemoryPopover(): void {
  menuList.innerHTML = "";
  menuList.style.height = "100%";

  const header = document.createElement("div");
  header.className = "menu-label";
  header.innerHTML = `<i class="fa-solid fa-microchip" aria-hidden="true"></i> ${l("Gerenciador de Memória", "Memory Manager")}`;
  menuList.appendChild(header);

  const grid = document.createElement("div");
  grid.className = "mem-stats-grid";
  const boxes: [string, string][] = [
    [`${currentMemory.total_private_mb} MB`, l("RAM total (Private)", "Total RAM (Private)")],
    [`${currentMemory.saved_mb} MB`, l("Devolvidos ao PC", "Returned to PC")],
    [
      `${currentMemory.live_tabs_count} / ${currentMemory.sleeping_tabs_count}`,
      l("Vivas / Dormindo", "Live / Sleeping"),
    ],
    [
      `${currentMemory.process_count}`,
      l("Processos CEF", "CEF processes"),
    ],
  ];
  for (const [val, lbl] of boxes) {
    const box = document.createElement("div");
    box.className = "mem-stat-box";
    const v = document.createElement("span");
    v.className = "mem-stat-val";
    v.textContent = val;
    const tEl = document.createElement("span");
    tEl.className = "mem-stat-lbl";
    tEl.textContent = lbl;
    box.appendChild(v);
    box.appendChild(tEl);
    grid.appendChild(box);
  }
  menuList.appendChild(grid);

  if (currentMemory.shared_renderer_detected) {
    const note = document.createElement("div");
    note.className = "mem-note";
    note.textContent = l(
      "Renderer compartilhado detectado: a RAM cai quando todas abas do mesmo processo dormirem.",
      "Shared renderer detected: OS RAM drops once all tabs sharing that process sleep.",
    );
    menuList.appendChild(note);
  }

  const sleepBtn = document.createElement("button");
  sleepBtn.type = "button";
  sleepBtn.className = "menu-btn";
  sleepBtn.innerHTML = `<i class="fa-solid fa-moon" aria-hidden="true"></i><span>${l("Dormir abas inativas agora", "Sleep inactive tabs now")}</span>`;
  sleepBtn.addEventListener("click", () => {
    void callBridge("memory.sleepInactive").then(() => refreshSnapshotAndMemory());
  });
  menuList.appendChild(sleepBtn);

  const tabs = currentSnapshot?.tabs ?? [];
  const sleepingTabs = tabs.filter((tab) => tab.state === "sleeping");
  const listLabel = document.createElement("div");
  listLabel.className = "menu-label";
  listLabel.textContent = `${l("Abas dormindo", "Sleeping tabs")} (${sleepingTabs.length})`;
  menuList.appendChild(listLabel);

  const list = document.createElement("div");
  list.className = "mem-tabs-list";

  if (sleepingTabs.length === 0) {
    const empty = document.createElement("div");
    empty.className = "mem-stat-lbl";
    empty.style.padding = "4px";
    empty.textContent = l(
      "Nenhuma aba dormindo agora. Clique no botão acima para liberar RAM das abas em segundo plano.",
      "No tabs sleeping yet. Click above to free RAM from background tabs.",
    );
    list.appendChild(empty);
  } else {
    for (const tab of sleepingTabs) {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "mem-tab-card";

      if (tab.has_thumbnail) {
        const img = document.createElement("img");
        img.className = "mem-thumb";
        img.src = `seu://thumb/tab_${tab.id}.jpg`;
        img.alt = "";
        card.appendChild(img);
      }

      const info = document.createElement("div");
      info.className = "mem-tab-info";
      const title = document.createElement("span");
      title.className = "mem-tab-title";
      title.textContent = tab.title;
      const sub = document.createElement("span");
      sub.className = "mem-tab-sub";
      sub.textContent = tab.url;
      info.appendChild(title);
      info.appendChild(sub);
      card.appendChild(info);

      card.addEventListener("click", () => {
        void callBridge("tabs.activate", { window_id: windowId, tab_id: tab.id });
        closeOverlay();
      });
      list.appendChild(card);
    }
  }
  menuList.appendChild(list);

  const cfgBtn = document.createElement("button");
  cfgBtn.type = "button";
  cfgBtn.className = "menu-btn";
  cfgBtn.innerHTML = `<i class="fa-solid fa-sliders" aria-hidden="true"></i><span>${l("Políticas de memória", "Memory policies")}</span>`;
  cfgBtn.addEventListener("click", () => {
    void (async () => {
      const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      });
      if (snap?.active_tab_id) {
        await callBridge("tabs.navigate", {
          tab_id: snap.active_tab_id,
          input: "seu://settings/",
        });
      }
      closeOverlay();
    })();
  });
  menuList.appendChild(cfgBtn);
}

async function refreshSnapshotAndMemory(): Promise<void> {
  const [snap, mem] = await Promise.all([
    callBridge<WindowSnapshot>("windows.getSnapshot", { window_id: windowId }).catch(
      () => undefined,
    ),
    callBridge<MemoryStats>("memory.getStats").catch(() => undefined),
  ]);
  if (snap) currentSnapshot = snap;
  if (mem) currentMemory = mem;
  renderActivePanel();
}

window.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape") {
    closeOverlay();
  }
});

window.addEventListener("blur", () => {
  if (Date.now() - openedAt > 180) {
    closeOverlay();
  }
});

onBridgeEvent((event) => {
  if (event.type === "overlay" && event.panel !== "closed") {
    openedAt = Date.now();
    currentPanel = event.panel === "memory" ? "memory" : "menu";
    void refreshSnapshotAndMemory();
  } else if (event.type === "memory") {
    currentMemory = event.stats;
    if (currentPanel === "memory") renderActivePanel();
  } else if (event.type === "state") {
    currentSnapshot = event.snapshot;
    if (currentPanel === "memory") renderActivePanel();
  } else if (event.type === "settings") {
    currentSettings = event.settings;
    setLocale(event.settings.locale);
    renderActivePanel();
  } else if (event.type === "theme") {
    applyThemeTokens(event.theme);
  }
});

async function bootstrap(): Promise<void> {
  try {
    const [settings, theme, snap, mem] = await Promise.all([
      callBridge<CoreSettings>("settings.get").catch(() => undefined),
      callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
      callBridge<WindowSnapshot>("windows.getSnapshot", { window_id: windowId }).catch(
        () => undefined,
      ),
      callBridge<MemoryStats>("memory.getStats").catch(() => undefined),
    ]);
    if (settings) {
      currentSettings = settings;
      setLocale(settings.locale);
    }
    if (theme) {
      applyThemeTokens(theme);
    }
    if (snap) {
      currentSnapshot = snap;
    }
    if (mem) {
      currentMemory = mem;
    }
  } catch {
    // Keep defaults.
  }
  renderActivePanel();
}

void bootstrap();
