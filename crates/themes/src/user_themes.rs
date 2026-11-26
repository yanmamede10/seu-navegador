//! User theme directory persistence, copy-on-write authoring, font import,
//! `.seutheme` ZIP packaging/import, and lightweight directory change fingerprinting.

use crate::sanitize::{validate_and_rewrite_css, validate_and_sanitize_html};
use crate::{
    tokens_to_css, validate_manifest, validate_tokens, LoadedTheme, ThemeEntrypoints, ThemeError,
    ThemeManifest, ThemePermissions, ThemeRegistry, ThemeTokens, BUILTIN_THEMES_DIR,
    DEFAULT_THEME_ID,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MAX_PACKAGE_FILES: usize = 200;
const MAX_PACKAGE_UNCOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FONT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveUserThemeInput {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub base_id: Option<String>,
    pub tokens: ThemeTokens,
    #[serde(default)]
    pub extra_shell_css: Option<String>,
    #[serde(default)]
    pub extra_newtab_css: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportedFontResult {
    pub family: String,
    pub filename: String,
    pub url: String,
    pub font_face_css: String,
}

/// Resolves the persistent directory for user-authored and imported themes.
pub fn default_user_themes_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("SeuNavegador").join("themes");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("seu-navegador").join("themes");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("seu-navegador")
            .join("themes");
    }
    std::env::temp_dir().join("seu-navegador-themes")
}

/// Validates a theme slug (`[a-z0-9-]+`, 2..=48 chars).
pub fn is_valid_theme_id(id: &str) -> bool {
    let len = id.len();
    (2..=48).contains(&len)
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Loads a single user theme folder from disk, enforcing full manifest, token,
/// HTML and CSS sanitization.
pub fn load_disk_theme_dir(dir: &Path) -> Result<LoadedTheme, ThemeError> {
    let read_rel = |rel: &str| -> Result<String, ThemeError> {
        let target = resolve_safe_rel(dir, rel)?;
        fs::read_to_string(&target).map_err(|_| ThemeError::MissingFile(rel.to_string()))
    };

    let manifest_raw = read_rel("manifest.json")?;
    let mut manifest: ThemeManifest =
        serde_json::from_str(&manifest_raw).map_err(|e| ThemeError::Json(e.to_string()))?;
    validate_manifest(&manifest)?;
    // User-imported themes cannot silently enable site scripts without explicit opt-in.
    manifest.permissions.site_scripts = false;

    let tokens_raw = read_rel(&manifest.entry.tokens)?;
    let tokens: ThemeTokens =
        serde_json::from_str(&tokens_raw).map_err(|e| ThemeError::Json(e.to_string()))?;
    validate_tokens(&tokens)?;

    let tokens_css = tokens_to_css(&tokens);
    let shell_css_raw = read_rel(&manifest.entry.shell_css)?;
    let shell_css = validate_and_rewrite_css(&shell_css_raw, &manifest.id)?;

    let shell_html = match &manifest.entry.shell_html {
        Some(path) => Some(validate_and_sanitize_html(&read_rel(path)?)?),
        None => None,
    };

    let newtab_html_raw = read_rel(&manifest.entry.newtab_html)?;
    let newtab_html = validate_and_sanitize_html(&newtab_html_raw)?;

    let newtab_css_raw = read_rel(&manifest.entry.newtab_css)?;
    let newtab_css = validate_and_rewrite_css(&newtab_css_raw, &manifest.id)?;

    let base_url = format!("seu://theme/{}/", manifest.id);

    Ok(LoadedTheme {
        manifest,
        tokens,
        tokens_css,
        shell_html,
        shell_css,
        newtab_html,
        newtab_css,
        base_url,
        builtin: false,
    })
}

/// Saves a copy-on-write user theme into `<user_dir>/<id>/`.
pub fn save_user_theme(
    registry: &ThemeRegistry,
    user_dir: &Path,
    input: SaveUserThemeInput,
) -> Result<LoadedTheme, ThemeError> {
    let id = input.id.trim().to_ascii_lowercase();
    if !is_valid_theme_id(&id) {
        return Err(ThemeError::InvalidId(id));
    }
    if registry.is_builtin(&id) {
        return Err(ThemeError::InvalidId(format!(
            "{id} (cannot overwrite builtin theme; pick a custom id)"
        )));
    }
    validate_tokens(&input.tokens)?;

    let base_id = input
        .base_id
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or(DEFAULT_THEME_ID);

    let dest = user_dir.join(&id);
    fs::create_dir_all(&dest).map_err(|e| ThemeError::Io(e.to_string()))?;

    // Copy base layout/CSS/assets if this is a brand new theme or missing layout files.
    let needs_seed = !dest.join("shell.html").exists() || !dest.join("newtab.html").exists();
    if needs_seed {
        seed_theme_files_from_base(registry, user_dir, base_id, &dest)?;
    }

    let manifest = ThemeManifest {
        id: id.clone(),
        name: if input.name.trim().is_empty() {
            id.clone()
        } else {
            input.name.trim().to_string()
        },
        version: "1.0.0".to_string(),
        author: if input.author.trim().is_empty() {
            "User".to_string()
        } else {
            input.author.trim().to_string()
        },
        description: if input.description.trim().is_empty() {
            format!("Tema personalizado baseado em {base_id}.")
        } else {
            input.description.trim().to_string()
        },
        base: Some(base_id.to_string()),
        entry: ThemeEntrypoints {
            tokens: "tokens.json".to_string(),
            shell_css: "shell.css".to_string(),
            shell_html: Some("shell.html".to_string()),
            newtab_html: "newtab.html".to_string(),
            newtab_css: "newtab.css".to_string(),
        },
        sites: BTreeMap::new(),
        permissions: ThemePermissions {
            site_scripts: false,
        },
        mobile: None,
    };
    validate_manifest(&manifest)?;

    // Apply optional extra CSS cleanly (replacing only the marked custom override block).
    if let Some(extra_shell) = input.extra_shell_css {
        validate_and_rewrite_css(&extra_shell, &id)?;
        update_custom_css_block(&dest.join("shell.css"), &extra_shell)?;
    }
    if let Some(extra_newtab) = input.extra_newtab_css {
        validate_and_rewrite_css(&extra_newtab, &id)?;
        update_custom_css_block(&dest.join("newtab.css"), &extra_newtab)?;
    }

    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| ThemeError::Json(e.to_string()))?;
    let tokens_json =
        serde_json::to_string_pretty(&input.tokens).map_err(|e| ThemeError::Json(e.to_string()))?;

    fs::write(dest.join("manifest.json"), manifest_json)
        .map_err(|e| ThemeError::Io(e.to_string()))?;
    fs::write(dest.join("tokens.json"), tokens_json).map_err(|e| ThemeError::Io(e.to_string()))?;

    load_disk_theme_dir(&dest)
}

