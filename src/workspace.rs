//! A multi file project model plus a simple run task.
//!
//! A [`Workspace`] owns one [`Analysis`] per file, keyed by name. Editing a file
//! updates its analysis incrementally. Running a file evaluates its program.

use crate::eval::{self, RunOutput, RuntimeError};
use crate::incremental::Analysis;
use crate::rename::RenameError;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Workspace {
    files: BTreeMap<String, Analysis>,
}

#[derive(Debug)]
pub enum RunError {
    NoSuchFile,
    Runtime(RuntimeError),
}

impl Workspace {
    pub fn new() -> Workspace {
        Workspace::default()
    }

    /// Add or replace a file, analysing it from scratch.
    pub fn set_file(&mut self, name: impl Into<String>, text: impl Into<String>) {
        self.files.insert(name.into(), Analysis::new(text));
    }

    /// Apply an incremental edit to a file. Returns false if the file is unknown.
    pub fn edit_file(&mut self, name: &str, start: u32, end: u32, replacement: &str) -> bool {
        match self.files.get_mut(name) {
            Some(a) => {
                a.edit(start, end, replacement);
                true
            }
            None => false,
        }
    }

    pub fn file(&self, name: &str) -> Option<&Analysis> {
        self.files.get(name)
    }

    pub fn file_names(&self) -> impl Iterator<Item = &String> {
        self.files.keys()
    }

    /// Rename the symbol at `pos` in `name` to `new_name`, rewriting the file's
    /// declaration and every reference. Returns the number of edits applied. The
    /// rename is refused (and the file left untouched) if it is unsafe or the new
    /// name is invalid. A file not found reports [`RenameError::NotRenameable`].
    pub fn rename_symbol(
        &mut self,
        name: &str,
        pos: u32,
        new_name: &str,
    ) -> Result<usize, RenameError> {
        let a = self.files.get(name).ok_or(RenameError::NotRenameable)?;
        let rename = a.rename(pos, new_name)?;
        let count = rename.edits.len();
        self.files
            .insert(name.to_string(), Analysis::new(rename.new_text));
        Ok(count)
    }

    /// Evaluate a file's program (the Run task).
    pub fn run_file(&self, name: &str) -> Result<RunOutput, RunError> {
        let a = self.files.get(name).ok_or(RunError::NoSuchFile)?;
        eval::run(a.program()).map_err(RunError::Runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_then_query_and_run() {
        let mut ws = Workspace::new();
        ws.set_file("main.at", "fn dbl(n) { n * 2 }\ndbl(21);\n");
        assert_eq!(ws.run_file("main.at").unwrap().lines, vec!["42"]);

        // rename the argument at the call site by editing text
        let a = ws.file("main.at").unwrap();
        let pos = a.text().find("21").unwrap() as u32;
        ws.edit_file("main.at", pos, pos + 2, "50");
        assert_eq!(ws.run_file("main.at").unwrap().lines, vec!["100"]);
    }

    #[test]
    fn unknown_file() {
        let ws = Workspace::new();
        assert!(matches!(ws.run_file("nope"), Err(RunError::NoSuchFile)));
    }

    #[test]
    fn rename_symbol_updates_file_and_run() {
        let mut ws = Workspace::new();
        ws.set_file("main.at", "fn dbl(n) { n * 2 }\ndbl(21);\n");
        let pos = ws.file("main.at").unwrap().text().find("dbl").unwrap() as u32;
        let count = ws.rename_symbol("main.at", pos, "twice").unwrap();
        assert_eq!(count, 2);
        assert!(ws.file("main.at").unwrap().text().contains("twice(21)"));
        assert_eq!(ws.run_file("main.at").unwrap().lines, vec!["42"]);
    }

    #[test]
    fn rename_symbol_unknown_file() {
        let mut ws = Workspace::new();
        assert_eq!(
            ws.rename_symbol("nope", 0, "x"),
            Err(RenameError::NotRenameable)
        );
    }
}
