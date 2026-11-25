# Especificação: **Seu Navegador** — navegador leve e personalizável em Rust (CEF)

> Documento consolidado da v1, unindo a especificação inicial com todas as decisões de arquitetura, UX, segurança e escopo fechadas antes da Fase 0.

---

## 1. Contexto e objetivo

- **Quem usa:** Yan, no dia a dia, no notebook e no PC de casa, ambos com **Windows**. Uso focado em programação (docs, GitHub), YouTube, navegação geral e jogos com o navegador aberto.
- **Por que existe:**
  1. **Gastar pouca RAM.** Ganho real e medido em uso multi-aba, não marketing.
  2. **Ser 100% personalizável.** Vem com 9 interfaces oficiais distintas, e o usuário pode criar e compartilhar as próprias.
  3. **Ser peça de portfólio**, com identidade de **dev criativo**. README, screenshots, GIFs e benchmark contam tanto quanto o código.
- **Identidade do produto:**
  - Nome: **Seu Navegador**
  - Executável: `seu-navegador.exe`
  - Repositório: `seu-navegador`
  - Esquema interno: `seu://`
  - Idiomas: **bilíngue (PT-BR e EN)** — UI com i18n leve em JSON e documentação/README em inglês (`README.md`) e português (`README.pt-BR.md`).
  - Licença open-source: **dual `MIT OR Apache-2.0`** (padrão do ecossistema Rust; compatível com CEF BSD-3, `rusqlite` MIT, `adblock` MPL-2.0 e fontes Google OFL 1.1).

### Não-objetivos (fora do escopo da v1)
- Netflix, Spotify web e qualquer site com DRM (Widevine).
- Codecs proprietários (H.264/AAC); usar o CEF padrão (YouTube funciona via VP9/AV1/Opus).
- Sincronização em nuvem, contas, extensões da Chrome Web Store, Linux e macOS na v1.
- **Android na v1** (previsto na seção 12.1; arquitetura da v1 já preparada).
- Motor de renderização próprio.

### Anti-objetivos (resultados que contam como fracasso)
- Gastar mais RAM que o Chrome no uso real com múltiplas abas (ex.: benchmark de 15 abas) ou ter baseline inflado desnecessariamente em 1 aba.
- Interface com cara de template genérico — os 9 temas devem ser fiéis à referência visual e ter identidade própria (incluindo titlebar e DevTools temático).
- Aba dormindo perder o que o usuário estava fazendo (formulário/editor preenchido, vídeo/áudio tocando, pilha Voltar/Avançar).
- Qualquer dependência de **componentes da Microsoft instalados no PC do usuário final**: nada de WebView2, Edge, .NET runtime, Visual C++ Redistributable ou instalador MSIX. API Win32 nativa do SO é permitida.

---

## 2. Referência visual

Arquivo de referência: `docs/design/seu-navegador-referencia-ui.html` (mover da raiz ao inicializar o repositório). Contém:
- As **9 interfaces oficiais da v1** em `<template id="tpl-<id>">` com Shadow DOM.
- O JSON `#theme-tokens` com inspiração de cores, fontes e notas visuais (a ser migrado para o contrato semântico da seção 6.1).
- Números (`412 MB`, `3 abas dormindo`, `128 bloqueados`) e nomes de exemplo (nota: no widget `interfaces.exe` do `y2k-maximal`, substituir os mocks antigos "Chrome Líquido" e "Minimal Noturno" pela lista real de temas).

| ID | Nome | Observação |
|---|---|---|
| `y2k-maximal` | Y2K Maximal | **Tema padrão** e vitrine principal do portfólio |
| `glassmorphism` | Glassmorphism | Vidro fosco + blobs coloridos |
| `neumorphism` | Neumorphism | Relevo suave (.up / .in) |
| `editorial` | Editorial | Layout de revista com índice romano |
| `brutalist` | Brutalista | HTML cru, alto contraste, fontes do sistema |
| `neobrutalist` | Neobrutalista | Pastel forte, bordas 3px, sombras duras |
| `retro-70s` | Retrô anos 70 | Paleta terrosa, listras e anéis |
| `y2k-classic` | Y2K clássico | Prata metálico, gel aqua/verde |
| `maximal` | Maximalista | Colagem em grade de 6 blocos sólidos |

