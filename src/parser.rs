//! A recursive descent parser with error recovery.
//!
//! The parser works on a slice of already lexed [`Token`]s so the incremental
//! layer can hand it just the tokens around an edit. It always makes progress
//! (every path either consumes a token or stops at a boundary) and it records
//! whether it ran out of tokens mid construct via [`ParseOutput::truncated`],
//! which the incremental layer uses to decide when a reused suffix has merged
//! into the edited region.

use crate::ast::*;
use crate::diagnostics::{DiagKind, Diagnostic};
use crate::lexer::{lex_at, Token, TokenKind};
use crate::span::Span;

pub struct ParseOutput {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
    /// True when parsing stopped because the token stream ended inside a
    /// construct that expected more input.
    pub truncated: bool,
}

/// One top level statement together with exactly the tokens it consumed and the
/// diagnostics its parse produced. Grouping by production (rather than by span)
/// keeps a diagnostic that points at the following statement attached to the
/// statement that actually emitted it, which the incremental analyser relies on
/// when it reuses unedited statements.
#[derive(Clone)]
pub struct ItemParse {
    pub stmt: Stmt,
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Lex and parse a whole source string.
pub fn parse(text: &str) -> ParseOutput {
    let tokens = lex_at(text, 0);
    parse_tokens(&tokens)
}

/// Parse a token slice into per statement groups. `truncated` is true when the
/// stream ended inside a construct.
pub fn parse_items(tokens: &[Token]) -> (Vec<ItemParse>, bool) {
    let mut p = Parser {
        tokens,
        pos: 0,
        diagnostics: Vec::new(),
        truncated: false,
    };
    let groups = p.parse_grouped();
    (groups, p.truncated)
}

/// Parse a token slice whose spans are already absolute.
pub fn parse_tokens(tokens: &[Token]) -> ParseOutput {
    let (groups, truncated) = parse_items(tokens);
    let mut program = Vec::with_capacity(groups.len());
    let mut diagnostics = Vec::new();
    for g in groups {
        program.push(g.stmt);
        diagnostics.extend(g.diagnostics);
    }
    ParseOutput {
        program,
        diagnostics,
        truncated,
    }
}

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    truncated: bool,
}

