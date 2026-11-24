//! A code editor for the Scripts window: Lua and Rust in colour, line numbers with
//! breakpoints beside them, the line a test run stopped at, problems underlined, indenting
//! that follows the code, completion of the game's API and the file's own names, find and
//! replace, go to a line, comment out lines.

use std::collections::BTreeSet;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontId, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};

use crate::i18n::tr;
use crate::script::api::{self, LUA_GLOBALS, LUA_KEYWORDS, LUA_LIBRARY};
use crate::script::lex::{self, Lang, Tok};

/// What the editor keeps between frames for one file.
#[derive(Default)]
pub struct CodeState {
    pub breakpoints: BTreeSet<usize>,
    find_open: bool,
    replace_open: bool,
    find: String,
    replace: String,
    case: bool,
    goto_open: bool,
    goto: String,
    completion: Option<Completion>,
    /// A line to show (and put the cursor on) on the next frame.
    pub jump: Option<usize>,
    /// The cursor's line, for the status line and F9.
    pub cursor_line: usize,
}

struct Completion {
    /// Where the word being completed starts (bytes).
    start: usize,
    items: Vec<(String, String)>,
    picked: usize,
}

pub struct Opts<'a> {
    pub lang: Lang,
    pub writable: bool,
    /// Problems: line and whether it is an error.
    pub problems: &'a [(usize, bool)],
    /// The line a test run stands at.
    pub exec_line: Option<usize>,
    /// Names the file can require (`modules.util`) and other words worth completing.
    pub extra_words: &'a [String],
}

#[derive(Default)]
pub struct CodeOut {
    pub changed: bool,
    pub save: bool,
    /// Help on the API name the cursor is in.
    pub help: Option<String>,
}

struct Colors {
    text: Color32,
    comment: Color32,
    string: Color32,
    number: Color32,
    keyword: Color32,
    constant: Color32,
    call: Color32,
    api: Color32,
    punct: Color32,
    exec: Color32,
    gutter: Color32,
    gutter_text: Color32,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            text: Color32::from_rgb(214, 216, 222),
            comment: Color32::from_rgb(110, 128, 110),
            string: Color32::from_rgb(206, 160, 110),
            number: Color32::from_rgb(170, 200, 150),
            keyword: Color32::from_rgb(110, 160, 230),
            constant: Color32::from_rgb(90, 180, 220),
            call: Color32::from_rgb(220, 210, 150),
            api: Color32::from_rgb(232, 140, 70),
            punct: Color32::from_rgb(170, 172, 180),
            exec: Color32::from_rgba_unmultiplied(230, 200, 60, 46),
            gutter: Color32::from_rgb(30, 31, 34),
            gutter_text: Color32::from_rgb(100, 104, 112),
        }
    } else {
        Colors {
            text: Color32::from_rgb(30, 32, 36),
            comment: Color32::from_rgb(90, 120, 90),
            string: Color32::from_rgb(160, 70, 20),
            number: Color32::from_rgb(30, 120, 60),
            keyword: Color32::from_rgb(20, 70, 180),
            constant: Color32::from_rgb(0, 110, 150),
            call: Color32::from_rgb(110, 80, 0),
            api: Color32::from_rgb(190, 80, 10),
            punct: Color32::from_rgb(80, 82, 90),
            exec: Color32::from_rgba_unmultiplied(240, 200, 40, 90),
            gutter: Color32::from_rgb(232, 233, 236),
            gutter_text: Color32::from_rgb(130, 134, 142),
        }
    }
}

fn byte_of_char(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map(|(b, _)| b).unwrap_or(s.len())
}

fn char_of_byte(s: &str, b: usize) -> usize {
    s[..b.min(s.len())].chars().count()
}

fn line_start(s: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    s.match_indices('\n').nth(line - 2).map(|(i, _)| i + 1).unwrap_or(s.len())
}

