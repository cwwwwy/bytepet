//! CJK font fallback.
//!
//! egui's bundled fonts cover Latin and Cyrillic, but not Chinese. Add the
//! platform's best available CJK font as a fallback for both proportional and
//! monospace text.

use std::sync::Arc;

pub fn install_cjk_font(ctx: &egui::Context) {
    let Some((bytes, index)) = load_cjk_font() else {
        tracing::warn!("no CJK font found; Chinese text may render as boxes");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    let mut data = egui::FontData::from_owned(bytes);
    data.index = index;
    fonts.font_data.insert("cjk".to_string(), Arc::new(data));

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("cjk".to_string());
    }
    ctx.set_fonts(fonts);
}

fn load_cjk_font() -> Option<(Vec<u8>, u32)> {
    if let Some(path) = std::env::var_os("BYTEPET_FONT") {
        if let Some(font) = read_font(std::path::PathBuf::from(path)) {
            return Some((font, 0));
        }
    }

    #[cfg(target_os = "windows")]
    {
        const CANDIDATES: &[(&str, u32)] = &[
            ("C:\\Windows\\Fonts\\SourceHanSansCN.ttf", 0),
            ("C:\\Windows\\Fonts\\msyh.ttc", 0),
            ("C:\\Windows\\Fonts\\msyh.ttf", 0),
            ("C:\\Windows\\Fonts\\Deng.ttf", 0),
            ("C:\\Windows\\Fonts\\simhei.ttf", 0),
            ("C:\\Windows\\Fonts\\simsun.ttc", 0),
        ];
        for &(path, index) in CANDIDATES {
            if let Some(font) = read_font(path) {
                tracing::info!("loaded CJK font: {path}");
                return Some((font, index));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        const CANDIDATES: &[(&str, u32)] = &[
            ("/System/Library/Fonts/PingFang.ttc", 0),
            ("/System/Library/Fonts/STHeiti Light.ttc", 0),
            ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
        ];
        for &(path, index) in CANDIDATES {
            if let Some(font) = read_font(path) {
                tracing::info!("loaded CJK font: {path}");
                return Some((font, index));
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        const CANDIDATES: &[(&str, u32)] = &[
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
            ("/usr/share/fonts/truetype/arphic/uming.ttc", 0),
        ];
        for &(path, index) in CANDIDATES {
            if let Some(font) = read_font(path) {
                tracing::info!("loaded CJK font: {path}");
                return Some((font, index));
            }
        }
    }

    None
}

fn read_font(path: impl AsRef<std::path::Path>) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}
