//! Theme manifest, semantic token contract, HTML/CSS sanitization, and built-in
//! theme package loader.
//!
//! Platform-agnostic (no CEF/Win32 dependencies) so the same manifest, tokens,
//! and validation layer can be shared with Android via UniFFI (SPEC §6.1, §12.1).

pub mod sanitize;
pub mod site_skins;
pub mod user_themes;
pub mod wcag;

use include_dir::{include_dir, Dir};
use sanitize::{validate_and_rewrite_css, validate_and_sanitize_html};
use serde::{Deserialize, Serialize};
use site_skins::{validate_site_js, validate_site_pattern, SiteSkinAsset};
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;
use user_themes::{
    default_user_themes_dir, export_theme_package, import_font_asset, import_theme_package,
    load_disk_theme_dir, save_user_theme as persist_user_theme, ImportedFontResult,
    SaveUserThemeInput,
};

pub const DEFAULT_THEME_ID: &str = "y2k-maximal";

pub(crate) static BUILTIN_THEMES_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../themes");

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThemeError {
    #[error("invalid theme id '{0}': only lowercase letters, digits, and '-' are allowed")]
    InvalidId(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("missing theme file: {0}")]
    MissingFile(String),
    #[error("invalid token value for '{0}'")]
    InvalidToken(String),
    #[error("unsafe theme html: {0}")]
    UnsafeHtml(String),
    #[error("unsafe theme css: {0}")]
    UnsafeCss(String),
    #[error("json parse error: {0}")]
    Json(String),
    #[error("io error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ThemeEntrypoints {
    pub tokens: String,
    pub shell_css: String,
    pub shell_html: Option<String>,
    pub newtab_html: String,
    pub newtab_css: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MobileEntrypoints {
    pub shell: Option<String>,
    pub newtab: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ThemePermissions {
    #[serde(default)]
    pub site_scripts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub base: Option<String>,
    pub entry: ThemeEntrypoints,
    #[serde(default)]
    pub sites: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub permissions: ThemePermissions,
    /// Reserved for future Android layouts (validated in v1).
    #[serde(default)]
    pub mobile: Option<MobileEntrypoints>,
}

/// Mandatory semantic tokens shared across all themes, internal `seu://` pages,
/// incognito windows, visual editor (WCAG AA), themed DevTools, and future Android.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticTokens {
    pub bg: String,
    pub surface: String,
    pub surface2: String,
    pub text: String,
    pub text_muted: String,
    pub accent: String,
    pub accent_text: String,
    pub border: String,
    pub danger: String,
    pub incognito_bg: String,
    pub incognito_accent: String,
    pub font_display: String,
    pub font_body: String,
    pub font_mono: String,
    pub radius: String,
    pub shadow: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeTokens {
    pub semantic: SemanticTokens,
    #[serde(default)]
    pub custom: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemePreview {
    pub bg: String,
    pub surface: String,
    pub surface2: String,
    pub text: String,
    pub accent: String,
    pub border: String,
    pub radius: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub builtin: bool,
    pub preview: ThemePreview,
    #[serde(default)]
    pub site_skin_patterns: Vec<String>,
    #[serde(default)]
    pub has_site_scripts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedTheme {
    pub manifest: ThemeManifest,
    pub tokens: ThemeTokens,
    pub tokens_css: String,
    pub shell_html: Option<String>,
    pub shell_css: String,
    pub newtab_html: String,
    pub newtab_css: String,
    #[serde(default)]
    pub site_skins: Vec<SiteSkinAsset>,
    pub base_url: String,
    pub builtin: bool,
}

pub fn validate_manifest(manifest: &ThemeManifest) -> Result<(), ThemeError> {
    if manifest.id.is_empty()
        || !manifest
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ThemeError::InvalidId(manifest.id.clone()));
    }
    if manifest.name.trim().is_empty() {
        return Err(ThemeError::MissingField("name"));
    }
    if manifest.version.trim().is_empty() {
        return Err(ThemeError::MissingField("version"));
    }
    if manifest.entry.tokens.trim().is_empty() {
        return Err(ThemeError::MissingField("entry.tokens"));
    }
    if manifest.entry.shell_css.trim().is_empty() {
        return Err(ThemeError::MissingField("entry.shell_css"));
    }
    if manifest.entry.newtab_html.trim().is_empty() {
        return Err(ThemeError::MissingField("entry.newtab_html"));
    }
    if manifest.entry.newtab_css.trim().is_empty() {
        return Err(ThemeError::MissingField("entry.newtab_css"));
    }
    Ok(())
}

pub fn validate_tokens(tokens: &ThemeTokens) -> Result<(), ThemeError> {
    let required = [
        ("bg", &tokens.semantic.bg),
        ("surface", &tokens.semantic.surface),
        ("surface2", &tokens.semantic.surface2),
        ("text", &tokens.semantic.text),
        ("text_muted", &tokens.semantic.text_muted),
        ("accent", &tokens.semantic.accent),
        ("accent_text", &tokens.semantic.accent_text),
        ("border", &tokens.semantic.border),
        ("danger", &tokens.semantic.danger),
        ("incognito_bg", &tokens.semantic.incognito_bg),
        ("incognito_accent", &tokens.semantic.incognito_accent),
        ("font_body", &tokens.semantic.font_body),
    ];

    for (name, val) in required {
        let v = val.trim();
        if v.is_empty()
            || v.contains("url(")
            || v.contains("@import")
            || v.contains(';')
            || v.contains('}')
        {
            return Err(ThemeError::InvalidToken(name.to_string()));
        }
    }

    for (k, v) in &tokens.custom {
        if !k
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ThemeError::InvalidToken(k.clone()));
        }
        if v.contains("http://")
            || v.contains("https://")
            || v.contains("@import")
            || v.contains(';')
            || v.contains('}')
        {
            return Err(ThemeError::InvalidToken(k.clone()));
        }
    }

    Ok(())
}

