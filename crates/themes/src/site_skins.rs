//! Per-site skins engine (SPEC §7): domain matching, CSS/JS validation,
//! security opt-in enforcement, anti-flash CSS injection, and isolated JS/SPA wrappers.
//!
//! Platform-agnostic (zero CEF/Win32 dependencies) so the same matching and
//! validation rules can be shared with Android via UniFFI (SPEC §12.1).

use crate::{LoadedTheme, ThemeError};
use serde::{Deserialize, Serialize};

pub const MAX_SITE_JS_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteSkinAsset {
    pub pattern: String,
    #[serde(default)]
    pub css_files: Vec<String>,
    #[serde(default)]
    pub js_files: Vec<String>,
    #[serde(default)]
    pub css: String,
    #[serde(default)]
    pub js: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedSiteSkin {
    pub theme_id: String,
    pub css: String,
    pub js: Option<String>,
    pub matched_patterns: Vec<String>,
    pub has_blocked_js: bool,
}

/// Validates a domain pattern declared in `manifest.json -> sites`.
pub fn validate_site_pattern(pattern: &str) -> Result<String, ThemeError> {
    let trimmed = pattern.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.len() > 253 {
        return Err(ThemeError::InvalidToken(format!("sites.{pattern}")));
    }
    if trimmed == "*" {
        return Ok(trimmed);
    }
    let host_part = trimmed.strip_prefix("*.").unwrap_or(&trimmed);
    if host_part.is_empty()
        || host_part.starts_with('.')
        || host_part.ends_with('.')
        || host_part.contains("..")
        || !host_part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
    {
        return Err(ThemeError::InvalidToken(format!("sites.{pattern}")));
    }
    Ok(trimmed)
}

/// Validates a site skin JS file for basic structural safety and size limits.
pub fn validate_site_js(js: &str, rel_path: &str) -> Result<String, ThemeError> {
    if js.len() > MAX_SITE_JS_BYTES {
        return Err(ThemeError::Io(format!(
            "site script '{rel_path}' exceeds {} KB limit",
            MAX_SITE_JS_BYTES / 1024
        )));
    }
    if js.as_bytes().contains(&0) {
        return Err(ThemeError::Io(format!(
            "site script '{rel_path}' contains binary null bytes"
        )));
    }
    Ok(js.to_string())
}

/// Extracts the normalized lowercase host from an `http://` or `https://` URL.
/// Returns `None` for all internal/privileged schemes (`seu://`, `chrome://`, `file://`, etc.).
pub fn extract_http_host(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let rest = if let Some(r) = trimmed.strip_prefix("https://") {
        r
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        r
    } else {
        // Case-insensitive scheme fallback.
        let lower_prefix = trimmed.get(..8).map(|s| s.to_ascii_lowercase());
        if lower_prefix.as_deref() == Some("https://") {
            &trimmed[8..]
        } else if trimmed
            .get(..7)
            .map(|s| s.eq_ignore_ascii_case("http://"))
            .unwrap_or(false)
        {
            &trimmed[7..]
        } else {
            return None;
        }
    };

    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }

    // Strip optional `user:pass@`.
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, hp)| hp)
        .unwrap_or(authority);

    // Handle bracketed IPv6 or standard `host:port`.
    let host = if host_port.starts_with('[') {
        host_port.split(']').next().unwrap_or(host_port)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };

    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Checks if a normalized hostname matches a site skin pattern (`youtube.com`, `*.youtube.com`, `*`).
pub fn domain_matches_pattern(host: &str, pattern: &str) -> bool {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let p = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || p.is_empty() {
        return false;
    }
    if p == "*" {
        return true;
    }
    let clean = p.strip_prefix("*.").unwrap_or(&p);
    if h == clean {
        return true;
    }
    if let Some(prefix) = h.strip_suffix(clean) {
        return prefix.ends_with('.');
    }
    false
}