/// Imports a `.woff2` or `.ttf` font file into `<user_dir>/<theme_id>/assets/fonts/`.
pub fn import_font_asset(
    user_dir: &Path,
    theme_id: &str,
    raw_filename: &str,
    bytes: &[u8],
) -> Result<ImportedFontResult, ThemeError> {
    if !is_valid_theme_id(theme_id) {
        return Err(ThemeError::InvalidId(theme_id.to_string()));
    }
    if bytes.is_empty() || bytes.len() > MAX_FONT_BYTES {
        return Err(ThemeError::Io(
            "font file empty or exceeds 2 MB".to_string(),
        ));
    }
    let clean_name = sanitize_font_filename(raw_filename)?;
    let lower = clean_name.to_ascii_lowercase();
    let format_str = if lower.ends_with(".woff2") {
        if bytes.len() < 4 || &bytes[0..4] != b"wOF2" {
            return Err(ThemeError::Io("invalid woff2 header".to_string()));
        }
        "woff2"
    } else if lower.ends_with(".ttf") {
        let valid_ttf = bytes.len() >= 4
            && (&bytes[0..4] == b"\x00\x01\x00\x00"
                || &bytes[0..4] == b"OTTO"
                || &bytes[0..4] == b"true");
        if !valid_ttf {
            return Err(ThemeError::Io("invalid ttf/otf header".to_string()));
        }
        "truetype"
    } else {
        return Err(ThemeError::Io(
            "only .woff2 and .ttf fonts are supported".to_string(),
        ));
    };

    let fonts_dir = user_dir.join(theme_id).join("assets").join("fonts");
    fs::create_dir_all(&fonts_dir).map_err(|e| ThemeError::Io(e.to_string()))?;
    fs::write(fonts_dir.join(&clean_name), bytes).map_err(|e| ThemeError::Io(e.to_string()))?;

    let stem = clean_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&clean_name);
    let family = format!("User-{stem}");
    let url = format!("seu://theme/{theme_id}/assets/fonts/{clean_name}");
    let font_face_css = format!(
        "@font-face {{\n  font-family: '{family}';\n  font-style: normal;\n  font-weight: 400;\n  font-display: swap;\n  src: url(./assets/fonts/{clean_name}) format('{format_str}');\n}}\n"
    );

    // Prepend @font-face into shell.css and newtab.css if those files exist.
    for css_file in ["shell.css", "newtab.css"] {
        let p = user_dir.join(theme_id).join(css_file);
        if let Ok(existing) = fs::read_to_string(&p) {
            if !existing.contains(&clean_name) {
                let _ = fs::write(&p, format!("{font_face_css}\n{existing}"));
            }
        }
    }

    Ok(ImportedFontResult {
        family,
        filename: clean_name,
        url,
        font_face_css,
    })
}

