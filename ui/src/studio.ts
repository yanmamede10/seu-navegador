import "./icons";
import "./studio.css";
import { basicSetup } from "codemirror";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { javascript } from "@codemirror/lang-javascript";
import { rust } from "@codemirror/lang-rust";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type CoreSettings,
  type LoadedTheme,
  type StudioFileContent,
  type StudioFileEntry,
  type StudioSearchMatch,
  type StudioTerminalSnapshot,
  type StudioWorkspaceSummary,
} from "./bridge";

const root = document.getElementById("studio-root") as HTMLDivElement;
const windowId = getWindowIdFromQuery();

interface OpenBuffer {
  rel_path: string;
  name: string;
  language: string;
  content: string;
  savedContent: string;
}

let locale: "pt-br" | "en" = "pt-br";
let workspace: StudioWorkspaceSummary | null = null;
let dirCache = new Map<string, StudioFileEntry[]>();
let expandedDirs = new Set<string>();
let sidebarVisible = true;
let sidebarTab: "files" | "search" = "files";
let searchQuery = "";
let searchResults: StudioSearchMatch[] = [];
let openBuffers: OpenBuffer[] = [];
let activePath: string | null = null;
let diffMode = false;
let diffText = "";
let terminalCollapsed = false;
let terminalSnap: StudioTerminalSnapshot | null = null;
let statusText = "Pronto";
let editorView: EditorView | null = null;

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

