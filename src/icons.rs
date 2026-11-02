//! Icons: Google's Material Symbols (Rounded), from a small font in the repository
//! (`assets/fonts/MaterialSymbolsSubset.ttf`, made by `tools/subset-icons.py` from the
//! full font kept beside it). Nothing is fetched at run time.

use egui::{FontData, FontDefinitions, FontFamily, RichText};

pub const FAMILY: &str = "material-symbols";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Select,
    Road,
    /// The complex junction editor's tools: a line, an arc, an arm; a saved object; a side view.
    Line,
    Arc,
    Arm,
    Cube,
    SideView,
    Up,
    Down,
    Objects,
    Terrain,
    Paint,
    Tiles,
    NewFile,
    Open,
    Save,
    Undo,
    Redo,
    Import,
    Settings,
    Help,
    Play,
    Raise,
    Lower,
    Smooth,
    Flatten,
    Add,
    Delete,
    Focus,
    Swap,
    Map,
    Bus,
    Info,
    Building,
    Tree,
    Junction,
    Tram,
    Walk,
    Fence,
    Layers,
    Brush,
    Eraser,
    Sensors,
    Parking,
    Route,
    Flag,
    Sound,
    Visible,
    Hidden,
    Traffic,
    CheckOn,
    CheckOff,
    RadioOn,
    RadioOff,
    ChevronRight,
    ExpandMore,
    Star,
    History,
    Schedule,
    Warning,
    Error,
    CheckCircle,
    Copy,
    Paste,
    SelectAll,
    Search,
    Move,
    Rotate,
    Scale,
    Code,
    Dropper,
}

impl Icon {
    pub fn ch(self) -> char {
        let cp = match self {
            Icon::Select => 0xf82f,
            Icon::Road => 0xf472,
            Icon::Line => 0xebbb,
            Icon::Arc => 0xf757,
            Icon::Arm => 0xe0b2,
            Icon::Cube => 0xf720,
            Icon::SideView => 0xea16,
            Icon::Up => 0xe5d8,
            Icon::Down => 0xe5db,
            Icon::Objects => 0xf030,
            Icon::Terrain => 0xe564,
            Icon::Paint => 0xe243,
            Icon::Tiles => 0xe3ec,
            Icon::NewFile => 0xe89c,
            Icon::Open => 0xe2c8,
            Icon::Save => 0xe161,
            Icon::Undo => 0xe166,
            Icon::Redo => 0xe15a,
            Icon::Import => 0xf090,
            Icon::Settings => 0xe8b8,
            Icon::Help => 0xe8fd,
            Icon::Play => 0xe037,
            Icon::Raise => 0xe5d8,
            Icon::Lower => 0xe5db,
            Icon::Smooth => 0xe176,
            Icon::Flatten => 0xf108,
            Icon::Add => 0xe145,
            Icon::Delete => 0xe92e,
            Icon::Focus => 0xe3b4,
            Icon::Swap => 0xe8d4,
            Icon::Map => 0xe55b,
            Icon::Bus => 0xeff6,
            Icon::Info => 0xe88e,
            Icon::Building => 0xea40,
            Icon::Tree => 0xea63,
            Icon::Junction => 0xf184,
            Icon::Tram => 0xe571,
            Icon::Walk => 0xe536,
            Icon::Fence => 0xf1f6,
            Icon::Layers => 0xe53b,
            Icon::Brush => 0xe3ae,
            Icon::Eraser => 0xe6d0,
            Icon::Sensors => 0xe51e,
            Icon::Parking => 0xe54f,
            Icon::Route => 0xeacd,
            Icon::Flag => 0xf0c6,
            Icon::Sound => 0xe050,
            Icon::Visible => 0xe8f4,
            Icon::Hidden => 0xe8f5,
            Icon::Traffic => 0xe565,
            Icon::CheckOn => 0xe9de,
            Icon::CheckOff => 0xe835,
            Icon::RadioOn => 0xe837,
            Icon::RadioOff => 0xe836,
            Icon::ChevronRight => 0xe5cc,
            Icon::ExpandMore => 0xe5cf,
            Icon::Star => 0xf09a,
            Icon::History => 0xe8b3,
            Icon::Schedule => 0xefd6,
            Icon::Warning => 0xf083,
            Icon::Error => 0xf8b6,
            Icon::CheckCircle => 0xf0be,
            Icon::Copy => 0xe14d,
            Icon::Paste => 0xe14f,
            Icon::SelectAll => 0xe162,
            Icon::Search => 0xef7a,
            Icon::Move => 0xe89f,
            Icon::Rotate => 0xe41a,
            Icon::Code => 0xe86f,
            Icon::Scale => 0xf1ce,
            Icon::Dropper => 0xe3b8,
        };
        char::from_u32(cp).unwrap_or('?')
    }

    /// The icon as text of a size.
    pub fn text(self, size: f32) -> RichText {
        RichText::new(self.ch().to_string()).family(FontFamily::Name(FAMILY.into())).size(size)
    }

    pub fn font(size: f32) -> egui::FontId {
        egui::FontId::new(size, FontFamily::Name(FAMILY.into()))
    }
}

/// egui's fonts plus the icon font (as its own family, and behind the text fonts so an
/// icon may also sit inside a label).
pub fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    f.font_data.insert(FAMILY.into(), std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/fonts/MaterialSymbolsSubset.ttf"))));
    // the system's font for a script the interface font has not (else the language shows
    // English rather than squares)
    let lang = crate::i18n::get();
    let mut drawable = true;
    if crate::sysfont::needed(lang) {
        match crate::sysfont::find(lang) {
            Some((data, index)) => {
                let mut font = FontData::from_owned(data);
                font.index = index;
                f.font_data.insert("system".into(), std::sync::Arc::new(font));
                for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                    f.families.entry(fam).or_default().push("system".into());
                }
            }
            None => {
                log::warn!("no system font for {}: its texts stay in English", lang.info().name);
                drawable = false;
            }
        }
    }
    crate::i18n::set_drawable(drawable);
    // (a text font behind the icons: egui wants its replacement character in every family)
    let text = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    f.families.insert(FontFamily::Name(FAMILY.into()), std::iter::once(FAMILY.to_string()).chain(text).collect());
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        f.families.entry(fam).or_default().push(FAMILY.into());
    }
    f
}

/// A label with an icon in front (the icon's own font, the text's own).
pub fn labelled(icon: Icon, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let c = egui::Color32::PLACEHOLDER;
    job.append(&icon.ch().to_string(), 0.0, egui::TextFormat { font_id: Icon::font(16.0), color: c, valign: egui::Align::Center, ..Default::default() });
    job.append(text, 5.0, egui::TextFormat { font_id: egui::FontId::proportional(13.0), color: c, valign: egui::Align::Center, ..Default::default() });
    job
}

/// Draw an icon centred on a point: centred on the glyph's own outline (not on the text
/// line, which sat the icons a pixel or two off), on whole pixels so its strokes stay sharp.
pub fn paint(painter: &egui::Painter, icon: Icon, center: egui::Pos2, size: f32, color: egui::Color32) {
    let galley = painter.layout_no_wrap(icon.ch().to_string(), Icon::font(size), color);
    let ppp = painter.ctx().pixels_per_point();
    let at = center - galley.mesh_bounds.center().to_vec2();
    let at = egui::pos2((at.x * ppp).round() / ppp, (at.y * ppp).round() / ppp);
    painter.galley(at, galley, color);
}