/// Exports any built-in or user theme as a `.seutheme` ZIP archive in memory.
pub fn export_theme_package(
    registry: &ThemeRegistry,
    user_dir: &Path,
    theme_id: &str,
) -> Result<Vec<u8>, ThemeError> {
    if !registry.exists(theme_id) {
        return Err(ThemeError::MissingFile(format!("theme '{theme_id}'")));
    }

    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        let disk_dir = user_dir.join(theme_id);
        if !registry.is_builtin(theme_id) && disk_dir.is_dir() {
            write_disk_dir_to_zip(&mut zip, &disk_dir, &disk_dir, opts)?;
        } else if let Some(dir) = BUILTIN_THEMES_DIR.get_dir(theme_id) {
            write_embedded_dir_to_zip(&mut zip, dir, theme_id, opts)?;
        } else {
            return Err(ThemeError::MissingFile(format!("theme dir '{theme_id}'")));
        }

        zip.finish().map_err(|e| ThemeError::Io(e.to_string()))?;
    }
    Ok(buf.into_inner())
}

/// Validates and extracts a `.seutheme` ZIP archive into `<user_dir>/<id>/`.
pub fn import_theme_package(
    registry: &ThemeRegistry,
    user_dir: &Path,
    zip_bytes: &[u8],
) -> Result<LoadedTheme, ThemeError> {
    let reader = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(reader).map_err(|e| ThemeError::Io(e.to_string()))?;
    if archive.is_empty() || archive.len() > MAX_PACKAGE_FILES {
        return Err(ThemeError::Io(
            "invalid .seutheme archive file count".to_string(),
        ));
    }

    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total_size: u64 = 0;

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| ThemeError::Io(e.to_string()))?;
        if file.is_dir() {
            continue;
        }
        let clean_rel = sanitize_zip_rel_path(file.name())?;
        total_size = total_size.saturating_add(file.size());
        if total_size > MAX_PACKAGE_UNCOMPRESSED_BYTES {
            return Err(ThemeError::Io(
                "archive exceeds 16 MB uncompressed limit".to_string(),
            ));
        }
        let mut data = Vec::new();
        file.take(MAX_PACKAGE_UNCOMPRESSED_BYTES)
            .read_to_end(&mut data)
            .map_err(|e| ThemeError::Io(e.to_string()))?;
        entries.insert(clean_rel, data);
    }

    // Support both flat zip (`manifest.json`) and single-root folder zip (`my-theme/manifest.json`).
    let normalized = strip_common_single_root(entries);

    let manifest_bytes = normalized
        .get("manifest.json")
        .ok_or_else(|| ThemeError::MissingFile("manifest.json".to_string()))?;
    let manifest_str =
        std::str::from_utf8(manifest_bytes).map_err(|e| ThemeError::Json(e.to_string()))?;
    let mut manifest: ThemeManifest =
        serde_json::from_str(manifest_str).map_err(|e| ThemeError::Json(e.to_string()))?;
    validate_manifest(&manifest)?;

    // Never allow an imported zip to clobber an official builtin theme.
    if registry.is_builtin(&manifest.id) {
        manifest.id = format!("{}-imported", manifest.id);
    }
    if !is_valid_theme_id(&manifest.id) {
        return Err(ThemeError::InvalidId(manifest.id));
    }
    manifest.permissions.site_scripts = false;

    // Validate tokens, HTML, and CSS BEFORE touching disk!
    let get_utf8 = |rel: &str| -> Result<&str, ThemeError> {
        let b = normalized
            .get(rel)
            .ok_or_else(|| ThemeError::MissingFile(rel.to_string()))?;
        std::str::from_utf8(b).map_err(|e| ThemeError::Io(e.to_string()))
    };

    let tokens: ThemeTokens = serde_json::from_str(get_utf8(&manifest.entry.tokens)?)
        .map_err(|e| ThemeError::Json(e.to_string()))?;
    validate_tokens(&tokens)?;
    validate_and_rewrite_css(get_utf8(&manifest.entry.shell_css)?, &manifest.id)?;
    if let Some(ref sh_html) = manifest.entry.shell_html {
        validate_and_sanitize_html(get_utf8(sh_html)?)?;
    }
    validate_and_sanitize_html(get_utf8(&manifest.entry.newtab_html)?)?;
    validate_and_rewrite_css(get_utf8(&manifest.entry.newtab_css)?, &manifest.id)?;

    let dest = user_dir.join(&manifest.id);
    if dest.exists() {
        let _ = fs::remove_dir_all(&dest);
    }
    fs::create_dir_all(&dest).map_err(|e| ThemeError::Io(e.to_string()))?;

    for (rel, data) in &normalized {
        let out_path = resolve_safe_rel(&dest, rel)?;
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| ThemeError::Io(e.to_string()))?;
        }
        fs::write(&out_path, data).map_err(|e| ThemeError::Io(e.to_string()))?;
    }

    // Persist sanitized manifest (with site_scripts=false and safe id).
    let manifest_out =
        serde_json::to_string_pretty(&manifest).map_err(|e| ThemeError::Json(e.to_string()))?;
    fs::write(dest.join("manifest.json"), manifest_out)
        .map_err(|e| ThemeError::Io(e.to_string()))?;

    load_disk_theme_dir(&dest)
}

