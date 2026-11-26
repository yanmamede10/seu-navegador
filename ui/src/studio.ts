import "./icons";
import "./studio.css";
import { basicSetup } from "codemirror";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import {
  callBridge,
  getWindowIdFromQuery,
  onBridgeEvent,
  type AgentProviderConfig,
  type AgentRole,
  type AgentRunSnapshot,
  type BrowserQaSnapshot,
  type CoreSettings,
  type DetectedLocalCli,
  type LoadedTheme,
  type ProviderKind,
  type QaActionResult,
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

// Phase 5.6: Multi-Agent Orchestrator & Browser-Native QA state
let agentsVisible = true;
let agentsTab: "board" | "config" = "board";
let selectedRole: AgentRole = "planner";
let agentPromptDraft = "";
let detectedClis: DetectedLocalCli[] = [];
let agentRuns: AgentRunSnapshot[] = [];
let qaSnapshot: BrowserQaSnapshot | null = null;
let lastQaAction: QaActionResult | null = null;
let qaSelectorDraft = "h1";
let qaValueDraft = "";

type RoleConfigs = Record<AgentRole, AgentProviderConfig>;

function defaultRoleConfigs(): RoleConfigs {
  return {
    planner: {
      provider: "gemini-api",
      model: "gemini-2.5-flash",
      api_key: "",
      base_url: "",
      custom_cli_cmd: "",
    },
    coder: {
      provider: "cli-claude",
      model: "claude-code",
      api_key: "",
      base_url: "",
      custom_cli_cmd: "",
    },
    "qa-browser": {
      provider: "ollama-local",
      model: "qwen2.5-coder:7b",
      api_key: "",
      base_url: "http://127.0.0.1:11434",
      custom_cli_cmd: "",
    },
  };
}

let roleConfigs: RoleConfigs = defaultRoleConfigs();

async function saveRoleConfigs(): Promise<void> {
  try {
    await callBridge("studio.agentsConfigSave", {
      config_json: JSON.stringify(roleConfigs),
    });
    statusText = l("Configuração de agentes salva.", "Agent provider config saved.");
    render();
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function runAgent(role: AgentRole, prompt: string): Promise<void> {
  const trimmed = prompt.trim();
  if (!trimmed) return;
  try {
    const run = await callBridge<AgentRunSnapshot>("studio.agentsStart", {
      role,
      config: roleConfigs[role],
      prompt: trimmed,
    });
    if (run) {
      agentRuns = [run, ...agentRuns.filter((r) => r.run_id !== run.run_id)];
      statusText = l(`Agente ${role} iniciado (#${run.run_id})`, `Agent ${role} started (#${run.run_id})`);
      render();
    }
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

async function approveAgentPatch(runId: number, patchIndex: number): Promise<void> {
  try {
    const updated = await callBridge<AgentRunSnapshot>("studio.agentsApprovePatch", {
      run_id: runId,
      patch_index: patchIndex,
    });
    if (updated) {
      agentRuns = agentRuns.map((r) => (r.run_id === updated.run_id ? updated : r));
      const patch = updated.patches[patchIndex];
      if (patch) {
        statusText = l(`Patch aprovado: ${patch.rel_path}`, `Patch approved: ${patch.rel_path}`);
        const buf = openBuffers.find((b) => b.rel_path === patch.rel_path);
        if (buf) {
          buf.content = patch.new_content;
          buf.savedContent = patch.new_content;
        }
      }
      render();
    }
  } catch (err) {
    statusText = err instanceof Error ? err.message : String(err);
    render();
  }
}

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

const langCompartment = new Compartment();

async function loadLanguageExtension(lang: string): Promise<Extension[]> {
  switch (lang) {
    case "rust": {
      const m = await import("@codemirror/lang-rust");
      return [m.rust()];
    }
    case "typescript": {
      const m = await import("@codemirror/lang-javascript");
      return [m.javascript({ typescript: true, jsx: true })];
    }
    case "javascript": {
      const m = await import("@codemirror/lang-javascript");
      return [m.javascript({ jsx: true })];
    }
    case "html": {
      const m = await import("@codemirror/lang-html");
      return [m.html()];
    }
    case "css": {
      const m = await import("@codemirror/lang-css");
      return [m.css()];
    }
    case "json": {
      const m = await import("@codemirror/lang-json");
      return [m.json()];
    }
    case "markdown": {
      const m = await import("@codemirror/lang-markdown");
      return [m.markdown()];
    }
    case "python": {
      const m = await import("@codemirror/lang-python");
      return [m.python()];
    }
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
      langCompartment.of([]),
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

  const view = new EditorView({
    state,
    parent: host,
  });
  editorView = view;

  void loadLanguageExtension(buf.language).then((exts) => {
    if (editorView === view && exts.length > 0) {
      view.dispatch({ effects: langCompartment.reconfigure(exts) });
    }
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

  const agentsToggle = document.createElement("button");
  agentsToggle.type = "button";
  agentsToggle.className = `studio-btn${agentsVisible ? " active" : ""}`;
  agentsToggle.innerHTML = `<i class="fa-solid fa-diagram-project" aria-hidden="true"></i><span>${l("Agentes", "Agents")}</span>`;
  agentsToggle.addEventListener("click", () => {
    agentsVisible = !agentsVisible;
    render();
  });
  topbar.appendChild(agentsToggle);

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
  body.appendChild(renderAgentsPanel());
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

function renderAgentsPanel(): HTMLElement {
  const aside = document.createElement("aside");
  aside.className = `studio-agents${agentsVisible ? "" : " hidden"}`;

  const head = document.createElement("div");
  head.className = "agents-head";
  head.innerHTML = `<span><i class="fa-solid fa-diagram-project" style="color: var(--seu-accent)" aria-hidden="true"></i> ${l("ORQUESTRADOR", "ORCHESTRATOR")}</span>`;

  const modeBtns = document.createElement("div");
  modeBtns.style.display = "flex";
  modeBtns.style.gap = "4px";
  for (const [id, lbl] of [
    ["board", "Board"],
    ["config", l("Provedores", "Providers")],
  ] as const) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = `studio-btn${agentsTab === id ? " active" : ""}`;
    b.style.height = "24px";
    b.style.padding = "0 7px";
    b.textContent = lbl;
    b.addEventListener("click", () => {
      agentsTab = id;
      render();
    });
    modeBtns.appendChild(b);
  }
  head.appendChild(modeBtns);
  aside.appendChild(head);

  const scroll = document.createElement("div");
  scroll.className = "agents-scroll";

  if (agentsTab === "config") {
    const cliCard = document.createElement("div");
    cliCard.className = "agent-card";
    cliCard.innerHTML = `<div class="agent-card-top"><span><i class="fa-solid fa-terminal" aria-hidden="true"></i> ${l("CLIs locais detectados", "Detected local CLIs")}</span></div>`;
    const chips = document.createElement("div");
    chips.className = "cli-chips";
    for (const c of detectedClis) {
      const chip = document.createElement("span");
      chip.className = `cli-chip${c.available ? " on" : ""}`;
      chip.textContent = `${c.id}: ${c.available ? "OK" : "--"}`;
      chips.appendChild(chip);
    }
    cliCard.appendChild(chips);
    scroll.appendChild(cliCard);

    const providerOptions: [ProviderKind, string][] = [
      ["gemini-api", "API: Google Gemini"],
      ["anthropic-api", "API: Anthropic Claude"],
      ["open-ai-api", "API: OpenAI / Compatível"],
      ["ollama-local", "Local: Ollama HTTP"],
      ["cli-gemini", "CLI Local: gemini"],
      ["cli-claude", "CLI Local: claude"],
      ["cli-codex", "CLI Local: codex"],
      ["cli-custom", "CLI Customizado (stdin)"],
    ];

    for (const [role, title] of [
      ["planner", "1. Planner"],
      ["coder", "2. Coder"],
      ["qa-browser", "3. QA Browser"],
    ] as [AgentRole, string][]) {
      const cfg = roleConfigs[role];
      const card = document.createElement("div");
      card.className = "agent-card";
      card.innerHTML = `<div class="agent-card-top"><span>${title}</span></div>`;

      const sel = document.createElement("select");
      sel.className = "studio-select";
      for (const [val, label] of providerOptions) {
        const opt = document.createElement("option");
        opt.value = val;
        opt.textContent = label;
        if (cfg.provider === val) opt.selected = true;
        sel.appendChild(opt);
      }
      sel.addEventListener("change", () => {
        cfg.provider = sel.value as ProviderKind;
        render();
      });
      card.appendChild(sel);

      const modelIn = document.createElement("input");
      modelIn.className = "studio-input";
      modelIn.value = cfg.model;
      modelIn.placeholder = l("Modelo (ex: gemini-2.5-flash, qwen2.5-coder:7b)", "Model name");
      modelIn.addEventListener("input", () => {
        cfg.model = modelIn.value;
      });
      card.appendChild(modelIn);

      if (
        cfg.provider === "gemini-api" ||
        cfg.provider === "anthropic-api" ||
        cfg.provider === "open-ai-api"
      ) {
        const keyIn = document.createElement("input");
        keyIn.type = "password";
        keyIn.className = "studio-input";
        keyIn.value = cfg.api_key;
        keyIn.placeholder = "API Key...";
        keyIn.addEventListener("input", () => {
          cfg.api_key = keyIn.value;
        });
        card.appendChild(keyIn);
      }

      if (cfg.provider === "ollama-local" || cfg.provider === "open-ai-api") {
        const urlIn = document.createElement("input");
        urlIn.className = "studio-input";
        urlIn.value = cfg.base_url;
        urlIn.placeholder =
          cfg.provider === "ollama-local"
            ? "http://127.0.0.1:11434"
            : "https://api.openai.com/v1";
        urlIn.addEventListener("input", () => {
          cfg.base_url = urlIn.value;
        });
        card.appendChild(urlIn);
      }

      if (cfg.provider === "cli-custom") {
        const cmdIn = document.createElement("input");
        cmdIn.className = "studio-input";
        cmdIn.value = cfg.custom_cli_cmd;
        cmdIn.placeholder = l("Comando CLI (lê prompt via stdin)", "CLI command (reads prompt from stdin)");
        cmdIn.addEventListener("input", () => {
          cfg.custom_cli_cmd = cmdIn.value;
        });
        card.appendChild(cmdIn);
      }

      scroll.appendChild(card);
    }

    const saveCfgBtn = document.createElement("button");
    saveCfgBtn.type = "button";
    saveCfgBtn.className = "studio-btn primary";
    saveCfgBtn.style.justifyContent = "center";
    saveCfgBtn.innerHTML = `<i class="fa-solid fa-floppy-disk" aria-hidden="true"></i><span>${l("Salvar provedores", "Save providers")}</span>`;
    saveCfgBtn.addEventListener("click", () => void saveRoleConfigs());
    scroll.appendChild(saveCfgBtn);
  } else {
    // 1. Live Paired-Tab Browser QA Card
    const qaCard = document.createElement("div");
    qaCard.className = "agent-card";
    const qaHead = document.createElement("div");
    qaHead.className = "agent-card-top";
    qaHead.innerHTML = `<span><i class="fa-solid fa-microscope" style="color: var(--seu-accent)" aria-hidden="true"></i> ${l("QA Aba Pareada", "Paired Tab QA")}</span>`;

    const inspectBtn = document.createElement("button");
    inspectBtn.type = "button";
    inspectBtn.className = "studio-btn";
    inspectBtn.style.height = "24px";
    inspectBtn.innerHTML = `<i class="fa-solid fa-radar" aria-hidden="true"></i><span>${l("Inspecionar", "Inspect")}</span>`;
    inspectBtn.addEventListener("click", () => {
      void callBridge("studio.qaInspectPairedTab", { window_id: windowId });
    });
    qaHead.appendChild(inspectBtn);
    qaCard.appendChild(qaHead);

    if (qaSnapshot && typeof qaSnapshot.url === "string") {
      const errCount = qaSnapshot.console_errors?.length ?? 0;
      const meta = document.createElement("div");
      meta.style.fontSize = "11px";
      meta.style.color = "var(--seu-text-muted)";
      meta.textContent = `${qaSnapshot.url} (${qaSnapshot.viewport_w ?? 0}x${qaSnapshot.viewport_h ?? 0})`;
      qaCard.appendChild(meta);

      const badges = document.createElement("div");
      badges.className = "cli-chips";
      const errChip = document.createElement("span");
      errChip.className = `agent-pill${errCount === 0 ? " ok" : " err"}`;
      errChip.textContent = `Console err: ${errCount}`;
      const elChip = document.createElement("span");
      elChip.className = "agent-pill";
      elChip.textContent = `DOM elems: ${qaSnapshot.interactive_count ?? 0}`;
      badges.appendChild(errChip);
      badges.appendChild(elChip);
      qaCard.appendChild(badges);
    } else {
      const hint = document.createElement("div");
      hint.style.fontSize = "11px";
      hint.style.color = "var(--seu-text-muted)";
      hint.textContent = l(
        "Clique em Inspecionar para ler DOM, elementos interativos e erros do console da aba real ao lado.",
        "Click Inspect to read DOM, interactive elements, and console errors from the paired live tab.",
      );
      qaCard.appendChild(hint);
    }

    // Quick QA Action runner (assertText / click)
    const qaRow = document.createElement("div");
    qaRow.style.display = "flex";
    qaRow.style.gap = "4px";
    const selIn = document.createElement("input");
    selIn.className = "studio-input";
    selIn.style.height = "26px";
    selIn.value = qaSelectorDraft;
    selIn.placeholder = "CSS selector";
    selIn.addEventListener("input", () => {
      qaSelectorDraft = selIn.value;
    });
    const valIn = document.createElement("input");
    valIn.className = "studio-input";
    valIn.style.height = "26px";
    valIn.value = qaValueDraft;
    valIn.placeholder = l("texto esperado", "expected text");
    valIn.addEventListener("input", () => {
      qaValueDraft = valIn.value;
    });
    qaRow.appendChild(selIn);
    qaRow.appendChild(valIn);
    qaCard.appendChild(qaRow);

    const qaBtns = document.createElement("div");
    qaBtns.style.display = "flex";
    qaBtns.style.gap = "4px";
    for (const [act, lbl] of [
      ["assertText", "Assert"],
      ["click", "Click"],
      ["reload", "Reload"],
    ] as const) {
      const ab = document.createElement("button");
      ab.type = "button";
      ab.className = "studio-btn";
      ab.style.height = "24px";
      ab.style.flex = "1";
      ab.style.justifyContent = "center";
      ab.textContent = lbl;
      ab.addEventListener("click", () => {
        void callBridge("studio.qaExecAction", {
          window_id: windowId,
          action: act,
          selector: qaSelectorDraft,
          value: qaValueDraft,
        });
      });
      qaBtns.appendChild(ab);
    }
    qaCard.appendChild(qaBtns);

    if (lastQaAction) {
      const resEl = document.createElement("div");
      resEl.className = `agent-pill${lastQaAction.ok ? " ok" : " err"}`;
      resEl.textContent = lastQaAction.detail;
      qaCard.appendChild(resEl);
    }
    scroll.appendChild(qaCard);

    // 2. Agent Run Cards
    if (agentRuns.length === 0) {
      const empty = document.createElement("div");
      empty.className = "agent-card";
      empty.style.fontSize = "11px";
      empty.style.color = "var(--seu-text-muted)";
      empty.textContent = l(
        "Escolha um papel abaixo (Planner, Coder ou QA Browser) e envie uma tarefa. Configure APIs ou CLIs locais na aba Provedores.",
        "Pick a role below (Planner, Coder, or QA Browser) and send a task. Configure APIs or local CLIs in the Providers tab.",
      );
      scroll.appendChild(empty);
    } else {
      for (const run of agentRuns) {
        const card = document.createElement("div");
        card.className = `agent-card${run.status === "running" ? " running" : ""}`;

        const top = document.createElement("div");
        top.className = "agent-card-top";
        const name = document.createElement("span");
        name.textContent = `#${run.run_id} ${run.role.toUpperCase()}`;
        const pill = document.createElement("span");
        pill.className = `agent-pill${run.status === "done" ? " ok" : run.status === "error" ? " err" : ""}`;
        pill.textContent = run.status.toUpperCase();
        top.appendChild(name);
        top.appendChild(pill);
        card.appendChild(top);

        const sub = document.createElement("div");
        sub.style.fontSize = "10px";
        sub.style.color = "var(--seu-text-muted)";
        sub.textContent = `${run.provider} • ${run.model}`;
        card.appendChild(sub);

        const out = document.createElement("pre");
        out.className = "agent-output";
        out.textContent = run.output;
        card.appendChild(out);

        run.patches.forEach((patch, idx) => {
          const pBox = document.createElement("div");
          pBox.className = "patch-box";
          const pLbl = document.createElement("span");
          pLbl.textContent = patch.preview;
          const pBtn = document.createElement("button");
          pBtn.type = "button";
          pBtn.className = patch.applied ? "studio-btn" : "studio-btn primary";
          pBtn.style.height = "24px";
          pBtn.disabled = patch.applied;
          pBtn.textContent = patch.applied
            ? l("Aplicado", "Applied")
            : l("Aprovar Patch", "Approve Patch");
          pBtn.addEventListener("click", () => void approveAgentPatch(run.run_id, idx));
          pBox.appendChild(pLbl);
          pBox.appendChild(pBtn);
          card.appendChild(pBox);
        });

        scroll.appendChild(card);
      }
    }
  }

  aside.appendChild(scroll);

  // Composer footer
  if (agentsTab === "board") {
    const comp = document.createElement("div");
    comp.className = "agents-composer";

    const roles = document.createElement("div");
    roles.className = "agents-role-row";
    for (const [r, lbl] of [
      ["planner", "Planner"],
      ["coder", "Coder"],
      ["qa-browser", "QA"],
    ] as [AgentRole, string][]) {
      const rb = document.createElement("button");
      rb.type = "button";
      rb.className = `studio-btn${selectedRole === r ? " active" : ""}`;
      rb.style.height = "26px";
      rb.style.justifyContent = "center";
      rb.textContent = lbl;
      rb.addEventListener("click", () => {
        selectedRole = r;
        render();
      });
      roles.appendChild(rb);
    }
    comp.appendChild(roles);

    const ta = document.createElement("textarea");
    ta.className = "studio-textarea";
    ta.value = agentPromptDraft;
    ta.placeholder = l(
      "Descreva a tarefa para o agente...",
      "Describe the task for the agent...",
    );
    ta.addEventListener("input", () => {
      agentPromptDraft = ta.value;
    });
    comp.appendChild(ta);

    const sendBtn = document.createElement("button");
    sendBtn.type = "button";
    sendBtn.className = "studio-btn primary";
    sendBtn.style.justifyContent = "center";
    sendBtn.innerHTML = `<i class="fa-solid fa-paper-plane" aria-hidden="true"></i><span>${l("Executar", "Run")} ${selectedRole}</span>`;
    sendBtn.addEventListener("click", () => {
      const prompt = agentPromptDraft.trim();
      if (!prompt) return;
      agentPromptDraft = "";
      void runAgent(selectedRole, prompt);
    });
    comp.appendChild(sendBtn);
    aside.appendChild(comp);
  }

  return aside;
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
  } else if (event.type === "studioGit") {
    if (workspace) {
      workspace.git = event.git;
      const statusMap = new Map(event.git.changed_files.map((f) => [f.rel_path, f.code]));
      workspace.entries = workspace.entries.map((e) => ({
        ...e,
        git_status: statusMap.get(e.rel_path) ?? null,
      }));
      dirCache.set("", workspace.entries);
      render();
    }
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
  } else if (event.type === "studioAgentUpdate") {
    agentRuns = [event.run, ...agentRuns.filter((r) => r.run_id !== event.run.run_id)];
    statusText = `Agent #${event.run.run_id} (${event.run.role}): ${event.run.status}`;
    render();
  } else if (event.type === "studioQaSnapshot") {
    qaSnapshot = event.snapshot;
    statusText = l(
      `QA inspecionou ${event.snapshot.url} (${event.snapshot.console_errors.length} erros)`,
      `QA inspected ${event.snapshot.url} (${event.snapshot.console_errors.length} errors)`,
    );
    render();
  } else if (event.type === "studioQaAction") {
    lastQaAction = event.result;
    statusText = event.result.detail;
    render();
  }
});

async function bootstrap(): Promise<void> {
  // Instant first paint (0 ms waiting on workspace/git/CLI calls).
  render();

  // Apply theme & locale first (< 10 ms).
  const [settings, theme] = await Promise.all([
    callBridge<CoreSettings>("settings.get").catch(() => undefined),
    callBridge<LoadedTheme>("themes.getActive").catch(() => undefined),
  ]);
  if (settings) locale = settings.locale;
  if (theme) applyThemeTokens(theme);
  render();

  // Hydrate workspace, CLIs, agent config & QA snapshot progressively without blocking paint.
  void callBridge<StudioWorkspaceSummary | null>("studio.getWorkspace")
    .then((ws) => {
      if (ws && typeof ws.root_path === "string") {
        workspace = ws;
        dirCache = new Map([["", ws.entries ?? []]]);
        statusText = l(`Workspace: ${ws.name}`, `Workspace: ${ws.name}`);
        render();
      }
    })
    .catch(() => {});

  void Promise.all([
    callBridge<DetectedLocalCli[]>("studio.detectClis").catch(() => []),
    callBridge<{ config_json: string | null }>("studio.agentsConfigGet").catch(() => undefined),
    callBridge<AgentRunSnapshot[]>("studio.agentsListRuns").catch(() => []),
    callBridge<BrowserQaSnapshot | null>("studio.qaGetLastSnapshot").catch(() => null),
  ]).then(([clis, cfgPayload, runs, lastQa]) => {
    if (clis) detectedClis = clis;
    if (cfgPayload?.config_json) {
      try {
        const parsed = JSON.parse(cfgPayload.config_json) as Partial<RoleConfigs>;
        roleConfigs = { ...defaultRoleConfigs(), ...parsed };
      } catch {
        // keep defaults
      }
    }
    if (runs) agentRuns = runs;
    if (lastQa) qaSnapshot = lastQa;
    render();
  });
}

void bootstrap();
