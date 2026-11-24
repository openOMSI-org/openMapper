//! What the script editor finds wrong as one types: Lua 5.4's syntax (as openOMSI reads it),
//! functions of `omsi.` the game does not have, events it never sends, modules `require`
//! does not find. Errors stop a script; warnings are what would fail later, at run time.

use crate::i18n::tr;
use crate::script::api::api;
use crate::script::lex::{lex, Lang, Tok};

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// 1-based; 0: the whole file.
    pub line: usize,
    pub message: String,
    pub error: bool,
}

/// What a file is checked against.
pub struct Context<'a> {
    /// The modules it can require (`modules.util`).
    pub modules: &'a [String],
    /// A script of objects (no `map` there).
    pub object: bool,
}

fn line_of(s: &str, at: usize) -> usize {
    s[..at.min(s.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

/// The text of a string literal word (quotes off; long brackets left as they are).
fn literal(word: &str) -> Option<&str> {
    let q = word.chars().next()?;
    (matches!(q, '"' | '\'') && word.len() >= 2 && word.ends_with(q)).then(|| &word[1..word.len() - 1])
}

pub fn check(text: &str, cx: &Context) -> Vec<Problem> {
    let mut out = Vec::new();
    if let Err(errors) = full_moon::parse(text) {
        for e in errors.iter() {
            out.push(Problem { line: e.range().0.line(), message: e.error_message().into_owned(), error: true });
        }
    }
    let toks: Vec<_> = lex(text, Lang::Lua).into_iter().filter(|t| !matches!(t.0, Tok::Space | Tok::Comment)).collect();
    let word = |i: usize| toks.get(i).map(|t| &text[t.1.clone()]).unwrap_or("");
    let api = api();
    let mut i = 0;
    while i < toks.len() {
        // an `omsi.` chain: the longest name of the API it is, or a table of it
        if word(i) == "omsi" && word(i + 1) == "." && (i == 0 || !matches!(word(i - 1), "." | ":")) {
            let mut name = String::new();
            let mut j = i + 1;
            while word(j) == "." && toks.get(j + 1).map(|t| matches!(t.0, Tok::Api | Tok::Name | Tok::Call)).unwrap_or(false) {
                if !name.is_empty() {
                    name.push('.');
                }
                name.push_str(word(j + 1));
                j += 2;
            }
            let known = api.function(&name).is_some() || api.functions.iter().any(|f| f.name.starts_with(&format!("{name}."))) || matches!(name.as_str(), "data" | "save");
            if !name.is_empty() && !known {
                out.push(Problem { line: line_of(text, toks[i].1.start), message: format!("omsi.{name}: {} {}", tr("openOMSI has no such function (the API of version", "в openOMSI нет такой функции (API версии"), api.version) + ")", error: false });
            }
            // an event named where a handler is added
            let call = name.as_str();
            if matches!(call, "on" | "off") || (cx.object && call == "emit") {
                check_event(text, &toks, j, call == "emit", &mut out);
            }
            i = j.max(i + 1);
            continue;
        }
        if word(i) == "map" && word(i + 1) == "." && (i == 0 || !matches!(word(i - 1), "." | ":")) {
            if cx.object {
                out.push(Problem { line: line_of(text, toks[i].1.start), message: tr("`map` is for the map's scripts; a script of objects has the omsi functions only", "`map` есть только у скриптов карты; у скрипта объектов только функции omsi").into(), error: false });
            } else if matches!(word(i + 2), "on") {
                check_event(text, &toks, i + 3, false, &mut out);
            }
        }
        if word(i) == "require" && (i == 0 || !matches!(word(i - 1), "." | ":" | "function")) {
            let at = if word(i + 1) == "(" { i + 2 } else { i + 1 };
            if let Some(m) = literal(word(at)) {
                let found = m == "openmapper.map" || cx.modules.iter().any(|x| x == m) || m.starts_with("scripts.");
                if !found {
                    out.push(Problem { line: line_of(text, toks[at].1.start), message: format!("require(\"{m}\"): {}", tr("no such module in the project (modules/…)", "такого модуля в проекте нет (modules/…)")), error: true });
                }
            }
        }
        i += 1;
    }
    out.sort_by_key(|p| (p.line, !p.error));
    out.dedup();
    out
}

/// After `on`/`off`: `("name"` must be an event of the game (an `emit` may make its own).
fn check_event(text: &str, toks: &[(Tok, std::ops::Range<usize>)], at: usize, own: bool, out: &mut Vec<Problem>) {
    let w = |i: usize| toks.get(i).map(|t| &text[t.1.clone()]).unwrap_or("");
    if w(at) != "(" || own {
        return;
    }
    let Some(ev) = literal(w(at + 1)) else { return };
    if api().event(ev).is_none() {
        // (an event of the plugin's own, sent with omsi.emit, is fine: only warn)
        let sent = text.contains(&format!("emit(\"{ev}\"")) || text.contains(&format!("emit('{ev}'"));
        if !sent {
            out.push(Problem { line: line_of(text, toks[at + 1].1.start), message: format!("\"{ev}\": {}", tr("the game sends no such event (unless a script sends it with omsi.emit)", "игра не посылает такого события (если только скрипт не пошлёт его через omsi.emit)")), error: false });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Vec<Problem> {
        check(s, &Context { modules: &["modules.util".into()], object: false })
    }

    #[test]
    fn a_good_script_has_no_problems() {
        let s = "local u = require(\"modules.util\")\nmap.on(\"stop_arrive\", function(name) omsi.ui.toast(omsi.fmt.delay(1)) end)\nomsi.on(\"hour\", print)\nlocal x = omsi.map.ground(1, 2)\nlocal t = omsi.data\n";
        assert_eq!(run(s), vec![]);
    }

    #[test]
    fn mistakes_are_named_on_their_line() {
        let s = "omsi.map.grond(1, 2)\nmap.on(\"stop_arive\", print)\nrequire(\"modules.nope\")\nif x then\n";
        let p = run(s);
        assert!(p.iter().any(|p| p.line == 1 && p.message.contains("omsi.map.grond") && !p.error));
        assert!(p.iter().any(|p| p.line == 2 && p.message.contains("stop_arive")));
        assert!(p.iter().any(|p| p.line == 3 && p.error && p.message.contains("modules.nope")));
        assert!(p.iter().any(|p| p.error && p.line >= 4));
        let o = check("map.on(\"hour\", print)", &Context { modules: &[], object: true });
        assert_eq!(o.len(), 1);
    }

    #[test]
    fn an_own_event_is_not_a_mistake() {
        assert_eq!(run("omsi.on(\"lap\", print)\nomsi.emit(\"lap\", 1)\n"), vec![]);
    }
}
