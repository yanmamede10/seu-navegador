import "./icons";
import "./settings.css";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type BookmarkItem,
  type CoreSettings,
  type DownloadItem,
  type HistoryItem,
  type LoadedTheme,
  type WindowSnapshot,
} from "./bridge";
import { setLocale } from "./i18n";

const root = document.getElementById("page-root") as HTMLDivElement;
const pageKind = (document.body.dataset.seuPage ?? "history") as
  | "history"
  | "bookmarks"
  | "downloads";
const windowId = getWindowIdFromQuery();

let locale: "pt-br" | "en" = "pt-br";
let historyItems: HistoryItem[] = [];
let historyQuery = "";
let bookmarkItems: BookmarkItem[] = [];
let selectedFolderId: number | null = null;
let downloadItems: DownloadItem[] = [];
let statusBanner: { text: string; isError: boolean } | null = null;

function l(pt: string, en: string): string {
  return locale === "en" ? en : pt;
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

async function navigateInActiveTab(url: string): Promise<void> {
  const snap = await callBridge<WindowSnapshot>("windows.getSnapshot", {
    window_id: windowId,
  }).catch(() => undefined);
  if (snap?.active_tab_id) {
    await callBridge("tabs.navigate", { tab_id: snap.active_tab_id, input: url });
  }
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let val = bytes;
  let idx = 0;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx += 1;
  }
  return `${val.toFixed(idx === 0 ? 0 : 1)} ${units[idx]}`;
}

function formatDate(tsSecs: number): string {
  if (!tsSecs) return "";
  try {
    return new Date(tsSecs * 1000).toLocaleString(locale === "en" ? "en-US" : "pt-BR");
  } catch {
    return "";
  }
}

async function reloadData(): Promise<void> {
  if (pageKind === "history") {
    const items = await callBridge<HistoryItem[]>("history.list", {
      query: historyQuery || undefined,
      limit: 200,
    }).catch(() => []);
    historyItems = items ?? [];
  } else if (pageKind === "bookmarks") {
    const items = await callBridge<BookmarkItem[]>("bookmarks.list").catch(() => []);
    bookmarkItems = items ?? [];
  } else {
    const items = await callBridge<DownloadItem[]>("downloads.list").catch(() => []);
    downloadItems = items ?? [];
  }
  render();
}

function renderNavHeader(shell: HTMLElement): void {
  const header = document.createElement("header");
  header.className = "settings-header";

  const title = document.createElement("h1");
  title.className = "settings-title";
  const meta: Record<typeof pageKind, [string, string, string]> = {
    history: ["fa-solid fa-clock-rotate-left", "Histórico", "History"],
    bookmarks: ["fa-solid fa-bookmark", "Favoritos", "Bookmarks"],
    downloads: ["fa-solid fa-download", "Downloads", "Downloads"],
  };
  const [icon, pt, en] = meta[pageKind];
  title.innerHTML = `<i class="${icon}" aria-hidden="true"></i><span>${l(pt, en)}</span>`;
  header.appendChild(title);

  const tabs = document.createElement("div");
  tabs.className = "settings-tabs";
  const links: [string, string, string, boolean][] = [
    ["seu://history/", "fa-solid fa-clock-rotate-left", l("Histórico", "History"), pageKind === "history"],
    ["seu://bookmarks/", "fa-solid fa-bookmark", l("Favoritos", "Bookmarks"), pageKind === "bookmarks"],
    ["seu://downloads/", "fa-solid fa-download", l("Downloads", "Downloads"), pageKind === "downloads"],
    ["seu://settings/", "fa-solid fa-sliders", l("Configurações", "Settings"), false],
  ];
  for (const [href, ic, lbl, active] of links) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `tab-btn${active ? " active" : ""}`;
    btn.innerHTML = `<i class="${ic}" aria-hidden="true"></i><span>${lbl}</span>`;
    btn.addEventListener("click", () => {
      if (!active) void navigateInActiveTab(href);
    });
    tabs.appendChild(btn);
  }
  header.appendChild(tabs);
  shell.appendChild(header);

  if (statusBanner) {
    const b = document.createElement("div");
    b.className = `status-banner${statusBanner.isError ? " error" : ""}`;
    b.innerHTML = `<i class="fa-solid ${statusBanner.isError ? "fa-triangle-exclamation" : "fa-circle-check"}" aria-hidden="true"></i><span></span>`;
    const span = b.querySelector("span");
    if (span) span.textContent = statusBanner.text;
    shell.appendChild(b);
  }
}

