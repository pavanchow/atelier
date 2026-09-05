//! Atelier: a dependency-free semantic tooling layer for a small language.
//!
//! The crate is organised as a pipeline: [`lexer`] turns text into tokens,
//! [`parser`] turns tokens into an AST, [`resolver`] builds a scoped symbol
//! table, and [`incremental`] wires it together into an [`incremental::Analysis`]
//! that can be updated edit by edit. IDE queries live in [`query`], evaluation
//! in [`eval`], and the multi file project model in [`workspace`].

pub mod ast;
pub mod diagnostics;
pub mod eval;
pub mod gen;
pub mod incremental;
pub mod lexer;
pub mod parser;
pub mod query;
pub mod reference;
pub mod resolver;
pub mod rng;
pub mod span;
pub mod workspace;

pub use diagnostics::{DiagKind, Diagnostic, Severity};
pub use incremental::Analysis;
pub use query::{Definition, Hover, Reference};
pub use span::Span;
pub use workspace::Workspace;