fn line_of(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

fn job(text: &str, o: &Opts, c: &Colors, font: &FontId, warn: Color32, err: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let exec = o.exec_line.map(|l| {
        let s = line_start(text, l);
        s..text[s..].find('\n').map(|i| s + i + 1).unwrap_or(text.len())
    });
    let bad: Vec<(std::ops::Range<usize>, bool)> = o
        .problems
        .iter()
        .filter(|p| p.0 > 0)
        .map(|&(l, e)| {
            let s = line_start(text, l);
            (s..text[s..].find('\n').map(|i| s + i).unwrap_or(text.len()), e)
        })
        .collect();
    for (tok, r) in lex::lex(text, o.lang) {
        let color = match tok {
            Tok::Comment => c.comment,
            Tok::Str => c.string,
            Tok::Number => c.number,
            Tok::Keyword => c.keyword,
            Tok::Constant => c.constant,
            Tok::Call => c.call,
            Tok::Api => c.api,
            Tok::Punct => c.punct,
            Tok::Name | Tok::Space => c.text,
        };
        // (split where the stopped line and the underlines begin and end)
        let mut cuts = vec![r.start, r.end];
        if let Some(e) = &exec {
            cuts.extend([e.start, e.end]);
        }
        for (b, _) in &bad {
            cuts.extend([b.start, b.end]);
        }
        cuts.retain(|&x| x >= r.start && x <= r.end && text.is_char_boundary(x));
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a == b {
                continue;
            }
            let mut f = TextFormat { font_id: font.clone(), color, ..Default::default() };
            if exec.as_ref().is_some_and(|e| e.start <= a && b <= e.end) {
                f.background = c.exec;
            }
            if let Some((_, e)) = bad.iter().find(|(x, _)| x.start <= a && b <= x.end && tok != Tok::Space) {
                f.underline = Stroke::new(1.5, if *e { err } else { warn });
            }
            job.append(&text[a..b], 0.0, f);
        }
    }
    job
}

/// The completions of `word` (what is typed before the cursor, dots and all).
fn completions(word: &str, text: &str, extra: &[String]) -> Vec<(String, String)> {
    let api = api::api();
    let lower = word.to_lowercase();
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(rest) = word.strip_prefix("omsi.") {
        let depth = rest.matches('.').count();
        let mut seen = BTreeSet::new();
        for f in &api.functions {
            if !f.name.to_lowercase().starts_with(&rest.to_lowercase()) {
                continue;
            }
            // (the next part only: `omsi.ma` offers `map` before every `map.` function)
            let parts: Vec<&str> = f.name.split('.').collect();
            if parts.len() > depth + 1 {
                let table = parts[..=depth].join(".");
                if seen.insert(table.clone()) {
                    out.push((format!("omsi.{table}"), tr("a table of functions", "таблица функций").into()));
                }
            } else if seen.insert(f.name.clone()) {
                out.push((format!("omsi.{}", f.name), f.signature()));
            }
        }
    } else if let Some(rest) = word.strip_prefix("map.") {
        for (n, d) in [("on", "map.on(event, fn)"), ("every", "map.every(seconds, fn)"), ("after", "map.after(seconds, fn)"), ("watch", "map.watch(kind, name, fn)"), ("here", "map.here()"), ("log", "map.log(...)"), ("folder", "map.folder"), ("name", "map.name")] {
            if n.starts_with(rest) {
                out.push((format!("map.{n}"), d.into()));
            }
        }
    } else if word.contains('.') {
        for l in LUA_LIBRARY.iter().filter(|l| l.starts_with(word)) {
            out.push((l.to_string(), String::new()));
        }
    } else {
        let mut words: BTreeSet<String> = BTreeSet::new();
        for k in LUA_KEYWORDS.iter().chain(LUA_GLOBALS) {
            words.insert(k.to_string());
        }
        words.insert("map".into());
        for (tok, r) in lex::lex(text, Lang::Lua) {
            if matches!(tok, Tok::Name | Tok::Call) && r.len() > 2 {
                words.insert(text[r].to_string());
            }
        }
        for w in words.into_iter().filter(|w| w.to_lowercase().starts_with(&lower) && w != word) {
            out.push((w, String::new()));
        }
        for e in extra.iter().filter(|e| e.starts_with(word)) {
            out.push((e.clone(), tr("a module", "модуль").into()));
        }
    }
    // (an event's name inside on("…")
    out.truncate(40);
    out
}

