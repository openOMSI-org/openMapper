//! Icons: Google's Material Symbols (Rounded), from a small font in the repository
//! (`assets/fonts/MaterialSymbolsSubset.ttf`, made by `tools/subset-icons.py` from the
//! full font kept beside it). Nothing is fetched at run time.
//!
//! One action has one icon wherever it shows (menu, toolbar, panel, command search), and
//! no symbol is drawn as text: the interface font has few of them and draws a box instead.

use egui::{FontData, FontDefinitions, FontFamily, RichText};

pub const FAMILY: &str = "material-symbols";

/// The icons: each its Material Symbols name (as `tools/subset-icons.py` lists it) and code.
macro_rules! icons {
    ($($(#[$doc:meta])* $v:ident = $name:literal $cp:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon {
            $($(#[$doc])* $v,)*
        }

        impl Icon {
            #[cfg(test)]
            const ALL: &[(Icon, &str)] = &[$((Icon::$v, $name)),*];

            fn code(self) -> u32 {
                match self {
                    $(Icon::$v => $cp,)*
                }
            }
        }
    };
}

icons! {
    // tools and what they make
    Select = "arrow_selector_tool" 0xf82f,
    Road = "road" 0xf472,
    /// The complex junction editor's tools: a line, an arc, an arm; a saved object; a side view.
    Line = "polyline" 0xebbb,
    Arc = "line_curve" 0xf757,
    Arm = "call_made" 0xe0b2,
    Cube = "deployed_code" 0xf720,
    SideView = "height" 0xea16,
    Objects = "home_work" 0xf030,
    Terrain = "landscape" 0xe564,
    Paint = "format_paint" 0xe243,
    Tiles = "grid_on" 0xe3ec,
    Sensors = "sensors" 0xe51e,
    Move = "open_with" 0xe89f,
    Rotate = "rotate_right" 0xe41a,
    RotateLeft = "rotate_left" 0xe419,
    Scale = "open_in_full" 0xf1ce,
    Dropper = "colorize" 0xe3b8,
    /// A vertex snapped onto another.
    Snap = "adjust" 0xe39e,
    Brush = "brush" 0xe3ae,
    Eraser = "ink_eraser" 0xe6d0,
    Raise = "arrow_upward" 0xe5d8,
    Lower = "arrow_downward" 0xe5db,
    Smooth = "waves" 0xe176,
    Flatten = "horizontal_rule" 0xf108,
    Water = "water" 0xf084,
    /// An even slope: a ramp, a grade along a road.
    Slope = "trending_up" 0xe8e5,
    /// A road's height profile.
    Profile = "elevation" 0xf6e7,
    Roundabout = "roundabout_right" 0xeba3,
    Cut = "content_cut" 0xe14e,
    Link = "link" 0xe250,
    LinkOff = "link_off" 0xe16f,
    /// Laid down on the ground.
    Ground = "vertical_align_bottom" 0xe258,
    /// A road or a path the other way round.
    Reverse = "u_turn_left" 0xeba1,
    /// A spline completed to a point.
    Complete = "add_road" 0xef3b,
    Flip = "flip" 0xe3e8,
    Swap = "swap_horiz" 0xe8d4,
    SwapVert = "swap_vert" 0xe8d5,
    /// Objects set out in a line, or spaced evenly.
    LineUp = "linear_scale" 0xe260,
    Spread = "horizontal_distribute" 0xe014,
    Shuffle = "shuffle" 0xe043,
    Polygon = "pentagon" 0xeb50,
    Shapes = "category" 0xe72c,
    Split = "call_split" 0xe0b6,
    Merge = "merge" 0xeb98,
    Straighten = "straighten" 0xe41c,
    AutoFix = "auto_fix_high" 0xe663,
    Texture = "texture" 0xe421,
    Image = "image" 0xe3f4,
    Light = "lightbulb" 0xe90f,
    // things on the map
    Map = "map" 0xe55b,
    Bus = "directions_bus" 0xeff6,
    Car = "directions_car" 0xeff7,
    Building = "apartment" 0xea40,
    Tree = "park" 0xea63,
    Junction = "alt_route" 0xf184,
    /// The crossing editor: a crossing's own paths and lights.
    Crossing = "fork_right" 0xebac,
    Tram = "tram" 0xe571,
    Walk = "directions_walk" 0xe536,
    Fence = "fence" 0xf1f6,
    Layers = "layers" 0xe53b,
    Parking = "local_parking" 0xe54f,
    Route = "route" 0xeacd,
    Gps = "gps_fixed" 0xe55c,
    Flag = "flag" 0xf0c6,
    Signpost = "signpost" 0xeb91,
    Sound = "volume_up" 0xe050,
    Traffic = "traffic" 0xe565,
    Cable = "cable" 0xefe6,
    Satellite = "satellite_alt" 0xeb3a,
    Public = "public" 0xe80b,
    Model3d = "view_in_ar" 0xefc9,
    Construction = "construction" 0xea3c,
    // files and commands
    NewFile = "note_add" 0xe89c,
    Open = "folder_open" 0xe2c8,
    Folder = "folder" 0xe2c7,
    Save = "save" 0xe161,
    Import = "download" 0xf090,
    /// A file written out (an export).
    UploadFile = "upload_file" 0xe9fc,
    Share = "share" 0xe80d,
    Code = "code" 0xe86f,
    Build = "build" 0xf8cd,
    Undo = "undo" 0xe166,
    Redo = "redo" 0xe15a,
    Copy = "content_copy" 0xe14d,
    Paste = "content_paste" 0xe14f,
    Duplicate = "library_add" 0xe03c,
    SelectAll = "select_all" 0xe162,
    Deselect = "deselect" 0xebb6,
    Group = "ad_group" 0xe65b,
    Ungroup = "ad_group_off" 0xeae5,
    /// Kept for other maps (a group, a selection).
    Inventory = "inventory_2" 0xe1a1,
    Add = "add" 0xe145,
    Delete = "delete" 0xe92e,
    Edit = "edit" 0xf097,
    Search = "search" 0xef7a,
    /// The command search.
    Command = "keyboard_command_key" 0xeae7,
    Focus = "center_focus_strong" 0xe3b4,
    Bookmark = "bookmark" 0xe8e7,
    Camera = "photo_camera" 0xe412,
    Video = "videocam" 0xe04b,
    ZoomIn = "zoom_in" 0xe8ff,
    ZoomOut = "zoom_out" 0xe900,
    Fit = "fit_screen" 0xea10,
    Label = "label" 0xe893,
    Settings = "settings" 0xe8b8,
    Tune = "tune" 0xe429,
    Restart = "restart_alt" 0xf053,
    Refresh = "refresh" 0xe5d5,
    Update = "update" 0xe923,
    /// An earlier version of the map brought back.
    Backup = "settings_backup_restore" 0xe8ba,
    History = "history" 0xe8b3,
    Chrono = "hourglass" 0xebff,
    Schedule = "schedule" 0xefd6,
    Calendar = "calendar_month" 0xebcc,
    Sun = "light_mode" 0xe518,
    Season = "ac_unit" 0xeb3b,
    Contrast = "contrast" 0xeb37,
    TextSize = "format_size" 0xe245,
    Translate = "translate" 0xe8e2,
    Keyboard = "keyboard" 0xe312,
    Memory = "memory" 0xe322,
    Quality = "high_quality" 0xe024,
    People = "group" 0xea21,
    Login = "login" 0xea77,
    Logout = "logout" 0xe9ba,
    Lock = "lock" 0xe899,
    Unlock = "lock_open" 0xe898,
    School = "school" 0xe80c,
    Info = "info" 0xe88e,
    // playing and checking
    Play = "play_arrow" 0xe037,
    Stop = "stop" 0xe047,
    FactCheck = "fact_check" 0xf0c5,
    Verified = "verified" 0xef76,
    Check = "check" 0xe668,
    DoneAll = "done_all" 0xe877,
    Close = "close" 0xe5cd,
    Block = "block" 0xf08c,
    Warning = "warning" 0xf083,
    Error = "error" 0xf8b6,
    CheckCircle = "check_circle" 0xf0be,
    // the Scripts window's and the trolleybus wire's
    Pause = "pause" 0xe034,
    StepOver = "step_over" 0xf6ff,
    StepInto = "step_into" 0xf701,
    StepOut = "step_out" 0xf700,
    Bug = "bug_report" 0xe868,
    Terminal = "terminal" 0xeb8e,
    DataObject = "data_object" 0xead3,
    Description = "description" 0xe873,
    Extension = "extension" 0xe87b,
    AddBox = "add_box" 0xe146,
    Bolt = "electric_bolt" 0xec1c,
    // the interface's own parts
    Up = "arrow_upward" 0xe5d8,
    Down = "arrow_downward" 0xe5db,
    Left = "arrow_back" 0xe5c4,
    Right = "arrow_forward" 0xe5c8,
    Visible = "visibility" 0xe8f4,
    Hidden = "visibility_off" 0xe8f5,
    CheckOn = "check_box" 0xe9de,
    CheckOff = "check_box_outline_blank" 0xe835,
    RadioOn = "radio_button_checked" 0xe837,
    RadioOff = "radio_button_unchecked" 0xe836,
    ChevronLeft = "chevron_left" 0xe5cb,
    ChevronRight = "chevron_right" 0xe5cc,
    ExpandMore = "expand_more" 0xe5cf,
    Star = "star" 0xf09a,
}

impl Icon {
    pub fn ch(self) -> char {
        char::from_u32(self.code()).unwrap_or('?')
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

/// A group's heading with an icon in front, its text strong.
pub fn heading(icon: Icon, text: &str) -> egui::text::LayoutJob {
    let mut job = labelled(icon, text);
    job.sections[1].format.color = crate::theme::pal().text_strong;
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

/// A small button that is only an icon, what it does in its hover text (a row's delete,
/// up and down).
pub fn small(ui: &mut egui::Ui, icon: Icon, hover: &str) -> egui::Response {
    ui.small_button(icon.text(15.0)).on_hover_text(hover)
}

/// A button that is only an icon, what it does in its hover text (a drawing's zoom).
pub fn button(ui: &mut egui::Ui, icon: Icon, hover: &str) -> egui::Response {
    ui.button(icon.text(17.0)).on_hover_text(hover)
}

#[cfg(test)]
mod tests {
    use super::Icon;
    use ab_glyph::Font;

    #[test]
    fn every_icon_is_the_material_symbol_it_names_and_in_the_subset() {
        let codes: std::collections::HashMap<&str, u32> = include_str!("../assets/fonts/MaterialSymbolsRounded.codepoints")
            .lines()
            .filter_map(|l| {
                let (name, code) = l.split_once(' ')?;
                Some((name, u32::from_str_radix(code, 16).ok()?))
            })
            .collect();
        let font = ab_glyph::FontRef::try_from_slice(include_bytes!("../assets/fonts/MaterialSymbolsSubset.ttf")).expect("the subset is a font");
        for (icon, name) in Icon::ALL {
            assert_eq!(codes.get(name), Some(&icon.code()), "{icon:?} is not {name}");
            assert_ne!(font.glyph_id(icon.ch()).0, 0, "{name} is not in the subset: add it to tools/subset-icons.py");
        }
    }
}
