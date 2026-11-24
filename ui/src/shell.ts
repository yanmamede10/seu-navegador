import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type WindowSnapshot,
} from "./bridge";
import { DeclarativeRuntime, type RepeatItem } from "./declarative";
import { setLocale } from "./i18n";

const windowId = getWindowIdFromQuery();
const mountEl = document.getElementById("theme-mount") as HTMLDivElement;

let currentSnapshot: WindowSnapshot | null = null;
let currentTheme: LoadedTheme | null = null;
let overlayOpen = false;

const runtime = new DeclarativeRuntime(mountEl, {
  onStartDragWindow: () => {
    void callBridge("windows.startDrag", { window_id: windowId });
  },
  onMaximizeWindow: () => {
    void callBridge("windows.maximize", { window_id: windowId });
  },
  onSubmit: (name, value) => {
    if (name === "omnibox" && currentSnapshot?.active_tab_id) {
      void callBridge("tabs.navigate", {
        tab_id: currentSnapshot.active_tab_id,
        input: value,
      });
    }
  },
  onReorder: (collection, draggedId, toIndex) => {
    if (collection === "tabs") {
      void callBridge("tabs.reorder", {
        window_id: windowId,
        tab_id: Number(draggedId),
        to_index: toIndex,
      });
    }
  },
  onAction: ({ action, item }) => {
    const activeTabId = currentSnapshot?.active_tab_id;
    switch (action) {
      case "windows.minimize":
        void callBridge("windows.minimize", { window_id: windowId });
        break;
      case "windows.maximize":
        void callBridge("windows.maximize", { window_id: windowId });
        break;
      case "windows.close":
        void callBridge("windows.close", { window_id: windowId });
        break;
      case "tabs.create":
        void callBridge("tabs.create", { window_id: windowId, activate: true });
        break;
      case "tabs.activate":
        if (item?.id !== undefined) {
          void callBridge("tabs.activate", {
            window_id: windowId,
            tab_id: Number(item.id),
          });
        }
        break;
      case "tabs.close":
        if (item?.id !== undefined) {
          void callBridge("tabs.close", { tab_id: Number(item.id) });
        }
        break;
      case "tabs.pin":
        if (item?.id !== undefined) {
          void callBridge("tabs.pin", { tab_id: Number(item.id) });
        }
        break;
      case "tabs.back":
        if (activeTabId) {
          void callBridge("tabs.back", { tab_id: activeTabId });
        }
        break;
      case "tabs.forward":
        if (activeTabId) {
          void callBridge("tabs.forward", { tab_id: activeTabId });
        }
        break;
      case "tabs.reload":
        if (activeTabId) {
          void callBridge("tabs.reload", { tab_id: activeTabId });
        }
        break;
      case "tabGroups.toggleCollapse":
        if (item?.id !== undefined) {
          void callBridge("tabGroups.toggleCollapse", {
            window_id: windowId,
            group_id: Number(item.id),
          });
        }
        break;
      case "overlay.toggleMenu":
        toggleMenuOverlay();
        break;
    }
  },
});

function toggleMenuOverlay(): void {
  if (overlayOpen) {
    overlayOpen = false;
    void callBridge("overlay.close", { window_id: windowId });
    return;
  }
  const dpr = window.devicePixelRatio || 1;
  const rect = mountEl.getBoundingClientRect();
  const widthCss = 276;
  const heightCss = 330;
  const xCss = Math.max(8, window.innerWidth - widthCss - 10);
  const yCss = rect.height + 4;
  overlayOpen = true;
  void callBridge("overlay.open", {
    window_id: windowId,
    panel: "menu",
    x: Math.round(xCss * dpr),
    y: Math.round(yCss * dpr),
    width: Math.round(widthCss * dpr),
    height: Math.round(heightCss * dpr),
  });
}