---

## 3. Decisões técnicas fechadas

| Área | Decisão |
|---|---|
| Linguagem | Rust (edição estável mais recente) |
| Motor | CEF via crate `cef` (`tauri-apps/cef-rs`), Chrome runtime com **Alloy style** (`CEF_RUNTIME_STYLE_ALLOY`), sem UI nativa do Chrome |
| Plataforma v1 | **Windows x64** |
| UI da Shell & Páginas internas | HTML/CSS/TypeScript rodando no CEF via esquema `seu://`, assets **embutidos no binário**, build com Vite (vanilla TS ou Svelte leve) |
| Bloqueador | Crate `adblock` (motor do Brave) com **EasyList + EasyPrivacy + uBlock Filters + scriptlets (`##+js`)** |
| Persistência | SQLite (`rusqlite`, feature `bundled`); perfil CEF em `%LOCALAPPDATA%\SeuNavegador\profile`; `User-Agent` limpo compatível com Chromium estável |
| Abstração do motor | Trait `Engine`/`BrowserBackend` enxuta isolando CEF (preparação para GeckoView/UniFFI no Android; sem tipos CEF no `core`) |
| Licença | Dual `MIT OR Apache-2.0` |
| Internacionalização | PT-BR e EN via arquivos JSON leves (`locales/pt-BR.json`, `locales/en.json`) |

### Toolchain Windows & CEF Sandbox (resolver e registrar no ADR 001 na Fase 0)
- **Preferência do Yan:** evitar instalar ferramentas pesadas da Microsoft localmente sempre que possível.
- **Realidade técnica do `cef-rs`:** as distribuições oficiais do CEF para Windows usam ABI MSVC (`libcef.lib` + `libcef_dll_wrapper` via CMake/C++), então `x86_64-pc-windows-gnu` geralmente não é suportado oficialmente.
- **Estratégia faseada:**
  1. Avaliar na Fase 0 as 3 rotas antes de instalar qualquer coisa:
     - **Rota A (Zero MSVC local):** compilar via **GitHub Actions** (runner Windows com MSVC + CMake já instalados) e baixar artefatos, ou testar `cargo-xwin` + `clang-cl`.
     - **Rota B (Local com MSVC mínimo):** instalar apenas Build Tools mínimos + CMake/Ninja se a iteração remota ficar lenta demais.
  2. **Sandbox do CEF:** em dev local, pode rodar com `--no-sandbox` se simplificar o ciclo; no build de release (GitHub Actions com MSVC), buscar habilitar o sandbox oficial do CEF (`cef_sandbox.lib`) ou documentar com transparência no ADR 001 e no README caso o `cef-rs` tenha limitação.
  3. **Usuário final livre de runtimes Microsoft:** linkar CRT estaticamente (`+crt-static`) ou empacotar tudo que for necessário no zip portátil.

**Regra fixa:** nunca instalar toolchains ou programas no PC sem confirmação explícita do Yan.

---

## 4. Arquitetura

### 4.1 Múltiplas janelas, views e overlays
- Modelo multiprocesso do CEF (`execute_process` no mesmo executável ou helper se exigido pelo crate).
- **Suporte a múltiplas janelas** (múltiplas janelas normais + janelas anônimas independentes).
- Cada janela nativa é **frameless (sem borda padrão do Windows)** com `WM_NCHITTEST` customizado para arrastar pela titlebar do tema, redimensionar nas bordas, duplo-clique maximizar e *Windows Snap*.
- Composição de views em cada janela:
  1. **View da Shell (`seu://shell/`):** topo da janela (e rodapé/regiões se o tema definir), reportando sua altura dinâmica ao Rust via `ResizeObserver` para reposicionar o conteúdo sem cortes.
  2. **View de Conteúdo:** uma por aba viva, posicionada abaixo da shell; apenas a aba ativa fica visível.
  3. **View de Overlay/Popup reutilizável:** janela/view popup borderless sob demanda posicionada **acima** da view de conteúdo para menus e popups que não podem ser cortados pela shell (sugestões da omnibox, menu `≡`, popover de RAM/bloqueador, prompts de permissão de site, busca `Ctrl+F`).
  4. **View de DevTools acoplado (docked):** criada sob demanda ao abrir `F12` (lateral ou inferior, com divisor arrastável) ou destacada em janela própria; recebe CSS injetado automaticamente a partir dos tokens semânticos do tema ativo (`--sys-color-*`, fontes, destaque) para manter a identidade do navegador.
