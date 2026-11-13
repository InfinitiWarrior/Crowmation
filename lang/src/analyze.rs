//! What each name in a scene refers to, for editors: semantic highlighting,
//! go to definition, completion of user names and hover.
//!
//! Works on the syntax tree, so it gives results even while the scene has
//! errors elsewhere.

use crate::parse::{Expr, Item, Path as NamePath, Stmt, Word};
use crate::{Diag, Source, Span};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    /// A component name.
    Component,
    /// An object or component instance.
    Object,
    /// A part of an instance or model: `row.cell[2]`, `router.lid`.
    Part,
    /// A component parameter.
    Parameter,
    /// A `let` constant or `repeat` counter.
    Variable,
}

#[derive(Clone, Debug)]
pub struct Def {
    pub name: String,
    pub role: Role,
    /// Source the definition is in (0 is the scene).
    pub file: usize,
    pub span: Span,
    /// Signature or kind, like `component byte_row(n, fill)` or `rect`.
    pub detail: String,
    /// Comment lines right above a component.
    pub doc: String,
}

/// A use or declaration of a name in the scene file.
#[derive(Clone, Debug)]
pub struct Token {
    pub span: Span,
    pub role: Role,
    pub declaration: bool,
    /// Index into `Analysis::defs`, when known.
    pub def: Option<usize>,
}

pub struct Analysis {
    pub sources: Vec<Source>,
    pub diags: Vec<Diag>,
    pub defs: Vec<Def>,
    /// In the scene file, sorted by position.
    pub tokens: Vec<Token>,
    /// Import statements in the scene file: span of the path string and the source it loaded.
    pub imports: Vec<(Span, usize)>,
}

impl Analysis {
    /// The token at a byte offset of the scene file.
    pub fn token_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.iter().find(|t| t.span.start <= offset && offset <= t.span.end)
    }
}

/// Analyze `text`, the current contents of the scene at `path`.
pub fn analyze(path: &Path, text: &str) -> Result<Analysis, String> {
    let (sources, files, mut diags) = crate::load_sources(path, Some(text))?;
    let paths: Vec<Option<std::path::PathBuf>> = sources.iter().map(|s| s.path.clone()).collect();
    if diags.is_empty() {
        diags = crate::finish(&files, &paths, Vec::new(), &mut Vec::new()).err().unwrap_or_default();
    }
    let mut a = Walker { defs: Vec::new(), tokens: Vec::new(), comps: HashMap::new() };
    // Components from every file first, so uses anywhere resolve.
    for (f, items) in files.iter().enumerate() {
        for item in items {
            if let Item::Component { name, params, .. } = item {
                let detail = format!("component {}({})", name.name, params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
                let doc = doc_comment(&sources[f].text, name.span.start);
                a.comps.insert(name.name.clone(), a.defs.len());
                a.defs.push(Def { name: name.name.clone(), role: Role::Component, file: f, span: name.span.clone(), detail, doc });
            }
        }
    }
    let scene_dir = sources[0].path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf());
    for item in &files[0] {
        match item {
            Item::Scene { body, .. } => a.block(body, scene_dir.as_deref(), &mut Vec::new()),
            Item::Component { name, params, body } => {
                a.tokens.push(Token { span: name.span.clone(), role: Role::Component, declaration: true, def: a.comps.get(&name.name).copied() });
                let mut scope: Vec<(String, usize)> = Vec::new();
                for p in params {
                    scope.push((p.name.clone(), a.defs.len()));
                    a.def(p, Role::Parameter, "parameter".into());
                }
                a.block(body, None, &mut scope);
            }
            Item::Import { .. } => {}
        }
    }
    a.tokens.sort_by_key(|t| t.span.start);
    // The scene's imports, in the order load_sources added them.
    let mut imports = Vec::new();
    for item in &files[0] {
        if let Item::Import { path: rel, span } = item {
            let target = if rel.starts_with("std:") {
                sources.iter().position(|s| s.name == *rel)
            } else {
                let p = scene_dir.as_ref().and_then(|d| d.join(rel).canonicalize().ok());
                sources.iter().position(|s| s.path.is_some() && s.path == p)
            };
            if let Some(t) = target {
                imports.push((span.clone(), t));
            }
        }
    }
    Ok(Analysis { sources, diags, defs: a.defs, tokens: a.tokens, imports })
}

