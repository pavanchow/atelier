//! Lexically scoped name resolution and the symbol table.
//!
//! Scoping rules:
//! - Functions are hoisted within their enclosing scope, so a call may appear
//!   before the function declaration and mutual recursion works.
//! - Parameters are visible throughout their function body.
//! - `let` bindings are visible only after their declaration, in order, within
//!   the enclosing block.
//! - An inner scope may shadow an outer name. Declaring the same name twice in
//!   one scope is a redefinition diagnostic.
//!
//! The resolver records every identifier occurrence (declarations and uses)
//! together with the binding it resolves to, which is what the IDE queries in
//! [`crate::query`] read.

use crate::ast::*;
use crate::diagnostics::{DiagKind, Diagnostic};
use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Fn,
    Param,
    Let,
}

impl BindingKind {
    pub fn describe(self) -> &'static str {
        match self {
            BindingKind::Fn => "function",
            BindingKind::Param => "parameter",
            BindingKind::Let => "variable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub kind: BindingKind,
    pub decl_span: Span,
    /// For functions, the number of declared parameters. Used by hover.
    pub arity: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub span: Span,
    pub name: String,
    pub binding: Option<usize>,
    pub is_decl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SymbolTable {
    pub bindings: Vec<Binding>,
    pub occurrences: Vec<Occurrence>,
    pub diagnostics: Vec<Diagnostic>,
}

struct Frame {
    fns: Vec<(String, usize)>,
    seq: Vec<(String, usize)>,
}

struct Resolver {
    table: SymbolTable,
    stack: Vec<Frame>,
}

/// Resolve a whole program into a symbol table.
pub fn resolve(program: &Program) -> SymbolTable {
    let mut r = Resolver {
        table: SymbolTable::default(),
        stack: Vec::new(),
    };
    r.scope(program, &[], &[]);
    r.table
        .occurrences
        .sort_by_key(|o| (o.span.start, o.span.end, o.is_decl));
    r.table.diagnostics.sort();
    r.table
}

impl Resolver {
    fn add_binding(&mut self, name: &str, kind: BindingKind, decl_span: Span, arity: Option<usize>) -> usize {
        let id = self.table.bindings.len();
        self.table.bindings.push(Binding {
            name: name.to_string(),
            kind,
            decl_span,
            arity,
        });
        self.table.occurrences.push(Occurrence {
            span: decl_span,
            name: name.to_string(),
            binding: Some(id),
            is_decl: true,
        });
        id
    }

    fn frame_has(&self, name: &str) -> bool {
        let f = self.stack.last().unwrap();
        f.fns.iter().any(|(n, _)| n == name) || f.seq.iter().any(|(n, _)| n == name)
    }

    fn redef(&mut self, name: &str, span: Span) {
        self.table.diagnostics.push(Diagnostic::error(
            span,
            DiagKind::Redefinition,
            format!("`{name}` is already defined in this scope"),
        ));
    }

    fn resolve_name(&self, name: &str) -> Option<usize> {
        for frame in self.stack.iter().rev() {
            if let Some((_, id)) = frame.seq.iter().rev().find(|(n, _)| n == name) {
                return Some(*id);
            }
            if let Some((_, id)) = frame.fns.iter().find(|(n, _)| n == name) {
                return Some(*id);
            }
        }
        None
    }

    /// Resolve a block scope. `params` seed the scope (function bodies), and the
    /// statements plus optional tail are the block body.
    fn scope(&mut self, stmts: &[Stmt], tail: &[&Expr], params: &[Ident]) {
        self.stack.push(Frame {
            fns: Vec::new(),
            seq: Vec::new(),
        });

        for p in params {
            if self.frame_has(&p.name) {
                self.redef(&p.name, p.span);
            }
            let id = self.add_binding(&p.name, BindingKind::Param, p.span, None);
            self.stack.last_mut().unwrap().seq.push((p.name.clone(), id));
        }

        // Hoist function declarations for this scope.
        let mut fn_binding_for = vec![None; stmts.len()];
        for (i, stmt) in stmts.iter().enumerate() {
            if let StmtKind::Fn(f) = &stmt.kind {
                if self.frame_has(&f.name.name) {
                    self.redef(&f.name.name, f.name.span);
                }
                let id = self.add_binding(
                    &f.name.name,
                    BindingKind::Fn,
                    f.name.span,
                    Some(f.params.len()),
                );
                self.stack.last_mut().unwrap().fns.push((f.name.name.clone(), id));
                fn_binding_for[i] = Some(id);
            }
        }

        for (i, stmt) in stmts.iter().enumerate() {
            match &stmt.kind {
                StmtKind::Let(l) => {
                    self.resolve_expr(&l.value);
                    if self.frame_has(&l.name.name) {
                        self.redef(&l.name.name, l.name.span);
                    }
                    let id = self.add_binding(&l.name.name, BindingKind::Let, l.name.span, None);
                    self.stack.last_mut().unwrap().seq.push((l.name.name.clone(), id));
                }
                StmtKind::Fn(f) => {
                    let _ = fn_binding_for[i];
                    self.scope_block(&f.body, &f.params);
                }
                StmtKind::Expr(e) => self.resolve_expr(e),
            }
        }

        for e in tail {
            self.resolve_expr(e);
        }

        self.stack.pop();
    }

    fn scope_block(&mut self, block: &Block, params: &[Ident]) {
        let tail: Vec<&Expr> = block.tail.iter().map(|b| b.as_ref()).collect();
        self.scope(&block.stmts, &tail, params);
    }

    fn resolve_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Error => {}
            ExprKind::Name(name) => {
                let binding = self.resolve_name(name);
                if binding.is_none() {
                    self.table.diagnostics.push(Diagnostic::error(
                        expr.span,
                        DiagKind::UnresolvedName,
                        format!("cannot find `{name}` in this scope"),
                    ));
                }
                self.table.occurrences.push(Occurrence {
                    span: expr.span,
                    name: name.clone(),
                    binding,
                    is_decl: false,
                });
            }
            ExprKind::Unary { expr, .. } => self.resolve_expr(expr),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            ExprKind::Call { callee, args } => {
                self.resolve_expr(callee);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.resolve_expr(cond);
                self.scope_block(then_block, &[]);
                if let Some(b) = else_block {
                    self.scope_block(b, &[]);
                }
            }
            ExprKind::Block(block) => self.scope_block(block, &[]),
            ExprKind::Paren(inner) => self.resolve_expr(inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn table(src: &str) -> SymbolTable {
        resolve(&parse(src).program)
    }

    #[test]
    fn resolves_let_use() {
        let t = table("let x = 1; let y = x;");
        let uses: Vec<_> = t.occurrences.iter().filter(|o| !o.is_decl).collect();
        assert_eq!(uses.len(), 1);
        assert!(uses[0].binding.is_some());
    }

    #[test]
    fn function_hoisting() {
        let t = table("fn a() { b() } fn b() { 1 }");
        assert!(t.diagnostics.is_empty(), "{:?}", t.diagnostics);
    }

    #[test]
    fn unresolved_name_flagged() {
        let t = table("let x = y;");
        assert!(t.diagnostics.iter().any(|d| d.kind == DiagKind::UnresolvedName));
    }

    #[test]
    fn redefinition_flagged() {
        let t = table("let x = 1; let x = 2;");
        assert!(t.diagnostics.iter().any(|d| d.kind == DiagKind::Redefinition));
    }

    #[test]
    fn shadowing_across_scopes_is_ok() {
        let t = table("let x = 1; fn f(x) { x }");
        assert!(t.diagnostics.is_empty(), "{:?}", t.diagnostics);
    }

    #[test]
    fn let_not_visible_before_declaration() {
        let t = table("fn f() { let a = b; let b = 1; }");
        assert!(t.diagnostics.iter().any(|d| d.kind == DiagKind::UnresolvedName));
    }
}
