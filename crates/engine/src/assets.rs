//! Embedded UI and theme assets served via the privileged `seu://` internal scheme.

use crate::metrics::read_tab_thumbnail_bytes;
use include_dir::{include_dir, Dir};
use seu_themes::ThemeRegistry;

static UI_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/dist");

pub const STRICT_SEU_CSP: &str = "default-src 'self' seu:; script-src 'self' seu:; style-src 'self' 'unsafe-inline' seu:; img-src 'self' seu: data:; font-src 'self' seu:; connect-src 'none'; object-src 'none'; frame-ancestors 'none';";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub status_code: i32,
    pub status_text: &'static str,
    pub mime_type: &'static str,
    pub bytes: Vec<u8>,
}

/// Resolves a `seu://` URL (e.g. `seu://shell/`, `seu://theme/y2k-maximal/assets/fonts/...`,
/// `seu://newtab/`, `seu://overlay/`) to embedded bytes and pure MIME type.
pub fn resolve_seu_url(raw_url: &str) -> ResolvedAsset {
    let stripped = raw_url.split(['?', '#']).next().unwrap_or(raw_url).trim();

    let Some(rest) = stripped
        .strip_prefix("seu://")
        .or_else(|| stripped.strip_prefix("SEU://"))
    else {
        return not_found();
    };

    let (host, raw_path) = match rest.split_once('/') {
        Some((h, p)) => (h.to_ascii_lowercase(), p.trim_start_matches('/')),
        None => (rest.to_ascii_lowercase(), ""),
    };

    if raw_path.contains("..") {
        return not_found();
    }

    if host == "theme" {
        if let Some((theme_id, rel_path)) = raw_path.split_once('/') {
            let user_dir = seu_themes::user_themes::default_user_themes_dir();
            if let Some(bytes) =
                seu_themes::user_themes::resolve_user_theme_asset(&user_dir, theme_id, rel_path)
            {
                return ResolvedAsset {
                    status_code: 200,
                    status_text: "OK",
                    mime_type: mime_for_path(rel_path),
                    bytes,
                };
            }
            if let Some(bytes) = ThemeRegistry::resolve_builtin_asset(theme_id, rel_path) {
                return ResolvedAsset {
                    status_code: 200,
                    status_text: "OK",
                    mime_type: mime_for_path(rel_path),
                    bytes: bytes.to_vec(),
                };
            }
        }
        return not_found();
    }

    if host == "thumb" {
        if let Some(bytes) = read_tab_thumbnail_bytes(raw_path) {
            return ResolvedAsset {
                status_code: 200,
                status_text: "OK",
                mime_type: "image/jpeg",
                bytes,
            };
        }
        return not_found();
    }

    let candidate_path = if host == "themes" && raw_path.trim_end_matches('/') == "editor" {
        "theme-editor.html"
    } else if raw_path.is_empty() {
        match host.as_str() {
            "shell" => "shell.html",
            "overlay" => "overlay.html",
            "newtab" => "newtab.html",
            "settings" => "settings.html",
            "history" => "history.html",
            "bookmarks" => "bookmarks.html",
            "downloads" => "downloads.html",
            "themes" => "themes.html",
            "studio" => "studio.html",
            _ => {
                return placeholder_internal_page(&host);
            }
        }
    } else {
        raw_path
    };

    if let Some(file) = UI_DIST.get_file(candidate_path) {
        return ResolvedAsset {
            status_code: 200,
            status_text: "OK",
            mime_type: mime_for_path(candidate_path),
            bytes: file.contents().to_vec(),
        };
    }

    not_found()
}

fn mime_for_path(path: &str) -> &'static str {
    // CEF's `response.set_mime_type` requires a pure MIME type (without `; charset=...`),
    // otherwise Chromium falls back to rendering the payload as plain text.
    if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".ttf") {
        "font/ttf"
    } else {
        "application/octet-stream"
    }
}

fn placeholder_internal_page(host: &str) -> ResolvedAsset {
    let safe_name: String = host
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>seu://{safe_name}</title><style>body{{margin:0;padding:48px;background:#070B24;color:#E6E9FF;font-family:system-ui,sans-serif}}h1{{color:#FF2E93}}</style></head><body><h1>seu://{safe_name}</h1><p>Página interna reservada para a próxima fase.</p></body></html>"
    );
    ResolvedAsset {
        status_code: 200,
        status_text: "OK",
        mime_type: "text/html",
        bytes: html.into_bytes(),
    }
}

fn not_found() -> ResolvedAsset {
    ResolvedAsset {
        status_code: 404,
        status_text: "Not Found",
        mime_type: "text/plain",
        bytes: b"404 Not Found".to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_core_seu_pages_and_blocks_traversal() {
        let shell = resolve_seu_url("seu://shell/?win=1");
        assert_eq!(shell.status_code, 200);
        assert_eq!(shell.mime_type, "text/html");
        assert!(!shell.bytes.is_empty());

        let overlay = resolve_seu_url("seu://overlay/?win=1");
        assert_eq!(overlay.status_code, 200);

        let newtab = resolve_seu_url("seu://newtab/");
        assert_eq!(newtab.status_code, 200);

        let themes = resolve_seu_url("seu://themes/?win=1");
        assert_eq!(themes.status_code, 200);
        assert_eq!(themes.mime_type, "text/html");

        let editor = resolve_seu_url("seu://themes/editor?base=y2k-maximal");
        assert_eq!(editor.status_code, 200);
        assert_eq!(editor.mime_type, "text/html");

        let bad = resolve_seu_url("seu://shell/../../secret");
        assert_eq!(bad.status_code, 404);

        let font =
            resolve_seu_url("seu://theme/y2k-maximal/assets/fonts/rubik-mono-one-400-latin.woff2");
        assert_eq!(font.status_code, 200);
        assert_eq!(font.mime_type, "font/woff2");
        assert!(!font.bytes.is_empty());
    }
}
