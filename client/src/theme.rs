//! Color themes. Every color the app draws comes from the active `Palette`.
//!
//! The app picks its own theme and never follows the Windows light/dark setting:
//! mixing the system's text colors with our backgrounds is what produced dark
//! text on dark backgrounds on PCs set to light mode.
//!
//! All palettes are checked for readable contrast (4.5:1 for secondary text,
//! 7:1 for body text) in the tests at the bottom.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, Theme, ThemePreference};
use std::cell::Cell;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub id: &'static str,
    pub label: &'static str,
    pub dark: bool,
    /// Sidebar.
    pub bg_deep: Color32,
    /// Chat area and windows.
    pub bg: Color32,
    /// Message box, hover.
    pub raised: Color32,
    /// Selected rows, buttons.
    pub raised_2: Color32,
    pub line: Color32,
    /// The "you" panel at the bottom left.
    pub me_bg: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    /// Speaking glow, links, primary buttons.
    pub accent: Color32,
    /// Text on top of the accent color.
    pub accent_ink: Color32,
    pub red: Color32,
    /// Online and "connected" indicators.
    pub teal: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const THEMES: [Palette; 6] = [
    Palette {
        id: "plum",
        label: "Plum",
        dark: true,
        bg_deep: hex(0x17111e),
        bg: hex(0x1f1828),
        raised: hex(0x2a2135),
        raised_2: hex(0x342942),
        line: hex(0x3a2e47),
        me_bg: hex(0x140f1a),
        text: hex(0xefe8f4),
        muted: hex(0xa99cb7),
        faint: hex(0x968aa5),
        accent: hex(0xf4b860),
        accent_ink: hex(0x2b1b06),
        red: hex(0xf07a88),
        teal: hex(0x74d1b0),
    },
    Palette {
        id: "midnight",
        label: "Midnight",
        dark: true,
        bg_deep: hex(0x0f1622),
        bg: hex(0x151d2b),
        raised: hex(0x1d2738),
        raised_2: hex(0x263247),
        line: hex(0x2c3a52),
        me_bg: hex(0x0c121c),
        text: hex(0xe6edf7),
        muted: hex(0xa3b3ca),
        faint: hex(0x8798b1),
        accent: hex(0x6cb8ff),
        accent_ink: hex(0x08223d),
        red: hex(0xff8590),
        teal: hex(0x5fd6a8),
    },
    Palette {
        id: "light",
        label: "Light",
        dark: false,
        bg_deep: hex(0xeef0f4),
        bg: hex(0xffffff),
        raised: hex(0xf2f3f6),
        raised_2: hex(0xe4e6ec),
        line: hex(0xd6d9e0),
        me_bg: hex(0xe6e8ee),
        text: hex(0x1b1d23),
        muted: hex(0x4a505c),
        faint: hex(0x5f6573),
        accent: hex(0x5243c9),
        accent_ink: hex(0xffffff),
        red: hex(0xb8283b),
        teal: hex(0x0f7353),
    },
    Palette {
        id: "cream",
        label: "Cream",
        dark: false,
        bg_deep: hex(0xefe6d4),
        bg: hex(0xfaf5ea),
        raised: hex(0xf2ead9),
        raised_2: hex(0xe8dcc4),
        line: hex(0xdccdb0),
        me_bg: hex(0xe9dec8),
        text: hex(0x2e2418),
        muted: hex(0x57493a),
        faint: hex(0x6b5c4a),
        accent: hex(0x9e4520),
        accent_ink: hex(0xfff8ef),
        red: hex(0xa32019),
        teal: hex(0x2a6e4c),
    },
    Palette {
        id: "pink",
        label: "Pastel pink",
        dark: false,
        bg_deep: hex(0xf9dbe6),
        bg: hex(0xfff0f5),
        raised: hex(0xfde4ee),
        raised_2: hex(0xf7cfdf),
        line: hex(0xefbfd2),
        me_bg: hex(0xf5d0de),
        text: hex(0x3d1830),
        muted: hex(0x66344f),
        faint: hex(0x7a4664),
        accent: hex(0xad1f5e),
        accent_ink: hex(0xffffff),
        red: hex(0x9c1a26),
        teal: hex(0x1d6b51),
    },
    Palette {
        id: "mint",
        label: "Mint",
        dark: false,
        bg_deep: hex(0xd9f1e6),
        bg: hex(0xeefaf4),
        raised: hex(0xe0f4ea),
        raised_2: hex(0xcdebdc),
        line: hex(0xb9dfcc),
        me_bg: hex(0xd1ecdf),
        text: hex(0x13322a),
        muted: hex(0x355a4d),
        faint: hex(0x466b5e),
        accent: hex(0x0b6e55),
        accent_ink: hex(0xffffff),
        red: hex(0xa3202a),
        teal: hex(0x17703f),
    },
];

