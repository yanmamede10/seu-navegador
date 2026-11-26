//! WCAG 2.1 relative luminance and contrast ratio calculator for semantic theme tokens.

use crate::SemanticTokens;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContrastCheck {
    pub pair: String,
    pub fg: String,
    pub bg: String,
    pub ratio: f64,
    pub aa_normal: bool,
    pub aa_large: bool,
    pub aaa_normal: bool,
}

/// Parses `#RGB` or `#RRGGBB` into `(r, g, b)` in `0..=255`.
pub fn parse_hex_color(raw: &str) -> Option<(u8, u8, u8)> {
    let hex = raw.trim().strip_prefix('#')?;
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some((r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b))
        }
        _ => None,
    }
}

fn srgb_channel_linear(c: u8) -> f64 {
    let s = f64::from(c) / 255.0;
    if s <= 0.03928 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

pub fn relative_luminance(rgb: (u8, u8, u8)) -> f64 {
    0.2126 * srgb_channel_linear(rgb.0)
        + 0.7152 * srgb_channel_linear(rgb.1)
        + 0.0722 * srgb_channel_linear(rgb.2)
}

pub fn contrast_ratio_hex(fg: &str, bg: &str) -> Option<f64> {
    let l1 = relative_luminance(parse_hex_color(fg)?);
    let l2 = relative_luminance(parse_hex_color(bg)?);
    let (bright, dark) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    let ratio = (bright + 0.05) / (dark + 0.05);
    Some((ratio * 100.0).round() / 100.0)
}

pub fn evaluate_semantic_contrast(s: &SemanticTokens) -> Vec<ContrastCheck> {
    let pairs = [
        ("text / bg", &s.text, &s.bg),
        ("text / surface", &s.text, &s.surface),
        ("text_muted / surface", &s.text_muted, &s.surface),
        ("accent_text / accent", &s.accent_text, &s.accent),
        ("text / surface2", &s.text, &s.surface2),
    ];

    pairs
        .into_iter()
        .map(|(label, fg, bg)| {
            let ratio = contrast_ratio_hex(fg, bg).unwrap_or(1.0);
            ContrastCheck {
                pair: label.to_string(),
                fg: fg.clone(),
                bg: bg.clone(),
                ratio,
                aa_normal: ratio >= 4.5,
                aa_large: ratio >= 3.0,
                aaa_normal: ratio >= 7.0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_exact_black_white_and_low_contrast_ratios() {
        assert_eq!(contrast_ratio_hex("#FFFFFF", "#000000"), Some(21.0));
        assert_eq!(contrast_ratio_hex("#000", "#fff"), Some(21.0));
        let low = contrast_ratio_hex("#777777", "#888888").unwrap_or(0.0);
        assert!(low < 2.0);
    }
}