- **Páginas internas:** `seu://newtab`, `seu://settings`, `seu://history`, `seu://bookmarks`, `seu://downloads`, `seu://themes`, `seu://themes/editor`, `seu://studio`. Todas consomem os tokens semânticos do tema ativo.
- **Split View & Studio (`seu://studio/` — IDE leve + Orquestrador de Agentes):**
  - A janela suporta 3 modos alternáveis em 1 clique (**Só Navegador**, **Split View** lado a lado com divisor arrastável, e **Só Studio**).
  - **Custo ZERO no modo Navegador:** tudo do Studio vive no crate isolado `crates/studio` + bundle lazy `seu://studio/`; fechado consome `0 MB` extras.
  - **100% personalizável por temas:** o Studio consome os mesmos tokens `--seu-*` e CSS declarativo do tema ativo.
  - **Orquestrador híbrido:** cada agente suporta tanto **APIs diretas** (Gemini, Claude, OpenAI, Ollama local) quanto **CLIs locais instalados** (`gemini-cli`, `claude-code`, `codex-cli`), com ferramentas de código restritas à pasta aberta e ponte nativa de **QA Browser** direto na aba real pareada no Split View (console, rede, DOM, screenshot).

### 4.2 Contrato mínimo da Shell (obrigatório em todos os temas)
Todo tema oficial ou criado por usuário deve suportar duas camadas de controles:

- **Camada 1 — Sempre visível na Shell:**
  1. **Titlebar própria:** região arrastável (`data-seu-drag-region`) + botões Minimizar, Maximizar/Restaurar e Fechar (pode ser barra dedicada como no `y2k-maximal` ou integrada à linha de abas nos temas compactos).
  2. **Barra de abas:** favicon + título + botão fechar (`×`, opcionalmente só no hover) + estados (`active`, `sleeping`, `playing`, `pinned`) + **grupos de abas** (etiqueta/cor + colapsar/expandir) + botão Nova aba (`+`).
  3. **Navegação e Omnibox:** Voltar, Avançar, Recarregar/Parar, campo de endereço com indicador de HTTPS/cadeado e botão Favoritar (`☆`).
  4. **Indicadores e Menu:** Badge de RAM (clicável → popover de memória/abas dormindo), Badge do Bloqueador (clicável → toggle para o site) e Botão Menu (`≡`).
- **Camada 2 — Menu / Popover da Shell (não polui temas limpos):**
  - Troca rápida de tema + atalho para Galeria (`seu://themes`) e Editor;
  - Toggle de Modo Jogo;
  - Mini-player "Tocando agora" (quando há mídia);
  - Nova janela / Nova janela anônima;
  - Histórico, Favoritos, Downloads, Configurações, DevTools (`F12`), Zoom e Idioma (PT-BR / EN).
- **Interações arrastáveis (Drag & Drop):**
  - Arrastar janela pela titlebar;
  - Reordenar abas na barra;
  - Arrastar abas para criar/entrar/sair de grupos de abas;
  - Arrastar aba para fora da janela (*tear-off* → nova janela) e mover aba entre janelas abertas;
  - Reordenar atalhos na nova aba;
  - Arrastar divisor (splitter) do DevTools acoplado.

