//! Module colors: a fixed palette of eleven hues, each with a light-theme and a dark-theme
//! value, and a stable assignment from module ids to palette entries.
//!
//! Every light value has at least 5:1 contrast on `surface` (#ffffff) and `bg` (#f3f4f7);
//! every dark value has at least 6.5:1 on the dark `surface` (#191e26) and `bg` (#11151b).
//! Hues are 25 to 60 degrees apart in OKLCH, and every pair differs by more than 0.05 in
//! OKLab. Color never carries meaning alone: the module name always goes with it.

use std::collections::{HashMap, HashSet};

/// One palette entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleColor {
    pub key: &'static str,
    /// CSS color for the light theme.
    pub light: &'static str,
    /// CSS color for the dark theme.
    pub dark: &'static str,
}

const fn c(key: &'static str, light: &'static str, dark: &'static str) -> ModuleColor {
    ModuleColor { key, light, dark }
}

/// The palette, in hue order.
pub const PALETTE: [ModuleColor; 11] = [
    c("red", "#b4413e", "#e88780"),
    c("orange", "#a94e00", "#e28f5e"),
    c("amber", "#896105", "#ca9a41"),
    c("olive", "#656d04", "#9ea94a"),
    c("green", "#137925", "#6cb26f"),
    c("teal", "#057662", "#2eb599"),
    c("sky", "#05709c", "#47abe0"),
    c("blue", "#3f64c0", "#7ea1ee"),
    c("indigo", "#6d58bb", "#a497ea"),
    c("purple", "#924ba2", "#c78dd5"),
    c("pink", "#aa4179", "#e087b2"),
];

/// Probe stride through the palette. Coprime with 11, so every entry is reached, and
/// consecutive probes land five hues apart.
const STRIDE: usize = 5;

/// The palette entry with this light (or dark) value, if any.
pub fn by_value(css: &str) -> Option<&'static ModuleColor> {
    let v = css.trim().to_ascii_lowercase();
    PALETTE.iter().find(|p| p.light == v || p.dark == v)
}

/// The palette entry with this key (`teal`), if any.
pub fn by_key(key: &str) -> Option<&'static ModuleColor> {
    PALETTE.iter().find(|p| p.key == key)
}

/// Assigns a light-theme color to every module id.
///
/// `existing` holds colors already shown to the user (stored module colors, user picks).
/// They are kept as they are, even when they are not palette colors. New ids are placed in
/// sorted order, each starting from a slot derived from a hash of its id and probing for a
/// palette entry no other module uses yet. When all eleven are taken, the hashed slot is
/// reused. The result depends only on the ids and `existing`, never on input order.
pub fn assign_colors(
    ids: &[String],
    existing: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut used: HashSet<&'static str> = HashSet::new();
    for id in ids {
        if let Some(color) = existing.get(id) {
            if let Some(p) = by_value(color) {
                used.insert(p.light);
            }
            out.insert(id.clone(), color.clone());
        }
    }
    let mut fresh: Vec<&String> = ids.iter().filter(|id| !out.contains_key(*id)).collect();
    fresh.sort();
    fresh.dedup();
    for id in fresh {
        let start = (iso_core::hash64(id.as_bytes()) % PALETTE.len() as u64) as usize;
        let slot = (0..PALETTE.len())
            .map(|i| (start + i * STRIDE) % PALETTE.len())
            .find(|&s| !used.contains(PALETTE[s].light))
            .unwrap_or(start);
        used.insert(PALETTE[slot].light);
        out.insert(id.clone(), PALETTE[slot].light.to_string());
    }
    out
}

/// The dark-theme value for a light palette color; other colors pass through unchanged.
pub fn dark_variant(light: &str) -> String {
    by_value(light)
        .map(|p| p.dark.to_string())
        .unwrap_or_else(|| light.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: &str) -> [f64; 3] {
        let v = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.0;
        [v(1), v(3), v(5)]
    }
    fn lin(c: f64) -> f64 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance(hex: &str) -> f64 {
        let [r, g, b] = rgb(hex);
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
    fn oklab(hex: &str) -> [f64; 3] {
        let [r, g, b] = rgb(hex).map(lin);
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        [
            0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        ]
    }
    fn delta(a: &str, b: &str) -> f64 {
        let (x, y) = (oklab(a), oklab(b));
        ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
    }

    #[test]
    fn readable_on_both_themes() {
        for p in PALETTE {
            for bg in ["#ffffff", "#f3f4f7"] {
                assert!(contrast(p.light, bg) >= 5.0, "{} on {bg}", p.light);
            }
            for bg in ["#191e26", "#11151b"] {
                assert!(contrast(p.dark, bg) >= 6.5, "{} on {bg}", p.dark);
            }
        }
    }

    #[test]
    fn distinct() {
        let keys: HashSet<_> = PALETTE.iter().map(|p| p.key).collect();
        assert_eq!(keys.len(), PALETTE.len());
        for (i, a) in PALETTE.iter().enumerate() {
            for b in &PALETTE[i + 1..] {
                assert!(delta(a.light, b.light) > 0.05, "{} vs {}", a.key, b.key);
                assert!(delta(a.dark, b.dark) > 0.05, "{} vs {}", a.key, b.key);
            }
        }
    }

    #[test]
    fn stable_assignment() {
        let ids: Vec<String> = ["src/ui", "src/engine", "src/index", "lib"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = assign_colors(&ids, &HashMap::new());
        let mut rev = ids.clone();
        rev.reverse();
        assert_eq!(a, assign_colors(&rev, &HashMap::new()));
        let distinct: HashSet<_> = a.values().collect();
        assert_eq!(distinct.len(), ids.len());

        // Known colors stay; new modules avoid them.
        let mut existing = HashMap::new();
        existing.insert("src/ui".to_string(), "#123456".to_string());
        existing.insert("src/engine".to_string(), a["src/index"].clone());
        let b = assign_colors(&ids, &existing);
        assert_eq!(b["src/ui"], "#123456");
        assert_eq!(b["src/engine"], a["src/index"]);
        assert_ne!(b["src/index"], a["src/index"]);
        assert_eq!(
            dark_variant(&b["src/engine"]),
            by_value(&b["src/engine"]).unwrap().dark
        );
        assert_eq!(dark_variant("#123456"), "#123456");

        // More modules than colors still get a color each.
        let many: Vec<String> = (0..30).map(|i| format!("m{i}")).collect();
        let c = assign_colors(&many, &HashMap::new());
        assert_eq!(c.len(), 30);
        let used: HashSet<_> = c.values().collect();
        assert_eq!(used.len(), PALETTE.len());
    }
}