/// Events the game sends, offered when typing the first argument of `on(`.
fn event_completions(text: &str, cursor: usize) -> Option<(usize, Vec<(String, String)>)> {
    let head = &text[..cursor];
    let q = head.rfind(['"', '\''])?;
    let before = head[..q].trim_end();
    if !(before.ends_with("on(") || before.ends_with("off(")) || head[q + 1..].contains(['"', '\'', ')']) {
        return None;
    }
    let typed = &head[q + 1..];
    let items = api::api().events.iter().filter(|e| e.name.starts_with(typed)).map(|e| (e.name.clone(), e.signature())).collect::<Vec<_>>();
    (!items.is_empty()).then_some((q + 1, items))
}

/// How a new line after `line` is indented: as it, one step more after an opener.
fn indent_after(line: &str, lang: Lang) -> String {
    let base: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let t = line.trim_end();
    let code = match lang {
        Lang::Lua => t.split("--").next().unwrap_or(t).trim_end(),
        _ => t.split("//").next().unwrap_or(t).trim_end(),
    };
    let opens = match lang {
        Lang::Lua => {
            code.ends_with("then") || code.ends_with("do") || code.ends_with("else") || code.ends_with("repeat") || code.ends_with('{') || code.ends_with('(') || (code.contains("function") && code.ends_with(')') && !code.ends_with("end)") && !code.trim_start().starts_with("end"))
        }
        _ => code.ends_with('{') || code.ends_with('(') || code.ends_with('['),
    };
    if opens { format!("{base}  ") } else { base }
}

/// Lines `a..=b` (1-based) commented out, or back in when all of them are.
fn toggle_comment(text: &mut String, a: usize, b: usize, lang: Lang) {
    let mark = if lang == Lang::Lua { "--" } else { "//" };
    let lines: Vec<&str> = text.split('\n').collect();
    let range = a.saturating_sub(1)..b.min(lines.len());
    let all = lines[range.clone()].iter().filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with(mark));
    let out: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if !range.contains(&i) || l.trim().is_empty() {
                return l.to_string();
            }
            let ind = l.len() - l.trim_start().len();
            if all {
                let rest = &l[ind + mark.len()..];
                format!("{}{}", &l[..ind], rest.strip_prefix(' ').unwrap_or(rest))
            } else {
                format!("{}{mark} {}", &l[..ind], &l[ind..])
            }
        })
        .collect();
    *text = out.join("\n");
}

/// The next place `needle` is found from `from` (wrapping), forwards or back.
fn find_in(text: &str, needle: &str, from: usize, back: bool, case: bool) -> Option<std::ops::Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let (hay, pat) = if case { (text.to_string(), needle.to_string()) } else { (text.to_lowercase(), needle.to_lowercase()) };
    if hay.len() != text.len() {
        // (lower case changed the byte lengths: search as it is)
        return find_in(text, needle, from, back, true);
    }
    let hits: Vec<usize> = hay.match_indices(&pat).map(|(i, _)| i).collect();
    let pick = if back { hits.iter().rev().find(|&&i| i < from).or(hits.last()) } else { hits.iter().find(|&&i| i >= from).or(hits.first()) };
    pick.map(|&i| i..i + needle.len())
}

const EDITORS: &str = "code editors";

/// Whether a code editor has the keyboard (its own undo and keys, not the map's).
pub fn has_focus(ctx: &egui::Context) -> bool {
    let Some(f) = ctx.memory(|m| m.focused()) else { return false };
    ctx.data(|d| d.get_temp::<Vec<Id>>(Id::new(EDITORS))).is_some_and(|v| v.contains(&f))
}

