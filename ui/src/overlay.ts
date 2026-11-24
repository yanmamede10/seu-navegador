import "./icons";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type WindowSnapshot,
} from "./bridge";
import { getLocale, setLocale, t } from "./i18n";

const windowId = getWindowIdFromQuery();
const menuList = document.getElementById("menu-list") as HTMLDivElement;

let openedAt = 0;
let currentSettings: CoreSettings = {
  locale: "pt-br",
  search_engine: "duck-duck-go",
  active_theme_id: "y2k-maximal",
  game_mode_enabled: false,
};

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
            renderMenu();
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
            renderMenu();
          }
        },
      );
    });
    engineRow.appendChild(btn);
  }
  menuList.appendChild(engineRow);
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
  } else if (event.type === "settings") {
    currentSettings = event.settings;
    setLocale(event.settings.locale);
    renderMenu();
  } else if (event.type === "theme") {
    applyThemeTokens(event.theme);
  }
});

async function bootstrap(): Promise<void> {
  try {
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
  } catch {
    // Keep defaults.
  }
  renderMenu();
}

void bootstrap();
