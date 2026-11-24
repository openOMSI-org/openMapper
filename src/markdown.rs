//! Markdown text shown styled: headings, bold, italic, `code`, links, lists, quotes, code
//! blocks and rules (what a release's notes and a README use).

use egui::text::LayoutJob;
use egui::{FontId, RichText, TextFormat, Ui};

use crate::icons::Icon;

/// One run of an inline line.
#[derive(Debug, PartialEq)]
enum Span {
    Text { text: String, bold: bool, italic: bool, code: bool },
    Link { text: String, url: String },
}

/// Inline markup: `**bold**`, `*italic*` / `_italic_`, `` `code` ``, `[text](url)`, and bare
/// `http(s)://` addresses.
fn inline(src: &str) -> Vec<Span> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut bold, mut italic) = (false, false);
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Span>, bold: bool, italic: bool| {
        if !buf.is_empty() {
            out.push(Span::Text { text: std::mem::take(buf), bold, italic, code: false });
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let rest = |n: usize| chars.get(i + n).copied();
        match c {
            '\\' if rest(1).map(|n| "\\`*_{}[]()#+-.!|>".contains(n)).unwrap_or(false) => {
                buf.push(chars[i + 1]);
                i += 2;
            }
            '`' => {
                if let Some(end) = chars[i + 1..].iter().position(|x| *x == '`') {
                    flush(&mut buf, &mut out, bold, italic);
                    out.push(Span::Text { text: chars[i + 1..i + 1 + end].iter().collect(), bold, italic, code: true });
                    i += end + 2;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            '*' | '_' => {
                let double = rest(1) == Some(c);
                // (an underscore inside a word, as in a file name, is not markup)
                let word_inside = c == '_' && i > 0 && chars[i - 1].is_alphanumeric() && rest(1).map(|n| n.is_alphanumeric()).unwrap_or(false);
                if word_inside {
                    buf.push(c);
                    i += 1;
                } else if double {
                    flush(&mut buf, &mut out, bold, italic);
                    bold = !bold;
                    i += 2;
                } else if rest(1).map(|n| !n.is_whitespace()).unwrap_or(false) || italic {
                    flush(&mut buf, &mut out, bold, italic);
                    italic = !italic;
                    i += 1;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            '[' => {
                let close = chars[i + 1..].iter().position(|x| *x == ']');
                let link = close.and_then(|e| {
                    let at = i + 1 + e;
                    (chars.get(at + 1) == Some(&'(')).then(|| chars[at + 2..].iter().position(|x| *x == ')').map(|p| (e, p))).flatten()
                });
                if let Some((e, p)) = link {
                    flush(&mut buf, &mut out, bold, italic);
                    let at = i + 1 + e;
                    out.push(Span::Link { text: chars[i + 1..at].iter().collect(), url: chars[at + 2..at + 2 + p].iter().collect() });
                    i = at + 2 + p + 1;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            'h' if chars[i..].iter().take(8).collect::<String>().starts_with("http://") || chars[i..].iter().take(8).collect::<String>().starts_with("https://") => {
                let len = chars[i..].iter().position(|x| x.is_whitespace() || *x == ')' || *x == '>').unwrap_or(chars.len() - i);
                let url: String = chars[i..i + len].iter().collect();
                let url = url.trim_end_matches(['.', ',', ';']).to_string();
                flush(&mut buf, &mut out, bold, italic);
                out.push(Span::Link { text: url.clone(), url: url.clone() });
                i += url.chars().count();
            }
            _ => {
                buf.push(c);
                i += 1;
            }
        }
    }
    flush(&mut buf, &mut out, bold, italic);
    out
}

fn line(ui: &mut Ui, src: &str, size: f32, strong: bool) {
    let spans = inline(src);
    let has_link = spans.iter().any(|s| matches!(s, Span::Link { .. }));
    let visuals = ui.visuals();
    let (text, code_bg) = (visuals.text_color(), visuals.code_bg_color);
    let link = visuals.hyperlink_color;
    if !has_link {
        let mut job = LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        for s in &spans {
            if let Span::Text { text: t, bold, italic, code } = s {
                let font = if *code { FontId::monospace(size * 0.92) } else { FontId::proportional(size) };
                let mut f = TextFormat { font_id: font, color: if *bold || strong { ui.visuals().strong_text_color() } else { text }, italics: *italic, ..Default::default() };
                if *code {
                    f.background = code_bg;
                }
                job.append(t, 0.0, f);
            }
        }
        ui.label(job);
        return;
    }
    // (a line with links: the runs side by side, wrapping)
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for s in spans {
            match s {
                Span::Text { text: t, bold, italic, code } => {
                    let mut r = RichText::new(t).size(size);
                    if bold || strong {
                        r = r.strong();
                    }
                    if italic {
                        r = r.italics();
                    }
                    if code {
                        r = r.monospace().background_color(code_bg);
                    }
                    ui.label(r);
                }
                Span::Link { text: t, url } => {
                    ui.hyperlink_to(RichText::new(t).size(size).color(link), url);
                }
            }
        }
    });
}

/// Show Markdown `src` in `ui`.
pub fn show(ui: &mut Ui, src: &str) {
    let base = ui.text_style_height(&egui::TextStyle::Body).max(13.0);
    let mut in_code = false;
    let mut code = String::new();
    let mut para = String::new();
    let flush_para = |ui: &mut Ui, para: &mut String| {
        if !para.is_empty() {
            line(ui, &std::mem::take(para), base, false);
            ui.add_space(3.0);
        }
    };
    for raw in src.replace("\r\n", "\n").lines() {
        let l = raw.trim_end();
        if l.trim_start().starts_with("```") {
            flush_para(ui, &mut para);
            if in_code {
                egui::Frame::new().fill(ui.visuals().code_bg_color).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(code.trim_end()).monospace().size(base * 0.92)).wrap());
                });
                code.clear();
                ui.add_space(3.0);
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(raw);
            code.push('\n');
            continue;
        }
        let t = l.trim_start();
        if t.is_empty() {
            flush_para(ui, &mut para);
        } else if let Some(h) = t.strip_prefix('#') {
            flush_para(ui, &mut para);
            let level = 1 + h.chars().take_while(|c| *c == '#').count();
            let text = h.trim_start_matches('#').trim();
            let size = base * match level { 1 => 1.5, 2 => 1.3, 3 => 1.15, _ => 1.05 };
            ui.add_space(4.0);
            line(ui, text, size, true);
            if level <= 2 {
                ui.separator();
            }
        } else if t.len() >= 3 && t.chars().all(|c| c == '-' || c == '*' || c == '_' || c == ' ') && t.chars().filter(|c| *c != ' ').count() >= 3 {
            flush_para(ui, &mut para);
            ui.separator();
        } else if let Some(q) = t.strip_prefix('>') {
            flush_para(ui, &mut para);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, base * 1.2), egui::Sense::hover());
                ui.painter().rect_filled(r, 1.0, ui.visuals().weak_text_color());
                line(ui, q.trim(), base, false);
            });
        } else if let Some(item) = bullet(t) {
            flush_para(ui, &mut para);
            let depth = (l.len() - t.len()) / 2;
            ui.horizontal_top(|ui| {
                ui.add_space(6.0 + depth as f32 * 14.0);
                ui.label(item.0.size(base));
                ui.vertical(|ui| line(ui, item.1, base, false));
            });
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t);
        }
    }
    flush_para(ui, &mut para);
    if in_code && !code.is_empty() {
        ui.add(egui::Label::new(RichText::new(code.trim_end()).monospace().size(base * 0.92)).wrap());
    }
}

