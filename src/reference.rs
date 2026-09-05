//! An independent reference resolver used only to check the production
//! [`crate::resolver`].
//!
//! This is a deliberately straightforward full program scope walk that maps
//! every identifier occurrence to the declaration it binds to. It shares the AST
//! but none of the production resolver's machinery, so agreement between the two
//! is real cross validation of go to definition and find references rather than
//! a tautology. The navigation gate asserts the two agree on every symbol of
//! every random program.

use crate::ast::*;
use crate::span::Span;
use std::collections::BTreeMap;

/// The declaration each identifier occurrence resolves to. A declaration maps to
/// its own span. An unresolved use maps to `None`.
pub type RefMap = BTreeMap<Span, Option<Span>>;

#[derive(Clone)]
struct Decl {
    name: String,
    span: Span,
}

/// A scope is split into hoisted functions and the sequential (let/param)
/// bindings visible so far.
#[derive(Clone, Default)]
struct Scope {
    fns: Vec<Decl>,
    seq: Vec<Decl>,
}

pub fn resolve_program(program: &Program) -> RefMap {
    let mut map = RefMap::new();
    let mut env: Vec<Scope> = Vec::new();
    walk_scope(program, None, &[], &mut env, &mut map);
    map
}

fn lookup(env: &[Scope], name: &str) -> Option<Span> {
    for scope in env.iter().rev() {
        if let Some(d) = scope.seq.iter().rev().find(|d| d.name == name) {
            return Some(d.span);
        }
        if let Some(d) = scope.fns.iter().find(|d| d.name == name) {
            return Some(d.span);
        }
    }
    None
}

fn walk_scope(
    stmts: &[Stmt],
    tail: Option<&Expr>,
    params: &[Ident],
    env: &mut Vec<Scope>,
    map: &mut RefMap,
) {
    let mut scope = Scope::default();
    for p in params {
        map.insert(p.span, Some(p.span));
        scope.seq.push(Decl {
            name: p.name.clone(),
            span: p.span,
        });
    }
    for stmt in stmts {
        if let StmtKind::Fn(f) = &stmt.kind {
            map.insert(f.name.span, Some(f.name.span));
            scope.fns.push(Decl {
                name: f.name.name.clone(),
                span: f.name.span,
            });
        }
    }
    env.push(scope);

    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Let(l) => {
                walk_expr(&l.value, env, map);
                map.insert(l.name.span, Some(l.name.span));
                env.last_mut().unwrap().seq.push(Decl {
                    name: l.name.name.clone(),
                    span: l.name.span,
                });
            }
            StmtKind::Fn(f) => {
                walk_scope(&f.body.stmts, f.body.tail.as_deref(), &f.params, env, map);
            }
            StmtKind::Expr(e) => walk_expr(e, env, map),
        }
    }
    if let Some(t) = tail {
        walk_expr(t, env, map);
    }

    env.pop();
}

fn walk_expr(expr: &Expr, env: &mut Vec<Scope>, map: &mut RefMap) {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Error => {}
        ExprKind::Name(name) => {
            let target = lookup(env, name);
            map.insert(expr.span, target);
        }
        ExprKind::Unary { expr, .. } => walk_expr(expr, env, map),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, env, map);
            walk_expr(rhs, env, map);
        }
        ExprKind::Call { callee, args } => {
            walk_expr(callee, env, map);
            for a in args {
                walk_expr(a, env, map);
            }
        }
        ExprKind::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_expr(cond, env, map);
            walk_scope(&then_block.stmts, then_block.tail.as_deref(), &[], env, map);
            if let Some(b) = else_block {
                walk_scope(&b.stmts, b.tail.as_deref(), &[], env, map);
            }
        }
        ExprKind::Block(block) => {
            walk_scope(&block.stmts, block.tail.as_deref(), &[], env, map);
        }
        ExprKind::Paren(inner) => walk_expr(inner, env, map),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn maps_use_to_decl() {
        let prog = parse("let x = 1; let y = x;").program;
        let map = resolve_program(&prog);
        // the use of x resolves to the decl of x at span 4..5
        let decl_x = Span::new(4, 5);
        let use_x = map
            .iter()
            .find(|(s, target)| **target == Some(decl_x) && **s != decl_x);
        assert!(use_x.is_some());
    }
}