### 4.3 Ponte shell ↔ Rust (`window.seu`)
- `cefQuery` / process messages com **JSON tipado**; tipos TS gerados/sincronizados a partir de Rust (`serde` + `ts-rs` ou `specta`).
- Exposta **exclusivamente** às origens `seu://`.
- Validação e limites rígidos no lado Rust.
- Módulos mínimos da API:
  - `windows`: `create`, `createIncognito`, `minimize`, `maximize`, `close`, `startDrag`, `setShellBounds`
  - `tabs`: `list`, `create`, `close`, `activate`, `navigate`, `reload`, `stop`, `back`, `forward`, `pin`, `duplicate`, `reopenClosed`, `reorder`, `moveToWindow`, `detachToWindow`, `onChanged`
  - `tabGroups`: `list`, `create`, `update`, `toggleCollapse`, `addTab`, `removeTab`, `ungroup`, `onChanged`
  - `overlay`: `open`, `close`, `resize`
  - `devtools`: `toggle`, `setDockSide`, `detach`, `onChanged`
  - `memory`: `stats()` (total, por processo/aba com indicador de renderer compartilhado, MB devolvidos), `sleepTab`, `wakeTab`, `onStats`
  - `blocker`: `stats()`, `toggleForSite`, `onChanged`
  - `media`: `nowPlaying()`, `playPause`, `next`, `prev`, `onChanged`
  - `gameMode`: `get`, `set`, `onChanged`
  - `history`, `bookmarks`, `downloads`, `shortcuts` (incluindo `reorder`), `search`
  - `themes`: `list`, `getActive`, `setActive`, `import`, `export`, `save`, `onChanged`
  - `settings`: `get`, `set`, `i18n` (`getLocale`, `setLocale`, `getStrings`)

### 4.4 Estrutura do repositório
```
seu-navegador/
  Cargo.toml            (workspace)
  LICENSE-MIT
  LICENSE-APACHE
  README.md             (EN)
  README.pt-BR.md       (PT-BR)
  crates/
    app/                (main, init CEF, múltiplas janelas Win32 frameless, overlays, ciclo de vida)
    engine/             (trait Engine + implementação CEF Alloy + DevTools dock/skin)
    core/               (abas, grupos, memória, sessão, modo jogo, mídia — sem CEF/Win32)
    storage/            (SQLite: histórico, favoritos, downloads, sessão, configs — sem CEF/Win32)
    blocker/            (adblock + EasyList/EasyPrivacy/uBlock + scriptlets + cosméticos)
    themes/             (validação de manifest/tokens, sanitização HTML/CSS, empacotamento .seutheme, skins)
    bridge/             (contratos tipados window.seu)
  ui/                   (shell, overlay, páginas seu://, runtime declarativo, i18n — Vite + TS)
  themes/               (os 9 temas oficiais como pacotes)
  docs/
    SPEC.md             (este documento)
    design/seu-navegador-referencia-ui.html
    adr/                (decisões de arquitetura)
    test-plan.md
  bench/                (scripts reproduzíveis de benchmark de RAM)
```

---

## 5. Gerenciador de memória

### 5.1 Ciclo de vida da aba
`Ativa` → `Segundo plano` → `Dormindo` (descarregada)

- **Segundo plano:** oculta com prioridade e frame rate reduzidos (`WasHidden` etc.).
- **Dormindo:** fecha o browser CEF da aba para liberar RAM. Salva:
  - URL atual, título, favicon;
  - **pilha Voltar/Avançar** da aba no `core`;
  - posição de rolagem (restaurada com retry breve pós-load para SPAs/lazy-load);
  - **miniatura comprimida em disco (WebP/JPEG)** — nunca mantida em RAM — exibida no hover da aba ou no popover de abas dormindo.

### 5.2 Políticas
- Inatividade configurável (padrão: 10 min em segundo plano).
- Limite opcional de abas vivas (dorme a LRU).
- **Nunca colocar para dormir:**
  - aba tocando áudio;
  - aba fixada marcada como "não dormir";
  - aba com formulário/editor alterado e não enviado (`<input>`, `<textarea>`, `<select>`, **`contenteditable`**) ou que tenha listener de `beforeunload`;
  - aba com download, upload ou chamada de mídia (câmera/mic) ativa;
  - domínios na lista configurável "nunca dormir".