/// Generates `:root` CSS custom properties from validated semantic + custom tokens,
/// plus automatic incognito overrides.
pub fn tokens_to_css(tokens: &ThemeTokens) -> String {
    let s = &tokens.semantic;
    let mut css = String::with_capacity(512);
    css.push_str(":root {\n");
    let semantic_pairs = [
        ("--seu-bg", &s.bg),
        ("--seu-surface", &s.surface),
        ("--seu-surface2", &s.surface2),
        ("--seu-text", &s.text),
        ("--seu-text-muted", &s.text_muted),
        ("--seu-accent", &s.accent),
        ("--seu-accent-text", &s.accent_text),
        ("--seu-border", &s.border),
        ("--seu-danger", &s.danger),
        ("--seu-incognito-bg", &s.incognito_bg),
        ("--seu-incognito-accent", &s.incognito_accent),
        ("--seu-font-display", &s.font_display),
        ("--seu-font-body", &s.font_body),
        ("--seu-font-mono", &s.font_mono),
        ("--seu-radius", &s.radius),
        ("--seu-shadow", &s.shadow),
    ];
    for (var, val) in semantic_pairs {
        css.push_str(&format!("  {var}: {val};\n"));
    }
    for (k, val) in &tokens.custom {
        css.push_str(&format!("  --seu-custom-{k}: {val};\n"));
    }
    css.push_str("}\n[data-incognito=\"true\"] {\n");
    css.push_str("  --seu-bg: var(--seu-incognito-bg);\n");
    css.push_str("  --seu-accent: var(--seu-incognito-accent);\n");
    css.push_str("}\n");
    css
}

#[derive(Debug)]
pub struct ThemeRegistry {
    themes: BTreeMap<String, LoadedTheme>,
    user_dir: PathBuf,
}

const OFFICIAL_THEME_ORDER: &[&str] = &[
    "y2k-maximal",
    "glassmorphism",
    "neumorphism",
    "editorial",
    "brutalist",
    "neobrutalist",
    "retro-70s",
    "y2k-classic",
    "maximal",
];

impl ThemeRegistry {
    pub fn load_builtin() -> Result<Self, ThemeError> {
        Self::load_with_user_dir(default_user_themes_dir())
    }

