use crate::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    /// Number with an optional unit suffix, e.g. `2.5s`, `45deg`.
    Num(f64, Option<String>),
    Str(String),
    Color([f32; 4]),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    Dot,
    Newline,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Num(n, u) => format!("`{n}{}`", u.as_deref().unwrap_or("")),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Color(_) => "a color".into(),
            Tok::Newline => "end of line".into(),
            Tok::Eof => "end of file".into(),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::LBrace => "`{`".into(),
            Tok::RBrace => "`}`".into(),
            Tok::LBracket => "`[`".into(),
            Tok::RBracket => "`]`".into(),
            Tok::Comma => "`,`".into(),
            Tok::Plus => "`+`".into(),
            Tok::Minus => "`-`".into(),
            Tok::Star => "`*`".into(),
            Tok::Slash => "`/`".into(),
            Tok::Eq => "`=`".into(),
            Tok::Dot => "`.`".into(),
        }
    }
}

pub fn lex(src: &str, diags: &mut Vec<Diag>) -> Vec<(Tok, Span)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let c = b[i];
        let tok = match c {
            b' ' | b'\t' | b'\r' => {
                i += 1;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'\n' => {
                i += 1;
                Tok::Newline
            }
            b'0'..=b'9' => {
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                    i += 1;
                }
                let n = src[start..i].parse().unwrap();
                let us = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                Tok::Num(n, (i > us).then(|| src[us..i].to_string()))
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                Tok::Ident(src[start..i].to_string())
            }
            b'"' => {
                // Strings may span lines. Escapes: \" \\ \n; others pass through to Typst.
                i += 1;
                let mut text = String::new();
                let mut closed = false;
                while i < b.len() {
                    match b[i] {
                        b'"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        b'\\' if i + 1 < b.len() => {
                            match b[i + 1] {
                                b'n' => text.push('\n'),
                                b'"' => text.push('"'),
                                b'\\' => text.push('\\'),
                                // Anything else (like `\#`) is passed through for Typst.
                                _ => text.push('\\'),
                            }
                            i += if matches!(b[i + 1], b'n' | b'"' | b'\\') { 2 } else { 1 };
                        }
                        _ => {
                            let ch = src[i..].chars().next().unwrap();
                            text.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                if !closed {
                    diags.push(Diag::new(start..i, "unterminated string"));
                    continue;
                }
                Tok::Str(text)
            }
            b'#' => {
                i += 1;
                while i < b.len() && b[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                match parse_hex(&src[start + 1..i]) {
                    Some(c) => Tok::Color(c),
                    None => {
                        diags.push(Diag::new(start..i, "bad color, expected #rrggbb or #rrggbbaa"));
                        continue;
                    }
                }
            }
            _ => {
                i += 1;
                match c {
                    b'(' => Tok::LParen,
                    b')' => Tok::RParen,
                    b'{' => Tok::LBrace,
                    b'}' => Tok::RBrace,
                    b'[' => Tok::LBracket,
                    b']' => Tok::RBracket,
                    b',' => Tok::Comma,
                    b'+' => Tok::Plus,
                    b'-' => Tok::Minus,
                    b'*' => Tok::Star,
                    b'/' => Tok::Slash,
                    b'=' => Tok::Eq,
                    b'.' => Tok::Dot,
                    _ => {
                        // Skip the whole UTF-8 character.
                        while i < b.len() && !src.is_char_boundary(i) {
                            i += 1;
                        }
                        diags.push(Diag::new(start..i, format!("unexpected character `{}`", &src[start..i])));
                        continue;
                    }
                }
            }
        };
        out.push((tok, start..i));
    }
    out.push((Tok::Eof, b.len()..b.len()));
    out
}

fn parse_hex(h: &str) -> Option<[f32; 4]> {
    if !(h.len() == 6 || h.len() == 8) || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let ch = |k: usize| u8::from_str_radix(&h[k..k + 2], 16).unwrap() as f32 / 255.0;
    Some([ch(0), ch(2), ch(4), if h.len() == 8 { ch(6) } else { 1.0 }])
}