function reportShellHeight(): void {
  const dpr = window.devicePixelRatio || 1;
  const cssHeight = mountEl.getBoundingClientRect().height || mountEl.offsetHeight;
  const physicalHeight = Math.ceil(cssHeight * dpr);
  if (physicalHeight > 0) {
    void callBridge("windows.setShellBounds", {
      window_id: windowId,
      height: physicalHeight,
    });
  }
}

const resizeObserver = new ResizeObserver(() => {
  reportShellHeight();
});
resizeObserver.observe(mountEl);

function renderAll(): void {
  if (!currentSnapshot || !currentTheme) return;

  document.documentElement.dataset.incognito = currentSnapshot.incognito ? "true" : "false";

  const activeTab =
    currentSnapshot.tabs.find((tab) => tab.id === currentSnapshot?.active_tab_id) ?? null;

  const groupMap = new Map(currentSnapshot.groups.map((g) => [g.id, g]));

  const tabItems: RepeatItem[] = [];
  for (const tab of currentSnapshot.tabs) {
    const grp = tab.group_id !== null ? groupMap.get(tab.group_id) : undefined;
    if (grp && grp.collapsed && tab.id !== currentSnapshot.active_tab_id) {
      continue;
    }

    const states: string[] = [];
    if (tab.id === currentSnapshot.active_tab_id) states.push("active");
    if (tab.state === "sleeping") states.push("sleeping");
    if (tab.pinned) states.push("pinned");
    if (tab.audible) states.push("playing");

    const glyph = tab.audible
      ? "♪"
      : tab.pinned
        ? "◆"
        : tab.id === currentSnapshot.active_tab_id
          ? "★"
          : "•";

    tabItems.push({
      id: tab.id,
      title: tab.loading ? `… ${tab.title}` : tab.title,
      glyph,
      isSleeping: tab.state === "sleeping",
      state: states.join(" "),
    });
  }

  const groupItems: RepeatItem[] = currentSnapshot.groups.map((g) => ({
    id: g.id,
    title: g.title,
    color: g.color,
    collapseGlyph: g.collapsed ? "▸" : "▾",
    state: g.collapsed ? "collapsed" : "expanded",
  }));

  runtime.render(
    {
      window: {
        id: currentSnapshot.id,
        incognito: currentSnapshot.incognito,
      },
      activeTab: {
        title: activeTab?.title ?? "nova aba",
        url: activeTab?.url ?? "seu://newtab/",
        can_go_back: activeTab?.can_go_back ?? false,
        can_go_forward: activeTab?.can_go_forward ?? false,
      },
      memory: {
        badgeText: "RAM ▮▮▯▯▯ -- MB",
      },
      blocker: {
        totalBlocked: 0,
      },
    },
    {
      tabs: tabItems,
      tabGroups: groupItems,
    },
  );

  if (activeTab) {
    runtime.setInputValue(
      "omnibox",
      activeTab.url === "seu://newtab/" ? "" : activeTab.url,
    );
  }

  requestAnimationFrame(() => reportShellHeight());
}

function mountTheme(theme: LoadedTheme): void {
  currentTheme = theme;
  runtime.applyStyles(theme.tokens_css, theme.shell_css);
  if (theme.shell_html) {
    runtime.mount(theme.shell_html);
  }
  renderAll();
}

onBridgeEvent((event) => {
  if (event.type === "state") {
    currentSnapshot = event.snapshot;
    renderAll();
  } else if (event.type === "settings") {
    setLocale(event.settings.locale);
    renderAll();
  } else if (event.type === "theme") {
    mountTheme(event.theme);
  } else if (event.type === "overlay" && event.panel === "closed") {
    overlayOpen = false;
  }
});

async function bootstrap(): Promise<void> {
  try {
    const [settings, theme, snap] = await Promise.all([
      callBridge<CoreSettings>("settings.get").catch(() => undefined),
      callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
      callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      }).catch(() => undefined),
    ]);

    if (settings?.locale) {
      setLocale(settings.locale);
    }
    if (snap) {
      currentSnapshot = snap;
    }
    if (theme) {
      mountTheme(theme);
    }
  } catch {
    // Wait for pushed events.
  }
}

void bootstrap();
