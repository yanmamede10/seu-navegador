export interface TabGroup {
  id: number;
  title: string;
  color: string;
  collapsed: boolean;
}

export interface TabInfo {
  id: number;
  window_id: number;
  url: string;
  title: string;
  favicon: string | null;
  loading: boolean;
  can_go_back: boolean;
  can_go_forward: boolean;
  pinned: boolean;
  audible: boolean;
  state: "active" | "background" | "sleeping";
  group_id: number | null;
}

export interface WindowSnapshot {
  id: number;
  incognito: boolean;
  active_tab_id: number | null;
  tabs: TabInfo[];
  groups: TabGroup[];
}

export interface CoreSettings {
  locale: "pt-br" | "en";
  search_engine: "duck-duck-go" | "google" | "brave" | "startpage";
  active_theme_id: string;
  game_mode_enabled: boolean;
}

export interface ThemeSummary {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  builtin: boolean;
}

export interface LoadedTheme {
  manifest: {
    id: string;
    name: string;
    version: string;
    author: string;
    description: string;
  };
  tokens_css: string;
  shell_html: string | null;
  shell_css: string;
  newtab_html: string;
  newtab_css: string;
  base_url: string;
  builtin: boolean;
}

export type BridgeEvent =
  | { type: "state"; snapshot: WindowSnapshot }
  | { type: "settings"; settings: CoreSettings }
  | { type: "theme"; theme: LoadedTheme }
  | { type: "overlay"; panel: string };

declare global {
  interface Window {
    cefQuery?: (opts: {
      request: string;
      persistent?: boolean;
      onSuccess?: (response: string) => void;
      onFailure?: (errorCode: number, errorMessage: string) => void;
    }) => number;
    __seuReceiveEvent?: (event: BridgeEvent) => void;
  }
}

type EventListener = (event: BridgeEvent) => void;
const listeners = new Set<EventListener>();

window.__seuReceiveEvent = (event: BridgeEvent) => {
  for (const listener of listeners) {
    listener(event);
  }
};

export function onBridgeEvent(listener: EventListener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getWindowIdFromQuery(): number {
  const params = new URLSearchParams(window.location.search);
  const raw = Number(params.get("win"));
  return Number.isFinite(raw) && raw > 0 ? raw : 1;
}

const mockSnapshot: WindowSnapshot = {
  id: 1,
  incognito: false,
  active_tab_id: 1,
  groups: [{ id: 1, title: "Dev", color: "#4FD8FF", collapsed: false }],
  tabs: [
    {
      id: 1,
      window_id: 1,
      url: "seu://newtab/",
      title: "Início",
      favicon: null,
      loading: false,
      can_go_back: false,
      can_go_forward: false,
      pinned: false,
      audible: false,
      state: "active",
      group_id: null,
    },
    {
      id: 2,
      window_id: 1,
      url: "https://github.com",
      title: "GitHub",
      favicon: null,
      loading: false,
      can_go_back: true,
      can_go_forward: false,
      pinned: false,
      audible: false,
      state: "sleeping",
      group_id: 1,
    },
  ],
};

const mockSettings: CoreSettings = {
  locale: "pt-br",
  search_engine: "duck-duck-go",
  active_theme_id: "y2k-maximal",
  game_mode_enabled: false,
};

export async function callBridge<T = unknown>(
  method: string,
  params?: Record<string, unknown>,
): Promise<T> {
  if (typeof window.cefQuery === "function") {
    const payload = params === undefined ? { method } : { method, params };
    return new Promise<T>((resolve, reject) => {
      window.cefQuery?.({
        request: JSON.stringify(payload),
        persistent: false,
        onSuccess: (raw) => {
          try {
            const parsed = JSON.parse(raw) as { kind: string; data?: T };
            resolve((parsed.data ?? parsed) as T);
          } catch (err) {
            reject(err);
          }
        },
        onFailure: (code, message) => {
          reject(new Error(`Bridge error (${code}): ${message}`));
        },
      });
    });
  }

  if (method === "windows.getSnapshot") {
    return structuredClone(mockSnapshot) as T;
  }
  if (method === "settings.get") {
    return structuredClone(mockSettings) as T;
  }
  if (method === "gameMode.toggle") {
    mockSettings.game_mode_enabled = !mockSettings.game_mode_enabled;
    return structuredClone(mockSettings) as T;
  }
  if (method === "themes.list") {
    return [
      {
        id: "y2k-maximal",
        name: "Y2K Maximal",
        version: "1.0.0",
        author: "Yan Mamede",
        description: "Default theme",
        builtin: true,
      },
    ] as T;
  }
  if (method === "settings.setLocale" && params?.locale) {
    mockSettings.locale = String(params.locale).toLowerCase().startsWith("en")
      ? "en"
      : "pt-br";
    return structuredClone(mockSettings) as T;
  }
  return undefined as T;
}
