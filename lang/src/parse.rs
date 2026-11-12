//! Recursive descent parser. Statements are one per line; on an error the
//! parser reports it, skips to the next line and keeps going.

use crate::lex::Tok;
use crate::{Diag, Span};

#[derive(Clone, Debug)]
pub struct Word {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Num(f64, Option<String>, Span),
    Ident(Word),
    Color([f32; 4], Span),
    Str(String, Span),
    Tuple(Vec<Expr>, Span),
    Neg(Box<Expr>, Span),
    Bin(Op, Box<Expr>, Box<Expr>, Span),
    /// `[a, b, c]`, may span lines.
    List(Vec<Expr>, Span),
    /// `xs[i]`
    Index(Box<Expr>, Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Ident(w) => w.span.clone(),
            Expr::Num(.., s) | Expr::Color(_, s) | Expr::Str(_, s) | Expr::Tuple(_, s) | Expr::Neg(_, s) | Expr::Bin(.., s) | Expr::List(_, s) | Expr::Index(.., s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Prop {
    pub key: Word,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum Timing {
    At(Expr),
    After(Option<Expr>),
    With,
}

/// Object name like `row.c[3]`: dot separated parts, each with an optional index.
#[derive(Clone, Debug)]
pub struct Path {
    pub segs: Vec<(Word, Option<Expr>)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let { name: Word, value: Expr },
    Wait(Expr),
    Setting { key: Word, value: Expr },
    Repeat { count: Expr, var: Word, body: Vec<Stmt> },
    /// `[timing] head target props...`: an object, component instance or action.
    /// A string right after the target is shorthand for `content "..."`.
    Cmd { timing: Option<(Timing, Span)>, head: Word, target: Path, props: Vec<Prop> },
}

#[derive(Clone, Debug)]
pub enum Item {
    Scene { body: Vec<Stmt>, span: Span },
    Component { name: Word, params: Vec<Word>, body: Vec<Stmt> },
    Import { path: String, span: Span },
}

pub const SETTINGS: &[&str] = &["size", "fps", "background", "duration"];

pub fn parse(toks: &[(Tok, Span)], diags: &mut Vec<Diag>) -> Vec<Item> {
    let mut p = Parser { toks, pos: 0, diags };
    let mut items = Vec::new();
    loop {
        p.skip_newlines();
        if *p.peek() == Tok::Eof {
            break;
        }
        match p.item() {
            Ok(i) => items.push(i),
            Err(d) => {
                p.diags.push(d);
                p.recover();
                if p.peek() == &Tok::RBrace {
                    p.pos += 1;
                }
            }
        }
    }
    items
}

struct Parser<'a> {
    toks: &'a [(Tok, Span)],
    pos: usize,
    diags: &'a mut Vec<Diag>,
}

type R<T> = Result<T, Diag>;

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }

    fn span(&self) -> Span {
        self.toks[self.pos].1.clone()
    }

    fn bump(&mut self) -> (Tok, Span) {
        let t = self.toks[self.pos].clone();
        if t.0 != Tok::Eof {
            self.pos += 1;
        }
        t
    }

    fn prev_end(&self) -> usize {
        self.toks[self.pos.saturating_sub(1)].1.end
    }

    fn expect(&mut self, want: Tok, what: &str) -> R<Span> {
        if *self.peek() == want {
            Ok(self.bump().1)
        } else {
            Err(Diag::new(self.span(), format!("expected {what}, found {}", self.peek().describe())))
        }
    }

    fn word(&mut self, what: &str) -> R<Word> {
        match self.peek().clone() {
            Tok::Ident(name) => Ok(Word { name, span: self.bump().1 }),
            t => Err(Diag::new(self.span(), format!("expected {what}, found {}", t.describe()))),
        }
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.pos += 1;
        }
    }

