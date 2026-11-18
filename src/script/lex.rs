//! Words of Lua and Rust source for the editor's colours, completion and checks: comments,
//! strings, numbers, keywords, names and the rest, with their byte ranges. Never fails: a
//! string left open runs to the end of its line, a comment to the end of the text.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Comment,
    Str,
    Number,
    Keyword,
    /// `true`, `false`, `nil` (Lua), `self`, `None`... (Rust).
    Constant,
    Name,
    /// A name followed by `(`: a call.
    Call,
    /// `omsi` and the names after it (`omsi.map.ground`).
    Api,
    Punct,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Lua,
    Rust,
    /// TOML, JSON, plain text: comments and strings only.
    Plain,
}

impl Lang {
    pub fn of(path: &str) -> Lang {
        if path.ends_with(".lua") {
            Lang::Lua
        } else if path.ends_with(".rs") {
            Lang::Rust
        } else {
            Lang::Plain
        }
    }
}

const RUST_KEYWORDS: &[&str] = &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while"];
const RUST_CONSTANTS: &[&str] = &["true", "false", "self", "Self", "None", "Some", "Ok", "Err"];

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The level of a Lua long bracket opening at `s` (`[[` 0, `[==[` 2), if one does.
fn long_bracket(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'[') {
        return None;
    }
    let eq = b[1..].iter().take_while(|&&c| c == b'=').count();
    (b.get(1 + eq) == Some(&b'[')).then_some(eq)
}

/// Where the long bracket of `level` that `s` is inside closes (just past it), or the end.
fn long_close(s: &str, from: usize, level: usize) -> usize {
    let close = format!("]{}]", "=".repeat(level));
    s[from..].find(&close).map(|i| from + i + close.len()).unwrap_or(s.len())
}

/// The words of `s`, end to end.
pub fn lex(s: &str, lang: Lang) -> Vec<(Tok, Range<usize>)> {
    let mut out: Vec<(Tok, Range<usize>)> = Vec::new();
    let mut i = 0;
    let len = s.len();
    while i < len {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap();
        let start = i;
        let tok = if c.is_whitespace() {
            i += rest.find(|ch: char| !ch.is_whitespace()).unwrap_or(rest.len());
            Tok::Space
        } else if (lang == Lang::Lua && rest.starts_with("--")) || (lang == Lang::Rust && rest.starts_with("//")) || (lang == Lang::Plain && c == '#') {
            if lang == Lang::Lua {
                if let Some(level) = long_bracket(&rest[2..]) {
                    i = long_close(s, i + 4 + level, level);
                    out.push((Tok::Comment, start..i));
                    continue;
                }
            }
            i += rest.find('\n').unwrap_or(rest.len());
            Tok::Comment
        } else if lang == Lang::Rust && rest.starts_with("/*") {
            i = s[i + 2..].find("*/").map(|j| i + 2 + j + 2).unwrap_or(len);
            Tok::Comment
        } else if c == '"' || (c == '\'' && lang != Lang::Rust) || (c == '\'' && lang == Lang::Rust && rest.chars().nth(2) == Some('\'')) {
            let mut j = i + 1;
            let b = s.as_bytes();
            while j < len && b[j] != c as u8 && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            i = (j + 1).min(len);
            while !s.is_char_boundary(i) {
                i += 1;
            }
            Tok::Str
        } else if lang == Lang::Lua && long_bracket(rest).is_some() {
            let level = long_bracket(rest).unwrap();
            i = long_close(s, i + 2 + level, level);
            Tok::Str
        } else if c.is_ascii_digit() || (c == '.' && rest[1..].starts_with(|d: char| d.is_ascii_digit())) {
            let b = s.as_bytes();
            let mut j = i + 1;
            while j < len && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'_' || ((b[j] == b'+' || b[j] == b'-') && matches!(b[j - 1], b'e' | b'E' | b'p' | b'P'))) {
                j += 1;
            }
            i = j;
            Tok::Number
        } else if is_name_start(c) {
            let n = rest.find(|ch: char| !is_name(ch)).unwrap_or(rest.len());
            let word = &rest[..n];
            i += n;
            let next = s[i..].trim_start_matches([' ', '\t']).chars().next();
            let after_dot = out.iter().rev().find(|t| t.0 != Tok::Space).filter(|t| &s[t.1.clone()] == "." || &s[t.1.clone()] == ":");
            match lang {
                Lang::Lua if crate::script::api::LUA_KEYWORDS.contains(&word) && !matches!(word, "true" | "false" | "nil") => Tok::Keyword,
                Lang::Lua if matches!(word, "true" | "false" | "nil") => Tok::Constant,
                Lang::Rust if RUST_KEYWORDS.contains(&word) => Tok::Keyword,
                Lang::Rust if RUST_CONSTANTS.contains(&word) => Tok::Constant,
                Lang::Lua if word == "omsi" && after_dot.is_none() => Tok::Api,
                Lang::Lua if after_dot.is_some() && api_chain_before(s, &out) => Tok::Api,
                _ if next == Some('(') || (lang == Lang::Rust && next == Some('!')) => Tok::Call,
                Lang::Plain => Tok::Punct,
                _ => Tok::Name,
            }
        } else {
            i += c.len_utf8();
            Tok::Punct
        };
        out.push((tok, start..i));
    }
    out
}

