import "./icons";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type BlockerStats,
  type CoreSettings,
  type LoadedTheme,
  type MemoryStats,
  type WindowSnapshot,
} from "./bridge";
import { DeclarativeRuntime, type RepeatItem } from "./declarative";
import { setLocale } from "./i18n";

const windowId = getWindowIdFromQuery();
const mountEl = document.getElementById("theme-mount") as HTMLDivElement;

let currentSnapshot: WindowSnapshot | null = null;
let currentTheme: LoadedTheme | null = null;
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
let activeOverlayPanel: "menu" | "memory" | "blocker" | null = null;
let activeUrlBookmarked = false;

function getNavTargetTab(snapshot: WindowSnapshot | null) {
  if (!snapshot) return null;
  const activeTab =
    snapshot.tabs.find((tab) => tab.id === snapshot.active_tab_id) ?? null;
  if (activeTab?.url.startsWith("seu://studio/") && snapshot.split_tab_id) {
    const splitTab =
      snapshot.tabs.find((tab) => tab.id === snapshot.split_tab_id) ?? null;
    if (splitTab) return splitTab;
  }
  return activeTab;
}

async function refreshActiveBookmarkState(): Promise<void> {
  const navTab = getNavTargetTab(currentSnapshot);
  const url = navTab?.url ?? "";
  if (!url || url.startsWith("seu://")) {
    activeUrlBookmarked = false;
    return;
  }
  const res = await callBridge<{ bookmarked: boolean }>("bookmarks.isBookmarked", {
    url,
  }).catch(() => undefined);
  activeUrlBookmarked = Boolean(res?.bookmarked);
}

