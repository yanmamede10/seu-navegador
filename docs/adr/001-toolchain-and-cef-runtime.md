# ADR 001: Toolchain Windows, CEF Runtime e Estratégia de Build

- **Status:** Aceito e validado na Fase 0
- **Data:** 2026-11-23
- **Decisores:** Yan

## Contexto
O **Seu Navegador** usa Rust + **Chromium Embedded Framework (CEF)** via crate `cef` (`tauri-apps/cef-rs`) no Windows x64.

Restrições do projeto:
1. **Zero ferramentas da Microsoft no PC de desenvolvimento local** (o PC local tem `rustc 1.100.0`, `cargo`, `git`, `node` e `npm`, sem Visual Studio / Build Tools / CMake / Windows SDK).
2. **Zero runtimes da Microsoft no PC do usuário final** (sem WebView2, Edge, .NET runtime, MSIX ou necessidade de instalar Visual C++ Redistributable).
3. As distribuições oficiais do CEF para Windows são compiladas com ABI **MSVC**, e o crate `cef-dll-sys` exige CMake + compilador C++ compatível com MSVC para compilar `libcef_dll_wrapper` e linkar contra `libcef.lib`. O target `x86_64-pc-windows-gnu` não possui suporte oficial no `cef-rs`.

## Opções avaliadas

| Opção | Descrição | Prós | Contras |
|---|---|---|---|
| **A (Escolhida)** | **GitHub Actions (`windows-latest`)** compila o binário Rust+CEF na nuvem; UI/TS e código Rust puro são desenvolvidos localmente | Zero ferramentas Microsoft no PC local; CI/CD pronto desde o dia 1; facilita testar sandbox CEF e release portátil | Ciclo de teste do executável CEF exige push/dispatch e download do artefato (~minutos) |
| **B** | `LLVM/Clang` + `CMake` + `cargo-xwin` localmente | Compila local sem instalador do Visual Studio | Atrito de configuração com `cef-dll-sys`/CMake no Windows nativo |
| **C** | Visual Studio Build Tools C++ mínimo + CMake local | Iteração local mais rápida | Instala ferramentas da Microsoft na máquina de dev |

## Decisão
1. **Build de Rust + CEF via GitHub Actions (`windows-latest`):**
   - O workflow usa `export-cef-dir` com cache (`actions/cache`) para evitar downloads repetidos do CEF.
   - Gera um artefato `.zip` portátil pronto para extrair e executar no Windows do desenvolvedor e do usuário final.
2. **Iteração rápida local para UI e lógica pura:**
   - A UI (`ui/` + os 9 temas) será desenvolvida e testada localmente com **Vite** em tempo real (`node`/`npm` já instalados).
   - Crates sem dependência de CEF/Win32 (`core`, `storage`, validação de `themes`) permanecem isolados e prontos inclusive para o target futuro `aarch64-linux-android`.
3. **CEF Runtime Style:**
   - Usar **Chrome runtime com Alloy style** (`RuntimeStyle::ALLOY`), conforme o modelo moderno do CEF (sem a UI nativa do Chrome).
4. **CEF Sandbox e CRT estático:**
   - No Spike da Fase 0, iniciar com `no_sandbox = 1` para validar boot, reprodução de YouTube e baseline de RAM sem bloqueios.
   - Nas fases seguintes / build de release no CI, avaliar e testar:
     - habilitação do sandbox oficial do CEF no runner MSVC;
     - linkagem estática do CRT (`+crt-static`) ou verificação de autossuficiência das DLLs empacotadas para garantir execução limpa sem instalar Visual C++ Redistributable.

## Consequências
- O PC local permanece 100% livre de Visual Studio / Build Tools / SDKs da Microsoft.
- O repositório nasce com pipeline reproduzível no GitHub Actions.
- Precisaremos de disciplina em testes unitários, `clippy` e preview local via Vite para reduzir a quantidade de ciclos de push necessários para depurar mudanças visuais ou de lógica.

## Resultados medidos na Fase 0 (Spike)
- **Versão CEF:** `cef` `156.7.0+156.0.31` (Chrome runtime com `RuntimeStyle::ALLOY`, `no_sandbox = 1`).
- **Portabilidade:** executável (`160 KB`) + runtime CEF abriram no Windows sem solicitar nenhuma DLL/runtime da Microsoft.
- **Reprodução web:** YouTube carregou em Alloy style (sem UI do Chrome) com vídeo e áudio funcionando normalmente (VP9/AV1/Opus).
- **RAM de base (sem bloqueador de anúncios e sem flags de otimização):**
  - **YouTube Home (estabilizado, 9 processos):** `377.3 MB` Private Bytes (`682.1 MB` Working Set).
  - **YouTube vídeo tocando (8 processos):** `344.9 MB` Private Bytes (`635.9 MB` Working Set) — `browser` `80.9 MB`, `renderer` principal `199.7 MB`, `renderer` subframe `16.2 MB`, `NetworkService` `12.7 MB`, `VideoCaptureService` `11.6 MB`, `gpu-process` `9.9 MB`, `AudioService` `7.1 MB`, `StorageService` `6.8 MB`.
- **Observação para a Fase 1:** o CEF emitiu warning pedindo `CefSettings.root_cache_path` customizado; na Fase 1 configuraremos explicitamente `%LOCALAPPDATA%\SeuNavegador\profile`.