thread_local! {
    static CURRENT: Cell<Palette> = const { Cell::new(THEMES[0]) };
}

/// The active palette (UI thread).
pub fn pal() -> Palette {
    CURRENT.with(|c| c.get())
}

pub fn by_id(id: &str) -> Palette {
    THEMES
        .iter()
        .copied()
        .find(|t| t.id == id)
        .unwrap_or(THEMES[0])
}

pub fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Mix two colors; t = 0 gives `a`, t = 1 gives `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Switch the whole app to a theme.
pub fn apply(ctx: &egui::Context, id: &str) {
    let p = by_id(id);
    CURRENT.with(|c| c.set(p));

    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = if p.dark {
        p.bg_deep
    } else {
        mix(p.bg, p.text, 0.035)
    };
    v.text_edit_bg_color = Some(v.extreme_bg_color);
    v.faint_bg_color = p.raised;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = with_alpha(p.accent, if p.dark { 90 } else { 70 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_stroke = Stroke::new(1.0, p.line);
    v.window_corner_radius = CornerRadius::same(14);
    let shadow_alpha = if p.dark { 110 } else { 40 };
    v.window_shadow = egui::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.faint);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.muted);
    for (st, fill) in [
        (&mut w.inactive, p.raised_2),
        (&mut w.hovered, p.line),
        (&mut w.active, p.line),
        (&mut w.open, p.raised_2),
    ] {
        st.weak_bg_fill = fill;
        st.bg_fill = fill;
        st.corner_radius = CornerRadius::same(5);
        st.bg_stroke = Stroke::NONE;
        st.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.hovered.bg_stroke = Stroke::new(1.0, mix(p.line, p.text, 0.25));
    // Checkbox and radio fill when not hovered.
    w.inactive.bg_fill = if p.dark {
        p.raised_2
    } else {
        mix(p.raised_2, p.text, 0.08)
    };

    // Use our visuals no matter what Windows' light/dark setting says.
    ctx.set_theme(if p.dark {
        ThemePreference::Dark
    } else {
        ThemePreference::Light
    });
    ctx.set_visuals_of(if p.dark { Theme::Dark } else { Theme::Light }, v);

    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(15.0));
        s.text_styles.insert(Button, FontId::proportional(15.0));
        s.text_styles.insert(Small, FontId::proportional(12.0));
        s.text_styles.insert(Heading, FontId::proportional(21.0));
        s.text_styles.insert(Monospace, FontId::monospace(14.0));
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 220.0;
        s.visuals.slider_trailing_fill = true;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color32) -> f64 {
        let f = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    fn ratio(a: Color32, b: Color32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_theme_is_readable() {
        for p in THEMES {
            let backgrounds = [p.bg, p.bg_deep, p.raised, p.raised_2, p.me_bg];
            for bg in backgrounds {
                assert!(ratio(p.text, bg) >= 7.0, "{}: body text", p.id);
                assert!(ratio(p.muted, bg) >= 4.5, "{}: muted text", p.id);
            }
            for bg in [p.bg, p.bg_deep, p.me_bg, p.raised] {
                assert!(ratio(p.faint, bg) >= 4.5, "{}: faint text", p.id);
            }
            for bg in [p.bg, p.bg_deep, p.raised_2] {
                assert!(ratio(p.accent, bg) >= 4.5, "{}: accent text", p.id);
            }
            for bg in [p.bg, p.bg_deep, p.me_bg, p.raised_2] {
                assert!(ratio(p.red, bg) >= 4.5, "{}: red text", p.id);
            }
            assert!(
                ratio(p.accent_ink, p.accent) >= 4.5,
                "{}: button text",
                p.id
            );
            assert!(ratio(p.teal, p.me_bg) >= 4.5, "{}: status text", p.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in THEMES.iter().enumerate() {
            assert!(THEMES[i + 1..].iter().all(|b| b.id != a.id));
        }
    }
}