    pub fn load_with_user_dir(user_dir: PathBuf) -> Result<Self, ThemeError> {
        let mut themes = BTreeMap::new();
        for dir in BUILTIN_THEMES_DIR.dirs() {
            let loaded = load_embedded_theme_dir(dir)?;
            themes.insert(loaded.manifest.id.clone(), loaded);
        }
        let mut reg = Self { themes, user_dir };
        reg.reload_user_themes();
        Ok(reg)
    }

    pub fn user_dir(&self) -> &std::path::Path {
        &self.user_dir
    }

    pub fn reload_user_themes(&mut self) -> usize {
        self.themes.retain(|_, t| t.builtin);
        let mut loaded_count = 0usize;
        if let Ok(rd) = std::fs::read_dir(&self.user_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                if let Ok(loaded) = load_disk_theme_dir(&p) {
                    if !self.is_builtin(&loaded.manifest.id) {
                        self.themes.insert(loaded.manifest.id.clone(), loaded);
                        loaded_count += 1;
                    }
                }
            }
        }
        loaded_count
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        self.themes.get(id).map(|t| t.builtin).unwrap_or(false)
    }

    pub fn get_exact(&self, id: &str) -> Option<&LoadedTheme> {
        self.themes.get(id)
    }

    pub fn save_user_theme(
        &mut self,
        input: SaveUserThemeInput,
    ) -> Result<LoadedTheme, ThemeError> {
        let loaded = persist_user_theme(self, &self.user_dir, input)?;
        self.themes
            .insert(loaded.manifest.id.clone(), loaded.clone());
        Ok(loaded)
    }

    pub fn delete_user_theme(&mut self, id: &str) -> Result<(), ThemeError> {
        if self.is_builtin(id) {
            return Err(ThemeError::InvalidId(format!(
                "{id} is a built-in theme and cannot be deleted"
            )));
        }
        if !user_themes::is_valid_theme_id(id) {
            return Err(ThemeError::InvalidId(id.to_string()));
        }
        self.themes.remove(id);
        let target = self.user_dir.join(id);
        if target.is_dir() {
            std::fs::remove_dir_all(&target).map_err(|e| ThemeError::Io(e.to_string()))?;
        }
        Ok(())
    }

    pub fn export_package(&self, id: &str) -> Result<Vec<u8>, ThemeError> {
        export_theme_package(self, &self.user_dir, id)
    }

    pub fn import_package(&mut self, zip_bytes: &[u8]) -> Result<LoadedTheme, ThemeError> {
        let loaded = import_theme_package(self, &self.user_dir, zip_bytes)?;
        self.themes
            .insert(loaded.manifest.id.clone(), loaded.clone());
        Ok(loaded)
    }

    pub fn import_font(
        &mut self,
        theme_id: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<ImportedFontResult, ThemeError> {
        let res = import_font_asset(&self.user_dir, theme_id, filename, bytes)?;
        let dir = self.user_dir.join(theme_id);
        if let Ok(reloaded) = load_disk_theme_dir(&dir) {
            self.themes.insert(reloaded.manifest.id.clone(), reloaded);
        }
        Ok(res)
    }

    pub fn list_summaries(&self) -> Vec<ThemeSummary> {
        let mut list: Vec<ThemeSummary> = self
            .themes
            .values()
            .map(|t| ThemeSummary {
                id: t.manifest.id.clone(),
                name: t.manifest.name.clone(),
                version: t.manifest.version.clone(),
                author: t.manifest.author.clone(),
                description: t.manifest.description.clone(),
                builtin: t.builtin,
                preview: ThemePreview {
                    bg: t.tokens.semantic.bg.clone(),
                    surface: t.tokens.semantic.surface.clone(),
                    surface2: t.tokens.semantic.surface2.clone(),
                    text: t.tokens.semantic.text.clone(),
                    accent: t.tokens.semantic.accent.clone(),
                    border: t.tokens.semantic.border.clone(),
                    radius: t.tokens.semantic.radius.clone(),
                },
                site_skin_patterns: t.site_skins.iter().map(|s| s.pattern.clone()).collect(),
                has_site_scripts: t.site_skins.iter().any(|s| !s.js.trim().is_empty()),
            })
            .collect();

        list.sort_by_key(|s| {
            let rank = OFFICIAL_THEME_ORDER
                .iter()
                .position(|&id| id == s.id)
                .unwrap_or(OFFICIAL_THEME_ORDER.len());
            (rank, s.id.clone())
        });
        list
    }

    pub fn get(&self, id: &str) -> Option<&LoadedTheme> {
        self.themes
            .get(id)
            .or_else(|| self.themes.get(DEFAULT_THEME_ID))
    }

    pub fn exists(&self, id: &str) -> bool {
        self.themes.contains_key(id)
    }

    /// Resolves a static asset inside an embedded theme package (e.g. `assets/fonts/...`).
    pub fn resolve_builtin_asset(theme_id: &str, relative_path: &str) -> Option<&'static [u8]> {
        if relative_path.contains("..")
            || !theme_id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return None;
        }
        let full = format!("{theme_id}/{relative_path}");
        BUILTIN_THEMES_DIR
            .get_file(&full)
            .map(|file| file.contents())
    }
}