/// Resolves a static asset inside `<user_dir>/<theme_id>/<rel_path>` safely.
pub fn resolve_user_theme_asset(
    user_dir: &Path,
    theme_id: &str,
    relative_path: &str,
) -> Option<Vec<u8>> {
    if !is_valid_theme_id(theme_id) {
        return None;
    }
    let root = user_dir.join(theme_id);
    if !root.is_dir() {
        return None;
    }
    let target = resolve_safe_rel(&root, relative_path).ok()?;
    let canon_root = fs::canonicalize(&root).ok()?;
    let canon_target = fs::canonicalize(&target).ok()?;
    if !canon_target.starts_with(&canon_root) || !canon_target.is_file() {
        return None;
    }
    fs::read(canon_target).ok()
}

/// Computes a fast recursive mtime+size fingerprint for hot-reload file watching.
pub fn compute_dir_fingerprint(user_dir: &Path) -> u64 {
    if !user_dir.is_dir() {
        return 0;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut stack = vec![user_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        let mut items: Vec<_> = rd.flatten().collect();
        items.sort_by_key(|e| e.file_name());
        for entry in items {
            let p = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(p);
                continue;
            }
            for b in p.to_string_lossy().as_bytes() {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            let secs = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            hash ^= secs ^ meta.len();
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

/// Minimal standard Base64 encoder (zero extra dependencies).
pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Minimal standard Base64 decoder (ignores ASCII whitespace).
pub fn base64_decode(input: &str) -> Result<Vec<u8>, ThemeError> {
    let mut vals = Vec::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            vals.push(64u8);
            continue;
        }
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(ThemeError::Io("invalid base64 character".to_string())),
        };
        vals.push(v);
    }
    if (vals.len() & 3) != 0 {
        return Err(ThemeError::Io("invalid base64 length".to_string()));
    }
    let mut out = Vec::with_capacity((vals.len() / 4) * 3);
    for chunk in vals.as_chunks::<4>().0 {
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk[1]);
        if a == 64 || b == 64 {
            return Err(ThemeError::Io("invalid base64 padding".to_string()));
        }
        out.push(((a << 2) | (b >> 4)) as u8);
        if chunk[2] != 64 {
            let c = u32::from(chunk[2]);
            out.push((((b & 0x0F) << 4) | (c >> 2)) as u8);
            if chunk[3] != 64 {
                let d = u32::from(chunk[3]);
                out.push((((c & 0x03) << 6) | d) as u8);
            }
        }
    }
    Ok(out)
}

