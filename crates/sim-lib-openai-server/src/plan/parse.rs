use sim_kernel::{Error, Expr, Result, Symbol};

use crate::plan::combinators::plan_symbol;

/// Parses the textual plan surface syntax into a plan [`Expr`], erroring on
/// trailing input or malformed combinators.
pub fn parse_plan(input: &str) -> Result<Expr> {
    let mut parser = PlanParser::new(input);
    let plan = parser.parse_plan()?;
    parser.skip_ws();
    if parser.is_done() {
        Ok(plan)
    } else {
        Err(Error::Eval(format!(
            "unexpected trailing plan input near {:?}",
            parser.remaining()
        )))
    }
}

struct PlanParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> PlanParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse_plan(&mut self) -> Result<Expr> {
        self.skip_ws();
        let start = self.pos;
        let name = self.parse_ident()?;
        self.skip_ws();
        if self.peek() == Some('(') {
            self.bump();
            return self.parse_combinator(name);
        }
        self.pos = start;
        self.parse_atom()
    }

    fn parse_combinator(&mut self, name: String) -> Result<Expr> {
        let mut args = Vec::new();
        self.skip_ws();
        if self.peek() == Some(')') {
            self.bump();
            return Ok(Expr::List(vec![Expr::Symbol(plan_symbol(&name))]));
        }

        loop {
            args.push(self.parse_arg()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.bump();
                    self.skip_ws();
                }
                Some(')') => {
                    self.bump();
                    break;
                }
                _ => {
                    return Err(Error::Eval(format!(
                        "expected ',' or ')' in plan combinator near {:?}",
                        self.remaining()
                    )));
                }
            }
        }

        let mut items = Vec::with_capacity(args.len() + 1);
        items.push(Expr::Symbol(plan_symbol(&name)));
        items.extend(args);
        Ok(Expr::List(items))
    }

    fn parse_arg(&mut self) -> Result<Expr> {
        self.skip_ws();
        let start = self.pos;
        if let Ok(name) = self.parse_ident() {
            self.skip_ws();
            if self.peek() == Some(':') {
                self.bump();
                let value = self.parse_plan_or_literal()?;
                return Ok(keyword_expr(name, value));
            }
        }
        self.pos = start;
        self.parse_plan()
    }

    fn parse_plan_or_literal(&mut self) -> Result<Expr> {
        self.skip_ws();
        let start = self.pos;
        if self.parse_ident().is_ok() {
            self.skip_ws();
            if self.peek() == Some('(') {
                self.pos = start;
                return self.parse_plan();
            }
        }
        self.pos = start;
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr> {
        self.skip_ws();
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == ',' || ch == ')' {
                break;
            }
            self.bump();
        }
        let atom = self.input[start..self.pos].trim();
        if atom.is_empty() {
            return Err(Error::Eval("plan atom cannot be empty".to_owned()));
        }
        if atom.contains('(') {
            return Err(Error::Eval(format!("invalid plan atom {atom:?}")));
        }
        Ok(Expr::List(vec![
            Expr::Symbol(plan_symbol("atom")),
            Expr::String(atom.to_owned()),
        ]))
    }

    fn parse_ident(&mut self) -> Result<String> {
        self.skip_ws();
        let mut chars = self.remaining().char_indices();
        let Some((_, first)) = chars.next() else {
            return Err(Error::Eval("expected plan identifier".to_owned()));
        };
        if !first.is_ascii_alphabetic() {
            return Err(Error::Eval("expected plan identifier".to_owned()));
        }
        let mut end = self.pos + first.len_utf8();
        for (offset, ch) in chars {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                end = self.pos + offset + ch.len_utf8();
            } else {
                break;
            }
        }
        let ident = self.input[self.pos..end].to_owned();
        self.pos = end;
        Ok(ident)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_ascii_whitespace()) {
            self.bump();
        }
    }

    fn is_done(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn bump(&mut self) {
        if let Some(ch) = self.peek() {
            self.pos += ch.len_utf8();
        }
    }
}

fn keyword_expr(name: String, value: Expr) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("keyword")),
            Expr::Symbol(Symbol::new(name)),
        ),
        (Expr::Symbol(Symbol::new("value")), value),
    ])
}
