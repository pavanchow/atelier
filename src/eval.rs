//! A small tree walking evaluator so a program can actually Run.
//!
//! Values are integers, booleans, and unit. Top level functions are hoisted and
//! may recurse or call one another. Functions see the global scope (top level
//! functions and top level `let` values) plus their own parameters and locals,
//! which keeps the model simple and predictable. Running a program prints the
//! value of each top level expression statement.

use crate::ast::*;
use crate::span::Span;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Unit,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Unit => write!(f, "()"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeError {
    pub span: Span,
    pub message: String,
}

/// The result of running a program: the value of each top level expression
/// statement, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct RunOutput {
    pub lines: Vec<String>,
}

struct Interp<'p> {
    globals_fn: HashMap<String, &'p FnDecl>,
    globals_var: HashMap<String, Value>,
}

/// Run a whole program.
pub fn run(program: &Program) -> Result<RunOutput, RuntimeError> {
    let mut globals_fn = HashMap::new();
    for stmt in program {
        if let StmtKind::Fn(f) = &stmt.kind {
            globals_fn.insert(f.name.name.clone(), f);
        }
    }
    let mut interp = Interp {
        globals_fn,
        globals_var: HashMap::new(),
    };
    let mut lines = Vec::new();
    let mut scopes: Vec<HashMap<String, Value>> = Vec::new();
    for stmt in program {
        match &stmt.kind {
            StmtKind::Fn(_) => {}
            StmtKind::Let(l) => {
                let v = interp.eval_expr(&l.value, &mut scopes)?;
                interp.globals_var.insert(l.name.name.clone(), v);
            }
            StmtKind::Expr(e) => {
                let v = interp.eval_expr(e, &mut scopes)?;
                if v != Value::Unit {
                    lines.push(v.to_string());
                }
            }
        }
    }
    Ok(RunOutput { lines })
}

impl<'p> Interp<'p> {
    fn lookup_var(&self, scopes: &[HashMap<String, Value>], name: &str) -> Option<Value> {
        for scope in scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        self.globals_var.get(name).cloned()
    }

    fn eval_block(&self, block: &Block, scopes: &mut Vec<HashMap<String, Value>>) -> Result<Value, RuntimeError> {
        scopes.push(HashMap::new());
        let result = (|| {
            for stmt in &block.stmts {
                match &stmt.kind {
                    StmtKind::Fn(_) => {}
                    StmtKind::Let(l) => {
                        let v = self.eval_expr(&l.value, scopes)?;
                        scopes.last_mut().unwrap().insert(l.name.name.clone(), v);
                    }
                    StmtKind::Expr(e) => {
                        self.eval_expr(e, scopes)?;
                    }
                }
            }
            match &block.tail {
                Some(e) => self.eval_expr(e, scopes),
                None => Ok(Value::Unit),
            }
        })();
        scopes.pop();
        result
    }

    fn eval_expr(&self, expr: &Expr, scopes: &mut Vec<HashMap<String, Value>>) -> Result<Value, RuntimeError> {
        match &expr.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Error => Err(self.err(expr.span, "cannot evaluate a syntax error")),
            ExprKind::Name(name) => self
                .lookup_var(scopes, name)
                .ok_or_else(|| self.err(expr.span, format!("`{name}` is not a value in scope"))),
            ExprKind::Paren(inner) => self.eval_expr(inner, scopes),
            ExprKind::Unary { op, expr: inner } => {
                let v = self.eval_expr(inner, scopes)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => Err(self.err(expr.span, "type error in unary operation")),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(lhs, scopes)?;
                let r = self.eval_expr(rhs, scopes)?;
                self.eval_binary(*op, l, r, expr.span)
            }
            ExprKind::Block(block) => self.eval_block(block, scopes),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                let c = self.eval_expr(cond, scopes)?;
                match c {
                    Value::Bool(true) => self.eval_block(then_block, scopes),
                    Value::Bool(false) => match else_block {
                        Some(b) => self.eval_block(b, scopes),
                        None => Ok(Value::Unit),
                    },
                    _ => Err(self.err(cond.span, "if condition must be a boolean")),
                }
            }
            ExprKind::Call { callee, args } => {
                let name = match &callee.kind {
                    ExprKind::Name(n) => n.clone(),
                    _ => return Err(self.err(callee.span, "call target must be a function name")),
                };
                let func = *self
                    .globals_fn
                    .get(&name)
                    .ok_or_else(|| self.err(callee.span, format!("`{name}` is not a function")))?;
                if func.params.len() != args.len() {
                    return Err(self.err(
                        expr.span,
                        format!("`{name}` expects {} arguments, got {}", func.params.len(), args.len()),
                    ));
                }
                let mut frame = HashMap::new();
                for (p, a) in func.params.iter().zip(args) {
                    let v = self.eval_expr(a, scopes)?;
                    frame.insert(p.name.clone(), v);
                }
                let mut call_scopes = vec![frame];
                self.eval_block(&func.body, &mut call_scopes)
            }
        }
    }

    fn eval_binary(&self, op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
        use Value::*;
        let arith = |f: fn(i64, i64) -> Option<i64>| match (&l, &r) {
            (Int(a), Int(b)) => f(*a, *b)
                .map(Int)
                .ok_or_else(|| self.err(span, "arithmetic error")),
            _ => Err(self.err(span, "arithmetic requires integers")),
        };
        match op {
            BinOp::Add => arith(|a, b| a.checked_add(b)),
            BinOp::Sub => arith(|a, b| a.checked_sub(b)),
            BinOp::Mul => arith(|a, b| a.checked_mul(b)),
            BinOp::Div => arith(|a, b| a.checked_div(b)),
            BinOp::Eq => Ok(Bool(values_eq(&l, &r))),
            BinOp::Ne => Ok(Bool(!values_eq(&l, &r))),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (l, r) {
                (Int(a), Int(b)) => Ok(Bool(match op {
                    BinOp::Lt => a < b,
                    BinOp::Le => a <= b,
                    BinOp::Gt => a > b,
                    BinOp::Ge => a >= b,
                    _ => unreachable!(),
                })),
                _ => Err(self.err(span, "comparison requires integers")),
            },
        }
    }

    fn err(&self, span: Span, message: impl Into<String>) -> RuntimeError {
        RuntimeError {
            span,
            message: message.into(),
        }
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Unit, Value::Unit) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn run_src(src: &str) -> Result<RunOutput, RuntimeError> {
        run(&parse(src).program)
    }

    #[test]
    fn arithmetic_and_precedence() {
        let out = run_src("1 + 2 * 3;").unwrap();
        assert_eq!(out.lines, vec!["7"]);
    }

    #[test]
    fn functions_and_recursion() {
        let src = "fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\nfib(10);";
        let out = run_src(src).unwrap();
        assert_eq!(out.lines, vec!["55"]);
    }

    #[test]
    fn let_bindings_and_blocks() {
        let out = run_src("let a = 3; let b = { let c = 4; a + c }; b;").unwrap();
        assert_eq!(out.lines, vec!["7"]);
    }

    #[test]
    fn division_by_zero_errors() {
        assert!(run_src("1 / 0;").is_err());
    }

    #[test]
    fn if_without_else_is_unit() {
        let out = run_src("if false { 1 };").unwrap();
        assert!(out.lines.is_empty());
    }
}
