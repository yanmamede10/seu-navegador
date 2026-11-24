//! HTML & CSS security validation/sanitization for theme packages.
//!
//! Runs in addition to the strict CSP on `seu://` origins to provide defense in
//! depth: third-party and built-in themes may never inject scripts, external
//! URLs, event handlers, or dangerous elements into privileged UI pages.

use crate::ThemeError;

const FORBIDDEN_HTML_TAGS: &[&str] = &[
    "<script",
    "</script",
    "<iframe",
    "<object",
    "<embed",
    "<applet",
    "<meta",
    "<base",
    "<link",
    "<style",
    "<foreignobject",
];

const FORBIDDEN_HTML_PATTERNS: &[&str] = &["javascript:", "vbscript:", "data:text/html"];

/// Validates declarative theme HTML, rejecting scripts, inline event handlers,
/// external URLs, and forbidden tags.
pub fn validate_and_sanitize_html(html: &str) -> Result<String, ThemeError> {
    let lower = html.to_ascii_lowercase();

    for tag in FORBIDDEN_HTML_TAGS {
        if lower.contains(tag) {
            return Err(ThemeError::UnsafeHtml(format!(
                "forbidden tag or construct: {tag}"
            )));
        }
    }

    for pat in FORBIDDEN_HTML_PATTERNS {
        if lower.contains(pat) {
            return Err(ThemeError::UnsafeHtml(format!(
                "forbidden URI scheme: {pat}"
            )));
        }
    }

    // Reject inline event handlers (`onclick=`, `onerror=`, `onload=`, etc.).
    if contains_inline_event_handler(&lower) {
        return Err(ThemeError::UnsafeHtml(
            "inline DOM event handler attribute (on*=) is not allowed".to_string(),
        ));
    }

    if lower.contains("http://") || lower.contains("https://") {
        return Err(ThemeError::UnsafeHtml(
            "external http(s) URLs are forbidden inside theme HTML".to_string(),
        ));
    }

    Ok(html.to_string())
}

/// Validates theme CSS and rewrites relative `./assets/...` URLs to
/// `seu://theme/<theme_id>/assets/...`.
pub fn validate_and_rewrite_css(css: &str, theme_id: &str) -> Result<String, ThemeError> {
    let lower = css.to_ascii_lowercase();

    for forbidden in ["@import", "expression(", "behavior:", "-moz-binding:"] {
        if lower.contains(forbidden) {
            return Err(ThemeError::UnsafeCss(format!(
                "forbidden CSS construct: {forbidden}"
            )));
        }
    }

    // Scan every `url(...)` argument and reject external/traversal URLs.
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] == b"url(" {
            let start = i + 4;
            let end = lower[start..]
                .find(')')
                .map(|off| start + off)
                .unwrap_or(lower.len());
            let inner = lower[start..end].trim().trim_matches(['"', '\'']).trim();

            if inner.starts_with("http://")
                || inner.starts_with("https://")
                || inner.starts_with("//")
                || inner.starts_with("javascript:")
                || inner.contains("..")
            {
                return Err(ThemeError::UnsafeCss(format!(
                    "disallowed CSS url(): {inner}"
                )));
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    let base_prefix = format!("seu://theme/{theme_id}/assets/");
    let rewritten = css
        .replace("url(./assets/", &format!("url({base_prefix}"))
        .replace("url(\"./assets/", &format!("url(\"{base_prefix}"))
        .replace("url('./assets/", &format!("url('{base_prefix}"));

    Ok(rewritten)
}

fn contains_inline_event_handler(lower_html: &str) -> bool {
    let bytes = lower_html.as_bytes();
    let mut in_tag = false;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                in_tag = true;
                i += 1;
            }
            b'>' => {
                in_tag = false;
                i += 1;
            }
            _ if in_tag => {
                // Match whitespace followed by `on[a-z]+\s*=`.
                if bytes[i].is_ascii_whitespace() {
                    let mut j = i + 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j + 3 < bytes.len() && bytes[j] == b'o' && bytes[j + 1] == b'n' {
                        let mut k = j + 2;
                        while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
                            k += 1;
                        }
                        if k > j + 2 {
                            let mut eq = k;
                            while eq < bytes.len() && bytes[eq].is_ascii_whitespace() {
                                eq += 1;
                            }
                            if eq < bytes.len() && bytes[eq] == b'=' {
                                return true;
                            }
                        }
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dangerous_html_and_allows_declarative_markup() {
        assert!(validate_and_sanitize_html(
            r#"<div data-seu-action="tabs.create"><svg viewBox="0 0 10 10"><path d="M0 0h10"/></svg></div>"#
        )
        .is_ok());

        assert!(matches!(
            validate_and_sanitize_html("<script>alert(1)</script>"),
            Err(ThemeError::UnsafeHtml(_))
        ));
        assert!(matches!(
            validate_and_sanitize_html(r#"<button onclick="evil()">hi</button>"#),
            Err(ThemeError::UnsafeHtml(_))
        ));
        assert!(matches!(
            validate_and_sanitize_html(r#"<svg onload="evil()"></svg>"#),
            Err(ThemeError::UnsafeHtml(_))
        ));
        assert!(matches!(
            validate_and_sanitize_html(r#"<a href="javascript:alert(1)">x</a>"#),
            Err(ThemeError::UnsafeHtml(_))
        ));
        assert!(matches!(
            validate_and_sanitize_html(r#"<iframe src="seu://shell"></iframe>"#),
            Err(ThemeError::UnsafeHtml(_))
        ));
    }

    #[test]
    fn rewrites_local_css_assets_and_blocks_external_urls() {
        let ok = validate_and_rewrite_css(
            "@font-face { src: url(./assets/fonts/vt323-400-latin.woff2) format('woff2'); }",
            "y2k-maximal",
        );
        assert_eq!(
            ok.ok().as_deref(),
            Some("@font-face { src: url(seu://theme/y2k-maximal/assets/fonts/vt323-400-latin.woff2) format('woff2'); }")
        );

        assert!(matches!(
            validate_and_rewrite_css("@import url('https://evil.test/a.css');", "y2k-maximal"),
            Err(ThemeError::UnsafeCss(_))
        ));
        assert!(matches!(
            validate_and_rewrite_css(
                "body { background: url(https://evil.test/x.png); }",
                "y2k-maximal"
            ),
            Err(ThemeError::UnsafeCss(_))
        ));
        assert!(matches!(
            validate_and_rewrite_css("body { background: url(../secret.png); }", "y2k-maximal"),
            Err(ThemeError::UnsafeCss(_))
        ));
    }
}
