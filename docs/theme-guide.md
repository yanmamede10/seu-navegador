# Theme Authoring Guide — Seu Navegador

Seu Navegador lets you customize **100% of the browser chrome, New Tab, DevTools, Seu Studio, and even specific websites** using declarative HTML/CSS packages — no JavaScript required for UI themes.

## Two Ways to Create a Theme

1. **Visual Theme Editor (`seu://themes/editor`):**
   - Pick any of the 9 official themes as a copy-on-write base.
   - Edit semantic colors, typography, radius, shadows, custom CSS tokens, and extra Shell/New Tab CSS.
   - Check live **WCAG 2.1 AA / AAA** contrast ratios across 5 semantic pairs.
   - Import offline `.woff2` / `.ttf` fonts.
   - Save, export as `.seutheme`, or open `%LOCALAPPDATA%\SeuNavegador\themes\` directly in **Seu Studio** with **automatic hot-reload**.

2. **Package Folder / `.seutheme` ZIP:**
   Place a folder inside `%LOCALAPPDATA%\SeuNavegador\themes\<your-theme-id>\` (hot-reloads automatically on save!):

```text
my-theme/
├── manifest.json     # Metadata, entrypoints, site skins, permissions
├── tokens.json       # Mandatory `semantic` contract + free `custom` variables
├── shell.html        # Custom titlebar, tabs, omnibox & badges (Layer 1 contract)
├── shell.css         # Shell styling (relative `./assets/...` rewritten automatically)
├── newtab.html       # Declarative New Tab layout
├── newtab.css        # New Tab styling
├── assets/
│   └── fonts/        # Bundled offline .woff2 / .ttf fonts + OFL license
└── sites/            # Optional per-site CSS (+ optional JS) skins
    ├── youtube.com.css
    └── youtube.com.js
```

## Mandatory Semantic Token Contract (`tokens.json`)

Every theme must provide the 16 semantic tokens in `tokens.json` — these automatically power all internal `seu://` pages, Incognito mode, themed DevTools (`F12`), Seu Studio, and WCAG AA validation:

| Token | CSS Variable | Purpose |
|---|---|---|
| `bg` | `--seu-bg` | Primary canvas background |
| `surface` | `--seu-surface` | Panels, bars, cards |
| `surface2` | `--seu-surface2` | Elevated/secondary surfaces |
| `text` | `--seu-text` | Primary foreground text |
| `text_muted` | `--seu-text-muted` | Secondary labels and hints |
| `accent` | `--seu-accent` | Primary highlight/action color |
| `accent_text` | `--seu-accent-text` | Foreground text placed on `--seu-accent` |
| `border` | `--seu-border` | Borders and hairlines |
| `danger` | `--seu-danger` | Destructive actions & warnings |
| `incognito_bg` | `--seu-incognito-bg` | Incognito window background |
| `incognito_accent` | `--seu-incognito-accent` | Incognito window accent |
| `font_display` | `--seu-font-display` | Display / headline font family |
| `font_body` | `--seu-font-body` | UI body font family |
| `font_mono` | `--seu-font-mono` | Monospace / terminal / code font family |
| `radius` | `--seu-radius` | Base corner radius |
| `shadow` | `--seu-shadow` | Base elevation / hard shadow |

Any entries inside `"custom": { ... }` are automatically exposed as `--seu-custom-<key>`.

## Declarative Primitives (`data-seu-*`)

Themes never execute JS inside `seu://shell/` or `seu://newtab/`. Instead, bind state and actions declaratively:
- `data-seu-bind="path"` + optional `data-seu-format="roman|meter|compact"`
- `data-seu-t="i18n.key"` / `data-seu-placeholder-t="i18n.key"`
- `<template data-seu-repeat="tabs|tabGroups|shortcuts|themes|searchCategories">`
- `data-seu-action="..."` + `data-seu-param="..."` (e.g. `tabs.create`, `tabs.activate`, `overlay.toggleMenu`, `overlay.toggleMemory`, `overlay.toggleBlocker`, `studio.setMode`, `gameMode.toggle`, `media.playPause`)
- `data-seu-input="omnibox|search"` / `data-seu-submit="omnibox|search"`
- `data-seu-if="..."`, `data-state="..."`, `data-seu-drag-region`

## Per-Site Skins & Security Rules

Declare site skins in `manifest.json`:
```json
"sites": {
  "youtube.com": ["sites/youtube.com.css", "sites/youtube.com.js"]
},
"permissions": {
  "site_scripts": true
}
```
- **CSS** is injected at `on_load_start` (anti-flash) and inherits `:root` `--seu-*` tokens. External `@import` and `url(http...)` are rejected by the Rust sanitizer.
- **JS** runs inside a strict isolated IIFE (`seu = undefined`) with automatic SPA navigation events (`seu:skin-navigate` / `seu:skin-disable`).
- **Imported `.seutheme` packages** always have `site_scripts` forced to `false` until the user explicitly opts in via `seu://themes/` or `seu://settings/`.