pub(crate) fn load_site_skins_with<F>(
    manifest: &ThemeManifest,
    read_rel: &F,
) -> Result<Vec<SiteSkinAsset>, ThemeError>
where
    F: Fn(&str) -> Result<String, ThemeError>,
{
    let mut out = Vec::new();
    for (raw_pattern, files) in &manifest.sites {
        let pattern = validate_site_pattern(raw_pattern)?;
        let mut css_files = Vec::new();
        let mut js_files = Vec::new();
        let mut css_chunks = Vec::new();
        let mut js_chunks = Vec::new();

        for rel in files {
            let trimmed = rel.trim();
            if trimmed.is_empty() || trimmed.contains("..") {
                return Err(ThemeError::MissingFile(rel.clone()));
            }
            let lower = trimmed.to_ascii_lowercase();
            if lower.ends_with(".css") {
                let raw_css = read_rel(trimmed)?;
                let safe_css = validate_and_rewrite_css(&raw_css, &manifest.id)?;
                css_files.push(trimmed.to_string());
                css_chunks.push(safe_css);
            } else if lower.ends_with(".js") {
                let raw_js = read_rel(trimmed)?;
                let safe_js = validate_site_js(&raw_js, trimmed)?;
                js_files.push(trimmed.to_string());
                js_chunks.push(safe_js);
            } else {
                return Err(ThemeError::InvalidToken(format!(
                    "unsupported site skin asset extension: {trimmed}"
                )));
            }
        }

        out.push(SiteSkinAsset {
            pattern,
            css_files,
            js_files,
            css: css_chunks.join("\n"),
            js: js_chunks.join("\n;\n"),
        });
    }
    Ok(out)
}