impl<'t> Parser<'t> {
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|t| t.kind)
    }

    fn is(&self, kind: TokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    fn bump(&mut self) -> Option<&'t Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// A zero width span positioned at the end of the last consumed token, used
    /// to anchor end of input diagnostics.
    fn eof_span(&self) -> Span {
        let end = self.tokens.last().map_or(0, |t| t.span.end);
        Span::new(end, end)
    }

    fn cur_span(&self) -> Span {
        self.peek().map_or_else(|| self.eof_span(), |t| t.span)
    }

    /// End offset of the last consumed token. Used to close a construct whose
    /// terminator is missing without referring to the global end of stream,
    /// which keeps a statement's span local to the tokens it consumed.
    fn prev_end(&self) -> u32 {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else {
            0
        }
    }

    fn error(&mut self, span: Span, kind: DiagKind, msg: impl Into<String>) {
        self.diagnostics.push(Diagnostic::error(span, kind, msg));
    }

    /// Consume a token of `kind` or record an error. Returns the token's span on
    /// success.
    fn expect(&mut self, kind: TokenKind, what: &str) -> Option<Span> {
        if self.is(kind) {
            let span = self.cur_span();
            self.bump();
            Some(span)
        } else if self.at_end() {
            self.truncated = true;
            self.error(
                self.eof_span(),
                DiagKind::UnexpectedEof,
                format!("expected {what}, found end of input"),
            );
            None
        } else {
            // Anchor the diagnostic at the end of the last consumed token rather
            // than at the current (following) token. The following token can
            // belong to the next statement, and pointing at it would make this
            // statement's diagnostics depend on the next one, which would break
            // incremental reuse.
            let at = self.prev_end();
            self.error(
                Span::new(at, at),
                DiagKind::ParseError,
                format!("expected {what}"),
            );
            None
        }
    }

    fn parse_grouped(&mut self) -> Vec<ItemParse> {
        let mut groups: Vec<ItemParse> = Vec::new();
        while !self.at_end() {
            let tok_start = self.pos;
            let diag_start = self.diagnostics.len();
            let stmt = self.parse_top_stmt();
            if self.pos == tok_start {
                // Guarantee progress on an unrecognised token.
                let span = self.cur_span();
                self.error(span, DiagKind::ParseError, "unexpected token");
                self.bump();
            }
            let tokens = self.tokens[tok_start..self.pos].to_vec();
            let diagnostics = self.diagnostics[diag_start..].to_vec();
            match stmt {
                Some(stmt) => groups.push(ItemParse {
                    stmt,
                    tokens,
                    diagnostics,
                }),
                None => {
                    if let Some(last) = groups.last_mut() {
                        last.tokens.extend(tokens);
                        last.diagnostics.extend(diagnostics);
                    }
                }
            }
        }
        groups
    }

    fn parse_top_stmt(&mut self) -> Option<Stmt> {
        match self.peek_kind() {
            Some(TokenKind::Fn) => self.parse_fn(),
            Some(TokenKind::Let) => self.parse_let(),
            _ => self.parse_expr_stmt_top(),
        }
    }

    fn parse_fn(&mut self) -> Option<Stmt> {
        let start = self.cur_span().start;
        self.bump(); // fn
        let name = self.parse_ident_or_missing("a function name");
        self.expect(TokenKind::LParen, "`(`");
        let mut params = Vec::new();
        if !self.is(TokenKind::RParen) {
            while let Some(id) = self.parse_ident("a parameter name") {
                params.push(id);
                if self.is(TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen, "`)`");
        let body = self.parse_block();
        let end = body.span.end;
        Some(Stmt {
            kind: StmtKind::Fn(FnDecl { name, params, body }),
            span: Span::new(start, end),
        })
    }

    fn parse_let(&mut self) -> Option<Stmt> {
        let start = self.cur_span().start;
        self.bump(); // let
        let name = self.parse_ident_or_missing("a variable name");
        self.expect(TokenKind::Eq, "`=`");
        let value = self.parse_expr();
        let semi = self.expect(TokenKind::Semi, "`;`");
        let end = semi.map_or(value.span.end, |s| s.end);
        Some(Stmt {
            kind: StmtKind::Let(LetStmt { name, value }),
            span: Span::new(start, end),
        })
    }

    fn parse_expr_stmt_top(&mut self) -> Option<Stmt> {
        let expr = self.parse_expr();
        let start = expr.span.start;
        let semi = self.expect(TokenKind::Semi, "`;`");
        let end = semi.map_or(expr.span.end, |s| s.end);
        if semi.is_none() {
            self.recover_to_stmt_boundary();
        }
        Some(Stmt {
            kind: StmtKind::Expr(expr),
            span: Span::new(start, end),
        })
    }

    fn recover_to_stmt_boundary(&mut self) {
        loop {
            match self.peek_kind() {
                // Ran out of tokens before reaching a boundary. In a reparsed
                // region this means the statement would have consumed further in
                // the full text, so flag truncation to extend the region.
                None => {
                    self.truncated = true;
                    break;
                }
                Some(TokenKind::Semi) => {
                    self.bump();
                    break;
                }
                Some(TokenKind::Fn | TokenKind::Let | TokenKind::RBrace) => break,
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// Like [`Parser::parse_ident`] but always yields an `Ident`, using an empty
    /// placeholder name at a zero width span when none is present. This keeps
    /// `fn` and `let` statements producing a node even on malformed input so
    /// every consumed token belongs to a group.
    fn parse_ident_or_missing(&mut self, what: &str) -> Ident {
        if self.is(TokenKind::Ident) {
            let t = self.bump().unwrap();
            Ident {
                name: t.text.clone(),
                span: t.span,
            }
        } else {
            self.expect(TokenKind::Ident, what);
            let at = self.cur_span().start;
            Ident {
                name: String::new(),
                span: Span::new(at, at),
            }
        }
    }

    fn parse_ident(&mut self, what: &str) -> Option<Ident> {
        if self.is(TokenKind::Ident) {
            let t = self.bump().unwrap();
            Some(Ident {
                name: t.text.clone(),
                span: t.span,
            })
        } else {
            self.expect(TokenKind::Ident, what);
            None
        }
    }

    fn parse_block(&mut self) -> Block {
        let open = self.expect(TokenKind::LBrace, "`{`");
        let start = open.map_or_else(|| self.cur_span().start, |s| s.start);
        let mut stmts = Vec::new();
        let mut tail = None;
        loop {
            if self.at_end() {
                self.truncated = true;
                break;
            }
            if self.is(TokenKind::RBrace) {
                break;
            }
            match self.peek_kind() {
                Some(TokenKind::Fn) => {
                    if let Some(s) = self.parse_fn() {
                        stmts.push(s);
                    }
                }
                Some(TokenKind::Let) => {
                    if let Some(s) = self.parse_let() {
                        stmts.push(s);
                    }
                }
                _ => {
                    let before = self.pos;
                    let expr = self.parse_expr();
                    if self.is(TokenKind::Semi) {
                        let semi = self.cur_span();
                        self.bump();
                        stmts.push(Stmt {
                            kind: StmtKind::Expr(expr.clone()),
                            span: Span::new(expr.span.start, semi.end),
                        });
                    } else if self.is(TokenKind::RBrace) || self.at_end() {
                        tail = Some(Box::new(expr));
                        break;
                    } else {
                        self.error(self.cur_span(), DiagKind::ParseError, "expected `;` or `}`");
                        self.recover_to_stmt_boundary();
                        stmts.push(Stmt {
                            kind: StmtKind::Expr(expr.clone()),
                            span: expr.span,
                        });
                    }
                    if self.pos == before {
                        let span = self.cur_span();
                        self.error(span, DiagKind::ParseError, "unexpected token");
                        self.bump();
                    }
                }
            }
        }
        let close = self.expect(TokenKind::RBrace, "`}`");
        let end = close
            .map(|s| s.end)
            .or_else(|| tail.as_ref().map(|t| t.span.end))
            .or_else(|| stmts.last().map(|s| s.span.end))
            .unwrap_or(start);
        Block {
            stmts,
            tail,
            span: Span::new(start, end),
        }
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_equality()
    }

    fn parse_equality(&mut self) -> Expr {
        let mut lhs = self.parse_comparison();
        while let Some(op) = match self.peek_kind() {
            Some(TokenKind::EqEq) => Some(BinOp::Eq),
            Some(TokenKind::Ne) => Some(BinOp::Ne),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_comparison();
            lhs = binary(lhs, op, rhs);
        }
        lhs
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut lhs = self.parse_additive();
        while let Some(op) = match self.peek_kind() {
            Some(TokenKind::Lt) => Some(BinOp::Lt),
            Some(TokenKind::Le) => Some(BinOp::Le),
            Some(TokenKind::Gt) => Some(BinOp::Gt),
            Some(TokenKind::Ge) => Some(BinOp::Ge),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_additive();
            lhs = binary(lhs, op, rhs);
        }
        lhs
    }

    fn parse_additive(&mut self) -> Expr {
        let mut lhs = self.parse_multiplicative();
        while let Some(op) = match self.peek_kind() {
            Some(TokenKind::Plus) => Some(BinOp::Add),
            Some(TokenKind::Minus) => Some(BinOp::Sub),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_multiplicative();
            lhs = binary(lhs, op, rhs);
        }
        lhs
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut lhs = self.parse_unary();
        while let Some(op) = match self.peek_kind() {
            Some(TokenKind::Star) => Some(BinOp::Mul),
            Some(TokenKind::Slash) => Some(BinOp::Div),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_unary();
            lhs = binary(lhs, op, rhs);
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let op = match self.peek_kind() {
            Some(TokenKind::Minus) => Some(UnOp::Neg),
            Some(TokenKind::Bang) => Some(UnOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.cur_span().start;
            self.bump();
            let expr = self.parse_unary();
            let span = Span::new(start, expr.span.end);
            Expr {
                kind: ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span,
            }
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        while self.is(TokenKind::LParen) {
            self.bump();
            let mut args = Vec::new();
            if !self.is(TokenKind::RParen) {
                loop {
                    args.push(self.parse_expr());
                    if self.is(TokenKind::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            let close = self.expect(TokenKind::RParen, "`)`");
            let end = close.map_or_else(|| self.prev_end(), |s| s.end);
            let span = Span::new(expr.span.start, end);
            expr = Expr {
                kind: ExprKind::Call {
                    callee: Box::new(expr),
                    args,
                },
                span,
            };
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek_kind() {
            Some(TokenKind::Int) => {
                let t = self.bump().unwrap();
                let value = t.text.parse::<i64>().unwrap_or(0);
                Expr {
                    kind: ExprKind::Int(value),
                    span: t.span,
                }
            }
            Some(TokenKind::True) => {
                let t = self.bump().unwrap();
                Expr {
                    kind: ExprKind::Bool(true),
                    span: t.span,
                }
            }
            Some(TokenKind::False) => {
                let t = self.bump().unwrap();
                Expr {
                    kind: ExprKind::Bool(false),
                    span: t.span,
                }
            }
            Some(TokenKind::Ident) => {
                let t = self.bump().unwrap();
                Expr {
                    kind: ExprKind::Name(t.text.clone()),
                    span: t.span,
                }
            }
            Some(TokenKind::LParen) => {
                let start = self.cur_span().start;
                self.bump();
                let inner = self.parse_expr();
                let close = self.expect(TokenKind::RParen, "`)`");
                let end = close.map_or(inner.span.end, |s| s.end);
                Expr {
                    kind: ExprKind::Paren(Box::new(inner)),
                    span: Span::new(start, end),
                }
            }
            Some(TokenKind::LBrace) => {
                let block = self.parse_block();
                let span = block.span;
                Expr {
                    kind: ExprKind::Block(block),
                    span,
                }
            }
            Some(TokenKind::If) => self.parse_if(),
            _ => {
                // An expression was required but none is here.
                if self.at_end() {
                    self.truncated = true;
                    self.error(
                        self.eof_span(),
                        DiagKind::UnexpectedEof,
                        "expected an expression",
                    );
                    Expr {
                        kind: ExprKind::Error,
                        span: self.eof_span(),
                    }
                } else {
                    let span = self.cur_span();
                    self.error(span, DiagKind::ParseError, "expected an expression");
                    self.bump();
                    Expr {
                        kind: ExprKind::Error,
                        span,
                    }
                }
            }
        }
    }

    fn parse_if(&mut self) -> Expr {
        let start = self.cur_span().start;
        self.bump(); // if
        let cond = self.parse_expr();
        let then_block = self.parse_block();
        let mut end = then_block.span.end;
        let else_block = if self.is(TokenKind::Else) {
            self.bump();
            let b = self.parse_block();
            end = b.span.end;
            Some(b)
        } else {
            None
        };
        Expr {
            kind: ExprKind::If {
                cond: Box::new(cond),
                then_block,
                else_block,
            },
            span: Span::new(start, end),
        }
    }
}

fn binary(lhs: Expr, op: BinOp, rhs: Expr) -> Expr {
    let span = Span::new(lhs.span.start, rhs.span.end);
    Expr {
        kind: ExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_let_and_fn() {
        let out = parse("let x = 1 + 2;\nfn f(a, b) { a + b }");
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(out.program.len(), 2);
        assert!(matches!(out.program[0].kind, StmtKind::Let(_)));
        assert!(matches!(out.program[1].kind, StmtKind::Fn(_)));
    }

    #[test]
    fn precedence_is_correct() {
        let out = parse("let x = 1 + 2 * 3;");
        if let StmtKind::Let(l) = &out.program[0].kind {
            if let ExprKind::Binary { op, rhs, .. } = &l.value.kind {
                assert_eq!(*op, BinOp::Add);
                assert!(matches!(rhs.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
                return;
            }
        }
        panic!("unexpected shape");
    }

    #[test]
    fn missing_semicolon_reports_error() {
        let out = parse("let x = 1");
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagKind::UnexpectedEof));
        assert!(out.truncated);
    }

    #[test]
    fn block_tail_expression() {
        let out = parse("fn f() { let a = 1; a }");
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        if let StmtKind::Fn(f) = &out.program[0].kind {
            assert!(f.body.tail.is_some());
            assert_eq!(f.body.stmts.len(), 1);
            return;
        }
        panic!("expected fn");
    }

    #[test]
    fn if_else_expression() {
        let out = parse("fn f(x) { if x { 1 } else { 2 } }");
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    }
}
