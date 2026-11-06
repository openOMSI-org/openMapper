//! The interface's languages, the same twenty-eight as openOMSI's. English and Russian are
//! written side by side where a text is used: `tr("Save", "Сохранить")`. Every other language
//! is a table found by the English text (`locales/<code>.tsv`: the English, a tab, the
//! translation), so a new text needs no change at the call sites and a text without an entry
//! is shown in English.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

static LANG: AtomicU8 = AtomicU8::new(0);
/// False while the language's script has no font on this system (its texts stay English).
static DRAWABLE: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    En,
    Ru,
    De,
    Fr,
    Uk,
    Be,
    Kk,
    Pl,
    Cs,
    Hu,
    Es,
    Ca,
    Pt,
    PtPt,
    It,
    Nl,
    Sv,
    Tr,
    Ja,
    ZhTw,
    Ko,
    Th,
    Vi,
    Id,
    Ms,
    Tl,
    Zh,
    Hi,
}

/// A language: its code (openOMSI's interface tables use the same), its name in itself, the
/// map description's code (`global_<code>.dsc`) and other spellings a settings file or the
/// system's locale may use.
pub struct Language {
    pub lang: Lang,
    pub code: &'static str,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

macro_rules! lang {
    ($l:ident, $code:literal, $name:literal, [$($a:literal),*]) => {
        Language { lang: Lang::$l, code: $code, name: $name, aliases: &[$($a),*] }
    };
}

/// Every language, in openOMSI's order.
pub const LANGUAGES: &[Language] = &[
    lang!(En, "en", "English", ["english"]),
    lang!(De, "de", "Deutsch", ["ger", "german", "deutsch"]),
    lang!(Fr, "fr", "Français", ["fre", "french", "francais", "français"]),
    lang!(Ru, "ru", "Русский", ["russian", "русский"]),
    lang!(Uk, "uk", "Українська", ["ua", "ukrainian", "українська"]),
    lang!(Be, "be", "Беларуская", ["by", "belarusian", "беларуская"]),
    lang!(Kk, "kk", "Қазақша", ["kz", "kazakh", "қазақша"]),
    lang!(Pl, "pl", "Polski", ["polish", "polski"]),
    lang!(Cs, "cs", "Čeština", ["cz", "czech", "čeština", "ces"]),
    lang!(Hu, "hu", "Magyar", ["hungarian", "magyar"]),
    lang!(Es, "es", "Español", ["spa", "spanish", "español"]),
    lang!(Ca, "ca", "Català", ["cat", "ca-es", "ca-ad", "catalan", "català", "catala"]),
    lang!(Pt, "pt", "Português (Brasil)", ["br", "pt-br", "por", "portuguese", "português"]),
    lang!(PtPt, "pt-pt", "Português (Portugal)", ["pt-portugal", "portuguese-portugal", "português (portugal)", "português de portugal"]),
    lang!(It, "it", "Italiano", ["italian", "italiano"]),
    lang!(Nl, "nl", "Nederlands", ["dutch", "nederlands"]),
    lang!(Sv, "sv", "Svenska", ["se", "sv-se", "sv-fi", "swedish", "svenska"]),
    lang!(Tr, "tr", "Türkçe", ["turkish", "türkçe"]),
    lang!(Ja, "ja", "日本語", ["jp", "japanese", "日本語"]),
    lang!(ZhTw, "zh-tw", "繁體中文", ["zh-hant", "zh-hk", "zh-mo", "cht", "traditional chinese", "繁體中文", "繁体中文"]),
    lang!(Ko, "ko", "한국어", ["kr", "korean", "한국어"]),
    lang!(Th, "th", "ไทย", ["thai", "ไทย"]),
    lang!(Vi, "vi", "Tiếng Việt", ["vietnamese", "tiếng việt"]),
    lang!(Id, "id", "Bahasa Indonesia", ["indonesian", "bahasa indonesia"]),
    lang!(Ms, "ms", "Bahasa Melayu", ["malay", "bahasa melayu"]),
    lang!(Tl, "tl", "Filipino", ["fil", "filipino", "tagalog"]),
    lang!(Zh, "zh", "中文 (简体)", ["zh-cn", "zh-sg", "zh-hans", "zhs", "chs", "cn", "chinese", "simplified chinese", "简体中文", "簡體中文", "中文"]),
    lang!(Hi, "hi", "हिन्दी", ["hindi", "हिन्दी"]),
];

/// The translations of each language (the same order as `Lang`; English and Russian have
/// none here: they are in the code).
const SOURCES: [&str; 28] = [
    "",
    "",
    include_str!("../locales/de.tsv"),
    include_str!("../locales/fr.tsv"),
    include_str!("../locales/uk.tsv"),
    include_str!("../locales/be.tsv"),
    include_str!("../locales/kk.tsv"),
    include_str!("../locales/pl.tsv"),
    include_str!("../locales/cs.tsv"),
    include_str!("../locales/hu.tsv"),
    include_str!("../locales/es.tsv"),
    include_str!("../locales/ca.tsv"),
    include_str!("../locales/pt.tsv"),
    include_str!("../locales/pt-pt.tsv"),
    include_str!("../locales/it.tsv"),
    include_str!("../locales/nl.tsv"),
    include_str!("../locales/sv.tsv"),
    include_str!("../locales/tr.tsv"),
    include_str!("../locales/ja.tsv"),
    include_str!("../locales/zh-tw.tsv"),
    include_str!("../locales/ko.tsv"),
    include_str!("../locales/th.tsv"),
    include_str!("../locales/vi.tsv"),
    include_str!("../locales/id.tsv"),
    include_str!("../locales/ms.tsv"),
    include_str!("../locales/tl.tsv"),
    include_str!("../locales/zh.tsv"),
    include_str!("../locales/hi.tsv"),
];

type Table = HashMap<&'static str, &'static str, std::hash::BuildHasherDefault<crate::library::Fx>>;

impl Lang {
    fn from_index(i: u8) -> Lang {
        LANGUAGES.iter().map(|l| l.lang).chain([Lang::Ru]).find(|l| *l as u8 == i).unwrap_or(Lang::En)
    }