### 5.3 Medição realista e Flags
- Somar RAM de **todos** os processos do navegador (browser + GPU + network/utility + renderers) via Win32 / `sysinfo`.
- Tratar **processos renderer compartilhados** (várias abas do mesmo site/limite de renderers): mostrar claramente quando o processo é compartilhado e contabilizar "MB devolvidos" com base no delta real medido após o sleep.
- Avaliar e documentar em ADR flags do Chromium (ex.: `--renderer-process-limit`, desativar serviços em segundo plano); só manter o que comprovadamente reduzir RAM sem quebrar sites.

---

## 6. Sistema de temas (interfaces)

### 6.1 Pacote de tema e contrato semântico de `tokens.json`
Pasta ou `.seutheme` (zip):
```
manifest.json
tokens.json        (bloco obrigatório "semantic" + bloco livre "custom")
shell.css
shell.html         (opcional; layout próprio respeitando Camada 1)
newtab.html
newtab.css
assets/            (fontes OFL, SVGs, imagens)
sites/             (skins por domínio: css e js opcional)
```
- **`manifest.json`:** `id`, `name`, `version`, `author`, `description`, `base`, `entry`, `sites`, `permissions`, e bloco opcional `mobile` (validado na v1 para a futura versão Android).
- **`tokens.json` — contrato obrigatório `semantic`:**
  - Cores: `bg`, `surface`, `surface2`, `text`, `textMuted`, `accent`, `accentText`, `border`, `danger`, `incognitoBg`, `incognitoAccent`;
  - Tipografia/forma: `fontDisplay`, `fontBody`, `fontMono`, `radius`, `shadow`.
  - Bloco livre `custom` para variáveis exclusivas do tema (ex.: `lime`, `cyan`, `blobPink`, `rust`).
  - Os tokens semânticos alimentam automaticamente: **páginas internas**, **modo anônimo**, **DevTools temático**, **checagem WCAG AA no editor visual** e **futuro Android**.

### 6.2 Templating declarativo e segurança
- Primitivas suportadas pelo runtime da shell/newtab (sem JS do tema):
  - `data-seu-bind="path"` + formatadores (`data-seu-format="roman|meter|compact"`), `data-seu-t="i18n.key"`;
  - `<template data-seu-repeat="tabs|tabGroups|shortcuts|themes">` com contexto do item + drag-and-drop automático;
  - `data-seu-action="..."` + `data-seu-param="..."`;
  - `data-seu-input="..."` / `data-seu-submit="..."`;
  - `data-seu-if="..."`, `data-state="..."`, `data-seu-drag-region`.
- **Dogfooding:** os 9 temas oficiais usam apenas essa API pública, sem exceções no core.
- **Segurança em profundidade:**
  - CSP rígida em `seu://` (sem rede externa, sem `eval`, sem inline scripts);
  - **Sanitização HTML e CSS no crate `themes` (Rust)** ao carregar pacotes (whitelist de tags/atributos; bloqueia `<script>`, `<iframe>`, `<object>`, `javascript:`, `@import` e `url(http...)` externos);
  - Fontes OFL embutidas nos temas oficiais (com arquivos de licença).

### 6.3 Criação de temas na v1
1. **Pacote de arquivos:** edição em pasta com hot-reload (file watch em modo dev) + importar/exportar `.seutheme`.
2. **Editor visual (`seu://themes/editor`):** copy-on-write, edição dos tokens semânticos e extras, checagem de contraste **WCAG AA**, importação de fontes, preview ao vivo da shell + nova aba lado a lado, salvar/exportar/resetar.
3. **Galeria (`seu://themes`):** mostra os 9 oficiais + temas do usuário, troca a quente sem reiniciar.

---

## 7. Skins por site (CSS + JS)

- CSS e JS por padrão de domínio declarados no manifest (`sites/youtube.com.css`, `sites/youtube.com.js`).
- CSS injetado no início do documento (evitar flash); JS em contexto isolado; suporte a navegação SPA (reaplicar ao mudar URL).
- **Segurança:** CSS sempre permitido; JS permitido só para temas oficiais/locais. Em tema importado, JS vem **desligado** e exige opt-in explícito após aviso claro. JS de skin nunca acessa `window.seu`.
- **Vitrine v1:** skin do **YouTube** no tema `y2k-maximal` (CSS + JS leve, ex.: esconder Shorts / ampliar player, seletores fáceis de manter).
- Interruptor global em `seu://settings`: "Desativar skins de sites".

