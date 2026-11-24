import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type ThemeSummary,
  type WindowSnapshot,
} from "./bridge";
import { DeclarativeRuntime, type RepeatItem } from "./declarative";
import { setLocale, t } from "./i18n";

const windowId = getWindowIdFromQuery();
const mountEl = document.getElementById("theme-mount") as HTMLDivElement;

type SearchCategoryId = "web" | "videos" | "images" | "code" | "docs";

interface ShortcutItem {
  id: string;
  code: string;
  name: string;
  url: string;
}

const shortcuts: ShortcutItem[] = [
  { id: "yt", code: "YT", name: "YouTube", url: "https://www.youtube.com" },
  { id: "gh", code: "GH", name: "GitHub", url: "https://github.com" },
  { id: "rs", code: "RS", name: "docs.rs", url: "https://docs.rs" },
  { id: "gd", code: "GD", name: "Godot", url: "https://godotengine.org" },
  { id: "io", code: "IO", name: "itch.io", url: "https://itch.io" },
  { id: "st", code: "ST", name: "Steam", url: "https://store.steampowered.com" },
];

let activeCategory: SearchCategoryId = "web";
let currentTheme: LoadedTheme | null = null;
let themeList: ThemeSummary[] = [];
let currentSnapshot: WindowSnapshot | null = null;
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
      const snap =
        currentSnapshot ??
        (await callBridge<WindowSnapshot>("windows.getSnapshot", {
          window_id: windowId,
        }).catch(() => undefined));
      if (snap?.active_tab_id) {
        await callBridge("search.submit", {
          tab_id: snap.active_tab_id,
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
      case "tabs.navigateThemes":
        if (currentSnapshot?.active_tab_id) {
          void callBridge("tabs.navigate", {
            tab_id: currentSnapshot.active_tab_id,
            input: "seu://themes",
          });
        }
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
  const sleepingCount = tabs.filter((tab) => tab.state === "sleeping").length;
  const liveTabsCount = Math.max(1, tabs.length - sleepingCount);

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
    activeGlyph: item.id === currentTheme?.manifest.id ? "●" : "○",
    state: item.id === currentTheme?.manifest.id ? "active" : "",
  }));

  runtime.render(
    {
      window: {
        liveTabsCount,
      },
      memory: {
        sleepingCount,
        savedText: "0 MB",
      },
      media: {
        source: "—",
        title:
          currentSettings.locale === "en"
            ? "nothing playing right now"
            : "nenhuma mídia em reprodução",
      },
      blocker: {
        totalBlocked: 0,
        statusLabel: "ON",
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
    setLocale(event.settings.locale);
    renderAll();
  } else if (event.type === "theme") {
    mountTheme(event.theme);
  }
});

async function bootstrap(): Promise<void> {
  try {
    const [settings, theme, themes, snap] = await Promise.all([
      callBridge<CoreSettings>("settings.get").catch(() => undefined),
      callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
      callBridge<ThemeSummary[]>("themes.list").catch(() => []),
      callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      }).catch(() => undefined),
    ]);

    if (settings) {
      currentSettings = settings;
      setLocale(settings.locale);
    }
    if (themes) {
      themeList = themes;
    }
    if (snap) {
      currentSnapshot = snap;
    }
    if (theme) {
      mountTheme(theme);
    }
  } catch {
    // Wait for bridge events.
  }
}

void bootstrap();