/// Resolves CSS + optional JS to inject for `url` under the active theme and security policy.
///
/// Security rules (SPEC §7):
/// - Global kill-switch (`site_skins_enabled == false`) disables all site CSS and JS.
/// - CSS is always allowed when skins are enabled.
/// - JS is allowed automatically ONLY for official built-in themes declaring `permissions.site_scripts = true`.
/// - User/imported themes NEVER run site JS unless explicitly opted in via `allowed_script_themes`.
pub fn resolve_site_skin(
    theme: &LoadedTheme,
    url: &str,
    site_skins_enabled: bool,
    allowed_script_themes: &[String],
) -> Option<ResolvedSiteSkin> {
    if !site_skins_enabled || theme.site_skins.is_empty() {
        return None;
    }

    let host = extract_http_host(url)?;
    let mut matched_patterns = Vec::new();
    let mut css_parts = Vec::new();
    let mut js_parts = Vec::new();

    for skin in &theme.site_skins {
        if domain_matches_pattern(&host, &skin.pattern) {
            matched_patterns.push(skin.pattern.clone());
            if !skin.css.trim().is_empty() {
                css_parts.push(skin.css.as_str());
            }
            if !skin.js.trim().is_empty() {
                js_parts.push(skin.js.as_str());
            }
        }
    }

    if matched_patterns.is_empty() {
        return None;
    }

    let css = if css_parts.is_empty() {
        String::new()
    } else {
        let mut combined =
            String::with_capacity(theme.tokens_css.len() + css_parts.len() * 64 + 128);
        combined.push_str(&theme.tokens_css);
        combined.push('\n');
        for part in css_parts {
            combined.push_str(part);
            combined.push('\n');
        }
        combined
    };

    let has_any_js = !js_parts.is_empty();
    let js_allowed = if theme.builtin {
        theme.manifest.permissions.site_scripts
    } else {
        allowed_script_themes
            .iter()
            .any(|id| id == &theme.manifest.id)
    };

    let (js, has_blocked_js) = if has_any_js && js_allowed {
        (Some(js_parts.join("\n;\n")), false)
    } else {
        (None, has_any_js)
    };

    Some(ResolvedSiteSkin {
        theme_id: theme.manifest.id.clone(),
        css,
        js,
        matched_patterns,
        has_blocked_js,
    })
}

/// Builds an idempotent JS snippet that injects/updates `<style id="seu-site-skin-css">`
/// as early as `on_load_start` to prevent white/unstyled flash.
pub fn build_site_skin_css_injection_js(theme_id: &str, css: &str) -> Option<String> {
    let theme_json = serde_json::to_string(theme_id).ok()?;
    let css_json = serde_json::to_string(css).ok()?;
    Some(format!(
        r#"(function(){{
  var tid = {theme_json};
  var cssText = {css_json};
  function apply(){{
    var root = document.documentElement;
    if(!root) return false;
    root.setAttribute('data-seu-site-skin', tid);
    var id = 'seu-site-skin-css';
    var s = document.getElementById(id);
    if(!s){{
      s = document.createElement('style');
      s.id = id;
      (document.head || root).appendChild(s);
    }}
    s.setAttribute('data-seu-theme', tid);
    if(s.textContent !== cssText){{ s.textContent = cssText; }}
    return true;
  }}
  if(!apply()){{
    document.addEventListener('DOMContentLoaded', apply, {{ once: true }});
  }}
}})();"#
    ))
}

/// Builds a JS snippet that cleanly removes any active site skin CSS & notifies
/// any running site skin JS to tear down its UI elements.
pub fn build_site_skin_clear_js() -> &'static str {
    r#"(function(){
  try {
    var s = document.getElementById('seu-site-skin-css');
    if (s && s.parentNode) s.parentNode.removeChild(s);
    if (document.documentElement) {
      document.documentElement.removeAttribute('data-seu-site-skin');
    }
    window.__seuSkinLoadedTheme = null;
    window.dispatchEvent(new CustomEvent('seu:skin-disable'));
  } catch (_) {}
})();"#
}