---

## 8. Bloqueador de anúncios e rastreadores

- Motor `adblock` (Brave) com **EasyList + EasyPrivacy + uBlock Filters + recursos de scriptlets (`##+js`)** embutidos, com atualização em segundo plano (máx. 1x/dia, fallback embutido).
- Bloqueio de rede no handler do CEF + filtros cosméticos + injeção controlada de scriptlets.
- Contadores por aba e global na shell + allowlist por site no popover do bloqueador.

---

## 9. Modo jogo

- **Detecção automática** (desligável):
  - `SHQueryUserNotificationState` (D3D fullscreen exclusivo);
  - **e** detecção de **Borderless Fullscreen** comparando janela em foco com o retângulo do monitor, **excluindo o próprio processo do Seu Navegador** (para YouTube em tela cheia não ativar) + **debounce (5–10s)** + lista configurável de `.exe` de jogos;
  - **ou** navegador minimizado por mais de X segundos.
- **Ao entrar:** dorme todas as abas elegíveis, reduz trabalho em segundo plano, notificação discreta.
- **Ao sair:** não acorda abas sozinho; mostra "X MB devolvidos durante o jogo".
- Toggle manual na shell/menu/nova aba.

---

## 10. "Tocando agora" (controle de mídia)

- Detecção de abas com áudio via CEF.
- **Shim leve em `document_start`** no main world para interceptar `navigator.mediaSession.setActionHandler` e ler `metadata` (com canal controlado para o script isolado), somado ao fallback direto em `<video>`/`<audio>` e título da página.
- Exibido no widget da nova aba e no indicador/popover da shell.

---

## 11. Recursos básicos da v1

- **Abas e Grupos:** criar, fechar, arrastar para reordenar, fixar, duplicar, reabrir fechada (`Ctrl+Shift+T`), **criar/nomear/colorir/colapsar/arrastar grupos de abas**, atalhos padrão (`Ctrl+T/W/L/N/Shift+N/Tab/Shift+Tab/1..9`, `F5`, `Alt+←/→`, `Ctrl+F`, zoom `Ctrl +/−/0`, `F12` DevTools).
- **Múltiplas janelas:** `Ctrl+N`, `Ctrl+Shift+N` (anônima), arrastar aba para fora (*tear-off*) e entre janelas.
- **Omnibox & Busca:** URL ou busca, sugestões locais de histórico/favoritos; motor configurável (padrão DuckDuckGo; opções Google, Brave Search, Startpage) + chips configuráveis (Web, Vídeos, Imagens, Código, Docs).
- **Histórico, Favoritos, Downloads, Atalhos:** histórico com busca/limpeza; favoritos (`Ctrl+D`, pastas, import Netscape HTML); downloads com progresso/pausa/retomar/cancelar; atalhos editáveis e arrastáveis na nova aba.
- **Restaurar sessão:** restaura janelas, abas e grupos; abas não ativas iniciam **dormindo**.
- **Modo anônimo:** `RequestContext` em memória, visual distinto via tokens semânticos (`incognitoBg`/`incognitoAccent`).
- **Permissões de site:** câmera, mic, notificações, localização com prompt na view de overlay e gestão em `seu://settings`.
- **DevTools acoplado e temático (`F12`):** acoplável na janela com splitter arrastável, destacável, estilizado com os tokens do tema ativo.

---

## 12. Fases de execução

Cada fase termina com testes, commit, demo e checklist (o que ficou pronto, como testar, RAM medida se aplicável, pendências).

