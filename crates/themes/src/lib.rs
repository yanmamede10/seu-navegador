//! Theme manifest, semantic token contract, and package validation.
//!
//! Platform-agnostic (no CEF/Win32 dependencies) so the same manifest and
//! tokens schema can be shared with Android in the future (SPEC §6.1, §12.1).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThemeError {
    #[error("invalid theme id '{0}': only lowercase letters, digits, and '-' are allowed")]
    InvalidId(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid token value for '{0}'")]
    InvalidToken(String),
    #[error("json parse error: {0}")]
    Json(String),
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
        if v.is_empty() || v.contains("url(") || v.contains("@import") {
            return Err(ThemeError::InvalidToken(name.to_string()));
        }
    }

    for (k, v) in &tokens.custom {
        if v.contains("http://") || v.contains("https://") || v.contains("@import") {
            return Err(ThemeError::InvalidToken(k.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_manifest_and_blocks_external_css_in_tokens() {
        let manifest = ThemeManifest {
            id: "y2k-maximal".to_string(),
            name: "Y2K Maximal".to_string(),
            version: "1.0.0".to_string(),
            author: "Yan".to_string(),
            description: "Default theme".to_string(),
            base: None,
            entry: ThemeEntrypoints {
                tokens: "tokens.json".to_string(),
                shell_css: "shell.css".to_string(),
                shell_html: Some("shell.html".to_string()),
                newtab_html: "newtab.html".to_string(),
                newtab_css: "newtab.css".to_string(),
            },
            sites: BTreeMap::new(),
            permissions: ThemePermissions { site_scripts: true },
            mobile: Some(MobileEntrypoints::default()),
        };
        assert!(validate_manifest(&manifest).is_ok());

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
