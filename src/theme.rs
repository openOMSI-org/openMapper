//! The interface's colours, as names of what they are for (a panel, dim text, a canvas's
//! grid) rather than numbers, in a dark and a light set. The interface asks `pal()` for the set
//! of the theme on screen; egui picks that theme itself (the user's choice, or the system's).

use std::sync::atomic::{AtomicBool, Ordering};

use egui::Color32;

use crate::i18n::tr;

/// The colours of one theme.
pub struct Palette {
    pub light: bool,
    /// Side panels, the toolbar and the inside of windows.
    pub panel: Color32,
    /// Floating windows (a step off the panels so they stand out).
    pub window: Color32,
    /// The menu bar and the status bar.
    pub bar: Color32,
    /// Section headers and the head of a card.
    pub header: Color32,
    /// Text fields, lists and other wells.
    pub field: Color32,
    /// Every other row of a list in a field.
    pub row_alt: Color32,
    /// Behind the welcome card and other empty areas.
    pub backdrop: Color32,
    /// Edges of panels, windows and wells.
    pub border: Color32,
    /// Thin lines between things inside a panel.
    pub separator: Color32,
    /// egui's own buttons: at rest, under the mouse, held.
    pub widget: Color32,
    pub widget_hover: Color32,
    pub widget_active: Color32,
    pub widget_stroke: Color32,
    pub widget_stroke_hover: Color32,
    /// egui's plain buttons (their `weak_bg_fill`): at rest and under the mouse.
    pub button: Color32,
    pub button_hover: Color32,
    pub text: Color32,
    /// Names and titles.
    pub text_strong: Color32,
    /// Help lines, units, second rows.
    pub text_dim: Color32,
    /// What cannot be used now.
    pub text_off: Color32,
    pub accent: Color32,
    /// What is chosen in a list.
    pub selection: Color32,
    pub selection_text: Color32,
    /// A custom-drawn row or button under the mouse.
    pub hover: Color32,
    /// A custom-drawn button that is on, and its edge.
    pub pressed: Color32,
    pub pressed_stroke: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub info: Color32,
    /// The 2D editors' drawing area (crossing, complex junction, profile, UV, marks...).
    pub canvas: Color32,
    pub grid: Color32,
    pub grid_major: Color32,
    /// Outlines and plain lines drawn on a canvas.
    pub ink: Color32,
    pub ink_dim: Color32,
    /// The ground under a road's height profile, and its edge.
    pub earth: Color32,
    pub earth_line: Color32,
    /// The place of a catalogue picture not made yet.
    pub thumb: Color32,
    /// The 2D drawings of the map (crossing paths, complex junctions) stay dark in both themes,
    /// as the 3D view does, so lanes and the colours of roads read the same on them: their
    /// ground and the text written on it.
    pub view: Color32,
    pub view_text: Color32,
    pub view_text_dim: Color32,
    /// Behind controls that float over the 3D view (the overview map, the 2D / 3D switch).
    pub overlay: Color32,
    pub shadow: Color32,
}

