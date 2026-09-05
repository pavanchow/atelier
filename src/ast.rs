//! The abstract syntax tree.
//!
//! Every node carries an absolute [`Span`]. Nodes derive [`PartialEq`] including
//! their spans, which the incremental gate relies on: an incrementally produced
//! tree must equal a batch produced one exactly, spans and all.

use crate::span::Span;

/// A declared name (a function name, a parameter, or a `let` binding target).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// A top level or block level statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StmtKind {
    Let(LetStmt),
    Fn(FnDecl),
    /// An expression used as a statement (it was followed by `;`).
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LetStmt {
    pub name: Ident,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
    pub body: Block,
}

/// A brace delimited scope. `tail` is the optional final expression that gives
/// the block its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Int(i64),
    Bool(bool),
    /// A use of a name. Resolution maps this to a binding.
    Name(String),
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_block: Option<Block>,
    },
    Block(Block),
    Paren(Box<Expr>),
    /// A placeholder inserted during error recovery so spans stay well formed.
    Error,
}

/// A parsed program is simply a sequence of top level statements.
pub type Program = Vec<Stmt>;