// Themeable CodeMirror 6 theme bound 100% to active browser theme `--seu-*` CSS tokens.
const seuCodeMirrorTheme = EditorView.theme({
  "&": {
    color: "var(--seu-text, #E6E9FF)",
    backgroundColor: "var(--seu-bg, #070B24)",
  },
  ".cm-content": {
    caretColor: "var(--seu-accent, #FF2E93)",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--seu-accent, #FF2E93)",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
    backgroundColor: "rgba(255, 46, 147, 0.28)",
  },
  ".cm-gutters": {
    backgroundColor: "var(--seu-surface, #0B1240)",
    color: "var(--seu-text-muted, #8F9BD9)",
    borderRight: "1px solid var(--seu-border, #2A3680)",
  },
  ".cm-activeLine": {
    backgroundColor: "rgba(255, 255, 255, 0.04)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "var(--seu-surface2, #0E1650)",
    color: "var(--seu-text, #E6E9FF)",
  },
});

function languageExtension(lang: string): Extension[] {
  switch (lang) {
    case "rust":
      return [rust()];
    case "typescript":
      return [javascript({ typescript: true, jsx: true })];
    case "javascript":
      return [javascript({ jsx: true })];
    case "html":
      return [html()];
    case "css":
      return [css()];
    case "json":
      return [json()];
    case "markdown":
      return [markdown()];
    case "python":
      return [python()];
    default:
      return [];
  }
}

function getActiveBuffer(): OpenBuffer | null {
  if (!activePath) return null;
  return openBuffers.find((b) => b.rel_path === activePath) ?? null;
}

async function openWorkspaceByPath(path: string): Promise<void> {
  const trimmed = path.trim();
  if (!trimmed) return;
  try {
    const ws = await callBridge<StudioWorkspaceSummary>("studio.openWorkspace", {
      path: trimmed,
    });
    if (ws) {
      workspace = ws;
      dirCache = new Map([["", ws.entries]]);
      expandedDirs = new Set();
      openBuffers = [];
      activePath = null;
      diffMode = false;
      statusText = l(`Workspace aberto: ${ws.name}`, `Workspace opened: ${ws.name}`);
      render();
    }
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function openFile(relPath: string): Promise<void> {
  const existing = openBuffers.find((b) => b.rel_path === relPath);
  if (existing) {
    activePath = existing.rel_path;
    if (diffMode) await refreshDiff();
    render();
    return;
  }
  try {
    const file = await callBridge<StudioFileContent>("studio.readFile", {
      rel_path: relPath,
    });
    if (file) {
      openBuffers.push({
        rel_path: file.rel_path,
        name: file.name,
        language: file.language,
        content: file.content,
        savedContent: file.content,
      });
      activePath = file.rel_path;
      statusText = `${file.rel_path} (${file.language})`;
      if (diffMode) await refreshDiff();
      render();
    }
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function saveActiveFile(): Promise<void> {
  const buf = getActiveBuffer();
  if (!buf) return;
  try {
    const res = await callBridge<{ bytes: number }>("studio.writeFile", {
      rel_path: buf.rel_path,
      content: buf.content,
    });
    buf.savedContent = buf.content;
    statusText = l(
      `Salvo: ${buf.rel_path} (${res?.bytes ?? buf.content.length} bytes)`,
      `Saved: ${buf.rel_path} (${res?.bytes ?? buf.content.length} bytes)`,
    );
    const git = await callBridge<StudioWorkspaceSummary["git"]>("studio.gitStatus").catch(
      () => undefined,
    );
    if (git && workspace) {
      workspace.git = git;
    }
    if (diffMode) await refreshDiff();
    render();
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function toggleDirectory(relPath: string): Promise<void> {
  if (expandedDirs.has(relPath)) {
    expandedDirs.delete(relPath);
    render();
    return;
  }
  try {
    const entries = await callBridge<StudioFileEntry[]>("studio.listDir", {
      rel_path: relPath,
    });
    dirCache.set(relPath, entries ?? []);
    expandedDirs.add(relPath);
    render();
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function refreshDiff(): Promise<void> {
  try {
    const res = await callBridge<{ diff: string }>("studio.gitDiff", {
      rel_path: activePath ?? undefined,
    });
    diffText = res?.diff ?? "";
  } catch {
    diffText = "";
  }
}

async function runTerminalCommand(cmd: string): Promise<void> {
  const trimmed = cmd.trim();
  if (!trimmed) return;
  terminalCollapsed = false;
  try {
    const snap = await callBridge<StudioTerminalSnapshot>("studio.runCommand", {
      command: trimmed,
    });
    if (snap) {
      terminalSnap = snap;
      statusText = l(`Executando: ${trimmed}`, `Running: ${trimmed}`);
      render();
    }
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

function mountCodeMirror(host: HTMLElement, buf: OpenBuffer): void {
  if (editorView) {
    editorView.destroy();
    editorView = null;
  }
  const saveKeymap = keymap.of([
    {
      key: "Mod-s",
      run: () => {
        void saveActiveFile();
        return true;
      },
    },
  ]);

  const state = EditorState.create({
    doc: buf.content,
    extensions: [
      basicSetup,
      seuCodeMirrorTheme,
      saveKeymap,
      ...languageExtension(buf.language),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          buf.content = update.state.doc.toString();
          const tabDirtyDot = root.querySelector(
            `.editor-tab[data-path="${CSS.escape(buf.rel_path)}"] .dirty-dot`,
          ) as HTMLElement | null;
          if (tabDirtyDot) {
            tabDirtyDot.style.display = buf.content !== buf.savedContent ? "inline-block" : "none";
          }
        }
      }),
    ],
  });

  editorView = new EditorView({
    state,
    parent: host,
  });
}

function renderFileNodes(
  container: HTMLElement,
  entries: StudioFileEntry[],
  depth: number,
): void {
  for (const entry of entries) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `tree-row${activePath === entry.rel_path ? " selected" : ""}`;
    btn.style.paddingLeft = `${8 + depth * 14}px`;

    const icon = document.createElement("i");
    if (entry.is_dir) {
      const open = expandedDirs.has(entry.rel_path);
      icon.className = open ? "fa-solid fa-folder-open" : "fa-solid fa-folder";
    } else {
      icon.className = "fa-regular fa-file-code";
    }
    icon.setAttribute("aria-hidden", "true");
    btn.appendChild(icon);

    const nameSpan = document.createElement("span");
    nameSpan.className = "tree-row-name";
    nameSpan.textContent = entry.name;
    btn.appendChild(nameSpan);

    if (entry.git_status) {
      const badge = document.createElement("span");
      badge.className = "git-badge";
      badge.textContent = entry.git_status;
      btn.appendChild(badge);
    }

    btn.addEventListener("click", () => {
      if (entry.is_dir) {
        void toggleDirectory(entry.rel_path);
      } else {
        void openFile(entry.rel_path);
      }
    });
    container.appendChild(btn);

    if (entry.is_dir && expandedDirs.has(entry.rel_path)) {
      const children = dirCache.get(entry.rel_path) ?? [];
      renderFileNodes(container, children, depth + 1);
    }
  }
}

function render(): void {
  if (editorView) {
    editorView.destroy();
    editorView = null;
  }
  root.innerHTML = "";

  const app = document.createElement("div");
  app.className = "studio-app";

  // 1. Topbar
  const topbar = document.createElement("header");
  topbar.className = "studio-topbar";

  const brand = document.createElement("span");
  brand.className = "studio-brand";
  brand.innerHTML = `<i class="fa-solid fa-code" aria-hidden="true"></i><span>SEU STUDIO</span>`;
  topbar.appendChild(brand);

  const pickBtn = document.createElement("button");
  pickBtn.type = "button";
  pickBtn.className = "studio-btn primary";
  pickBtn.innerHTML = `<i class="fa-solid fa-folder-open" aria-hidden="true"></i><span>${l("Abrir pasta", "Open folder")}</span>`;
  pickBtn.addEventListener("click", () => {
    void callBridge("studio.pickFolder", { window_id: windowId });
  });
  topbar.appendChild(pickBtn);

  const pathForm = document.createElement("form");
  pathForm.className = "studio-path-form";
  const pathInput = document.createElement("input");
  pathInput.className = "studio-input";
  pathInput.value = workspace?.root_path ?? "";
  pathInput.placeholder = l(
    "Cole o caminho do projeto (ex: C:\\Projetos\\meu-app) e aperte Enter",
    "Paste project path (e.g. C:\\Projects\\my-app) and press Enter",
  );
  const openBtn = document.createElement("button");
  openBtn.type = "submit";
  openBtn.className = "studio-btn";
  openBtn.innerHTML = `<i class="fa-solid fa-arrow-right" aria-hidden="true"></i>`;
  pathForm.appendChild(pathInput);
  pathForm.appendChild(openBtn);
  pathForm.addEventListener("submit", (ev) => {
    ev.preventDefault();
    void openWorkspaceByPath(pathInput.value);
  });
  topbar.appendChild(pathForm);

  if (workspace?.git.is_repo) {
    const gitPill = document.createElement("span");
    gitPill.className = "studio-git-pill";
    gitPill.innerHTML = `<i class="fa-solid fa-code-branch" aria-hidden="true"></i><span></span>`;
    const sp = gitPill.querySelector("span");
    if (sp) {
      sp.textContent = `${workspace.git.branch} (${workspace.git.changed_files.length})`;
    }
    topbar.appendChild(gitPill);
  }

  const saveBtn = document.createElement("button");
  saveBtn.type = "button";
  saveBtn.className = "studio-btn";
  saveBtn.innerHTML = `<i class="fa-solid fa-floppy-disk" aria-hidden="true"></i><span>${l("Salvar", "Save")}</span>`;
  saveBtn.addEventListener("click", () => void saveActiveFile());
  topbar.appendChild(saveBtn);

  const diffBtn = document.createElement("button");
  diffBtn.type = "button";
  diffBtn.className = `studio-btn${diffMode ? " active" : ""}`;
  diffBtn.innerHTML = `<i class="fa-solid fa-code-compare" aria-hidden="true"></i><span>Diff</span>`;
  diffBtn.addEventListener("click", () => {
    diffMode = !diffMode;
    if (diffMode) {
      void refreshDiff().then(() => render());
    } else {
      render();
    }
  });
  topbar.appendChild(diffBtn);

  const sideToggle = document.createElement("button");
  sideToggle.type = "button";
  sideToggle.className = `studio-btn${sidebarVisible ? " active" : ""}`;
  sideToggle.innerHTML = `<i class="fa-solid fa-bars-staggered" aria-hidden="true"></i>`;
  sideToggle.title = l("Mostrar/ocultar Explorer", "Toggle Explorer");
  sideToggle.addEventListener("click", () => {
    sidebarVisible = !sidebarVisible;
    render();
  });
  topbar.appendChild(sideToggle);

  app.appendChild(topbar);

  // 2. Main Body (Sidebar + Editor/Terminal)
  const body = document.createElement("div");
  body.className = "studio-body";

  // Left Sidebar
  const sidebar = document.createElement("aside");
  sidebar.className = `studio-sidebar${sidebarVisible ? "" : " hidden"}`;

  const sTabs = document.createElement("div");
  sTabs.className = "sidebar-tabs";
  const filesTab = document.createElement("button");
  filesTab.type = "button";
  filesTab.className = `sidebar-tab${sidebarTab === "files" ? " active" : ""}`;
  filesTab.innerHTML = `<i class="fa-solid fa-folder-tree" aria-hidden="true"></i><span>${l("Arquivos", "Files")}</span>`;
  filesTab.addEventListener("click", () => {
    sidebarTab = "files";
    render();
  });

  const searchTab = document.createElement("button");
  searchTab.type = "button";
  searchTab.className = `sidebar-tab${sidebarTab === "search" ? " active" : ""}`;
  searchTab.innerHTML = `<i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><span>${l("Buscar", "Search")}</span>`;
  searchTab.addEventListener("click", () => {
    sidebarTab = "search";
    render();
  });
  sTabs.appendChild(filesTab);
  sTabs.appendChild(searchTab);
  sidebar.appendChild(sTabs);

  if (sidebarTab === "files") {
    if (workspace) {
      const newForm = document.createElement("form");
      newForm.className = "sidebar-toolbar";
      const newIn = document.createElement("input");
      newIn.className = "studio-input";
      newIn.placeholder = l("novo-arquivo.ts", "new-file.ts");
      const newBtn = document.createElement("button");
      newBtn.type = "submit";
      newBtn.className = "studio-btn";
      newBtn.innerHTML = `<i class="fa-solid fa-plus" aria-hidden="true"></i>`;
      newForm.appendChild(newIn);
      newForm.appendChild(newBtn);
      newForm.addEventListener("submit", (ev) => {
        ev.preventDefault();
        const rel = newIn.value.trim();
        if (!rel) return;
        void callBridge("studio.createEntry", { rel_path: rel, is_dir: false }).then(async () => {
          if (workspace) {
            const entries = await callBridge<StudioFileEntry[]>("studio.listDir", {
              rel_path: "",
            });
            dirCache.set("", entries ?? []);
          }
          await openFile(rel);
        });
      });
      sidebar.appendChild(newForm);
    }

    const tree = document.createElement("div");
    tree.className = "tree-scroll";
    if (!workspace) {
      const empty = document.createElement("div");
      empty.style.padding = "12px";
      empty.style.fontSize = "12px";
      empty.style.color = "var(--seu-text-muted)";
      empty.textContent = l(
        "Abra uma pasta acima para explorar arquivos, editar código e rodar comandos.",
        "Open a folder above to browse files, edit code, and run commands.",
      );
      tree.appendChild(empty);
    } else {
      renderFileNodes(tree, dirCache.get("") ?? workspace.entries, 0);
    }
    sidebar.appendChild(tree);
  } else {
    const searchForm = document.createElement("form");
    searchForm.className = "sidebar-toolbar";
    const sIn = document.createElement("input");
    sIn.className = "studio-input";
    sIn.value = searchQuery;
    sIn.placeholder = l("Buscar no projeto...", "Search workspace...");
    const sBtn = document.createElement("button");
    sBtn.type = "submit";
    sBtn.className = "studio-btn primary";
    sBtn.innerHTML = `<i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i>`;
    searchForm.appendChild(sIn);
    searchForm.appendChild(sBtn);
    searchForm.addEventListener("submit", (ev) => {
      ev.preventDefault();
      searchQuery = sIn.value.trim();
      if (!searchQuery) return;
      void callBridge<StudioSearchMatch[]>("studio.search", { query: searchQuery }).then((res) => {
        searchResults = res ?? [];
        render();
      });
    });
    sidebar.appendChild(searchForm);

    const resScroll = document.createElement("div");
    resScroll.className = "tree-scroll";
    for (const m of searchResults) {
      const item = document.createElement("div");
      item.className = "search-match";
      const fEl = document.createElement("div");
      fEl.className = "search-match-file";
      fEl.textContent = `${m.rel_path}:${m.line}`;
      const pEl = document.createElement("div");
      pEl.className = "search-match-line";
      pEl.textContent = m.preview;
      item.appendChild(fEl);
      item.appendChild(pEl);
      item.addEventListener("click", () => void openFile(m.rel_path));
      resScroll.appendChild(item);
    }
    sidebar.appendChild(resScroll);
  }
  body.appendChild(sidebar);

  // Center: Editor tabs + CodeMirror 6 + Terminal
  const main = document.createElement("main");
  main.className = "studio-main";

  const eTabs = document.createElement("div");
  eTabs.className = "editor-tabs";
  for (const buf of openBuffers) {
    const tab = document.createElement("div");
    tab.className = `editor-tab${buf.rel_path === activePath ? " active" : ""}`;
    tab.dataset.path = buf.rel_path;

    const dot = document.createElement("span");
    dot.className = "dirty-dot";
    dot.style.display = buf.content !== buf.savedContent ? "inline-block" : "none";
    tab.appendChild(dot);

    const name = document.createElement("span");
    name.textContent = buf.name;
    tab.appendChild(name);

    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.style.border = "none";
    closeBtn.style.background = "transparent";
    closeBtn.style.color = "inherit";
    closeBtn.style.cursor = "pointer";
    closeBtn.style.padding = "0 2px";
    closeBtn.innerHTML = `<i class="fa-solid fa-xmark" aria-hidden="true"></i>`;
    closeBtn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      openBuffers = openBuffers.filter((b) => b.rel_path !== buf.rel_path);
      if (activePath === buf.rel_path) {
        activePath = openBuffers[0]?.rel_path ?? null;
      }
      render();
    });
    tab.appendChild(closeBtn);

    tab.addEventListener("click", () => {
      activePath = buf.rel_path;
      if (diffMode) {
        void refreshDiff().then(() => render());
      } else {
        render();
      }
    });
    eTabs.appendChild(tab);
  }
  main.appendChild(eTabs);

  const stage = document.createElement("div");
  stage.className = "editor-stage";
  const activeBuf = getActiveBuffer();
  if (diffMode) {
    const diffBox = document.createElement("div");
    diffBox.className = "diff-view";
    if (!diffText.trim()) {
      diffBox.textContent = l(
        "Sem alterações git diff para exibir.",
        "No git diff changes to display.",
      );
    } else {
      for (const line of diffText.split("\n")) {
        const span = document.createElement("span");
        if (line.startsWith("+") && !line.startsWith("+++")) {
          span.className = "diff-add";
        } else if (line.startsWith("-") && !line.startsWith("---")) {
          span.className = "diff-del";
        } else if (line.startsWith("@@")) {
          span.className = "diff-hunk";
        }
        span.textContent = line;
        diffBox.appendChild(span);
        diffBox.appendChild(document.createTextNode("\n"));
      }
    }
    stage.appendChild(diffBox);
  } else if (activeBuf) {
    const host = document.createElement("div");
    host.className = "cm-host";
    stage.appendChild(host);
    mountCodeMirror(host, activeBuf);
  } else {
    const empty = document.createElement("div");
    empty.className = "studio-empty";
    empty.innerHTML = `
      <i class="fa-solid fa-laptop-code fa-2x" style="color: var(--seu-accent)" aria-hidden="true"></i>
      <h2>${l("Seu Navegador Studio (IDE Leve)", "Seu Navegador Studio (Lightweight IDE)")}</h2>
      <p style="max-width:460px;font-size:13px;margin:0">${l(
        "Abra uma pasta de projeto, edite com CodeMirror 6 herdando 100% do seu tema ativo e use o modo Split na barra superior para manter seu app ou documentação aberto ao lado.",
        "Open a project folder, edit with CodeMirror 6 inheriting 100% of your active theme, and use Split mode in the top toolbar to keep your live app or docs side by side.",
      )}</p>
    `;
    stage.appendChild(empty);
  }
  main.appendChild(stage);

  // Integrated Terminal
  const term = document.createElement("section");
  term.className = `studio-terminal${terminalCollapsed ? " collapsed" : ""}`;

  const tHead = document.createElement("div");
  tHead.className = "terminal-head";
  tHead.innerHTML = `<i class="fa-solid fa-terminal" style="color: var(--seu-accent)" aria-hidden="true"></i><span>${l("TERMINAL INTEGRADO", "INTEGRATED TERMINAL")}</span>`;

  if (terminalSnap) {
    const pill = document.createElement("span");
    pill.style.fontFamily = "monospace";
    pill.style.fontSize = "11px";
    pill.style.color = terminalSnap.running ? "#B6FF3B" : "var(--seu-text-muted)";
    pill.textContent = terminalSnap.running
      ? "RUNNING"
      : `EXIT ${terminalSnap.exit_code ?? 0}`;
    tHead.appendChild(pill);
  }

  const tSpacer = document.createElement("div");
  tSpacer.style.flex = "1";
  tHead.appendChild(tSpacer);

  if (terminalSnap?.running) {
    const killBtn = document.createElement("button");
    killBtn.type = "button";
    killBtn.className = "studio-btn danger";
    killBtn.style.height = "24px";
    killBtn.innerHTML = `<i class="fa-solid fa-stop" aria-hidden="true"></i><span>${l("Parar", "Stop")}</span>`;
    killBtn.addEventListener("click", () => {
      if (terminalSnap) {
        void callBridge("studio.killCommand", { session_id: terminalSnap.session_id });
      }
    });
    tHead.appendChild(killBtn);
  }

  const collapseBtn = document.createElement("button");
  collapseBtn.type = "button";
  collapseBtn.className = "studio-btn";
  collapseBtn.style.height = "24px";
  collapseBtn.innerHTML = `<i class="fa-solid ${terminalCollapsed ? "fa-chevron-up" : "fa-chevron-down"}" aria-hidden="true"></i>`;
  collapseBtn.addEventListener("click", () => {
    terminalCollapsed = !terminalCollapsed;
    render();
  });
  tHead.appendChild(collapseBtn);
  term.appendChild(tHead);

  if (!terminalCollapsed) {
    const out = document.createElement("pre");
    out.className = "terminal-output";
    out.textContent =
      terminalSnap?.output ??
      l(
        "Digite um comando abaixo (ex: git status, npm run dev, cargo --version) para rodar na pasta do projeto.",
        "Type a command below (e.g. git status, npm run dev, cargo --version) to run inside the workspace.",
      );
    term.appendChild(out);

    const tForm = document.createElement("form");
    tForm.className = "terminal-form";
    const tIn = document.createElement("input");
    tIn.className = "studio-input";
    tIn.placeholder = l("Comando no workspace...", "Run command in workspace...");
    const runBtn = document.createElement("button");
    runBtn.type = "submit";
    runBtn.className = "studio-btn primary";
    runBtn.innerHTML = `<i class="fa-solid fa-play" aria-hidden="true"></i><span>${l("Rodar", "Run")}</span>`;
    tForm.appendChild(tIn);
    tForm.appendChild(runBtn);
    tForm.addEventListener("submit", (ev) => {
      ev.preventDefault();
      const cmd = tIn.value.trim();
      if (!cmd) return;
      tIn.value = "";
      void runTerminalCommand(cmd);
    });
    term.appendChild(tForm);
    requestAnimationFrame(() => {
      out.scrollTop = out.scrollHeight;
    });
  }

  main.appendChild(term);
  body.appendChild(main);
  app.appendChild(body);

  // 3. Status bar
  const footer = document.createElement("footer");
  footer.className = "studio-status";
  const leftStatus = document.createElement("span");
  leftStatus.textContent = statusText;
  const rightStatus = document.createElement("span");
  rightStatus.textContent = workspace
    ? `Sandbox: ${workspace.name} • CodeMirror 6 • Ctrl+S ${l("salvar", "save")}`
    : `CodeMirror 6 • Ctrl+Shift+E Split`;
  footer.appendChild(leftStatus);
  footer.appendChild(rightStatus);
  app.appendChild(footer);

  root.appendChild(app);
}

onBridgeEvent((event) => {
  if (event.type === "theme") {
    applyThemeTokens(event.theme);
  } else if (event.type === "settings") {
    locale = event.settings.locale;
    render();
  } else if (event.type === "studioWorkspace") {
    workspace = event.workspace;
    dirCache = new Map([["", event.workspace.entries]]);
    expandedDirs = new Set();
    statusText = l(
      `Workspace aberto: ${event.workspace.name}`,
      `Workspace opened: ${event.workspace.name}`,
    );
    render();
  } else if (event.type === "studioTerminal") {
    terminalSnap = event.snapshot;
    const pre = root.querySelector(".terminal-output") as HTMLPreElement | null;
    if (pre && !terminalCollapsed) {
      pre.textContent = event.snapshot.output;
      pre.scrollTop = pre.scrollHeight;
      if (!event.snapshot.running) {
        render();
      }
    } else {
      render();
    }
  }
});

async function bootstrap(): Promise<void> {
  const [settings, theme, ws] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
    callBridge<StudioWorkspaceSummary | null>("studio.getWorkspace").catch(() => null),
  ]);
  if (settings) {
    locale = settings.locale;
  }
  if (theme) {
    applyThemeTokens(theme);
  }
  if (ws) {
    workspace = ws;
    dirCache = new Map([["", ws.entries]]);
    statusText = l(`Workspace: ${ws.name}`, `Workspace: ${ws.name}`);
  }
  render();
}

void bootstrap();