pub const DARK: Palette = Palette {
    light: false,
    panel: Color32::from_rgb(36, 37, 40),
    window: Color32::from_rgb(36, 37, 40),
    bar: Color32::from_rgb(27, 28, 30),
    header: Color32::from_rgb(45, 46, 50),
    field: Color32::from_rgb(22, 23, 25),
    row_alt: Color32::from_rgb(28, 29, 32),
    backdrop: Color32::from_rgb(30, 31, 34),
    border: Color32::from_rgb(18, 18, 20),
    separator: Color32::from_rgb(24, 24, 27),
    widget: Color32::from_rgb(56, 58, 63),
    widget_hover: Color32::from_rgb(70, 72, 78),
    widget_active: Color32::from_rgb(78, 80, 88),
    widget_stroke: Color32::from_rgb(24, 24, 27),
    widget_stroke_hover: Color32::from_rgb(90, 92, 100),
    button: Color32::from_rgb(52, 54, 59),
    button_hover: Color32::from_rgb(66, 68, 74),
    text: Color32::from_gray(205),
    text_strong: Color32::from_gray(240),
    text_dim: Color32::from_rgb(140, 143, 150),
    text_off: Color32::from_gray(90),
    accent: Color32::from_rgb(232, 122, 50),
    selection: Color32::from_rgb(64, 86, 122),
    selection_text: Color32::WHITE,
    hover: Color32::from_gray(44),
    pressed: Color32::from_rgb(70, 74, 82),
    pressed_stroke: Color32::from_rgb(98, 102, 112),
    ok: Color32::from_rgb(130, 200, 120),
    warn: Color32::from_rgb(230, 165, 90),
    error: Color32::from_rgb(240, 110, 95),
    info: Color32::from_rgb(120, 170, 230),
    canvas: Color32::from_rgb(24, 25, 28),
    grid: Color32::from_gray(38),
    grid_major: Color32::from_gray(56),
    ink: Color32::from_gray(200),
    ink_dim: Color32::from_gray(110),
    earth: Color32::from_rgb(70, 60, 45),
    earth_line: Color32::from_rgb(130, 110, 80),
    thumb: Color32::from_gray(40),
    view: Color32::from_rgb(32, 36, 33),
    view_text: Color32::from_gray(200),
    view_text_dim: Color32::from_gray(140),
    overlay: Color32::from_rgba_premultiplied(17, 18, 21, 220),
    shadow: Color32::from_black_alpha(110),
};

/// Calm light greys with white wells; the accent a shade deeper so it still reads as text
/// on them, the dim text dark enough for small type.
pub const LIGHT: Palette = Palette {
    light: true,
    panel: Color32::from_rgb(238, 239, 242),
    window: Color32::from_rgb(246, 246, 248),
    bar: Color32::from_rgb(226, 228, 232),
    header: Color32::from_rgb(222, 224, 229),
    field: Color32::WHITE,
    row_alt: Color32::from_rgb(245, 246, 249),
    backdrop: Color32::from_rgb(214, 217, 223),
    border: Color32::from_rgb(190, 193, 200),
    separator: Color32::from_rgb(212, 214, 220),
    widget: Color32::from_rgb(251, 251, 252),
    widget_hover: Color32::from_rgb(236, 238, 243),
    widget_active: Color32::from_rgb(220, 224, 232),
    widget_stroke: Color32::from_rgb(192, 195, 202),
    widget_stroke_hover: Color32::from_rgb(150, 155, 165),
    button: Color32::from_rgb(251, 251, 252),
    button_hover: Color32::from_rgb(236, 238, 243),
    text: Color32::from_rgb(38, 40, 45),
    text_strong: Color32::from_rgb(12, 13, 16),
    text_dim: Color32::from_rgb(96, 100, 108),
    text_off: Color32::from_rgb(160, 163, 170),
    accent: Color32::from_rgb(212, 100, 26),
    selection: Color32::from_rgb(196, 216, 244),
    selection_text: Color32::from_rgb(14, 30, 60),
    hover: Color32::from_rgb(228, 231, 237),
    pressed: Color32::from_rgb(214, 219, 228),
    pressed_stroke: Color32::from_rgb(160, 166, 178),
    ok: Color32::from_rgb(34, 130, 52),
    warn: Color32::from_rgb(176, 98, 0),
    error: Color32::from_rgb(196, 44, 32),
    info: Color32::from_rgb(30, 100, 190),
    canvas: Color32::from_rgb(250, 250, 251),
    grid: Color32::from_rgb(230, 232, 236),
    grid_major: Color32::from_rgb(208, 211, 218),
    ink: Color32::from_rgb(50, 53, 60),
    ink_dim: Color32::from_rgb(150, 154, 162),
    earth: Color32::from_rgb(226, 214, 192),
    earth_line: Color32::from_rgb(168, 140, 100),
    thumb: Color32::from_rgb(225, 227, 232),
    view: Color32::from_rgb(44, 49, 45),
    view_text: Color32::from_gray(215),
    view_text_dim: Color32::from_gray(160),
    overlay: Color32::from_rgba_premultiplied(214, 215, 218, 235),
    shadow: Color32::from_black_alpha(40),
};