/// A list item: its marker (a task's box a Material Symbol: the text font has none) and the
/// text after it.
fn bullet(t: &str) -> Option<(RichText, &str)> {
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            let r = r.trim_start();
            return Some(match r.strip_prefix("[ ] ") {
                Some(x) => (Icon::CheckOff.text(13.0), x),
                None => match r.strip_prefix("[x] ").or_else(|| r.strip_prefix("[X] ")) {
                    Some(x) => (Icon::CheckOn.text(13.0), x),
                    None => (RichText::new("•"), r),
                },
            });
        }
    }
    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
    (n > 0 && n < 4).then(|| t[n..].strip_prefix(". ").map(|r| (RichText::new(format!("{}.", &t[..n])), r.trim_start()))).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_italic_code_and_links_are_found() {
        let s = inline("a **b** *c* `d` [e](http://x.y) f_g");
        assert!(s.contains(&Span::Text { text: "b".into(), bold: true, italic: false, code: false }));
        assert!(s.contains(&Span::Text { text: "c".into(), bold: false, italic: true, code: false }));
        assert!(s.contains(&Span::Text { text: "d".into(), bold: false, italic: false, code: true }));
        assert!(s.contains(&Span::Link { text: "e".into(), url: "http://x.y".into() }));
        assert!(matches!(s.last(), Some(Span::Text { text, .. }) if text == " f_g"));
    }

    #[test]
    fn list_markers() {
        assert_eq!(bullet("- a").unwrap().0.text(), "•");
        assert_eq!(bullet("12. a").unwrap().0.text(), "12.");
        assert_eq!(bullet("- [x] a").unwrap().0.text(), Icon::CheckOn.ch().to_string());
        assert!(bullet("text").is_none());
    }
}
