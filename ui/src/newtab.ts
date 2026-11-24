import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type WindowSnapshot,
} from "./bridge";
import { setLocale, t } from "./i18n";

const windowId = getWindowIdFromQuery();
const subtitle = document.getElementById("subtitle") as HTMLParagraphElement;
const searchForm = document.getElementById("search-form") as HTMLFormElement;
const searchInput = document.getElementById("search-input") as HTMLInputElement;
const searchBtn = document.getElementById("search-btn") as HTMLButtonElement;
const shortcutsHeading = document.getElementById("shortcuts-heading") as HTMLHeadingElement;
const shortcutsGrid = document.getElementById("shortcuts-grid") as HTMLDivElement;

const DEFAULT_SHORTCUTS = [
  { code: "YT", name: "YouTube", url: "https://www.youtube.com" },
  { code: "GH", name: "GitHub", url: "https://github.com" },
  { code: "RS", name: "docs.rs", url: "https://docs.rs" },
  { code: "GD", name: "Godot", url: "https://godotengine.org" },
  { code: "IO", name: "itch.io", url: "https://itch.io" },
  { code: "ST", name: "Steam", url: "https://store.steampowered.com" },
];

function applyLocale(): void {
  subtitle.textContent = t("newtab.subtitle");
  searchInput.placeholder = t("newtab.searchPlaceholder");
  searchBtn.textContent = t("newtab.searchButton");
  shortcutsHeading.textContent = t("newtab.shortcuts");
}

function renderShortcuts(): void {
  shortcutsGrid.innerHTML = "";
  for (const item of DEFAULT_SHORTCUTS) {
    const a = document.createElement("a");
    a.className = "shortcut-card";
    a.href = item.url;
    a.innerHTML = `<span class="shortcut-badge">${item.code}</span><span>${item.name}</span>`;
    shortcutsGrid.appendChild(a);
  }
}

searchForm.addEventListener("submit", (ev) => {
  ev.preventDefault();
  const query = searchInput.value.trim();
  if (!query) return;

  void (async () => {
    try {
      const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
        window_id: windowId,
      });
      if (snap?.active_tab_id) {
        await callBridge("tabs.navigate", {
          tab_id: snap.active_tab_id,
          input: query,
        });
        return;
      }
    } catch {
      // Fallback to direct navigation if running standalone in preview.
    }
    window.location.href = `https://duckduckgo.com/?q=${encodeURIComponent(query)}`;
  })();
});

onBridgeEvent((event) => {
  if (event.type === "settings") {
    setLocale(event.settings.locale);
    applyLocale();
  }
});

async function bootstrap(): Promise<void> {
  try {
    const settings = await callBridge<CoreSettings>("settings.get");
    if (settings?.locale) {
      setLocale(settings.locale);
    }
  } catch {
    // Keep defaults.
  }
  applyLocale();
  renderShortcuts();
}

void bootstrap();
