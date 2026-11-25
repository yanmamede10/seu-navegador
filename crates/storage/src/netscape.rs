//! Netscape Bookmark File Format (HTML) parser.
//!
//! Supports standard Chrome / Firefox / Edge / Safari bookmark HTML exports,
//! preserving folder hierarchy (`<H3>` + `<DL>`) and `<A HREF="...">Title</A>` items.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedNetscapeBookmark {
    pub title: String,
    pub url: String,
    pub folder_path: Vec<String>,
}

/// Parses a Netscape Bookmark HTML file into flat bookmark records annotated with
/// their folder path (e.g. `["Bookmarks Bar", "Dev"]`).
pub fn parse_netscape_bookmarks_html(html: &str) -> Vec<ParsedNetscapeBookmark> {
    let mut results = Vec::new();
    let mut folder_stack: Vec<String> = Vec::new();
    let mut pending_folder: Option<String> = None;

    for raw_line in html.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();

        if upper.contains("<DL") {
            if let Some(folder) = pending_folder.take() {
                folder_stack.push(folder);
            }
        }

        if let Some(h3_start) = upper.find("<H3") {
            if let Some(title) = extract_tag_inner_text(&line[h3_start..], "H3") {
                let decoded = decode_html_entities(title.trim());
                if !decoded.is_empty() {
                    pending_folder = Some(decoded);
                }
            }
        }

        if let Some(a_start) = upper.find("<A ") {
            let slice = &line[a_start..];
            if let Some(href) = extract_attr_value(slice, "HREF") {
                let url = decode_html_entities(href.trim());
                if url.starts_with("http://") || url.starts_with("https://") {
                    let raw_title = extract_tag_inner_text(slice, "A").unwrap_or_default();
                    let title = decode_html_entities(raw_title.trim());
                    let display_title = if title.is_empty() { url.clone() } else { title };
                    results.push(ParsedNetscapeBookmark {
                        title: display_title,
                        url,
                        folder_path: folder_stack.clone(),
                    });
                }
            }
        }

        if upper.contains("</DL>") && !folder_stack.is_empty() {
            folder_stack.pop();
        }
    }

    results
}

fn extract_attr_value(tag_slice: &str, attr_name_upper: &str) -> Option<String> {
    let tag_end = tag_slice.find('>')?;
    let head = &tag_slice[..tag_end];
    let head_upper = head.to_ascii_uppercase();
    let pattern = format!("{attr_name_upper}=");
    let pos = head_upper.find(&pattern)?;
    let after = head[pos + pattern.len()..].trim_start();

    if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        Some(rest[..end].to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after.len());
        Some(after[..end].to_string())
    }
}

fn extract_tag_inner_text(slice: &str, tag_upper: &str) -> Option<String> {
    let open_end = slice.find('>')?;
    let rest = &slice[open_end + 1..];
    let rest_upper = rest.to_ascii_uppercase();
    let close_pat = format!("</{tag_upper}>");
    let close_pos = rest_upper.find(&close_pat)?;
    Some(rest[..close_pos].to_string())
}

fn decode_html_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_netscape_html_bookmarks() {
        let sample = r#"
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1700000000">Barra de Favoritos</H3>
    <DL><p>
        <DT><A HREF="https://github.com/yanmamede10" ADD_DATE="1700000001">GitHub &amp; Code</A>
        <DT><H3>Rust</H3>
        <DL><p>
            <DT><A HREF="https://docs.rs/">docs.rs</A>
            <DT><A HREF="javascript:alert(1)">Ignorar XSS</A>
        </DL><p>
    </DL><p>
    <DT><A HREF="https://www.youtube.com/">YouTube</A>
</DL><p>
"#;
        let parsed = parse_netscape_bookmarks_html(sample);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].title, "GitHub & Code");
        assert_eq!(parsed[0].url, "https://github.com/yanmamede10");
        assert_eq!(parsed[0].folder_path, vec!["Barra de Favoritos"]);

        assert_eq!(parsed[1].title, "docs.rs");
        assert_eq!(parsed[1].folder_path, vec!["Barra de Favoritos", "Rust"]);

        assert_eq!(parsed[2].title, "YouTube");
        assert!(parsed[2].folder_path.is_empty());
    }
}
