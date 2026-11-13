//! Language server over stdio: `crowmation lsp`.
//!
//! - errors as you type (the real compiler, on the unsaved buffer)
//! - semantic highlighting: components (also imported ones), objects, parts,
//!   parameters and constants get their own colors
//! - go to definition, including into imported files
//! - completion of keywords and of the scene's own names
//! - hover docs for keywords, components (with their doc comments) and objects

use crowmation_lang::analyze::{Analysis, Role, analyze};
use crowmation_lang::docs::{DOCS, Kind, lookup};
use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Semantic token types and modifiers, in the order the indices below use.
const TOKEN_TYPES: [&str; 4] = ["function", "variable", "property", "parameter"];
const TOKEN_MODIFIERS: [&str; 2] = ["declaration", "readonly"];

struct Doc {
    text: String,
    /// None when the file could not be read as a scene (no path, deleted, ...).
    analysis: Option<Analysis>,
}

pub fn run() -> Result<(), String> {
    let (conn, io) = Connection::stdio();
    let caps = json!({
        "textDocumentSync": 1, // full text on every change
        "completionProvider": {"triggerCharacters": ["."]},
        "hoverProvider": true,
        "definitionProvider": true,
        "semanticTokensProvider": {
            "legend": {"tokenTypes": TOKEN_TYPES, "tokenModifiers": TOKEN_MODIFIERS},
            "full": true,
        },
    });
    conn.initialize(caps).map_err(|e| e.to_string())?;
    let mut docs: HashMap<String, Doc> = HashMap::new();
    for msg in &conn.receiver {
        match msg {
            Message::Request(req) => {
                if conn.handle_shutdown(&req).map_err(|e| e.to_string())? {
                    break;
                }
                let resp = respond(&docs, &req);
                conn.sender.send(Message::Response(resp)).map_err(|e| e.to_string())?;
            }
            Message::Notification(n) => {
                let p = &n.params;
                let uri = p["textDocument"]["uri"].as_str().unwrap_or_default().to_string();
                let text = match n.method.as_str() {
                    "textDocument/didOpen" => p["textDocument"]["text"].as_str().map(String::from),
                    // Full sync: the last change holds the whole text.
                    "textDocument/didChange" => p["contentChanges"].as_array().and_then(|c| c.last()).and_then(|c| c["text"].as_str()).map(String::from),
                    "textDocument/didClose" => {
                        docs.remove(&uri);
                        publish(&conn, &uri, vec![])?;
                        continue;
                    }
                    _ => continue,
                };
                let Some(text) = text else { continue };
                let doc = update(&uri, text);
                publish(&conn, &uri, diagnostics(&doc))?;
                docs.insert(uri, doc);
            }
            Message::Response(_) => {}
        }
    }
    drop(conn);
    io.join().map_err(|e| e.to_string())
}

fn update(uri: &str, text: String) -> Doc {
    let analysis = uri_to_path(uri).and_then(|p| analyze(&p, &text).ok());
    Doc { text, analysis }
}

fn publish(conn: &Connection, uri: &str, diags: Vec<Value>) -> Result<(), String> {
    let n = Notification::new("textDocument/publishDiagnostics".into(), json!({"uri": uri, "diagnostics": diags}));
    conn.sender.send(Message::Notification(n)).map_err(|e| e.to_string())
}

/// Compiler errors as LSP diagnostics. Errors in imported files are shown
/// on the first line, naming the file.
fn diagnostics(doc: &Doc) -> Vec<Value> {
    let Some(a) = &doc.analysis else {
        return vec![diag(range(&doc.text, 0..0), "cannot read this file as a scene; save it as a .cwm file")];
    };
    a.diags
        .iter()
        .map(|d| match d.file {
            0 => diag(range(&doc.text, d.span.clone()), &d.msg),
            f => diag(range(&doc.text, 0..0), &format!("in {}: {}", a.sources[f].name, d.msg)),
        })
        .collect()
}

fn diag(range: Value, msg: &str) -> Value {
    json!({"range": range, "severity": 1, "source": "crowmation", "message": msg})
}