/// Wraps site skin JS inside a strict isolated IIFE that:
/// - Explicitly shadows `seu` as `undefined` (defense-in-depth alongside `seu://`-only bridge);
/// - Guards against duplicate evaluation on SPA route changes;
/// - Hooks SPA `pushState` / `replaceState` / `popstate` and dispatches `seu:skin-navigate`.
pub fn build_isolated_site_js_wrapper(theme_id: &str, url: &str, raw_js: &str) -> Option<String> {
    let theme_json = serde_json::to_string(theme_id).ok()?;
    let url_json = serde_json::to_string(url).ok()?;
    Some(format!(
        r#"(function(){{
  var tid = {theme_json};
  var curUrl = {url_json};
  if (!window.__seuSkinSpaHooked) {{
    window.__seuSkinSpaHooked = true;
    try {{
      var emitNav = function() {{
        try {{
          window.dispatchEvent(new CustomEvent('seu:skin-navigate', {{ detail: {{ url: location.href, themeId: window.__seuSkinLoadedTheme }} }}));
        }} catch (_) {{}}
      }};
      var origPush = history.pushState;
      if (typeof origPush === 'function') {{
        history.pushState = function() {{
          var r = origPush.apply(this, arguments);
          emitNav();
          return r;
        }};
      }}
      var origReplace = history.replaceState;
      if (typeof origReplace === 'function') {{
        history.replaceState = function() {{
          var r = origReplace.apply(this, arguments);
          emitNav();
          return r;
        }};
      }}
      window.addEventListener('popstate', emitNav);
    }} catch (_) {{}}
  }}
  if (window.__seuSkinLoadedTheme === tid) {{
    try {{
      window.dispatchEvent(new CustomEvent('seu:skin-navigate', {{ detail: {{ url: curUrl, themeId: tid }} }}));
    }} catch (_) {{}}
    return;
  }}
  if (window.__seuSkinLoadedTheme && window.__seuSkinLoadedTheme !== tid) {{
    try {{ window.dispatchEvent(new CustomEvent('seu:skin-disable')); }} catch (_) {{}}
  }}
  window.__seuSkinLoadedTheme = tid;
  try {{
    (function(window, document, seu) {{
      "use strict";
      {raw_js}
    }})(window, document, undefined);
  }} catch (_err) {{}}
}})();"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SemanticTokens, ThemeEntrypoints, ThemeManifest, ThemePermissions, ThemeTokens};
    use std::collections::BTreeMap;

    fn sample_theme(builtin: bool, perm_scripts: bool) -> LoadedTheme {
        let mut sites = BTreeMap::new();
        sites.insert(
            "youtube.com".to_string(),
            vec![
                "sites/youtube.com.css".to_string(),
                "sites/youtube.com.js".to_string(),
            ],
        );
        LoadedTheme {
            manifest: ThemeManifest {
                id: if builtin {
                    "y2k-maximal".to_string()
                } else {
                    "custom-imported".to_string()
                },
                name: "Test".to_string(),
                version: "1.0.0".to_string(),
                author: "Test".to_string(),
                description: "Test".to_string(),
                base: None,
                entry: ThemeEntrypoints::default(),
                sites,
                permissions: ThemePermissions {
                    site_scripts: perm_scripts,
                },
                mobile: None,
            },
            tokens: ThemeTokens {
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
                    font_display: "sans-serif".into(),
                    font_body: "sans-serif".into(),
                    font_mono: "monospace".into(),
                    radius: "8px".into(),
                    shadow: "none".into(),
                },
                custom: BTreeMap::new(),
            },
            tokens_css: ":root { --seu-bg: #070B24; }\n".to_string(),
            shell_html: None,
            shell_css: String::new(),
            newtab_html: String::new(),
            newtab_css: String::new(),
            site_skins: vec![SiteSkinAsset {
                pattern: "youtube.com".to_string(),
                css_files: vec!["sites/youtube.com.css".to_string()],
                js_files: vec!["sites/youtube.com.js".to_string()],
                css: "body { background: var(--seu-bg) !important; }".to_string(),
                js: "document.documentElement.dataset.test = '1';".to_string(),
            }],
            base_url: "seu://theme/test/".to_string(),
            builtin,
        }
    }

    #[test]
    fn matches_domains_and_subdomains_safely() {
        assert_eq!(
            extract_http_host("https://www.youtube.com/watch?v=123"),
            Some("www.youtube.com".to_string())
        );
        assert_eq!(
            extract_http_host("http://m.youtube.com:8080/shorts/abc"),
            Some("m.youtube.com".to_string())
        );
        assert_eq!(extract_http_host("seu://newtab/"), None);
        assert_eq!(extract_http_host("chrome://settings"), None);
        assert_eq!(extract_http_host("file:///C:/test.html"), None);

        assert!(domain_matches_pattern("youtube.com", "youtube.com"));
        assert!(domain_matches_pattern("www.youtube.com", "youtube.com"));
        assert!(domain_matches_pattern("music.youtube.com", "*.youtube.com"));
        assert!(!domain_matches_pattern("notyoutube.com", "youtube.com"));
        assert!(!domain_matches_pattern(
            "youtube.com.evil.test",
            "youtube.com"
        ));
    }

    #[test]
    fn enforces_security_opt_in_and_global_kill_switch() {
        let builtin = sample_theme(true, true);
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

        // Global kill-switch disables both CSS and JS.
        assert!(resolve_site_skin(&builtin, url, false, &[]).is_none());

        // Builtin theme with site_scripts=true executes both CSS and JS.
        let resolved_builtin = resolve_site_skin(&builtin, url, true, &[]);
        assert!(resolved_builtin.is_some());
        if let Some(r) = resolved_builtin {
            assert!(r.css.contains("--seu-bg"));
            assert!(r.css.contains("background: var(--seu-bg)"));
            assert!(r.js.is_some());
            assert!(!r.has_blocked_js);
        }

        // Imported/user theme blocks JS by default even if it has JS files.
        let imported = sample_theme(false, false);
        let blocked = resolve_site_skin(&imported, url, true, &[]);
        assert!(blocked.is_some());
        if let Some(r) = blocked {
            assert!(!r.css.is_empty());
            assert!(r.js.is_none());
            assert!(r.has_blocked_js);
        }

        // After explicit user opt-in, imported theme JS is allowed.
        let opted_in = resolve_site_skin(&imported, url, true, &["custom-imported".to_string()]);
        assert!(opted_in.is_some());
        if let Some(r) = opted_in {
            assert!(r.js.is_some());
            assert!(!r.has_blocked_js);
        }
    }

    #[test]
    fn wrapper_shadows_seu_and_handles_spa() {
        let wrapped = build_isolated_site_js_wrapper(
            "y2k-maximal",
            "https://www.youtube.com/",
            "console.log(typeof seu);",
        );
        assert!(wrapped.is_some());
        if let Some(code) = wrapped {
            assert!(code.contains("(function(window, document, seu)"));
            assert!(code.contains("(window, document, undefined)"));
            assert!(code.contains("seu:skin-navigate"));
        }
    }
}