    /// Skip to the start of the next line, or stop before a closing brace.
    /// Blocks opened on the skipped line are skipped whole.
    fn recover(&mut self) {
        let mut depth = 0;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::RBrace if depth == 0 => return,
                Tok::Newline if depth == 0 => {
                    self.pos += 1;
                    return;
                }
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn item(&mut self) -> R<Item> {
        let kw = self.word("`scene`, `component` or `import`")?;
        match kw.name.as_str() {
            "scene" => {
                // ponytail: scene name parsed but unused; use it for the default output file name in `render`
                if let Tok::Str(_) = self.peek() {
                    self.bump();
                }
                Ok(Item::Scene { body: self.block()?, span: kw.span })
            }
            "component" => {
                let name = self.word("a component name")?;
                self.expect(Tok::LParen, "`(`")?;
                let mut params = Vec::new();
                while *self.peek() != Tok::RParen {
                    params.push(self.word("a parameter name")?);
                    if *self.peek() != Tok::Comma {
                        break;
                    }
                    self.bump();
                }
                self.expect(Tok::RParen, "`)`")?;
                Ok(Item::Component { name, params, body: self.block()? })
            }
            "import" => match self.bump() {
                (Tok::Str(path), span) => Ok(Item::Import { path, span }),
                (t, span) => Err(Diag::new(span, format!("expected a file path in quotes, found {}", t.describe()))),
            },
            _ => Err(Diag::new(kw.span, format!("expected `scene`, `component` or `import`, found `{}`", kw.name))),
        }
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    return Ok(body);
                }
                Tok::Eof => return Err(Diag::new(self.span(), "missing `}`")),
                _ => match self.stmt() {
                    Ok(s) => body.push(s),
                    Err(d) => {
                        self.diags.push(d);
                        self.recover();
                    }
                },
            }
        }
    }

    fn stmt(&mut self) -> R<Stmt> {
        let start = self.span().start;
        let head = self.word("a statement")?;
        let stmt = match head.name.as_str() {
            "let" => {
                let name = self.word("a name")?;
                self.expect(Tok::Eq, "`=`")?;
                Stmt::Let { name, value: self.expr()? }
            }
            "wait" => Stmt::Wait(self.expr()?),
            "repeat" => {
                let count = self.expr()?;
                let kw = self.word("`as`")?;
                if kw.name != "as" {
                    return Err(Diag::new(kw.span, format!("expected `as`, found `{}`", kw.name)));
                }
                let var = self.word("a loop variable")?;
                Stmt::Repeat { count, var, body: self.block()? }
            }
            "at" | "after" | "with" => {
                let timing = match head.name.as_str() {
                    "at" => Timing::At(self.expr()?),
                    "after" if matches!(self.peek(), Tok::Num(..) | Tok::Minus) => Timing::After(Some(self.expr()?)),
                    "after" => Timing::After(None),
                    _ => Timing::With,
                };
                let span = start..self.prev_end();
                let head = self.word("an action")?;
                self.cmd(Some((timing, span)), head)?
            }
            s if SETTINGS.contains(&s) => Stmt::Setting { key: head, value: self.expr()? },
            _ => self.cmd(None, head)?,
        };
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(stmt)
            }
            Tok::RBrace | Tok::Eof => Ok(stmt),
            t => Err(Diag::new(self.span(), format!("unexpected {} after statement", t.describe()))),
        }
    }

    fn cmd(&mut self, timing: Option<(Timing, Span)>, head: Word) -> R<Stmt> {
        let target = self.path()?;
        let mut props = Vec::new();
        if let Tok::Str(text) = self.peek().clone() {
            let span = self.bump().1;
            props.push(Prop { key: Word { name: "content".into(), span: span.clone() }, value: Expr::Str(text, span) });
        }
        while let Tok::Ident(_) = self.peek() {
            let key = self.word("a property")?;
            props.push(Prop { key, value: self.expr()? });
        }
        Ok(Stmt::Cmd { timing, head, target, props })
    }

    fn path(&mut self) -> R<Path> {
        let start = self.span().start;
        let mut segs = Vec::new();
        loop {
            let w = self.word("a name")?;
            let index = if *self.peek() == Tok::LBracket {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::RBracket, "`]`")?;
                Some(e)
            } else {
                None
            };
            segs.push((w, index));
            if *self.peek() != Tok::Dot {
                return Ok(Path { segs, span: start..self.prev_end() });
            }
            self.bump();
        }
    }

    fn expr(&mut self) -> R<Expr> {
        let mut l = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => Op::Add,
                Tok::Minus => Op::Sub,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.term()?;
            let span = l.span().start..r.span().end;
            l = Expr::Bin(op, Box::new(l), Box::new(r), span);
        }
    }

    fn term(&mut self) -> R<Expr> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => Op::Mul,
                Tok::Slash => Op::Div,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.unary()?;
            let span = l.span().start..r.span().end;
            l = Expr::Bin(op, Box::new(l), Box::new(r), span);
        }
    }

    fn unary(&mut self) -> R<Expr> {
        if *self.peek() == Tok::Minus {
            let s = self.bump().1;
            let e = self.unary()?;
            let span = s.start..e.span().end;
            return Ok(Expr::Neg(Box::new(e), span));
        }
        let mut e = self.atom()?;
        while *self.peek() == Tok::LBracket {
            self.bump();
            let i = self.expr()?;
            let end = self.expect(Tok::RBracket, "`]`")?.end;
            let span = e.span().start..end;
            e = Expr::Index(Box::new(e), Box::new(i), span);
        }
        Ok(e)
    }

    fn atom(&mut self) -> R<Expr> {
        let (tok, span) = self.bump();
        Ok(match tok {
            Tok::Num(n, u) => Expr::Num(n, u, span),
            Tok::Ident(mut name) => {
                // Dotted names like `router.flash_chip` refer to parts.
                let mut end = span.end;
                while *self.peek() == Tok::Dot && matches!(self.toks.get(self.pos + 1), Some((Tok::Ident(_), _))) {
                    self.bump();
                    let (Tok::Ident(part), s) = self.bump() else { unreachable!() };
                    name = format!("{name}.{part}");
                    end = s.end;
                }
                Expr::Ident(Word { name, span: span.start..end })
            }
            Tok::Color(c) => Expr::Color(c, span),
            Tok::Str(s) => Expr::Str(s, span),
            Tok::LParen => {
                let mut items = vec![self.expr()?];
                while *self.peek() == Tok::Comma {
                    self.bump();
                    items.push(self.expr()?);
                }
                let end = self.expect(Tok::RParen, "`)`")?.end;
                if items.len() == 1 { items.pop().unwrap() } else { Expr::Tuple(items, span.start..end) }
            }
            Tok::LBracket => {
                let mut items = Vec::new();
                self.skip_newlines();
                while *self.peek() != Tok::RBracket {
                    items.push(self.expr()?);
                    self.skip_newlines();
                    if *self.peek() != Tok::Comma {
                        break;
                    }
                    self.bump();
                    self.skip_newlines();
                }
                let end = self.expect(Tok::RBracket, "`,` or `]`")?.end;
                Expr::List(items, span.start..end)
            }
            t => {
                self.pos -= (t != Tok::Eof) as usize;
                return Err(Diag::new(span, format!("expected a value, found {}", t.describe())));
            }
        })
    }
}