fn respond(docs: &HashMap<String, Doc>, req: &Request) -> Response {
    let p = &req.params;
    let doc = docs.get(p["textDocument"]["uri"].as_str().unwrap_or_default());
    let pos = || {
        let doc = doc?;
        Some(offset(&doc.text, p["position"]["line"].as_u64()? as usize, p["position"]["character"].as_u64()? as usize))
    };
    let result = match req.method.as_str() {
        "textDocument/completion" => completion(doc),
        "textDocument/hover" => doc.zip(pos()).and_then(|(d, off)| hover(d, off)).unwrap_or(Value::Null),
        "textDocument/definition" => doc.zip(pos()).and_then(|(d, off)| definition(d, off)).unwrap_or(Value::Null),
        "textDocument/semanticTokens/full" => json!({"data": doc.map(semantic_tokens).unwrap_or_default()}),
        _ => {
            return Response::new_err(req.id.clone(), lsp_server::ErrorCode::MethodNotFound as i32, format!("unsupported: {}", req.method));
        }
    };
    Response::new_ok(req.id.clone(), result)
}

fn completion(doc: Option<&Doc>) -> Value {
    // LSP CompletionItemKind numbers.
    let mut items: Vec<Value> = DOCS
        .iter()
        .map(|(word, kind, doc)| {
            let k = match kind {
                Kind::Keyword => 14,
                Kind::Setting | Kind::Property => 10,
                Kind::Object => 7,
                Kind::Action => 3,
                Kind::Value => 21,
                Kind::Function => 3,
            };
            json!({"label": word, "kind": k, "documentation": {"kind": "markdown", "value": doc}})
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    for d in doc.and_then(|d| d.analysis.as_ref()).map_or(&[][..], |a| &a.defs[..]) {
        // Only the scene's own names, and components from anywhere (imports included).
        if (d.file != 0 && d.role != Role::Component) || d.role == Role::Parameter || !seen.insert((d.name.clone(), d.role as u8)) {
            continue;
        }
        let k = match d.role {
            Role::Component => 7,
            Role::Object => 6,
            Role::Part => 5,
            Role::Parameter | Role::Variable => 21,
        };
        items.push(json!({"label": d.name, "kind": k, "detail": d.detail, "documentation": d.doc}));
    }
    json!(items)
}

fn hover(doc: &Doc, off: usize) -> Option<Value> {
    let a = doc.analysis.as_ref()?;
    let md = match a.token_at(off).and_then(|t| t.def) {
        Some(i) => {
            let d = &a.defs[i];
            let mut s = format!("```cwm\n{}\n```", if d.role == Role::Object { format!("{} {}", d.detail, d.name) } else { d.detail.clone() });
            if !d.doc.is_empty() {
                s += &format!("\n\n{}", d.doc);
            }
            if d.file != 0 {
                s += &format!("\n\nfrom `{}`", a.sources[d.file].name);
            }
            s
        }
        None => lookup(word_at(&doc.text, off)?)?.1.to_string(),
    };
    Some(json!({"contents": {"kind": "markdown", "value": md}}))
}

fn definition(doc: &Doc, off: usize) -> Option<Value> {
    let a = doc.analysis.as_ref()?;
    // On an import path: the imported file.
    if let Some((_, f)) = a.imports.iter().find(|(s, _)| s.start <= off && off <= s.end) {
        let path = a.sources[*f].path.as_ref()?;
        return Some(json!({"uri": path_to_uri(path), "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}}));
    }
    let d = &a.defs[a.token_at(off)?.def?];
    let src = &a.sources[d.file];
    let path = src.path.as_ref()?;
    Some(json!({"uri": path_to_uri(path), "range": range(&src.text, d.span.clone())}))
}

/// LSP semantic tokens: 5 numbers per token, positions relative to the previous token.
fn semantic_tokens(doc: &Doc) -> Vec<u32> {
    let Some(a) = &doc.analysis else { return vec![] };
    let mut out = Vec::with_capacity(a.tokens.len() * 5);
    let (mut line, mut col) = (0u32, 0u32);
    for t in &a.tokens {
        let (l, c) = line_col(&doc.text, t.span.start);
        let len = doc.text.get(t.span.clone()).map_or(0, |s| s.encode_utf16().count()) as u32;
        let (l, c) = (l as u32, c as u32);
        if len == 0 || (l, c) < (line, col) {
            continue;
        }
        let (ty, mut mods) = match t.role {
            Role::Component => (0, 0),
            Role::Object => (1, 0),
            Role::Part => (2, 0),
            Role::Parameter => (3, 0),
            Role::Variable => (1, 2),
        };
        if t.declaration {
            mods |= 1;
        }
        out.extend([l - line, if l == line { c - col } else { c }, len, ty, mods]);
        (line, col) = (l, c);
    }
    out
}

/// Byte offset to (line, UTF-16 column).
fn line_col(text: &str, off: usize) -> (usize, usize) {
    let before = &text[..text.floor_char_boundary(off.min(text.len()))];
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().unwrap_or("").encode_utf16().count();
    (line, col)
}

/// (line, UTF-16 column) to a byte offset.
fn offset(text: &str, line: usize, col16: usize) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let l = text[start.min(text.len())..].split('\n').next().unwrap_or("");
    let mut units = 0;
    for (i, c) in l.char_indices() {
        if units >= col16 {
            return start + i;
        }
        units += c.len_utf16();
    }
    start + l.len()
}

/// Byte span to an LSP range.
fn range(text: &str, span: std::ops::Range<usize>) -> Value {
    let pos = |off| {
        let (line, col) = line_col(text, off);
        json!({"line": line, "character": col})
    };
    json!({"start": pos(span.start), "end": pos(span.end.max(span.start))})
}

/// The identifier around a byte offset.
fn word_at(text: &str, off: usize) -> Option<&str> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let off = off.min(text.len());
    let start = text[..off].rfind(|c| !is_word(c)).map_or(0, |i| i + 1);
    let end = text[off..].find(|c| !is_word(c)).map_or(text.len(), |i| off + i);
    (start < end).then(|| &text[start..end])
}

/// `file:///a/b%20c.cwm` -> `/a/b c.cwm`.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && let Some(b) = raw.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?))
}

/// `/a/b c.cwm` -> `file:///a/b%20c.cwm`.
fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(name: &str) -> (String, Doc) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples").join(name).canonicalize().unwrap();
        let uri = path_to_uri(&path);
        let doc = update(&uri, std::fs::read_to_string(&path).unwrap());
        (uri, doc)
    }

    #[test]
    fn positions_and_uris() {
        let text = "scene {\n  rect r size (1, 1) colr #fff\n}";
        assert_eq!(word_at(text, offset(text, 1, 4)), Some("rect"));
        assert_eq!(line_col(text, 10), (1, 2));
        assert_eq!(offset(text, 1, 2), 10);
        assert_eq!(uri_to_path("file:///tmp/a%20b.cwm"), Some(PathBuf::from("/tmp/a b.cwm")));
        assert_eq!(path_to_uri(Path::new("/tmp/a b.cwm")), "file:///tmp/a%20b.cwm");
    }

    #[test]
    fn errors_hover_definition_tokens() {
        let (uri, doc) = example("overflow.cwm");
        assert!(diagnostics(&doc).is_empty());
        let bad = update(&uri, doc.text.replace("fill #2a3340", "fil #2a3340"));
        let d = diagnostics(&bad);
        assert!(d[0]["message"].as_str().unwrap().contains("did you mean `fill`"), "{d:?}");

        let at = |s: &str| doc.text.find(s).unwrap() + 2;
        let h = hover(&doc, at("byte_row buf")).unwrap();
        let md = h["contents"]["value"].as_str().unwrap();
        assert!(md.contains("component byte_row(n, fill)") && md.contains("from `lib/memory.cwm`"), "{md}");
        let def = definition(&doc, at("byte_row buf")).unwrap();
        assert!(def["uri"].as_str().unwrap().ends_with("examples/lib/memory.cwm"));
        assert_eq!(def["range"]["start"]["line"], 3, "line 4, zero based");
        let imp = definition(&doc, at("lib/memory.cwm")).unwrap();
        assert!(imp["uri"].as_str().unwrap().ends_with("memory.cwm"));

        let data = semantic_tokens(&doc);
        assert_eq!(data.len() % 5, 0);
        assert!(data.chunks(5).any(|t| t[3] == 0), "a component token");
    }
}