fn seed_theme_files_from_base(
    registry: &ThemeRegistry,
    user_dir: &Path,
    base_id: &str,
    dest: &Path,
) -> Result<(), ThemeError> {
    let user_base = user_dir.join(base_id);
    if !registry.is_builtin(base_id) && user_base.is_dir() {
        copy_dir_recursive(&user_base, dest)?;
        return Ok(());
    }
    let embedded = BUILTIN_THEMES_DIR
        .get_dir(base_id)
        .or_else(|| BUILTIN_THEMES_DIR.get_dir(DEFAULT_THEME_ID))
        .ok_or_else(|| ThemeError::MissingFile(base_id.to_string()))?;
    copy_embedded_dir_to_disk(embedded, base_id, dest)
}

fn copy_embedded_dir_to_disk(
    dir: &include_dir::Dir<'_>,
    prefix_id: &str,
    dest: &Path,
) -> Result<(), ThemeError> {
    for file in dir.files() {
        let rel = file
            .path()
            .strip_prefix(prefix_id)
            .unwrap_or_else(|_| file.path());
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| ThemeError::Io(e.to_string()))?;
        }
        fs::write(&out, file.contents()).map_err(|e| ThemeError::Io(e.to_string()))?;
    }
    for sub in dir.dirs() {
        copy_embedded_dir_to_disk(sub, prefix_id, dest)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), ThemeError> {
    fs::create_dir_all(dest).map_err(|e| ThemeError::Io(e.to_string()))?;
    let rd = fs::read_dir(src).map_err(|e| ThemeError::Io(e.to_string()))?;
    for entry in rd.flatten() {
        let p = entry.path();
        let target = dest.join(entry.file_name());
        if p.is_dir() {
            copy_dir_recursive(&p, &target)?;
        } else {
            fs::copy(&p, &target).map_err(|e| ThemeError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

const CUSTOM_CSS_MARKER_START: &str = "/* === SEU-EDITOR-CUSTOM-START === */";
const CUSTOM_CSS_MARKER_END: &str = "/* === SEU-EDITOR-CUSTOM-END === */";

fn update_custom_css_block(path: &Path, extra_css: &str) -> Result<(), ThemeError> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let stripped = match (
        existing.find(CUSTOM_CSS_MARKER_START),
        existing.find(CUSTOM_CSS_MARKER_END),
    ) {
        (Some(s), Some(e)) if e >= s => {
            let after = e + CUSTOM_CSS_MARKER_END.len();
            format!("{}{}", existing[..s].trim_end(), &existing[after..])
        }
        _ => existing,
    };
    let trimmed = extra_css.trim();
    let next = if trimmed.is_empty() {
        stripped
    } else {
        format!(
            "{}\n\n{CUSTOM_CSS_MARKER_START}\n{trimmed}\n{CUSTOM_CSS_MARKER_END}\n",
            stripped.trim_end()
        )
    };
    fs::write(path, next).map_err(|e| ThemeError::Io(e.to_string()))
}

fn resolve_safe_rel(root: &Path, rel: &str) -> Result<PathBuf, ThemeError> {
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(ThemeError::UnsafeHtml("absolute path disallowed".into()));
    }
    let mut clean = root.to_path_buf();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => clean.push(seg),
            Component::CurDir => {}
            _ => return Err(ThemeError::UnsafeHtml("path traversal disallowed".into())),
        }
    }
    Ok(clean)
}

fn sanitize_zip_rel_path(raw: &str) -> Result<String, ThemeError> {
    let norm = raw.replace('\\', "/");
    let p = Path::new(&norm);
    if p.is_absolute() {
        return Err(ThemeError::Io("absolute path in archive".into()));
    }
    let mut parts = Vec::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => parts.push(seg.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return Err(ThemeError::Io("path traversal in archive".into())),
        }
    }
    if parts.is_empty() {
        return Err(ThemeError::Io("empty archive entry".into()));
    }
    Ok(parts.join("/"))
}