const runtime = new DeclarativeRuntime(mountEl, {
  onStartDragWindow: () => {
    void callBridge("windows.startDrag", { window_id: windowId });
  },
  onMaximizeWindow: () => {
    void callBridge("windows.maximize", { window_id: windowId });
  },
  onSubmit: (name, value) => {
    if (name !== "omnibox") return;
    const trimmed = value.trim();
    if (!trimmed) return;

    void (async () => {
      const snap =
        currentSnapshot ??
        (await callBridge<WindowSnapshot>("windows.getSnapshot", {
          window_id: windowId,
        }).catch(() => undefined));
      const navTab = getNavTargetTab(snap ?? null);
      if (navTab && !navTab.url.startsWith("seu://studio/")) {
        await callBridge("tabs.navigate", {
          tab_id: navTab.id,
          input: trimmed,
        });
        return;
      }
      // In pure Studio-only mode, open the search/URL in a new browser tab without wiping Studio.
      await callBridge("tabs.create", {
        window_id: windowId,
        url: trimmed,
        activate: true,
      });
    })();
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
  onAction: ({ action, param, item }) => {
    const navTab = getNavTargetTab(currentSnapshot);
    const navTabId = navTab?.id;
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
        if (navTabId) {
          void callBridge("tabs.back", { tab_id: navTabId });
        }
        break;
      case "tabs.forward":
        if (navTabId) {
          void callBridge("tabs.forward", { tab_id: navTabId });
        }
        break;
      case "tabs.reload":
        if (navTabId) {
          void callBridge("tabs.reload", { tab_id: navTabId });
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
        toggleOverlayPanel("menu");
        break;
      case "overlay.toggleMemory":
        toggleOverlayPanel("memory");
        break;
      case "overlay.toggleBlocker":
        toggleOverlayPanel("blocker");
        break;
      case "bookmarks.toggle": {
        if (navTab && !navTab.url.startsWith("seu://")) {
          void callBridge<{ bookmarked: boolean }>("bookmarks.toggle", {
            title: navTab.title,
            url: navTab.url,
            parent_id: null,
          }).then((res) => {
            activeUrlBookmarked = Boolean(res?.bookmarked);
            renderAll();
          });
        }
        break;
      }
      case "split.toggle":
        void callBridge("split.toggle", { window_id: windowId });
        break;
      case "studio.setMode":
        void callBridge("studio.setMode", {
          window_id: windowId,
          mode: param || "split",
        });
        break;
      case "devtools.toggle":
        void callBridge("devtools.toggle", { window_id: windowId });
        break;
    }
  },
});

function toggleOverlayPanel(panel: "menu" | "memory" | "blocker"): void {
  if (activeOverlayPanel === panel) {
    activeOverlayPanel = null;
    void callBridge("overlay.close", { window_id: windowId });
    return;
  }
  const dpr = window.devicePixelRatio || 1;
  const rect = mountEl.getBoundingClientRect();
  const widthCss = panel === "menu" ? 296 : 350;
  const heightCss = panel === "menu" ? 560 : 440;
  const xCss = Math.max(8, window.innerWidth - widthCss - 10);
  const yCss = rect.height + 4;
  activeOverlayPanel = panel;
  void callBridge("overlay.open", {
    window_id: windowId,
    panel,
    x: Math.round(xCss * dpr),
    y: Math.round(yCss * dpr),
    width: Math.round(widthCss * dpr),
    height: Math.round(heightCss * dpr),
  });
}

document.addEventListener("mousedown", (ev) => {
  if (!activeOverlayPanel) return;
  const target = ev.target as HTMLElement | null;
  if (
    target?.closest('[data-seu-action="overlay.toggleMenu"]') ||
    target?.closest('[data-seu-action="overlay.toggleMemory"]') ||
    target?.closest('[data-seu-action="overlay.toggleBlocker"]')
  ) {
    return;
  }
  activeOverlayPanel = null;
  void callBridge("overlay.close", { window_id: windowId });
});

let lastReportedShellHeight = 0;

function reportShellHeight(): void {
  const dpr = window.devicePixelRatio || 1;
  const cssHeight = mountEl.getBoundingClientRect().height || mountEl.offsetHeight;
  const physicalHeight = Math.ceil(cssHeight * dpr);
  if (physicalHeight > 0 && physicalHeight !== lastReportedShellHeight) {
    lastReportedShellHeight = physicalHeight;
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
  document.documentElement.dataset.split = currentSnapshot.split_tab_id ? "true" : "false";

  const activeTab =
    currentSnapshot.tabs.find((tab) => tab.id === currentSnapshot?.active_tab_id) ?? null;
  const navTab = getNavTargetTab(currentSnapshot);

  const isStudioActive = Boolean(activeTab?.url.startsWith("seu://studio/"));
  const uiMode =
    currentSnapshot.split_tab_id !== null && currentSnapshot.split_tab_id !== undefined
      ? "split"
      : isStudioActive
        ? "studio"
        : "browser";
  document.documentElement.dataset.mode = uiMode;

  const groupMap = new Map(currentSnapshot.groups.map((g) => [g.id, g]));

  const tabItems: RepeatItem[] = [];
  for (const tab of currentSnapshot.tabs) {
    const grp = tab.group_id !== null ? groupMap.get(tab.group_id) : undefined;
    if (
      grp &&
      grp.collapsed &&
      tab.id !== currentSnapshot.active_tab_id &&
      tab.id !== currentSnapshot.split_tab_id
    ) {
      continue;
    }

    const states: string[] = [];
    if (tab.id === currentSnapshot.active_tab_id) states.push("active");
    if (tab.id === currentSnapshot.split_tab_id) states.push("split");
    if (tab.state === "sleeping") states.push("sleeping");
    if (tab.pinned) states.push("pinned");
    if (tab.audible) states.push("playing");

    const icon = tab.audible
      ? "fa-solid fa-volume-high"
      : tab.pinned
        ? "fa-solid fa-thumbtack"
        : tab.id === currentSnapshot.split_tab_id
          ? "fa-solid fa-table-columns"
          : tab.id === currentSnapshot.active_tab_id
            ? "fa-solid fa-bolt"
            : "fa-solid fa-globe";

    tabItems.push({
      id: tab.id,
      title: tab.loading ? `... ${tab.title}` : tab.title,
      icon,
      isSleeping: tab.state === "sleeping",
      state: states.join(" "),
    });
  }

  const groupItems: RepeatItem[] = currentSnapshot.groups.map((g) => ({
    id: g.id,
    title: g.title,
    color: g.color,
    collapseIcon: g.collapsed ? "fa-solid fa-caret-right" : "fa-solid fa-caret-down",
    state: g.collapsed ? "collapsed" : "expanded",
  }));

  runtime.render(
    {
      window: {
        id: currentSnapshot.id,
        incognito: currentSnapshot.incognito,
      },
      activeTab: {
        title: navTab?.title ?? activeTab?.title ?? "nova aba",
        url: navTab?.url ?? "seu://newtab/",
        can_go_back: navTab?.can_go_back ?? false,
        can_go_forward: navTab?.can_go_forward ?? false,
        bookmarkIcon: activeUrlBookmarked ? "fa-solid fa-star" : "fa-regular fa-star",
      },
      memory: {
        badgeText:
          currentMemory.total_private_mb > 0
            ? `RAM ${currentMemory.total_private_mb} MB`
            : "RAM -- MB",
      },
      blocker: {
        totalBlocked:
          navTab && !navTab.url.startsWith("seu://")
            ? (navTab.blocked_count ?? 0)
            : currentBlocker.total_blocked,
      },
    },
    {
      tabs: tabItems,
      tabGroups: groupItems,
    },
  );

  if (navTab) {
    const omniboxText =
      navTab.url === "seu://newtab/" || navTab.url.startsWith("seu://studio/")
        ? ""
        : navTab.url;
    runtime.setInputValue("omnibox", omniboxText);
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
    void refreshActiveBookmarkState().then(() => renderAll());
  } else if (event.type === "bookmarksChanged") {
    void refreshActiveBookmarkState().then(() => renderAll());
  } else if (event.type === "settings") {
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
  } else if (event.type === "overlay" && event.panel === "closed") {
    activeOverlayPanel = null;
  } else if (event.type === "toggleMenu") {
    toggleOverlayPanel("menu");
  } else if (event.type === "focusOmnibox") {
    const input = mountEl.querySelector<HTMLInputElement>('[data-seu-input="omnibox"]');
    if (input) {
      input.focus();
      input.select();
    }
  }
});

async function bootstrap(): Promise<void> {
  try {
    const [settings, theme, snap, mem, blk] = await Promise.all([
      callBridge<CoreSettings>("settings.get").catch(() => undefined),
      callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
      callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      }).catch(() => undefined),
      callBridge<MemoryStats>("memory.getStats").catch(() => undefined),
      callBridge<BlockerStats>("blocker.getStats").catch(() => undefined),
    ]);

    if (settings?.locale) {
      setLocale(settings.locale);
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
    // Wait for pushed events.
  }
}

void bootstrap();