function renderHistory(shell: HTMLElement): void {
  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";
  head.innerHTML = `<h2><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><span>${l("Histórico de navegação", "Browsing history")} (${historyItems.length})</span></h2>`;

  const clearBtn = document.createElement("button");
  clearBtn.type = "button";
  clearBtn.className = "btn btn-danger";
  clearBtn.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i><span>${l("Limpar tudo", "Clear all")}</span>`;
  clearBtn.addEventListener("click", () => {
    void callBridge("history.clear").then(() => {
      statusBanner = { text: l("Histórico limpo.", "History cleared."), isError: false };
      void reloadData();
    });
  });
  head.appendChild(clearBtn);
  card.appendChild(head);

  const searchForm = document.createElement("form");
  searchForm.className = "form-inline";
  const input = document.createElement("input");
  input.className = "text-input";
  input.value = historyQuery;
  input.placeholder = l("Buscar no histórico por título ou URL...", "Search history by title or URL...");
  const searchBtn = document.createElement("button");
  searchBtn.type = "submit";
  searchBtn.className = "btn btn-primary";
  searchBtn.innerHTML = `<i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><span>${l("Buscar", "Search")}</span>`;
  searchForm.appendChild(input);
  searchForm.appendChild(searchBtn);
  searchForm.addEventListener("submit", (ev) => {
    ev.preventDefault();
    historyQuery = input.value.trim();
    void reloadData();
  });
  card.appendChild(searchForm);

  const list = document.createElement("div");
  list.className = "shortcut-list";
  if (historyItems.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = l("Nenhum item de histórico encontrado.", "No history items found.");
    list.appendChild(empty);
  } else {
    for (const item of historyItems) {
      const row = document.createElement("div");
      row.className = "shortcut-row";

      const link = document.createElement("a");
      link.href = item.url;
      link.className = "data-item";
      link.style.flex = "1";
      link.style.border = "none";
      link.style.background = "transparent";
      link.style.padding = "0";
      link.addEventListener("click", (ev) => {
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
      uEl.textContent = `${item.url}  •  ${formatDate(item.visited_at)}`;
      link.appendChild(tEl);
      link.appendChild(uEl);
      row.appendChild(link);

      const delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.className = "btn btn-danger";
      delBtn.innerHTML = `<i class="fa-solid fa-xmark" aria-hidden="true"></i>`;
      delBtn.addEventListener("click", () => {
        void callBridge("history.delete", { id: item.id }).then(() => reloadData());
      });
      row.appendChild(delBtn);
      list.appendChild(row);
    }
  }
  card.appendChild(list);
  shell.appendChild(card);
}

function renderBookmarks(shell: HTMLElement): void {
  const folders = bookmarkItems.filter((b) => b.is_folder);
  const links = bookmarkItems.filter(
    (b) => !b.is_folder && (selectedFolderId === null || b.parent_id === selectedFolderId),
  );

  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";
  head.innerHTML = `<h2><i class="fa-solid fa-bookmark" aria-hidden="true"></i><span>${l("Favoritos e Pastas", "Bookmarks & Folders")} (${links.length})</span></h2>`;

  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = ".html,.htm,text/html";
  fileInput.style.display = "none";
  fileInput.addEventListener("change", () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    void file.text().then((html) => {
      void callBridge<{ count: number }>("bookmarks.importNetscape", { html }).then((res) => {
        statusBanner = {
          text: l(
            `Importados ${res?.count ?? 0} favoritos do arquivo HTML Netscape.`,
            `Imported ${res?.count ?? 0} bookmarks from Netscape HTML file.`,
          ),
          isError: false,
        };
        void reloadData();
      });
    });
  });

  const importBtn = document.createElement("button");
  importBtn.type = "button";
  importBtn.className = "btn btn-primary";
  importBtn.innerHTML = `<i class="fa-solid fa-file-import" aria-hidden="true"></i><span>${l("Importar HTML (Netscape)", "Import Netscape HTML")}</span>`;
  importBtn.addEventListener("click", () => fileInput.click());
  head.appendChild(fileInput);
  head.appendChild(importBtn);
  card.appendChild(head);

  // Create folder form
  const folderForm = document.createElement("form");
  folderForm.className = "form-inline";
  const folderInput = document.createElement("input");
  folderInput.className = "text-input";
  folderInput.placeholder = l("Nova pasta (ex: Trabalho, Dev)...", "New folder (e.g. Work, Dev)...");
  const folderBtn = document.createElement("button");
  folderBtn.type = "submit";
  folderBtn.className = "btn";
  folderBtn.innerHTML = `<i class="fa-solid fa-folder-plus" aria-hidden="true"></i><span>${l("Criar pasta", "Create folder")}</span>`;
  folderForm.appendChild(folderInput);
  folderForm.appendChild(folderBtn);
  folderForm.addEventListener("submit", (ev) => {
    ev.preventDefault();
    const title = folderInput.value.trim();
    if (!title) return;
    void callBridge("bookmarks.createFolder", { title, parent_id: null }).then(() => {
      folderInput.value = "";
      void reloadData();
    });
  });
  card.appendChild(folderForm);

  // Folder filter chips
  const folderRow = document.createElement("div");
  folderRow.className = "settings-tabs";
  const allChip = document.createElement("button");
  allChip.type = "button";
  allChip.className = `tab-btn${selectedFolderId === null ? " active" : ""}`;
  allChip.innerHTML = `<i class="fa-solid fa-layer-group" aria-hidden="true"></i><span>${l("Todos", "All")}</span>`;
  allChip.addEventListener("click", () => {
    selectedFolderId = null;
    render();
  });
  folderRow.appendChild(allChip);

  for (const f of folders) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = `tab-btn${selectedFolderId === f.id ? " active" : ""}`;
    chip.innerHTML = `<i class="fa-solid fa-folder" aria-hidden="true"></i><span></span>`;
    const sp = chip.querySelector("span");
    if (sp) sp.textContent = f.title;
    chip.addEventListener("click", () => {
      selectedFolderId = f.id;
      render();
    });
    folderRow.appendChild(chip);
  }
  card.appendChild(folderRow);

  // Add bookmark form
  const addForm = document.createElement("form");
  addForm.className = "form-inline";
  const titleIn = document.createElement("input");
  titleIn.className = "text-input";
  titleIn.placeholder = l("Título do favorito", "Bookmark title");
  const urlIn = document.createElement("input");
  urlIn.className = "text-input";
  urlIn.placeholder = "https://...";
  const addBtn = document.createElement("button");
  addBtn.type = "submit";
  addBtn.className = "btn btn-primary";
  addBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i><span>${l("Salvar", "Save")}</span>`;
  addForm.appendChild(titleIn);
  addForm.appendChild(urlIn);
  addForm.appendChild(addBtn);
  addForm.addEventListener("submit", (ev) => {
    ev.preventDefault();
    let url = urlIn.value.trim();
    if (!url) return;
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      url = `https://${url}`;
    }
    const title = titleIn.value.trim() || url;
    void callBridge("bookmarks.toggle", {
      title,
      url,
      parent_id: selectedFolderId,
    }).then(() => reloadData());
  });
  card.appendChild(addForm);

  // Bookmark rows
  const list = document.createElement("div");
  list.className = "shortcut-list";
  if (links.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = l(
      "Nenhum favorito nesta pasta. Use Ctrl+D em qualquer site ou importe um HTML Netscape.",
      "No bookmarks in this folder. Press Ctrl+D on any site or import a Netscape HTML file.",
    );
    list.appendChild(empty);
  } else {
    for (const item of links) {
      const row = document.createElement("div");
      row.className = "shortcut-row";

      const link = document.createElement("a");
      link.href = item.url;
      link.className = "data-item";
      link.style.flex = "1";
      link.style.border = "none";
      link.style.background = "transparent";
      link.style.padding = "0";
      link.addEventListener("click", (ev) => {
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
      link.appendChild(tEl);
      link.appendChild(uEl);
      row.appendChild(link);

      const delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.className = "btn btn-danger";
      delBtn.innerHTML = `<i class="fa-solid fa-trash" aria-hidden="true"></i>`;
      delBtn.addEventListener("click", () => {
        void callBridge("bookmarks.delete", { id: item.id }).then(() => reloadData());
      });
      row.appendChild(delBtn);
      list.appendChild(row);
    }
  }
  card.appendChild(list);
  shell.appendChild(card);
}

function renderDownloads(shell: HTMLElement): void {
  const card = document.createElement("section");
  card.className = "panel-card";

  const head = document.createElement("div");
  head.className = "panel-head";
  head.innerHTML = `<h2><i class="fa-solid fa-download" aria-hidden="true"></i><span>${l("Gerenciador de Downloads", "Downloads Manager")} (${downloadItems.length})</span></h2>`;

  const clearBtn = document.createElement("button");
  clearBtn.type = "button";
  clearBtn.className = "btn";
  clearBtn.innerHTML = `<i class="fa-solid fa-broom" aria-hidden="true"></i><span>${l("Limpar concluídos", "Clear finished")}</span>`;
  clearBtn.addEventListener("click", () => {
    void callBridge("downloads.clearFinished").then(() => reloadData());
  });
  head.appendChild(clearBtn);
  card.appendChild(head);

  const list = document.createElement("div");
  list.className = "shortcut-list";
  if (downloadItems.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = l(
      "Nenhum download registrado ainda. Arquivos baixados são salvos na sua pasta Downloads.",
      "No downloads recorded yet. Downloaded files are saved to your Downloads folder.",
    );
    list.appendChild(empty);
  } else {
    for (const dl of downloadItems) {
      const row = document.createElement("div");
      row.className = "shortcut-row";
      row.style.flexWrap = "wrap";

      const info = document.createElement("div");
      info.style.flex = "1";
      info.style.minWidth = "220px";
      const pct =
        dl.total_bytes > 0
          ? Math.min(100, Math.round((dl.received_bytes / dl.total_bytes) * 100))
          : dl.state === "complete"
            ? 100
            : 0;
      const title = document.createElement("div");
      title.className = "data-item-title";
      title.textContent = dl.filename;
      const sub = document.createElement("div");
      sub.className = "data-item-url";
      sub.textContent = `${dl.state.toUpperCase()} • ${formatBytes(dl.received_bytes)}${dl.total_bytes > 0 ? ` / ${formatBytes(dl.total_bytes)} (${pct}%)` : ""} • ${dl.url}`;
      info.appendChild(title);
      info.appendChild(sub);
      row.appendChild(info);

      const controls = document.createElement("div");
      controls.style.display = "flex";
      controls.style.gap = "6px";

      if (dl.state === "in_progress") {
        controls.appendChild(makeDlBtn("fa-solid fa-pause", l("Pausar", "Pause"), dl.id, "pause"));
        controls.appendChild(makeDlBtn("fa-solid fa-xmark", l("Cancelar", "Cancel"), dl.id, "cancel", true));
      } else if (dl.state === "paused") {
        controls.appendChild(makeDlBtn("fa-solid fa-play", l("Retomar", "Resume"), dl.id, "resume"));
        controls.appendChild(makeDlBtn("fa-solid fa-xmark", l("Cancelar", "Cancel"), dl.id, "cancel", true));
      } else if (dl.state === "complete") {
        controls.appendChild(makeDlBtn("fa-solid fa-arrow-up-right-from-square", l("Abrir", "Open"), dl.id, "open"));
        controls.appendChild(makeDlBtn("fa-solid fa-folder-open", l("Pasta", "Folder"), dl.id, "showInFolder"));
        controls.appendChild(makeDlBtn("fa-solid fa-trash", "", dl.id, "remove", true));
      } else {
        controls.appendChild(makeDlBtn("fa-solid fa-trash", l("Remover", "Remove"), dl.id, "remove", true));
      }

      row.appendChild(controls);
      list.appendChild(row);
    }
  }
  card.appendChild(list);
  shell.appendChild(card);
}

function makeDlBtn(
  icon: string,
  label: string,
  id: number,
  action: string,
  danger = false,
): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = danger ? "btn btn-danger" : "btn";
  btn.innerHTML = `<i class="${icon}" aria-hidden="true"></i>${label ? `<span>${label}</span>` : ""}`;
  btn.addEventListener("click", () => {
    void callBridge("downloads.action", { id, action }).then(() => reloadData());
  });
  return btn;
}

function render(): void {
  root.innerHTML = "";
  const shell = document.createElement("div");
  shell.className = "settings-shell";
  renderNavHeader(shell);

  if (pageKind === "history") {
    renderHistory(shell);
  } else if (pageKind === "bookmarks") {
    renderBookmarks(shell);
  } else {
    renderDownloads(shell);
  }
  root.appendChild(shell);
}

onBridgeEvent((event) => {
  if (event.type === "theme") {
    applyThemeTokens(event.theme);
  } else if (event.type === "settings") {
    locale = event.settings.locale;
    setLocale(locale);
    render();
  } else if (event.type === "bookmarksChanged" && pageKind === "bookmarks") {
    void reloadData();
  } else if (event.type === "downloads" && pageKind === "downloads") {
    downloadItems = event.items;
    render();
  }
});

async function bootstrap(): Promise<void> {
  const [settings, theme] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
  ]);
  if (settings) {
    locale = settings.locale;
    setLocale(locale);
  }
  if (theme) {
    applyThemeTokens(theme);
  }
  await reloadData();
}

void bootstrap();
