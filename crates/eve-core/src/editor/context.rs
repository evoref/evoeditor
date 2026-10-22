//! `ContextExpr`: コマンドの `when` 句とキーマップの条件式 (docs/02 §6.1、docs/07 §7.1 / §11.6)。
//!
//! 文法: `or := and ('||' and)*`、`and := not ('&&' not)*`、`not := '!' not | cmp`、
//! `cmp := atom (('==' | '!=') 文字列)?`、`atom := '(' or ')' | 識別子 | true | false`。
//! 識別子 (`doc.read_only` など) の値は、評価のたびに呼び出し側が渡す関数で引く。知らない識別子は偽。

use std::fmt;

/// 識別子の値。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtxValue {
    Bool(bool),
    Str(String),
}

impl CtxValue {
    fn truthy(&self) -> bool {
        match self {
            CtxValue::Bool(b) => *b,
            CtxValue::Str(s) => !s.is_empty(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Const(bool),
    Var(String),
    /// `ident == "文字列"` / `ident != "文字列"`
    Cmp {
        var: String,
        value: String,
        equal: bool,
    },
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
}

/// 構文木。空の式 (`—`) は常に真。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextExpr(Option<Node>);

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    pub pos: usize,
    pub message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "条件式の {} 文字目: {}", self.pos, self.message)
    }
}

impl std::error::Error for ParseError {}

impl ContextExpr {
    /// 常に真の式。
    pub fn always() -> ContextExpr {
        ContextExpr(None)
    }

    pub fn parse(src: &str) -> Result<ContextExpr, ParseError> {
        if src.trim().is_empty() {
            return Ok(ContextExpr(None));
        }
        let mut p = Parser { src, pos: 0 };
        let node = p.or()?;
        p.skip_ws();
        if p.pos != src.len() {
            return Err(p.err("余分な文字"));
        }
        Ok(ContextExpr(Some(node)))
    }