    pub fn info(self) -> &'static Language {
        LANGUAGES.iter().find(|l| l.lang == self).unwrap_or(&LANGUAGES[0])
    }
}

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn set_drawable(ok: bool) {
    DRAWABLE.store(ok, Ordering::Relaxed);
}

pub fn get() -> Lang {
    Lang::from_index(LANG.load(Ordering::Relaxed))
}

pub fn code(l: Lang) -> &'static str {
    l.info().code
}

/// The language a settings value names, else the system's.
pub fn from_code(c: &str) -> Lang {
    if let Some(l) = by_name(c) {
        return l;
    }
    system().unwrap_or(Lang::En)
}

/// The language a code or another spelling of one names (`pt_BR`, `zh-Hant`, `Deutsch`).
pub fn by_name(c: &str) -> Option<Lang> {
    let s = c.trim().to_lowercase().replace('_', "-");
    // (`de-DE.UTF-8`, `ru@euro`: the encoding and the modifier are not the language)
    let s = s.split(['.', '@']).next().unwrap_or("").to_string();
    if s.is_empty() {
        return None;
    }
    let exact = |s: &str| LANGUAGES.iter().find(|l| l.code == s || l.aliases.contains(&s)).map(|l| l.lang);
    if let Some(l) = exact(&s) {
        return Some(l);
    }
    // Chinese by its script or its country: Taiwan, Hong Kong and Macau write the traditional one
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or("");
    if first == "zh" {
        return Some(if s.contains("hant") || s.ends_with("-tw") || s.ends_with("-hk") || s.ends_with("-mo") { Lang::ZhTw } else { Lang::Zh });
    }
    exact(first)
}

/// The system's language from its locale variables, else the platform's own setting.
fn system() -> Option<Lang> {
    for v in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
        if let Some(l) = std::env::var(v).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX").and_then(|v| v.split(':').find_map(by_name)) {
            return Some(l);
        }
    }
    platform_locales().into_iter().find_map(|l| by_name(&l))
}

#[cfg(target_os = "macos")]
fn platform_locales() -> Vec<String> {
    // (`("zh-Hant-TW", "en-US")`: the user's languages, the first they chose first)
    let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    out.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).filter(|w| w.len() >= 2).map(str::to_string).collect()
}