impl Palette {
    /// The fill of a row of a list drawn by hand: chosen, under the mouse, every other one.
    pub fn row_fill(&self, on: bool, hovered: bool, odd: bool) -> Color32 {
        if on {
            self.selection
        } else if hovered {
            self.hover
        } else if odd {
            self.row_alt
        } else {
            Color32::TRANSPARENT
        }
    }

    /// The name on such a row.
    pub fn row_text(&self, on: bool) -> Color32 {
        if on {
            self.selection_text
        } else {
            self.text_strong
        }
    }
}

/// A colour that means something on the map (a lane, a kind of road) made dark enough to read
/// as text or a thin line on the light panels; the dark theme takes it as it is.
pub fn readable(c: Color32) -> Color32 {
    if !pal().light {
        return c;
    }
    let lum = |c: Color32| 0.2126 * (c.r() as f32 / 255.0).powf(2.2) + 0.7152 * (c.g() as f32 / 255.0).powf(2.2) + 0.0722 * (c.b() as f32 / 255.0).powf(2.2);
    // (a contrast of 3.5 or so with the panel: luminance under 0.2)
    let l = lum(c);
    if l <= 0.2 {
        return c;
    }
    let k = (0.2 / l).powf(1.0 / 2.2);
    Color32::from_rgba_unmultiplied((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8, c.a())
}

static LIGHT_ON: AtomicBool = AtomicBool::new(false);

/// The colours of the theme on screen.
pub fn pal() -> &'static Palette {
    if LIGHT_ON.load(Ordering::Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

/// Follow the theme egui shows this frame (the system's may have changed under it).
pub fn sync(ctx: &egui::Context) {
    LIGHT_ON.store(ctx.theme() == egui::Theme::Light, Ordering::Relaxed);
}

/// The theme the user chose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Dark,
    Light,
    System,
}

impl Choice {
    pub const ALL: [Choice; 3] = [Choice::Dark, Choice::Light, Choice::System];

    /// As the settings file keeps it (empty: dark, as openMapper always was).
    pub fn from_code(s: &str) -> Choice {
        match s {
            "light" => Choice::Light,
            "system" => Choice::System,
            _ => Choice::Dark,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Choice::Dark => "dark",
            Choice::Light => "light",
            Choice::System => "system",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Choice::Dark => tr("Dark", "Тёмная"),
            Choice::Light => tr("Light", "Светлая"),
            Choice::System => tr("As the system", "Как в системе"),
        }
    }

    /// What the interface is set to now.
    pub fn current(ctx: &egui::Context) -> Choice {
        match ctx.options(|o| o.theme_preference) {
            egui::ThemePreference::Dark => Choice::Dark,
            egui::ThemePreference::Light => Choice::Light,
            egui::ThemePreference::System => Choice::System,
        }
    }

    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Choice::Dark => egui::ThemePreference::Dark,
            Choice::Light => egui::ThemePreference::Light,
            Choice::System => egui::ThemePreference::System,
        });
        sync(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG's contrast ratio of two opaque colours.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_reads_on_every_surface() {
        for p in [&DARK, &LIGHT] {
            for bg in [p.panel, p.window, p.bar, p.header, p.field, p.row_alt, p.hover] {
                assert!(contrast(p.text, bg) >= 7.0, "text on {bg:?} (light: {})", p.light);
                assert!(contrast(p.text_dim, bg) >= 4.0, "dim text on {bg:?} (light: {})", p.light);
            }
            assert!(contrast(p.selection_text, p.selection) >= 7.0);
            assert!(contrast(p.accent, p.panel) >= 3.0, "accent (light: {})", p.light);
            for c in [p.ok, p.warn, p.error, p.info] {
                assert!(contrast(c, p.panel) >= 3.0, "{c:?} (light: {})", p.light);
            }
            assert!(contrast(p.ink, p.canvas) >= 7.0);
            assert!(contrast(p.view_text, p.view) >= 7.0);
            assert!(contrast(p.view_text_dim, p.view) >= 4.5);
        }
    }

    #[test]
    fn choice_round_trips() {
        for c in Choice::ALL {
            assert_eq!(Choice::from_code(c.code()), c);
        }
        assert_eq!(Choice::from_code(""), Choice::Dark);
    }
}