    /// 式に現れる識別子 (出現順。重複あり)。
    pub fn vars(&self) -> Vec<String> {
        fn walk(n: &Node, out: &mut Vec<String>) {
            match n {
                Node::Const(_) => {}
                Node::Var(v) | Node::Cmp { var: v, .. } => out.push(v.clone()),
                Node::Not(x) => walk(x, out),
                Node::And(a, b) | Node::Or(a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(n) = &self.0 {
            walk(n, &mut out);
        }
        out
    }

    /// `lookup` で識別子の値を引いて評価する。
    pub fn eval(&self, lookup: &dyn Fn(&str) -> Option<CtxValue>) -> bool {
        self.0.as_ref().is_none_or(|n| eval(n, lookup))
    }
}

fn eval(n: &Node, lookup: &dyn Fn(&str) -> Option<CtxValue>) -> bool {
    match n {
        Node::Const(b) => *b,
        Node::Var(v) => lookup(v).is_some_and(|x| x.truthy()),
        Node::Cmp { var, value, equal } => {
            let got = match lookup(var) {
                Some(CtxValue::Str(s)) => s,
                Some(CtxValue::Bool(b)) => b.to_string(),
                None => String::new(),
            };
            (got == *value) == *equal
        }
        Node::Not(x) => !eval(x, lookup),
        Node::And(a, b) => eval(a, lookup) && eval(b, lookup),
        Node::Or(a, b) => eval(a, lookup) || eval(b, lookup),
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, message: &'static str) -> ParseError {
        ParseError {
            pos: self.src[..self.pos].chars().count(),
            message,
        }
    }

    fn skip_ws(&mut self) {
        while self.src[self.pos..].starts_with(|c: char| c.is_whitespace()) {
            self.pos += self.src[self.pos..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
        }
    }

    fn eat(&mut self, tok: &str) -> bool {
        self.skip_ws();
        if self.src[self.pos..].starts_with(tok) {
            self.pos += tok.len();
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Node, ParseError> {
        let mut left = self.and()?;
        while self.eat("||") {
            left = Node::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, ParseError> {
        let mut left = self.not()?;
        while self.eat("&&") {
            left = Node::And(Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Node, ParseError> {
        self.skip_ws();
        // `!=` は比較の演算子。式の先頭の `!` だけが否定
        if self.src[self.pos..].starts_with('!') && !self.src[self.pos..].starts_with("!=") {
            self.pos += 1;
            return Ok(Node::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Node, ParseError> {
        let atom = self.atom()?;
        let Node::Var(var) = &atom else {
            return Ok(atom);
        };
        let equal = if self.eat("==") {
            true
        } else if self.eat("!=") {
            false
        } else {
            return Ok(atom);
        };
        let value = self.string()?;
        Ok(Node::Cmp {
            var: var.clone(),
            value,
            equal,
        })
    }

    fn atom(&mut self) -> Result<Node, ParseError> {
        if self.eat("(") {
            let n = self.or()?;
            if !self.eat(")") {
                return Err(self.err("`)` が必要"));
            }
            return Ok(n);
        }
        self.skip_ws();
        let rest = &self.src[self.pos..];
        let len = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        if len == 0 {
            return Err(self.err("識別子が必要"));
        }
        let ident = &rest[..len];
        self.pos += len;
        Ok(match ident {
            "true" => Node::Const(true),
            "false" => Node::Const(false),
            _ => Node::Var(ident.to_owned()),
        })
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        if !self.src[self.pos..].starts_with('"') {
            return Err(self.err("`\"` で囲んだ文字列が必要"));
        }
        let rest = &self.src[self.pos + 1..];
        let Some(end) = rest.find('"') else {
            return Err(self.err("文字列が閉じていない"));
        };
        let s = rest[..end].to_owned();
        self.pos += end + 2;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_with(src: &str, vars: &[(&str, CtxValue)]) -> bool {
        let e = ContextExpr::parse(src).unwrap();
        e.eval(&|name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        })
    }

    fn b(v: bool) -> CtxValue {
        CtxValue::Bool(v)
    }

    #[test]
    fn 空の式は常に真() {
        assert!(ContextExpr::parse("").unwrap().eval(&|_| None));
        assert!(ContextExpr::parse("  ").unwrap().eval(&|_| None));
        assert!(ContextExpr::always().eval(&|_| None));
    }

    #[test]
    fn 論理演算と優先順() {
        let v = [("a", b(true)), ("b", b(false))];
        assert!(eval_with("a", &v));
        assert!(!eval_with("b", &v));
        assert!(eval_with("!b", &v));
        assert!(!eval_with("a && b", &v));
        assert!(eval_with("a || b", &v));
        assert!(eval_with("b || a && a", &v), "&& は || より強い");
        assert!(!eval_with("(b || a) && b", &v));
        assert!(eval_with("!(a && b)", &v));
        assert!(eval_with("!!a", &v));
    }

    #[test]
    fn 比較() {
        let v = [
            ("overlay.active", CtxValue::Str("dialog".into())),
            ("x", b(true)),
        ];
        assert!(eval_with("overlay.active == \"dialog\"", &v));
        assert!(!eval_with("overlay.active == \"list\"", &v));
        assert!(eval_with("overlay.active != \"\"", &v));
        assert!(eval_with("overlay.active != \"\" && x", &v));
        // 知らない識別子は空文字列として比較される
        assert!(eval_with("nothing == \"\"", &v));
        assert!(!eval_with("nothing", &v), "知らない識別子は偽");
    }

    #[test]
    fn カタログの_when_句を読める() {
        for src in [
            "!doc.read_only",
            "doc.has_path",
            "!selection.empty && !doc.read_only",
            "doc.read_only_releasable || !doc.read_only",
            "search.replace_active && !doc.read_only",
            "overlay.active == \"buffer_list\"",
            "frontend == \"tui\"",
            "caps.can_suspend",
        ] {
            ContextExpr::parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        }
    }

    #[test]
    fn 構文エラー() {
        for src in ["a &&", "(a", "a b", "== \"x\"", "a == b", "a == \"x"] {
            assert!(ContextExpr::parse(src).is_err(), "{src}");
        }
    }
}