struct Walker {
    defs: Vec<Def>,
    tokens: Vec<Token>,
    comps: HashMap<String, usize>,
}

impl Walker {
    /// A definition in the scene file, with its declaration token.
    fn def(&mut self, w: &Word, role: Role, detail: String) -> usize {
        let i = self.defs.len();
        self.defs.push(Def { name: w.name.clone(), role, file: 0, span: w.span.clone(), detail, doc: String::new() });
        self.tokens.push(Token { span: w.span.clone(), role, declaration: true, def: Some(i) });
        i
    }

    fn find(&self, name: &str, role: Role) -> Option<usize> {
        self.defs.iter().rposition(|d| d.name == name && d.role == role && d.file == 0)
    }

    fn block(&mut self, stmts: &[Stmt], dir: Option<&Path>, scope: &mut Vec<(String, usize)>) {
        let depth = scope.len();
        for s in stmts {
            match s {
                Stmt::Let { name, value } => {
                    self.expr(value, scope);
                    let i = self.def(name, Role::Variable, "let".into());
                    scope.push((name.name.clone(), i));
                }
                Stmt::Wait(e) | Stmt::Setting { value: e, .. } => self.expr(e, scope),
                Stmt::Repeat { count, var, body } => {
                    self.expr(count, scope);
                    let i = self.def(var, Role::Variable, "repeat counter".into());
                    scope.push((var.name.clone(), i));
                    self.block(body, dir, scope);
                    scope.pop();
                }
                Stmt::Cmd { timing, head, target, props } => {
                    if let Some((crate::parse::Timing::At(e) | crate::parse::Timing::After(Some(e)), _)) = timing {
                        self.expr(e, scope);
                    }
                    let comp = self.comps.get(&head.name).copied();
                    if let Some(c) = comp {
                        self.tokens.push(Token { span: head.span.clone(), role: Role::Component, declaration: false, def: Some(c) });
                    }
                    let is_decl = comp.is_some() || crate::compile::SHAPES.contains(&head.name.as_str());
                    if is_decl {
                        if let Some((w, idx)) = target.segs.first() {
                            let detail = match comp {
                                Some(c) => self.defs[c].detail.clone(),
                                None => head.name.clone(),
                            };
                            self.def(w, Role::Object, detail);
                            if let Some(e) = idx {
                                self.expr(e, scope);
                            }
                        }
                        if head.name == "model" {
                            self.model_parts(target, props, dir);
                        }
                    } else {
                        self.path(target, scope);
                    }
                    for p in props {
                        self.expr(&p.value, scope);
                    }
                }
            }
        }
        scope.truncate(depth);
    }

    /// Declare `name.part` for every part of a model, so completion and hover know them.
    fn model_parts(&mut self, target: &NamePath, props: &[crate::parse::Prop], dir: Option<&Path>) {
        let (Some((w, _)), Some(dir)) = (target.segs.first(), dir) else { return };
        let Some(Expr::Str(rel, _)) = props.iter().find(|p| p.key.name == "content").map(|p| &p.value) else { return };
        let Ok(path) = dir.join(rel).canonicalize() else { return };
        if !path.starts_with(dir) || path.extension().is_none_or(|e| e != "glb") {
            return;
        }
        if let Ok(mesh) = crowmation_mesh::load(&path.to_string_lossy()) {
            for p in &mesh.parts {
                let name = format!("{}.{}", w.name, p.name);
                self.defs.push(Def { name, role: Role::Part, file: 0, span: w.span.clone(), detail: format!("part of model `{}`", w.name), doc: String::new() });
            }
        }
    }

    /// An action target like `row.cell[i]`: the object, then its parts.
    fn path(&mut self, p: &NamePath, scope: &mut Vec<(String, usize)>) {
        for (k, (w, idx)) in p.segs.iter().enumerate() {
            let (role, def) = if k == 0 {
                (Role::Object, self.find(&w.name, Role::Object))
            } else {
                let full: Vec<&str> = p.segs[..=k].iter().map(|s| s.0.name.as_str()).collect();
                (Role::Part, self.find(&full.join("."), Role::Part))
            };
            self.tokens.push(Token { span: w.span.clone(), role, declaration: false, def });
            if let Some(e) = idx {
                self.expr(e, scope);
            }
        }
    }

