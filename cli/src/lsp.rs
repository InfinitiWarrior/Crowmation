//! Language server over stdio: `crowmation lsp`.
//!
//! Errors as you type (the real compiler, on the unsaved buffer), completion
//! and hover docs for every word the language knows.

use crowmation_lang::docs::{DOCS, Kind, lookup};
use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn run() -> Result<(), String> {
    let (conn, io) = Connection::stdio();
    let caps = json!({
        "textDocumentSync": 1, // full text on every change
        "completionProvider": {},
        "hoverProvider": true,
    });
    conn.initialize(caps).map_err(|e| e.to_string())?;
    // Open documents: uri -> text.
    let mut docs: HashMap<String, String> = HashMap::new();
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
                match n.method.as_str() {
                    "textDocument/didOpen" => {
                        docs.insert(uri.clone(), p["textDocument"]["text"].as_str().unwrap_or_default().into());
                    }
                    // Full sync: the last change holds the whole text.
                    "textDocument/didChange" => {
                        if let Some(t) = p["contentChanges"].as_array().and_then(|c| c.last()).and_then(|c| c["text"].as_str()) {
                            docs.insert(uri.clone(), t.into());
                        }
                    }
                    "textDocument/didSave" => {}
                    "textDocument/didClose" => {
                        docs.remove(&uri);
                        publish(&conn, &uri, vec![])?;
                        continue;
                    }
                    _ => continue,
                }
                if let Some(text) = docs.get(&uri) {
                    publish(&conn, &uri, diagnostics(&uri, text))?;
                }
            }
            Message::Response(_) => {}
        }
    }
    drop(conn);
    io.join().map_err(|e| e.to_string())
}

fn publish(conn: &Connection, uri: &str, diags: Vec<Value>) -> Result<(), String> {
    let n = Notification::new("textDocument/publishDiagnostics".into(), json!({"uri": uri, "diagnostics": diags}));
    conn.sender.send(Message::Notification(n)).map_err(|e| e.to_string())
}

fn respond(docs: &HashMap<String, String>, req: &Request) -> Response {
    let result = match req.method.as_str() {
        "textDocument/completion" => {
            let items: Vec<Value> = DOCS
                .iter()
                .map(|(word, kind, doc)| {
                    // LSP CompletionItemKind numbers.
                    let k = match kind {
                        Kind::Keyword => 14,
                        Kind::Setting | Kind::Property => 10,
                        Kind::Object => 7,
                        Kind::Action => 3,
                        Kind::Value => 21,
                    };
                    json!({"label": word, "kind": k, "documentation": {"kind": "markdown", "value": doc}})
                })
                .collect();
            json!(items)
        }
        "textDocument/hover" => {
            let p = &req.params;
            let text = docs.get(p["textDocument"]["uri"].as_str().unwrap_or_default());
            let word = text.and_then(|t| word_at(t, p["position"]["line"].as_u64()? as usize, p["position"]["character"].as_u64()? as usize));
            match word.and_then(lookup) {
                Some((_, doc)) => json!({"contents": {"kind": "markdown", "value": doc}}),
                None => Value::Null,
            }
        }
        _ => {
            return Response::new_err(req.id.clone(), lsp_server::ErrorCode::MethodNotFound as i32, format!("unsupported: {}", req.method));
        }
    };
    Response::new_ok(req.id.clone(), result)
}

/// Compile the buffer and turn errors into LSP diagnostics. Errors in
/// imported files are shown on the first line, naming the file.
fn diagnostics(uri: &str, text: &str) -> Vec<Value> {
    let Some(path) = uri_to_path(uri) else { return vec![] };
    let (sources, diags) = match crowmation_lang::check(&path, text) {
        Ok(x) => x,
        Err(e) => return vec![diag(range(text, 0..0), &e)],
    };
    diags
        .iter()
        .map(|d| match d.file {
            0 => diag(range(text, d.span.clone()), &d.msg),
            f => diag(range(text, 0..0), &format!("in {}: {}", sources[f].name, d.msg)),
        })
        .collect()
}

fn diag(range: Value, msg: &str) -> Value {
    json!({"range": range, "severity": 1, "source": "crowmation", "message": msg})
}

/// Byte span to an LSP range (lines, UTF-16 columns).
fn range(text: &str, span: std::ops::Range<usize>) -> Value {
    let pos = |off: usize| {
        let off = off.min(text.len());
        let before = &text[..text.floor_char_boundary(off)];
        let line = before.matches('\n').count();
        let col = before.rsplit('\n').next().unwrap_or("").encode_utf16().count();
        json!({"line": line, "character": col})
    };
    json!({"start": pos(span.start), "end": pos(span.end.max(span.start))})
}

/// The identifier under an LSP position (line, UTF-16 column).
fn word_at(text: &str, line: usize, col16: usize) -> Option<&str> {
    let l = text.split('\n').nth(line)?;
    let mut byte = l.len();
    let mut units = 0;
    for (i, c) in l.char_indices() {
        if units >= col16 {
            byte = i;
            break;
        }
        units += c.len_utf16();
    }
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let start = l[..byte].rfind(|c| !is_word(c)).map_or(0, |i| i + 1);
    let end = l[byte..].find(|c| !is_word(c)).map_or(l.len(), |i| byte + i);
    (start < end).then(|| &l[start..end])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_and_words() {
        let text = "scene {\n  rect r size (1, 1) colr #fff\n}";
        assert_eq!(word_at(text, 1, 4), Some("rect"));
        assert_eq!(word_at(text, 0, 0), Some("scene"));
        assert_eq!(range(text, 9..13)["start"], json!({"line": 1, "character": 1}));
        assert_eq!(uri_to_path("file:///tmp/a%20b.cwm"), Some(PathBuf::from("/tmp/a b.cwm")));
    }

    #[test]
    fn reports_compiler_errors() {
        let dir = std::env::temp_dir();
        let uri = format!("file://{}/lsp_test.cwm", dir.display());
        std::fs::write(dir.join("lsp_test.cwm"), "").unwrap();
        let d = diagnostics(&uri, "scene {\n  rect r size (1, 1) colr #ffffff\n}");
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0]["range"]["start"]["line"], 1);
        assert!(d[0]["message"].as_str().unwrap().contains("colr"));
        assert!(diagnostics(&uri, "scene {\n  rect r size (1, 1)\n}").is_empty());
    }
}
