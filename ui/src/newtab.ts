import "./icons";
import {
  callBridge,
  getTabIdFromQuery,
  getWindowIdFromQuery,
  onBridgeEvent,
  type BlockerStats,
  type CoreSettings,
  type LoadedTheme,
  type MemoryStats,
  type ThemeSummary,
  type WindowSnapshot,
} from "./bridge";
import { DeclarativeRuntime, type RepeatItem } from "./declarative";
import { setLocale, t } from "./i18n";

const windowId = getWindowIdFromQuery();
const selfTabIdFromUrl = getTabIdFromQuery();
const mountEl = document.getElementById("theme-mount") as HTMLDivElement;

async function resolveOwnTabId(): Promise<number | null> {
  if (selfTabIdFromUrl !== null) {
    return selfTabIdFromUrl;
  }
  const snap =
    currentSnapshot ??
    (await callBridge<WindowSnapshot>("windows.getSnapshot", {
      window_id: windowId,
    }).catch(() => undefined));
  if (!snap) return null;
  if (snap.split_tab_id) {
    const splitTab = snap.tabs.find((tab) => tab.id === snap.split_tab_id);
    if (splitTab?.url.startsWith("seu://newtab/")) {
      return splitTab.id;
    }
  }
  const activeTab = snap.tabs.find((tab) => tab.id === snap.active_tab_id);
  if (activeTab?.url.startsWith("seu://newtab/")) {
    return activeTab.id;
  }
  const anyNewTab = snap.tabs.find((tab) => tab.url.startsWith("seu://newtab/"));
  return anyNewTab?.id ?? snap.active_tab_id ?? null;
}

type SearchCategoryId = "web" | "videos" | "images" | "code" | "docs";

interface ShortcutItem {
  id: string;
  code: string;
  name: string;
  url: string;
}

let shortcuts: ShortcutItem[] = [
  { id: "yt", code: "YT", name: "YouTube", url: "https://www.youtube.com" },
  { id: "gh", code: "GH", name: "GitHub", url: "https://github.com" },
  { id: "rs", code: "RS", name: "docs.rs", url: "https://docs.rs" },
  { id: "gd", code: "GD", name: "Godot", url: "https://godotengine.org" },
  { id: "io", code: "IO", name: "itch.io", url: "https://itch.io" },
  { id: "st", code: "ST", name: "Steam", url: "https://store.steampowered.com" },
];

let activeCategory: SearchCategoryId = "web";
let alertDismissed = false;
let currentTheme: LoadedTheme | null = null;
let themeList: ThemeSummary[] = [];
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
let currentBlocker: BlockerStats = {
  enabled: true,
  total_blocked: 0,
  allowlisted_domains: [],
  recent_blocked: [],
  lists_loaded: 1,
  last_updated_secs: null,
};
let currentSettings: CoreSettings = {
  locale: "pt-br",
  search_engine: "duck-duck-go",
  active_theme_id: "y2k-maximal",
  game_mode_enabled: false,
};

const runtime = new DeclarativeRuntime(mountEl, {
  onSubmit: (name, value) => {
    if (name !== "search") return;
    const query = value.trim();
    if (!query) return;

    void (async () => {
      const targetTabId = await resolveOwnTabId();
      if (targetTabId !== null) {
        await callBridge("search.submit", {
          tab_id: targetTabId,
          query,
          category: activeCategory,
        });
      }
    })();
  },
  onReorder: (collection, draggedId, toIndex) => {
    if (collection !== "shortcuts") return;
    const fromIdx = shortcuts.findIndex((s) => s.id === draggedId);
    if (fromIdx >= 0 && toIndex >= 0 && toIndex < shortcuts.length) {
      const [moved] = shortcuts.splice(fromIdx, 1);
      if (moved) {
        shortcuts.splice(toIndex, 0, moved);
        renderAll();
        void callBridge("quickShortcuts.set", { items: shortcuts });
      }
    }
  },
  onAction: ({ action, param }) => {
    switch (action) {
      case "search.setCategory":
        if (
          param === "web" ||
          param === "videos" ||
          param === "images" ||
          param === "code" ||
          param === "docs"
        ) {
          activeCategory = param;
          renderAll();
        }
        break;
      case "themes.setActive":
        if (param) {
          void callBridge("themes.setActive", { id: param });
        }
        break;
      case "gameMode.toggle":
        void callBridge<CoreSettings>("gameMode.toggle").then((updated) => {
          if (updated) {
            currentSettings = updated;
            renderAll();
          }
        });
        break;
      case "tabs.navigate":
        if (param) {
          void (async () => {
            const targetTabId = await resolveOwnTabId();
            if (targetTabId !== null) {
              await callBridge("tabs.navigate", {
                tab_id: targetTabId,
                input: param,
              });
            }
          })();
        }
        break;
      case "tabs.navigateThemes":
        void (async () => {
          const targetTabId = await resolveOwnTabId();
          if (targetTabId !== null) {
            await callBridge("tabs.navigate", {
              tab_id: targetTabId,
              input: "seu://themes",
            });
          }
        })();
        break;
      case "ui.dismissAlert":
        alertDismissed = true;
        renderAll();
        break;
      case "tabs.create":
        void callBridge("tabs.create", { window_id: windowId, activate: true });
        break;
    }
  },
});