#[cfg(windows)]
fn platform_locales() -> Vec<String> {
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n > 1 { vec![String::from_utf16_lossy(&buf[..n as usize - 1])] } else { Vec::new() }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_locales() -> Vec<String> {
    Vec::new()
}

/// A table's text with its escapes (`\n`, `\t`, `\\`) undone; plain text stays borrowed.
fn unescape(s: &'static str) -> &'static str {
    if !s.contains('\\') {
        return s;
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    Box::leak(out.into_boxed_str())
}

/// The lines of a table: the English, a tab, the translation.
fn parse(src: &'static str) -> Table {
    src.lines().filter_map(|l| l.split_once('\t')).filter(|(_, t)| !t.is_empty()).map(|(en, t)| (unescape(en), unescape(t))).collect()
}

pub(crate) fn table(l: Lang) -> &'static Table {
    static TABLES: [OnceLock<Table>; 28] = [const { OnceLock::new() }; 28];
    TABLES[l as usize].get_or_init(|| parse(SOURCES[l as usize]))
}

/// The text in the current language.
pub fn tr(en: &'static str, ru: &'static str) -> &'static str {
    match get() {
        Lang::En => en,
        Lang::Ru => ru,
        _ if !DRAWABLE.load(Ordering::Relaxed) => en,
        l => table(l).get(en).copied().unwrap_or(en),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_found_by_every_spelling() {
        assert_eq!(by_name("ru_RU.UTF-8"), Some(Lang::Ru));
        assert_eq!(by_name("pt_BR"), Some(Lang::Pt));
        assert_eq!(by_name("pt-PT"), Some(Lang::PtPt));
        assert_eq!(by_name("zh-Hant-TW"), Some(Lang::ZhTw));
        assert_eq!(by_name("zh_CN"), Some(Lang::Zh));
        assert_eq!(by_name("fil"), Some(Lang::Tl));
        assert_eq!(by_name("de-AT"), Some(Lang::De));
        assert_eq!(by_name("xx"), None);
        assert_eq!(by_name(""), None);
    }

    #[test]
    fn every_language_has_its_place() {
        for (i, l) in LANGUAGES.iter().enumerate() {
            assert_eq!(Lang::from_index(l.lang as u8), l.lang, "{}", l.code);
            assert_eq!(by_name(l.code), Some(l.lang), "{}", l.code);
            let _ = i;
        }
        assert_eq!(LANGUAGES.len(), 28);
    }

    #[test]
    fn every_language_has_its_table() {
        for l in LANGUAGES.iter().filter(|l| !matches!(l.lang, Lang::En | Lang::Ru)) {
            let t = table(l.lang);
            assert!(t.len() > 2000, "{}: {} texts", l.code, t.len());
            assert_eq!(t.get("Save").copied().filter(|s| !s.is_empty()), t.get("Save").copied(), "{}", l.code);
            assert_ne!(t.get("Save").copied(), Some("Save"), "{} has no word for Save", l.code);
        }
    }

    /// The Hong Kong table speaks of the roads as Hong Kong does: traffic lights are
    /// red-green lights, a phase is a change of lights, a lane is a traffic line, and a
    /// spline is a spline. A crossing's blocking is a give-way or mutual-exclusion rule, a
    /// save overwrites the file, and things are picked with 選取, never with 挑. (The words
    /// these replace are listed: a translation that brings one back is caught here.)
    #[test]
    fn the_hong_kong_table_uses_hong_kong_road_words() {
        let t = table(Lang::ZhTw);
        assert!(t.len() > 2000, "{} texts", t.len());
        let taiwan = ["車道", "拖曳", "相位", "時相", "樣條", "號誌燈", "公車", "既有", "封鎖", "挑", "人行道", "方向燈", "坡度", "寬度", "轉燈階段", "保持路口淨空"];
        for (en, tr) in t.iter() {
            for w in taiwan {
                // (行車道 is the roadway as a whole - a carriageway - and Hong Kong's own word;
                // 車道 alone, a lane, is not)
                let hay = tr.replace("行車道", "");
                assert!(!hay.contains(w), "{en:?} says {tr:?}: {w} is not Hong Kong's word");
            }
        }
        assert_eq!(t.get("Fit").copied(), Some("置中縮放"));
        assert_eq!(t.get("Traffic light").copied(), Some("紅綠燈"));
        assert_eq!(t.get("Phase").copied(), Some("燈號階段"));
        assert_eq!(t.get("Lanes").copied(), Some("行車線"));
        assert_eq!(t.get("Blocks").copied(), Some("路徑互斥"));
        assert_eq!(t.get("Turn signal").copied(), Some("指揮燈"));
        assert_eq!(t.get("Width").copied(), Some("闊度"));
        assert_eq!(t.get("Slope").copied(), Some("斜度"));
        assert_eq!(t.get("Footway").copied(), Some("行人路"));
        assert_eq!(t.get("Request loops").copied(), Some("車輛感應線圈"));
        assert!(t.get("Pick a crossing object of the map").is_some_and(|s| s.contains("選取")), "{:?}", t.get("Pick a crossing object of the map"));
        // (the railway's signals keep their own word: they are not traffic lights)
        assert!(t.get("Signal routes").is_some_and(|s| s.contains("號誌")), "{:?}", t.get("Signal routes"));
    }

    #[test]
    fn a_text_without_a_translation_is_shown_in_english() {
        set(Lang::De);
        let translate = tr;
        assert_eq!(translate("A text no table has", "x"), "A text no table has");
        set(Lang::En);
    }
}