fn strip_common_single_root(entries: BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    if entries.contains_key("manifest.json") {
        return entries;
    }
    let first_prefix = entries
        .keys()
        .next()
        .and_then(|k| k.split_once('/').map(|(p, _)| format!("{p}/")));
    if let Some(prefix) = first_prefix {
        if entries.keys().all(|k| k.starts_with(&prefix)) {
            return entries
                .into_iter()
                .filter_map(|(k, v)| k.strip_prefix(&prefix).map(|rest| (rest.to_string(), v)))
                .collect();
        }
    }
    entries
}

fn sanitize_font_filename(raw: &str) -> Result<String, ThemeError> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    if cleaned.is_empty() || cleaned.contains("..") {
        return Err(ThemeError::Io("invalid font filename".into()));
    }
    Ok(cleaned)
}

fn write_disk_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    root: &Path,
    cur: &Path,
    opts: SimpleFileOptions,
) -> Result<(), ThemeError> {
    let rd = fs::read_dir(cur).map_err(|e| ThemeError::Io(e.to_string()))?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            write_disk_dir_to_zip(zip, root, &p, opts)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            let name = rel.to_string_lossy().replace('\\', "/");
            let bytes = fs::read(&p).map_err(|e| ThemeError::Io(e.to_string()))?;
            zip.start_file(name, opts)
                .map_err(|e| ThemeError::Io(e.to_string()))?;
            zip.write_all(&bytes)
                .map_err(|e| ThemeError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

fn write_embedded_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    dir: &include_dir::Dir<'_>,
    prefix_id: &str,
    opts: SimpleFileOptions,
) -> Result<(), ThemeError> {
    for file in dir.files() {
        let rel = file
            .path()
            .strip_prefix(prefix_id)
            .unwrap_or_else(|_| file.path());
        let name = rel.to_string_lossy().replace('\\', "/");
        zip.start_file(name, opts)
            .map_err(|e| ThemeError::Io(e.to_string()))?;
        zip.write_all(file.contents())
            .map_err(|e| ThemeError::Io(e.to_string()))?;
    }
    for sub in dir.dirs() {
        write_embedded_dir_to_zip(zip, sub, prefix_id, opts)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_exports_and_reimports_seutheme_package() {
        let tmp = std::env::temp_dir().join(format!(
            "seu-theme-test-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(1)
        ));
        let _ = fs::create_dir_all(&tmp);
        let mut reg = ThemeRegistry::load_with_user_dir(tmp.clone())
            .unwrap_or_else(|e| panic!("load failed: {e}"));

        let base = reg
            .get("y2k-maximal")
            .cloned()
            .unwrap_or_else(|| panic!("missing y2k-maximal"));
        let mut custom_tokens = base.tokens;
        custom_tokens.semantic.accent = "#00FFAA".to_string();

        let saved = reg
            .save_user_theme(SaveUserThemeInput {
                id: "my-neon-test".to_string(),
                name: "My Neon Test".to_string(),
                author: "Yan".to_string(),
                description: "Custom neon theme".to_string(),
                base_id: Some("y2k-maximal".to_string()),
                tokens: custom_tokens,
                extra_shell_css: Some(".y2k-shell { letter-spacing: 0.2px; }".to_string()),
                extra_newtab_css: None,
            })
            .unwrap_or_else(|e| panic!("save failed: {e}"));

        assert!(!saved.builtin);
        assert_eq!(saved.manifest.id, "my-neon-test");
        assert!(saved.tokens_css.contains("--seu-accent: #00FFAA;"));
        assert!(reg.exists("my-neon-test"));

        let zip_bytes = reg
            .export_package("my-neon-test")
            .unwrap_or_else(|e| panic!("export failed: {e}"));
        assert!(zip_bytes.len() > 200);

        let b64 = base64_encode(&zip_bytes);
        let decoded = base64_decode(&b64).unwrap_or_else(|e| panic!("b64 failed: {e}"));
        assert_eq!(decoded, zip_bytes);

        reg.delete_user_theme("my-neon-test")
            .unwrap_or_else(|e| panic!("delete failed: {e}"));
        assert!(!reg.exists("my-neon-test"));

        let reimported = reg
            .import_package(&zip_bytes)
            .unwrap_or_else(|e| panic!("reimport failed: {e}"));
        assert_eq!(reimported.manifest.id, "my-neon-test");
        assert!(reg.exists("my-neon-test"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