fn load_embedded_theme_dir(dir: &Dir<'_>) -> Result<LoadedTheme, ThemeError> {
    let dir_name = dir
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let read_str = |rel: &str| -> Result<String, ThemeError> {
        let full_path = format!("{dir_name}/{rel}");
        let file = BUILTIN_THEMES_DIR
            .get_file(&full_path)
            .ok_or(ThemeError::MissingFile(full_path))?;
        file.contents_utf8()
            .map(|s| s.to_string())
            .ok_or_else(|| ThemeError::MissingFile(rel.to_string()))
    };

    let manifest_raw = read_str("manifest.json")?;
    let manifest: ThemeManifest =
        serde_json::from_str(&manifest_raw).map_err(|e| ThemeError::Json(e.to_string()))?;
    validate_manifest(&manifest)?;

    let tokens_raw = read_str(&manifest.entry.tokens)?;
    let tokens: ThemeTokens =
        serde_json::from_str(&tokens_raw).map_err(|e| ThemeError::Json(e.to_string()))?;
    validate_tokens(&tokens)?;

    let tokens_css = tokens_to_css(&tokens);

    let shell_css_raw = read_str(&manifest.entry.shell_css)?;
    let shell_css = validate_and_rewrite_css(&shell_css_raw, &manifest.id)?;

    let shell_html = match &manifest.entry.shell_html {
        Some(path) => Some(validate_and_sanitize_html(&read_str(path)?)?),
        None => None,
    };

    let newtab_html_raw = read_str(&manifest.entry.newtab_html)?;
    let newtab_html = validate_and_sanitize_html(&newtab_html_raw)?;

    let newtab_css_raw = read_str(&manifest.entry.newtab_css)?;
    let newtab_css = validate_and_rewrite_css(&newtab_css_raw, &manifest.id)?;

    let site_skins = load_site_skins_with(&manifest, &read_str)?;
    let base_url = format!("seu://theme/{}/", manifest.id);

    Ok(LoadedTheme {
        manifest,
        tokens,
        tokens_css,
        shell_html,
        shell_css,
        newtab_html,
        newtab_css,
        site_skins,
        base_url,
        builtin: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_validates_all_nine_official_builtin_themes() {
        let registry = ThemeRegistry::load_builtin();
        assert!(
            registry.is_ok(),
            "failed to load builtin themes: {registry:?}"
        );
        if let Ok(reg) = registry {
            let summaries = reg.list_summaries();
            assert_eq!(summaries.len(), 9, "expected all 9 official themes");
            let ids: Vec<&str> = summaries.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(ids, OFFICIAL_THEME_ORDER);

            for expected_id in OFFICIAL_THEME_ORDER {
                let loaded = reg.get(expected_id);
                assert!(loaded.is_some(), "missing official theme: {expected_id}");
                if let Some(theme) = loaded {
                    assert_eq!(theme.manifest.id, *expected_id);
                    assert!(theme.shell_html.is_some());
                    assert!(!theme.shell_css.is_empty());
                    assert!(!theme.newtab_html.is_empty());
                    assert!(!theme.newtab_css.is_empty());
                    assert!(theme.tokens_css.contains("--seu-bg:"));
                    assert!(theme.tokens_css.contains("--seu-accent:"));
                }
            }

            let y2k = reg.get(DEFAULT_THEME_ID);
            if let Some(theme) = y2k {
                assert!(theme.tokens_css.contains("--seu-accent: #FF2E93;"));
                assert!(theme
                    .newtab_css
                    .contains("seu://theme/y2k-maximal/assets/fonts/"));
                assert!(theme.manifest.permissions.site_scripts);
                assert_eq!(theme.site_skins.len(), 1);
                assert_eq!(theme.site_skins[0].pattern, "youtube.com");
                assert!(theme.site_skins[0].css.contains("data-seu-site-skin"));
                assert!(theme.site_skins[0].js.contains("seu-yt-y2k-bar"));
            }

            let font = ThemeRegistry::resolve_builtin_asset(
                "y2k-maximal",
                "assets/fonts/rubik-mono-one-400-latin.woff2",
            );
            assert!(font.map(|b| !b.is_empty()).unwrap_or(false));

            let outfit = ThemeRegistry::resolve_builtin_asset(
                "glassmorphism",
                "assets/fonts/outfit-400-latin.woff2",
            );
            assert!(outfit.map(|b| !b.is_empty()).unwrap_or(false));
        }
    }

    #[test]
    fn rejects_external_css_in_tokens() {
        let mut bad_tokens = ThemeTokens {
            semantic: SemanticTokens {
                bg: "#070B24".into(),
                surface: "#0B1240".into(),
                surface2: "#0E1650".into(),
                text: "#E6E9FF".into(),
                text_muted: "#8F9BD9".into(),
                accent: "#FF2E93".into(),
                accent_text: "#070B24".into(),
                border: "#2A3680".into(),
                danger: "#FF3B5C".into(),
                incognito_bg: "#120826".into(),
                incognito_accent: "#8A7CFF".into(),
                font_display: "Rubik Mono One".into(),
                font_body: "Space Grotesk".into(),
                font_mono: "VT323".into(),
                radius: "16px".into(),
                shadow: "8px 8px 0 #FF2E93".into(),
            },
            custom: BTreeMap::new(),
        };
        assert!(validate_tokens(&bad_tokens).is_ok());

        bad_tokens
            .custom
            .insert("exfil".into(), "url(https://evil.test/ping)".into());
        assert_eq!(
            validate_tokens(&bad_tokens),
            Err(ThemeError::InvalidToken("exfil".into()))
        );
    }
}