0. **Spike (descartável):** resolver toolchain Windows (ADR 001), janela Win32 frameless + CEF Alloy carregando YouTube e tocando vídeo, medir RAM base (1 aba + shell).
1. **Esqueleto:** workspace Rust, trait `Engine`, múltiplas janelas frameless, esquema `seu://`, overlay popup reutilizável, ponte `window.seu`, i18n (PT-BR/EN), abas básicas + grupos básicos + navegação.
2. **Tema padrão:** motor de temas (manifest, contrato semântico de tokens, templating declarativo, sanitizador HTML/CSS, CSP) + **Y2K Maximal** fiel à referência (incluindo titlebar própria e drag-and-drop).
3. **Memória:** ciclo de vida completo, políticas (`contenteditable`/`beforeunload`), pilha Voltar/Avançar, miniaturas em disco, medição real (tratando renderers compartilhados) + ADR de flags do Chromium.
4. **Bloqueador:** `adblock` com EasyList + EasyPrivacy + uBlock Filters + scriptlets + cosméticos, contadores e allowlist.
5. **Persistência, Split View & DevTools:** histórico automático com busca/limpeza, favoritos (`Ctrl+D`, pastas, import Netscape HTML), downloads (progresso, pausar/retomar/cancelar), restaurar sessão (janelas/abas/grupos dormindo), modo anônimo, permissões de site, configurações, **Split View nativo (2 views lado a lado com divisor arrastável)** e **DevTools (`F12`) acoplado + CSS temático**.
5.5. **Studio MVP (`seu://studio/`):** crate isolado `crates/studio` (workspace, árvore de arquivos, Git status/diff, terminal integrado) + editor leve CodeMirror 6 100% temável (`--seu-*`) + toggle **Navegador / Split View / Studio** pareado com qualquer aba real.
5.6. **Orquestrador de Agentes & QA Nativo:** board multi-agente híbrido (**API direta**: Gemini/Claude/OpenAI/Ollama + **CLI local**: `gemini-cli`/`claude-code`/`codex-cli`), aprovação de diff/comandos e ferramentas de QA conectadas direto na aba pareada do navegador.
6. **Os outros 8 temas:** implementados só com a API pública (dogfooding) + titlebar/controles mínimos em todos + galeria e troca a quente.
7. **Criar interface:** pacote de arquivos (hot-reload, importar/exportar `.seutheme`) e editor visual de tokens com WCAG AA e preview lado a lado.
8. **Skins por site:** motor CSS + JS isolado com regras de segurança + skin do YouTube no `y2k-maximal`.
9. **Modo jogo & Tocando agora:** detecção Win32 (D3D + Borderless excluindo o navegador + debounce + lista `.exe`) e controle de mídia via shim MediaSession + fallback.
10. **Benchmark e portfólio:** script reproduzível em `bench/` (15 abas: Seu Navegador vs. Chrome vs. Firefox aos 0, 5 e 15 min) + `README.md` (EN) e `README.pt-BR.md` (PT-BR) com GIFs, screenshots dos 9 temas, arquitetura, guia de temas e limites honestos.
11. **Distribuição:** CI no GitHub Actions (tentando sandbox + CRT estático), pacote portátil zipado e instalador opcional (Inno Setup/NSIS, sem MSIX).

### 12.1 Preparação para Android (futuro — não implementar na v1)
- App futuro em **Kotlin + GeckoView**, compartilhando `core`, `storage` e validação de `themes` via **UniFFI**.
- Na v1: `core`, `storage` e validação de `themes` **não dependem de CEF nem Win32** (compilam para `aarch64-linux-android` sem alterações) e expõem tipos planos na fronteira pública (sem adicionar UniFFI agora). Repositório pronto para receber `android/`.

---

## 13. Qualidade e testes

- **Unitários:** ciclo de vida/políticas de memória (relógio injetável), matching do bloqueador, validação/sanitização de temas e tokens semânticos, runtime de templating, storage, parser Netscape de favoritos, i18n.
- **Integração:** ponte `window.seu` (chamadas válidas/inválidas e bloqueio fora de `seu://`).
- **QA manual (`docs/test-plan.md`):** ID, pré-condição, passos, resultado esperado (formulário/`contenteditable` não dormir, áudio não dormir, pilha Voltar/Avançar ao acordar, restaurar sessão com grupos, modo anônimo sem rastros, troca de tema a quente, tema importado com JS desligado, skin YouTube em SPA, DevTools temático, arrastar abas entre janelas).
- **Higiene:** `cargo clippy -- -D warnings`, `cargo fmt`, zero `unwrap()` em produção, logs com `tracing` em `%LOCALAPPDATA%\SeuNavegador\logs`.