/// Whether the words before the last `.` are an `omsi.` chain.
fn api_chain_before(s: &str, out: &[(Tok, Range<usize>)]) -> bool {
    let mut it = out.iter().rev().filter(|t| t.0 != Tok::Space);
    let _dot = it.next();
    matches!(it.next(), Some((Tok::Api, _))) && !s.is_empty()
}

/// The dotted name ending at byte `at` (`omsi.map.gr` for the cursor after `gr`), and where
/// it starts: what completion goes on from.
pub fn word_before(s: &str, at: usize) -> (usize, &str) {
    let head = &s[..at];
    let start = head.char_indices().rev().take_while(|&(_, c)| is_name(c) || c == '.').last().map(|(i, _)| i).unwrap_or(at);
    (start, &head[start..])
}

/// The whole dotted name round byte `at` (for help on what the cursor is in).
pub fn word_at(s: &str, at: usize) -> &str {
    let (start, _) = word_before(s, at);
    let end = s[at..].find(|c: char| !is_name(c) && c != '.').map(|i| at + i).unwrap_or(s.len());
    &s[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str, lang: Lang) -> Vec<(Tok, String)> {
        lex(s, lang).into_iter().filter(|t| t.0 != Tok::Space).map(|(t, r)| (t, s[r].to_string())).collect()
    }

    #[test]
    fn lua_words_are_told_apart() {
        let k = kinds("local x = omsi.map.ground(1, 2.5e3) -- here\nprint(\"a\\\"b\", [[long\n]]) --[==[ c\n]==] y", Lang::Lua);
        assert_eq!(k[0], (Tok::Keyword, "local".into()));
        assert_eq!(k[3], (Tok::Api, "omsi".into()));
        assert_eq!(k[5], (Tok::Api, "map".into()));
        assert_eq!(k[7], (Tok::Api, "ground".into()));
        assert_eq!(k[11], (Tok::Number, "2.5e3".into()));
        assert_eq!(k[13], (Tok::Comment, "-- here".into()));
        assert_eq!(k[14], (Tok::Call, "print".into()));
        assert_eq!(k[16], (Tok::Str, "\"a\\\"b\"".into()));
        assert_eq!(k[18], (Tok::Str, "[[long\n]]".into()));
        assert_eq!(k[20], (Tok::Comment, "--[==[ c\n]==]".into()));
        assert_eq!(k[21], (Tok::Name, "y".into()));
    }

    #[test]
    fn open_strings_and_rust_are_read_to_an_end() {
        assert_eq!(kinds("\"open\nx", Lang::Lua)[1], (Tok::Name, "x".into()));
        let k = kinds("fn main() { let s = 'c'; /* x */ println!(\"{s}\"); }", Lang::Rust);
        assert_eq!(k[0], (Tok::Keyword, "fn".into()));
        assert_eq!(k[1], (Tok::Call, "main".into()));
        assert!(k.contains(&(Tok::Str, "'c'".into())));
        assert!(k.contains(&(Tok::Comment, "/* x */".into())));
        assert!(k.contains(&(Tok::Call, "println".into())));
    }

    #[test]
    fn the_word_before_the_cursor() {
        let s = "x = omsi.map.gr + 1";
        assert_eq!(word_before(s, 15), (4, "omsi.map.gr"));
        assert_eq!(word_at(s, 10), "omsi.map.gr");
    }
}
