import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type WindowSnapshot,
} from "./bridge";
import { setLocale, t } from "./i18n";

const windowId = getWindowIdFromQuery();
let currentSnapshot: WindowSnapshot | null = null;
let draggedTabId: number | null = null;
let overlayOpen = false;

const shellRoot = document.getElementById("shell-root") as HTMLDivElement;
const dragRegion = document.getElementById("drag-region") as HTMLDivElement;
const brandTitle = document.getElementById("brand-title") as HTMLSpanElement;
const incognitoPill = document.getElementById("incognito-pill") as HTMLSpanElement;
const tabsList = document.getElementById("tabs-list") as HTMLDivElement;
const btnMin = document.getElementById("btn-min") as HTMLButtonElement;
const btnMax = document.getElementById("btn-max") as HTMLButtonElement;
const btnClose = document.getElementById("btn-close") as HTMLButtonElement;
const btnNewTab = document.getElementById("btn-new-tab") as HTMLButtonElement;
const btnBack = document.getElementById("btn-back") as HTMLButtonElement;
const btnForward = document.getElementById("btn-forward") as HTMLButtonElement;
const btnReload = document.getElementById("btn-reload") as HTMLButtonElement;
const btnMenu = document.getElementById("btn-menu") as HTMLButtonElement;
const omniboxForm = document.getElementById("omnibox-form") as HTMLFormElement;
const omniboxInput = document.getElementById("omnibox-input") as HTMLInputElement;
const omniboxLock = document.getElementById("omnibox-lock") as HTMLSpanElement;
const badgeRam = document.getElementById("badge-ram") as HTMLSpanElement;
const badgeBlocker = document.getElementById("badge-blocker") as HTMLSpanElement;

function applyLocalizedLabels(): void {
  brandTitle.textContent = t("app.name");
  incognitoPill.textContent = t("app.incognito");
  btnMin.title = t("window.minimize");
  btnMax.title = t("window.maximize");
  btnClose.title = t("window.close");
  btnNewTab.title = t("tabs.new");
  btnBack.title = t("nav.back");
  btnForward.title = t("nav.forward");
  btnReload.title = t("nav.reload");
  btnMenu.title = t("nav.menu");
  omniboxInput.placeholder = t("nav.omniboxPlaceholder");
  badgeRam.textContent = `${t("badge.ram")} -- MB`;
  badgeBlocker.textContent = `${t("badge.blocker")} 0`;
}

function renderSnapshot(snap: WindowSnapshot): void {
  currentSnapshot = snap;
  shellRoot.classList.toggle("incognito", snap.incognito);
  incognitoPill.hidden = !snap.incognito;

  tabsList.innerHTML = "";
  const renderedGroups = new Set<number>();

  snap.tabs.forEach((tab, index) => {
    const group =
      tab.group_id !== null
        ? snap.groups.find((g) => g.id === tab.group_id) ?? null
        : null;

    if (group && !renderedGroups.has(group.id)) {
      renderedGroups.add(group.id);
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "group-chip";
      chip.style.background = group.color;
      chip.textContent = `${group.collapsed ? "▸" : "▾"} ${group.title}`;
      chip.addEventListener("click", () => {
        void callBridge("tabGroups.toggleCollapse", {
          window_id: windowId,
          group_id: group.id,
        });
      });
      tabsList.appendChild(chip);
    }

    if (group && group.collapsed && tab.id !== snap.active_tab_id) {
      return;
    }

    const tabEl = document.createElement("div");
    tabEl.className = "tab-item";
    tabEl.draggable = true;
    tabEl.dataset.tabId = String(tab.id);
    if (tab.id === snap.active_tab_id) {
      tabEl.classList.add("active");
    }
    if (tab.pinned) {
      tabEl.classList.add("pinned");
    }
    if (group) {
      tabEl.style.borderTop = `2px solid ${group.color}`;
    }

    const titleEl = document.createElement("span");
    titleEl.className = "tab-title";
    titleEl.textContent = tab.loading ? `… ${tab.title}` : tab.title;
    tabEl.appendChild(titleEl);

    if (!tab.pinned) {
      const closeBtn = document.createElement("button");
      closeBtn.type = "button";
      closeBtn.className = "tab-close";
      closeBtn.title = t("tabs.close");
      closeBtn.textContent = "×";
      closeBtn.addEventListener("click", (ev) => {
        ev.stopPropagation();
        void callBridge("tabs.close", { tab_id: tab.id });
      });
      tabEl.appendChild(closeBtn);
    }

    tabEl.addEventListener("click", () => {
      void callBridge("tabs.activate", { window_id: windowId, tab_id: tab.id });
    });

    tabEl.addEventListener("auxclick", (ev) => {
      if (ev.button === 1) {
        ev.preventDefault();
        void callBridge("tabs.close", { tab_id: tab.id });
      }
    });

    tabEl.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      void callBridge("tabs.pin", { tab_id: tab.id });
    });

    tabEl.addEventListener("dragstart", (ev) => {
      draggedTabId = tab.id;
      ev.dataTransfer?.setData("text/plain", String(tab.id));
    });

    tabEl.addEventListener("dragover", (ev) => {
      ev.preventDefault();
      tabEl.classList.add("drag-over");
    });

    tabEl.addEventListener("dragleave", () => {
      tabEl.classList.remove("drag-over");
    });

    tabEl.addEventListener("drop", (ev) => {
      ev.preventDefault();
      tabEl.classList.remove("drag-over");
      if (draggedTabId !== null && draggedTabId !== tab.id) {
        void callBridge("tabs.reorder", {
          window_id: windowId,
          tab_id: draggedTabId,
          to_index: index,
        });
      }
      draggedTabId = null;
    });

    tabsList.appendChild(tabEl);
  });

  const activeTab = snap.tabs.find((tab) => tab.id === snap.active_tab_id);
  btnBack.disabled = !activeTab?.can_go_back;
  btnForward.disabled = !activeTab?.can_go_forward;

  if (activeTab && document.activeElement !== omniboxInput) {
    omniboxInput.value =
      activeTab.url === "seu://newtab/" ? "" : activeTab.url;
  }

  if (activeTab?.url.startsWith("https://")) {
    omniboxLock.textContent = "🔒";
  } else if (activeTab?.url.startsWith("seu://")) {
    omniboxLock.textContent = "★";
  } else {
    omniboxLock.textContent = "●";
  }
}