    fn expr(&mut self, e: &Expr, scope: &mut Vec<(String, usize)>) {
        match e {
            Expr::Ident(w) => {
                if let Some((_, d)) = scope.iter().rev().find(|(n, _)| *n == w.name) {
                    let role = self.defs[*d].role;
                    self.tokens.push(Token { span: w.span.clone(), role, declaration: false, def: Some(*d) });
                } else if let Some(d) = self.find(&w.name, Role::Object) {
                    // An object named in a value: `below title`, `pin box`.
                    self.tokens.push(Token { span: w.span.clone(), role: Role::Object, declaration: false, def: Some(d) });
                } else if w.name.contains('.') && w.span.len() == w.name.len() {
                    // `router.flash_chip` in a `pin`: the object, then the part.
                    let dot = w.name.find('.').unwrap();
                    let obj = &w.name[..dot];
                    let (s, e) = (w.span.start, w.span.end);
                    self.tokens.push(Token { span: s..s + dot, role: Role::Object, declaration: false, def: self.find(obj, Role::Object) });
                    self.tokens.push(Token { span: s + dot + 1..e, role: Role::Part, declaration: false, def: self.find(&w.name, Role::Part) });
                }
            }
            Expr::Tuple(items, _) | Expr::List(items, _) | Expr::Call(_, items, _) => items.iter().for_each(|i| self.expr(i, scope)),
            Expr::Neg(i, _) => self.expr(i, scope),
            Expr::Bin(_, l, r, _) | Expr::Index(l, r, _) => {
                self.expr(l, scope);
                self.expr(r, scope);
            }
            Expr::Num(..) | Expr::Color(..) | Expr::Str(..) => {}
        }
    }
}

/// `//` comment lines directly above the line containing `offset`.
pub(crate) fn doc_comment(text: &str, offset: usize) -> String {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let mut lines: Vec<&str> = text[..line_start].lines().rev().map(str::trim).take_while(|l| l.starts_with("//")).collect();
    lines.reverse();
    lines.iter().map(|l| l.trim_start_matches('/').trim()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_imported_components_objects_and_parts() {
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/overflow.cwm"));
        let text = std::fs::read_to_string(path).unwrap();
        let a = analyze(path, &text).unwrap();
        assert!(a.diags.is_empty());
        let at = |needle: &str| a.token_at(text.find(needle).unwrap()).unwrap_or_else(|| panic!("no token at {needle}"));
        // `byte_row` comes from lib/memory.cwm.
        let t = at("byte_row buf");
        assert_eq!(t.role, Role::Component);
        let def = &a.defs[t.def.unwrap()];
        assert!(def.detail == "component byte_row(n, fill)" && a.sources[def.file].name.ends_with("memory.cwm"));
        assert!(def.doc.contains("row of n byte cells"), "{:?}", def.doc);
        // `ret.cell[i]` in an action: object then part.
        let t = at("ret.cell[i] to");
        assert_eq!((t.role, a.defs[t.def.unwrap()].detail.as_str()), (Role::Object, "component byte_row(n, fill)"));
        assert_eq!(a.imports.len(), 1);

        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/router.cwm"));
        let text = std::fs::read_to_string(path).unwrap();
        let a = analyze(path, &text).unwrap();
        assert!(a.defs.iter().any(|d| d.name == "router.flash_chip" && d.role == Role::Part));
        let t = a.token_at(text.find("flash_chip at").unwrap()).unwrap();
        assert_eq!(t.role, Role::Part);
        assert!(t.def.is_some());
    }

    #[test]
    fn works_with_errors_elsewhere() {
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/overflow.cwm"));
        let text = std::fs::read_to_string(path).unwrap().replace("scale ret.cell[i]", "scael ret.cell[i]");
        let a = analyze(path, &text).unwrap();
        assert!(!a.diags.is_empty());
        assert_eq!(a.token_at(text.find("byte_row buf").unwrap()).unwrap().role, Role::Component);
    }
}