pub fn code_editor(ui: &mut egui::Ui, id: Id, text: &mut String, st: &mut CodeState, o: &Opts) -> CodeOut {
    let mut out = CodeOut::default();
    let dark = ui.visuals().dark_mode;
    let c = colors(dark);
    let font = FontId::monospace(13.0);
    let edit_id = id.with("text");
    ui.data_mut(|d| {
        let v = d.get_temp_mut_or_default::<Vec<Id>>(Id::new(EDITORS));
        if !v.contains(&edit_id) {
            v.push(edit_id);
        }
    });
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
    let cur = state.cursor.char_range().unwrap_or(CCursorRange::one(CCursor::new(0)));
    let (c0, c1) = (cur.primary.index.0.min(cur.secondary.index.0), cur.primary.index.0.max(cur.secondary.index.0));
    let (b0, b1) = (byte_of_char(text, c0), byte_of_char(text, c1));
    st.cursor_line = line_of(text, byte_of_char(text, cur.primary.index.0));
    let mut set_cursor: Option<(usize, usize)> = None;

    // keys the editor takes before the text box (completion, indenting, the bars, saving)
    if focused {
        let k = |key: Key, m: Modifiers| ui.input_mut(|i| i.consume_key(m, key));
        if let Some(comp) = st.completion.as_mut() {
            if k(Key::ArrowDown, Modifiers::NONE) {
                comp.picked = (comp.picked + 1) % comp.items.len().max(1);
            } else if k(Key::ArrowUp, Modifiers::NONE) {
                comp.picked = (comp.picked + comp.items.len().max(1) - 1) % comp.items.len().max(1);
            } else if k(Key::Escape, Modifiers::NONE) {
                st.completion = None;
            } else if o.writable && (k(Key::Enter, Modifiers::NONE) || k(Key::Tab, Modifiers::NONE)) {
                if let Some((w, _)) = comp.items.get(comp.picked) {
                    let start = comp.start.min(b1);
                    text.replace_range(start..b1, w);
                    let at = char_of_byte(text, start + w.len());
                    set_cursor = Some((at, at));
                    out.changed = true;
                }
                st.completion = None;
            }
        }
        if o.writable && set_cursor.is_none() && st.completion.is_none() {
            if k(Key::Enter, Modifiers::NONE) {
                let ls = line_start(text, line_of(text, b0));
                let ind = indent_after(&text[ls..b0], o.lang);
                // (a closing word typed on its own line steps back)
                let ins = format!("\n{ind}");
                text.replace_range(b0..b1, &ins);
                let at = char_of_byte(text, b0 + ins.len());
                set_cursor = Some((at, at));
                out.changed = true;
            } else if k(Key::Tab, Modifiers::SHIFT) {
                let (l0, l1) = (line_of(text, b0), line_of(text, b1));
                let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
                for l in lines.iter_mut().take(l1).skip(l0 - 1) {
                    let n = l.chars().take(2).take_while(|c| *c == ' ').count();
                    l.drain(..n);
                }
                *text = lines.join("\n");
                out.changed = true;
            } else if k(Key::Tab, Modifiers::NONE) {
                if line_of(text, b0) != line_of(text, b1) {
                    let (l0, l1) = (line_of(text, b0), line_of(text, b1));
                    let lines: Vec<String> = text.split('\n').enumerate().map(|(i, l)| if i + 1 >= l0 && i < l1 { format!("  {l}") } else { l.to_string() }).collect();
                    *text = lines.join("\n");
                } else {
                    text.replace_range(b0..b1, "  ");
                    let at = char_of_byte(text, b0 + 2);
                    set_cursor = Some((at, at));
                }
                out.changed = true;
            } else if k(Key::Slash, Modifiers::COMMAND) {
                toggle_comment(text, line_of(text, b0), line_of(text, b1), o.lang);
                out.changed = true;
            } else if k(Key::Space, Modifiers::CTRL) {
                let (start, word) = lex::word_before(text, b1);
                let items = completions(word, text, o.extra_words);
                if !items.is_empty() {
                    st.completion = Some(Completion { start, items, picked: 0 });
                }
            }
        }
        if k(Key::S, Modifiers::COMMAND) {
            out.save = true;
        }
        if k(Key::F, Modifiers::COMMAND) {
            st.find_open = true;
            if b0 != b1 && !text[b0..b1].contains('\n') {
                st.find = text[b0..b1].to_string();
            }
        }
        if k(Key::H, Modifiers::COMMAND) {
            st.find_open = true;
            st.replace_open = true;
        }
        if k(Key::G, Modifiers::COMMAND) {
            st.goto_open = true;
        }
        if k(Key::F9, Modifiers::NONE) && !st.breakpoints.remove(&st.cursor_line) {
            st.breakpoints.insert(st.cursor_line);
        }
    }

    // the find, replace and go-to bars
    if st.find_open || st.goto_open {
        ui.horizontal(|ui| {
            if st.find_open {
                let r = ui.add(egui::TextEdit::singleline(&mut st.find).hint_text(tr("Find", "Найти")).desired_width(180.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let back = ui.input(|i| i.modifiers.shift);
                if ui.small_button(crate::icons::labelled(crate::icons::Icon::Down, "")).on_hover_text(tr("Next (Enter)", "Следующее (Enter)")).clicked() || (enter && !back) {
                    if let Some(r) = find_in(text, &st.find, b1, false, st.case) {
                        set_cursor = Some((char_of_byte(text, r.start), char_of_byte(text, r.end)));
                        st.jump = Some(line_of(text, r.start));
                    }
                }
                if ui.small_button(crate::icons::labelled(crate::icons::Icon::Up, "")).on_hover_text(tr("Previous (Shift+Enter)", "Предыдущее (Shift+Enter)")).clicked() || (enter && back) {
                    if let Some(r) = find_in(text, &st.find, b0, true, st.case) {
                        set_cursor = Some((char_of_byte(text, r.start), char_of_byte(text, r.end)));
                        st.jump = Some(line_of(text, r.start));
                    }
                }
                ui.toggle_value(&mut st.case, "Aa").on_hover_text(tr("Match case", "С учётом регистра"));
                let n = if st.find.is_empty() { 0 } else if st.case { text.matches(&st.find).count() } else { text.to_lowercase().matches(&st.find.to_lowercase()).count() };
                ui.label(egui::RichText::new(format!("{n}")).small().weak());
                if o.writable {
                    if st.replace_open {
                        ui.add(egui::TextEdit::singleline(&mut st.replace).hint_text(tr("Replace with", "Заменить на")).desired_width(150.0));
                        if ui.small_button(crate::icons::labelled(crate::icons::Icon::Swap, tr("Replace", "Заменить"))).clicked() && b0 != b1 && text[b0..b1].eq_ignore_ascii_case(&st.find) {
                            text.replace_range(b0..b1, &st.replace);
                            let at = char_of_byte(text, b0 + st.replace.len());
                            set_cursor = Some((at, at));
                            out.changed = true;
                        }
                        if ui.small_button(crate::icons::labelled(crate::icons::Icon::DoneAll, tr("All", "Все"))).clicked() && !st.find.is_empty() {
                            let mut from = 0;
                            while let Some(r) = find_in(&text[from..], &st.find, 0, false, st.case).map(|r| from + r.start..from + r.end) {
                                if r.start < from {
                                    break;
                                }
                                text.replace_range(r.clone(), &st.replace);
                                from = r.start + st.replace.len();
                            }
                            out.changed = true;
                        }
                    } else if ui.small_button(crate::icons::labelled(crate::icons::Icon::Swap, tr("Replace…", "Заменить…"))).clicked() {
                        st.replace_open = true;
                    }
                }
                if crate::icons::small(ui, crate::icons::Icon::Close, tr("Close", "Закрыть")).clicked() {
                    st.find_open = false;
                    st.replace_open = false;
                }
            }
            if st.goto_open {
                ui.label(tr("Line", "Строка"));
                let r = ui.add(egui::TextEdit::singleline(&mut st.goto).desired_width(50.0));
                r.request_focus();
                if r.lost_focus() {
                    if let Ok(l) = st.goto.trim().parse::<usize>() {
                        st.jump = Some(l.max(1));
                    }
                    st.goto_open = false;
                    st.goto.clear();
                }
            }
        });
    }

    if let Some(line) = st.jump {
        if set_cursor.is_none() {
            let at = char_of_byte(text, line_start(text, line));
            set_cursor = Some((at, at));
        }
    }
    if let Some((a, b)) = set_cursor {
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.clone().store(ui.ctx(), edit_id);
        ui.memory_mut(|m| m.request_focus(edit_id));
    }

    let lines = text.matches('\n').count() + 1;
    let gutter_w = 14.0 + 8.0 * (lines.max(10) as f32).log10().ceil() + 10.0;
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let (warn, err) = (ui.visuals().warn_fg_color, ui.visuals().error_fg_color);
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut j = job(buf.as_str(), o, &c, &font, warn, err);
        j.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(j))
    };
    let scroll = egui::ScrollArea::both().id_salt(id.with("scroll")).auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (gutter, _) = ui.allocate_exact_size(Vec2::new(gutter_w, (lines as f32) * row_h + 8.0), Sense::hover());
            let edit = egui::TextEdit::multiline(text).id(edit_id).code_editor().frame(egui::Frame::NONE).desired_width(f32::INFINITY).desired_rows(30).lock_focus(true).interactive(o.writable).layouter(&mut layouter);
            let r = edit.show(ui);
            if r.response.changed() {
                out.changed = true;
            }
            let galley = r.galley.clone();
            let top = r.galley_pos.y;
            // the gutter: numbers, breakpoints (a click toggles one), the stopped line's arrow
            let gutter = Rect::from_min_max(Pos2::new(gutter.left(), top), Pos2::new(gutter.right(), top + galley.rect.height().max(row_h)));
            ui.painter().rect_filled(gutter, 0.0, c.gutter);
            let mut line = 1;
            let mut ys = Vec::with_capacity(lines);
            let mut new_line = true;
            for row in &galley.rows {
                if new_line {
                    ys.push((line, top + row.rect().top(), row.rect().height()));
                    line += 1;
                }
                new_line = row.ends_with_newline;
            }
            let problem_at = |l: usize| o.problems.iter().find(|p| p.0 == l).map(|p| p.1);
            for &(l, y, h) in &ys {
                let mid = y + h * 0.5;
                if st.breakpoints.contains(&l) {
                    ui.painter().circle_filled(Pos2::new(gutter.left() + 8.0, mid), 4.5, Color32::from_rgb(220, 60, 60));
                } else if let Some(e) = problem_at(l) {
                    ui.painter().circle_filled(Pos2::new(gutter.left() + 8.0, mid), 2.5, if e { err } else { warn });
                }
                if o.exec_line == Some(l) {
                    let (x, yel) = (gutter.left() + 8.0, Color32::from_rgb(240, 200, 40));
                    ui.painter().add(egui::Shape::convex_polygon(vec![Pos2::new(x - 4.0, mid - 5.0), Pos2::new(x + 5.0, mid), Pos2::new(x - 4.0, mid + 5.0)], yel, Stroke::NONE));
                }
                ui.painter().text(Pos2::new(gutter.right() - 6.0, mid), egui::Align2::RIGHT_CENTER, l.to_string(), font.clone(), if l == st.cursor_line { c.text } else { c.gutter_text });
            }
            let hit = ui.interact(gutter, id.with("gutter"), Sense::click());
            if hit.clicked() {
                if let Some(p) = hit.interact_pointer_pos() {
                    if let Some(&(l, _, _)) = ys.iter().find(|(_, y, h)| p.y >= *y && p.y < y + h) {
                        if !st.breakpoints.remove(&l) {
                            st.breakpoints.insert(l);
                        }
                    }
                }
            }
            hit.on_hover_text(tr("Click: a breakpoint (F9 on the cursor's line)", "Клик: точка останова (F9 на строке курсора)"));
            // a line asked for, brought into view
            if let Some(l) = st.jump.take() {
                if let Some(&(_, y, h)) = ys.iter().find(|x| x.0 == l) {
                    ui.scroll_to_rect(Rect::from_min_size(Pos2::new(gutter.left(), y), Vec2::new(10.0, h)), Some(egui::Align::Center));
                }
            }
            // completion as one types a name or a dot
            let cursor = r.cursor_range.map(|c| byte_of_char(text, c.primary.index.0)).unwrap_or(0);
            if out.changed && o.writable && o.lang == Lang::Lua && set_cursor.is_none() {
                let typed = text[..cursor].chars().last();
                if let Some((start, items)) = event_completions(text, cursor) {
                    st.completion = Some(Completion { start, items, picked: 0 });
                } else if typed.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.') {
                    let (start, word) = lex::word_before(text, cursor);
                    let items = if word.len() >= 2 || word.ends_with('.') { completions(word, text, o.extra_words) } else { Vec::new() };
                    st.completion = (!items.is_empty()).then_some(Completion { start, items, picked: 0 });
                } else {
                    st.completion = None;
                }
            }
            if !focused && !r.response.has_focus() {
                st.completion = None;
            }
            if let Some(comp) = &mut st.completion {
                let at = galley.pos_from_cursor(CCursor::new(char_of_byte(text, cursor)));
                let pos = r.galley_pos + at.left_bottom().to_vec2() + Vec2::new(0.0, 2.0);
                let mut chosen = None;
                egui::Area::new(id.with("completion")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(460.0);
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            for (i, (w, d)) in comp.items.iter().enumerate() {
                                let text = egui::RichText::new(w).monospace();
                                let r = ui.selectable_label(i == comp.picked, text);
                                if i == comp.picked {
                                    r.scroll_to_me(None);
                                }
                                if !d.is_empty() && i == comp.picked {
                                    ui.label(egui::RichText::new(d).small().weak());
                                }
                                if r.clicked() {
                                    chosen = Some(i);
                                }
                            }
                        });
                    });
                });
                if let Some(i) = chosen {
                    let (w, _) = comp.items[i].clone();
                    text.replace_range(comp.start.min(cursor)..cursor, &w);
                    let at = char_of_byte(text, comp.start + w.len());
                    let mut s = r.state.clone();
                    s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
                    s.store(ui.ctx(), edit_id);
                    ui.memory_mut(|m| m.request_focus(edit_id));
                    st.completion = None;
                    out.changed = true;
                }
            }
            // help on the API name the cursor is in
            if o.lang == Lang::Lua {
                let w = lex::word_at(text, cursor.min(text.len()));
                if let Some(f) = w.strip_prefix("omsi.").and_then(|n| api::api().function(n)) {
                    let perm = f.permission.as_ref().map(|p| format!("  ·  {} {p}", tr("permission", "разрешение"))).unwrap_or_default();
                    out.help = Some(format!("{}  ->  {}{perm}\n{}", f.signature(), f.returns, f.doc));
                }
            }
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_line_takes_the_indent_the_code_asks_for() {
        assert_eq!(indent_after("  if x then", Lang::Lua), "    ");
        assert_eq!(indent_after("map.on(\"hour\", function(h)", Lang::Lua), "  ");
        assert_eq!(indent_after("  local x = 1 -- then", Lang::Lua), "  ");
        assert_eq!(indent_after("fn main() {", Lang::Rust), "  ");
    }

    #[test]
    fn lines_are_commented_out_and_back_in() {
        let mut s = "a = 1\n  b = 2\n\nc = 3".to_string();
        toggle_comment(&mut s, 1, 2, Lang::Lua);
        assert_eq!(s, "-- a = 1\n  -- b = 2\n\nc = 3");
        toggle_comment(&mut s, 1, 2, Lang::Lua);
        assert_eq!(s, "a = 1\n  b = 2\n\nc = 3");
    }

    #[test]
    fn completion_offers_the_games_api_a_part_at_a_time() {
        let c = completions("omsi.m", "", &[]);
        assert!(c.iter().any(|x| x.0 == "omsi.map"));
        assert!(c.iter().any(|x| x.0 == "omsi.message"));
        assert!(!c.iter().any(|x| x.0 == "omsi.map.ground"));
        let c = completions("omsi.map.gr", "", &[]);
        assert_eq!(c[0].0, "omsi.map.ground");
        assert!(completions("modules.u", "", &["modules.util".into()]).is_empty() || completions("mod", "", &["modules.util".into()]).iter().any(|x| x.0 == "modules.util"));
        let (at, ev) = event_completions("map.on(\"stop_a", 14).unwrap();
        assert_eq!(at, 8);
        assert!(ev.iter().any(|e| e.0 == "stop_arrive"));
    }

    #[test]
    fn find_wraps_round_and_goes_back() {
        let s = "one two one two";
        assert_eq!(find_in(s, "two", 5, false, true), Some(12..15));
        assert_eq!(find_in(s, "two", 13, false, true), Some(4..7));
        assert_eq!(find_in(s, "ONE", 8, true, false), Some(0..3));
    }
}
