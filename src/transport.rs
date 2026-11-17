//! The transport modes a map is made for, openOMSI's bare keywords in `global.cfg`
//! (`[bus_transport]`, `[tram_transport]`, `[railway_transport]`, `[metro_transport]`,
//! `[trolleybus_transport]`): the game puts only vehicles of these modes on the map, and a map
//! naming none takes every vehicle. OMSI 2 skips them. They are kept among the global
//! settings' keywords the reader does not know, so undo and saving carry them like the rest.

use omsi_map::GlobalCfg;

use crate::i18n::tr;

/// The keywords, in the order they are written.
pub const KEYWORDS: [&str; 5] = ["bus_transport", "tram_transport", "railway_transport", "metro_transport", "trolleybus_transport"];

/// A mode's name, by its place in `KEYWORDS`.
pub fn name(i: usize) -> &'static str {
    match i {
        0 => tr("buses", "автобусы"),
        1 => tr("trams", "трамваи"),
        2 => tr("railway", "железная дорога"),
        3 => tr("metro", "метро"),
        _ => tr("trolleybuses", "троллейбусы"),
    }
}

/// The keywords of the file the settings were read from, kept with them (a reader that knows
/// them takes them as fields of its own, and saving would lose them).
pub fn read(g: &mut GlobalCfg) {
    let Ok(bytes) = std::fs::read(&g.path) else { return };
    let text = omsi_cfg::decode_text(&bytes);
    let found: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix('[').and_then(|r| r.strip_suffix(']'))).filter_map(|k| KEYWORDS.iter().find(|w| w.eq_ignore_ascii_case(k.trim())).copied()).collect();
    for k in found {
        set(g, k, true);
    }
}

pub fn has(g: &GlobalCfg, keyword: &str) -> bool {
    g.unknown_keywords.iter().any(|(k, _)| k.eq_ignore_ascii_case(keyword))
}

pub fn set(g: &mut GlobalCfg, keyword: &str, on: bool) {
    g.unknown_keywords.retain(|(k, _)| !k.eq_ignore_ascii_case(keyword));
    if on {
        g.unknown_keywords.push((keyword.to_string(), 0));
    }
}

/// The modes on, in the order of `KEYWORDS`.
pub fn modes(g: &GlobalCfg) -> Vec<&'static str> {
    KEYWORDS.iter().copied().filter(|k| has(g, k)).collect()
}

/// The modes as a short text ("buses, trams"; "any vehicle" for none).
pub fn summary(g: &GlobalCfg) -> String {
    let v: Vec<&str> = (0..KEYWORDS.len()).filter(|i| has(g, KEYWORDS[*i])).map(name).collect();
    if v.is_empty() { tr("any vehicle", "любой транспорт").to_string() } else { v.join(", ") }
}

/// The settings' row: a box for each mode.
pub fn row(ui: &mut egui::Ui, g: &mut GlobalCfg) {
    ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, k) in KEYWORDS.iter().enumerate() {
                let mut on = has(g, k);
                if crate::ui::check(ui, &mut on, name(i)).changed() {
                    set(g, k, on);
                }
            }
        });
        ui.label(egui::RichText::new(tr("openOMSI puts only vehicles of these modes on the map (none ticked: any vehicle). OMSI 2 ignores it.", "openOMSI ставит на карту только транспорт этих видов (ничего не отмечено — любой). OMSI 2 это не читает.")).small().color(crate::ui::TEXT_DIM));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read from the file whatever the reader made of them, written back in their order.
    #[test]
    fn modes_are_read_and_written_back() {
        let dir = crate::doc::tests::scratch("transport_modes").content;
        let cfg = dir.join("global.cfg");
        std::fs::write(&cfg, "[name]\nT\n\n[trolleybus_transport]\n\n[BUS_TRANSPORT]\n\n[version]\n14\n").unwrap();
        let mut g = GlobalCfg::load(&cfg).unwrap();
        g.unknown_keywords.clear();
        read(&mut g);
        assert_eq!(modes(&g), vec!["bus_transport", "trolleybus_transport"]);
        set(&mut g, "trolleybus_transport", false);
        set(&mut g, "tram_transport", true);
        assert_eq!(modes(&g), vec!["bus_transport", "tram_transport"]);
        let mut doc = crate::selection::tests::flat_map(1);
        doc.global = g;
        let text = crate::doc::save::global_text(&doc);
        assert!(text.contains("[bus_transport]\n") && text.contains("[tram_transport]\n") && !text.contains("trolleybus"));
    }
}