function renderAll(): void {
  if (!currentTheme) return;

  const tabs = currentSnapshot?.tabs ?? [];
  const sleepingCount = Math.max(
    currentMemory.sleeping_tabs_count,
    tabs.filter((tab) => tab.state === "sleeping").length,
  );
  const liveTabsCount = Math.max(
    1,
    currentMemory.live_tabs_count || tabs.length - sleepingCount,
  );
  const savedLabel =
    currentSettings.locale === "en"
      ? `${currentMemory.saved_mb} MB returned`
      : `${currentMemory.saved_mb} MB devolvidos`;

  const categoryItems: RepeatItem[] = (
    [
      ["web", "search.web"],
      ["videos", "search.videos"],
      ["images", "search.images"],
      ["code", "search.code"],
      ["docs", "search.docs"],
    ] as const
  ).map(([id, key]) => ({
    id,
    label: t(key),
    state: activeCategory === id ? "active" : "",
  }));

  const shortcutItems: RepeatItem[] = shortcuts.map((s) => ({
    id: s.id,
    code: s.code,
    name: s.name,
    url: s.url,
  }));

  const themeItems: RepeatItem[] = themeList.map((item) => ({
    id: item.id,
    name: item.name,
    activeIcon:
      item.id === currentTheme?.manifest.id
        ? "fa-solid fa-circle-dot"
        : "fa-regular fa-circle",
    state: item.id === currentTheme?.manifest.id ? "active" : "",
  }));

  runtime.render(
    {
      ui: {
        showAlert: !alertDismissed,
      },
      window: {
        liveTabsCount,
      },
      memory: {
        sleepingCount,
        savedText: savedLabel,
      },
      media: {
        source: "—",
        title:
          currentSettings.locale === "en"
            ? "nothing playing right now"
            : "nenhuma mídia em reprodução",
      },
      blocker: {
        totalBlocked: currentBlocker.total_blocked,
        statusLabel: currentBlocker.enabled ? "ON" : "OFF",
      },
      gameMode: {
        enabled: currentSettings.game_mode_enabled,
        label: currentSettings.game_mode_enabled
          ? t("gameMode.on")
          : t("gameMode.off"),
      },
      theme: {
        name: currentTheme.manifest.name,
        nameUpper: currentTheme.manifest.name.toUpperCase(),
      },
    },
    {
      searchCategories: categoryItems,
      shortcuts: shortcutItems,
      themes: themeItems,
    },
  );
}

function mountTheme(theme: LoadedTheme): void {
  currentTheme = theme;
  runtime.applyStyles(theme.tokens_css, theme.newtab_css);
  runtime.mount(theme.newtab_html);
  renderAll();
}

onBridgeEvent((event) => {
  if (event.type === "state") {
    currentSnapshot = event.snapshot;
    renderAll();
  } else if (event.type === "settings") {
    currentSettings = event.settings;
    if (event.settings.quick_shortcuts && event.settings.quick_shortcuts.length > 0) {
      shortcuts = event.settings.quick_shortcuts.map((s) => ({ ...s }));
    }
    setLocale(event.settings.locale);
    renderAll();
  } else if (event.type === "theme") {
    mountTheme(event.theme);
  } else if (event.type === "memory") {
    currentMemory = event.stats;
    renderAll();
  } else if (event.type === "blocker") {
    currentBlocker = event.stats;
    renderAll();
  }
});

async function bootstrap(): Promise<void> {
  try {
    const [settings, theme, themes, snap, mem, blk] = await Promise.all([
      callBridge<CoreSettings>("settings.get").catch(() => undefined),
      callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
      callBridge<ThemeSummary[]>("themes.list").catch(() => []),
      callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      }).catch(() => undefined),
      callBridge<MemoryStats>("memory.getStats").catch(() => undefined),
      callBridge<BlockerStats>("blocker.getStats").catch(() => undefined),
    ]);

    if (settings) {
      currentSettings = settings;
      if (settings.quick_shortcuts && settings.quick_shortcuts.length > 0) {
        shortcuts = settings.quick_shortcuts.map((s) => ({ ...s }));
      }
      setLocale(settings.locale);
    }
    if (themes) {
      themeList = themes;
    }
    if (snap) {
      currentSnapshot = snap;
    }
    if (mem) {
      currentMemory = mem;
    }
    if (blk) {
      currentBlocker = blk;
    }
    if (theme) {
      mountTheme(theme);
    }
  } catch {
    // Wait for bridge events.
  }
}

void bootstrap();