dragRegion.addEventListener("mousedown", (ev) => {
  if (ev.button === 0 && ev.detail === 1) {
    void callBridge("windows.startDrag", { window_id: windowId });
  }
});

dragRegion.addEventListener("dblclick", () => {
  void callBridge("windows.maximize", { window_id: windowId });
});

btnMin.addEventListener("click", () => {
  void callBridge("windows.minimize", { window_id: windowId });
});

btnMax.addEventListener("click", () => {
  void callBridge("windows.maximize", { window_id: windowId });
});

btnClose.addEventListener("click", () => {
  void callBridge("windows.close", { window_id: windowId });
});

btnNewTab.addEventListener("click", () => {
  void callBridge("tabs.create", { window_id: windowId, activate: true });
});

btnBack.addEventListener("click", () => {
  if (currentSnapshot?.active_tab_id) {
    void callBridge("tabs.back", { tab_id: currentSnapshot.active_tab_id });
  }
});

btnForward.addEventListener("click", () => {
  if (currentSnapshot?.active_tab_id) {
    void callBridge("tabs.forward", { tab_id: currentSnapshot.active_tab_id });
  }
});

btnReload.addEventListener("click", () => {
  if (currentSnapshot?.active_tab_id) {
    void callBridge("tabs.reload", { tab_id: currentSnapshot.active_tab_id });
  }
});

omniboxForm.addEventListener("submit", (ev) => {
  ev.preventDefault();
  if (!currentSnapshot?.active_tab_id) return;
  void callBridge("tabs.navigate", {
    tab_id: currentSnapshot.active_tab_id,
    input: omniboxInput.value,
  });
  omniboxInput.blur();
});

btnMenu.addEventListener("click", () => {
  if (overlayOpen) {
    overlayOpen = false;
    void callBridge("overlay.close", { window_id: windowId });
    return;
  }
  const rect = shellRoot.getBoundingClientRect();
  const width = 270;
  const x = Math.max(8, Math.round(window.innerWidth - width - 10));
  const y = Math.round(rect.height + 4);
  overlayOpen = true;
  void callBridge("overlay.open", {
    window_id: windowId,
    panel: "menu",
    x,
    y,
    width,
    height: 310,
  });
});

// Report dynamic shell height to Rust so content view stays positioned cleanly below.
const resizeObserver = new ResizeObserver((entries) => {
  const height = Math.ceil(entries[0]?.contentRect.height ?? shellRoot.offsetHeight);
  if (height > 0) {
    void callBridge("windows.setShellBounds", { window_id: windowId, height });
  }
});
resizeObserver.observe(shellRoot);

onBridgeEvent((event) => {
  if (event.type === "state") {
    renderSnapshot(event.snapshot);
  } else if (event.type === "settings") {
    setLocale(event.settings.locale);
    applyLocalizedLabels();
    if (currentSnapshot) {
      renderSnapshot(currentSnapshot);
    }
  } else if (event.type === "overlay" && event.panel === "closed") {
    overlayOpen = false;
  }
});

async function bootstrap(): Promise<void> {
  try {
    const settings = await callBridge<CoreSettings>("settings.get");
    if (settings?.locale) {
      setLocale(settings.locale);
    }
  } catch {
    // Keep default locale if unavailable during preview.
  }
  applyLocalizedLabels();

  try {
    const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
      window_id: windowId,
    });
    if (snap) {
      renderSnapshot(snap);
    }
  } catch {
    // Initial state will arrive via push event.
  }
}

void bootstrap();
