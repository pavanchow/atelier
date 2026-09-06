//! The `atelier` command line tool.
//!
//! Subcommands:
//!   analyze <file>              print diagnostics
//!   def <file> <pos>           print the definition of the symbol at <pos>
//!   refs <file> <pos>          print all references to the symbol at <pos>
//!   hover <file> <pos>         print symbol info at <pos>
//!   rename <file> <pos> <name> print the file with the symbol at <pos> renamed
//!   run <file>                 evaluate the program
//!   demo                       run a built in tour of every feature
//!
//! <pos> is either a byte offset, or a one based `line:col`.

use atelier::eval;
use atelier::incremental::Analysis;
use atelier::resolver::BindingKind;
use atelier::span::{self, Span};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map_or("", String::as_str);
    match cmd {
        "analyze" => need_file(&args, cmd, cmd_analyze),
        "def" => need_file_pos(&args, cmd, cmd_def),
        "refs" => need_file_pos(&args, cmd, cmd_refs),
        "hover" => need_file_pos(&args, cmd, cmd_hover),
        "rename" => cmd_rename(&args),
        "run" => need_file(&args, cmd, cmd_run),
        "demo" => {
            cmd_demo();
            ExitCode::SUCCESS
        }
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "atelier: a dependency-free code intelligence engine\n\n\
         usage:\n  \
         atelier analyze <file>\n  \
         atelier def <file> <pos>\n  \
         atelier refs <file> <pos>\n  \
         atelier hover <file> <pos>\n  \
         atelier rename <file> <pos> <name>\n  \
         atelier run <file>\n  \
         atelier demo\n\n\
         <pos> is a byte offset or a one-based line:col"
    );
}

fn need_file(args: &[String], cmd: &str, f: fn(&Analysis)) -> ExitCode {
    let Some(path) = args.get(1) else {
        eprintln!("{cmd}: missing <file>");
        return ExitCode::from(2);
    };
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let a = Analysis::new(text);
            f(&a);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{cmd}: cannot read {path}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn need_file_pos(args: &[String], cmd: &str, f: fn(&Analysis, u32)) -> ExitCode {
    let (Some(path), Some(pos_str)) = (args.get(1), args.get(2)) else {
        eprintln!("{cmd}: usage: atelier {cmd} <file> <pos>");
        return ExitCode::from(2);
    };
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let pos = parse_pos(&text, pos_str);
            let a = Analysis::new(text);
            f(&a, pos);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{cmd}: cannot read {path}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_pos(text: &str, s: &str) -> u32 {
    if let Some((l, c)) = s.split_once(':') {
        let line = l.parse().unwrap_or(1);
        let col = c.parse().unwrap_or(1);
        span::linecol_to_offset(text, line, col)
    } else {
        s.parse().unwrap_or(0)
    }
}

fn show_span(text: &str, span: Span) -> String {
    let lc = span::offset_to_linecol(text, span.start);
    format!(
        "{}:{} (bytes {}..{})",
        lc.line, lc.col, span.start, span.end
    )
}

fn cmd_analyze(a: &Analysis) {
    let diags = a.diagnostics();
    if diags.is_empty() {
        println!("no diagnostics");
        return;
    }
    for d in diags {
        let lc = span::offset_to_linecol(a.text(), d.span.start);
        println!(
            "{}:{}: {:?}: {} [{:?}]",
            lc.line, lc.col, d.severity, d.message, d.kind
        );
    }
}

fn cmd_def(a: &Analysis, pos: u32) {
    match a.go_to_definition(pos) {
        Some(def) => println!(
            "{} `{}` defined at {}",
            def.kind.describe(),
            def.name,
            show_span(a.text(), def.span)
        ),
        None => println!("no definition at that position"),
    }
}

fn cmd_refs(a: &Analysis, pos: u32) {
    let refs = a.find_references(pos);
    if refs.is_empty() {
        println!("no references at that position");
        return;
    }
    println!("{} reference(s):", refs.len());
    for r in refs {
        let tag = if r.is_decl { "def" } else { "use" };
        println!("  {tag} {}", show_span(a.text(), r.span));
    }
}

fn cmd_hover(a: &Analysis, pos: u32) {
    match a.hover(pos) {
        Some(h) => {
            let sig = match (h.kind, h.arity) {
                (BindingKind::Fn, Some(n)) => format!("fn {}({} params)", h.name, n),
                _ => format!("{} {}", h.kind.describe(), h.name),
            };
            println!("{sig}\ndefined at {}", show_span(a.text(), h.decl_span));
        }
        None => println!("nothing to hover at that position"),
    }
}

fn cmd_rename(args: &[String]) -> ExitCode {
    let (Some(path), Some(pos_str), Some(name)) = (args.get(1), args.get(2), args.get(3)) else {
        eprintln!("rename: usage: atelier rename <file> <pos> <new-name>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("rename: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pos = parse_pos(&text, pos_str);
    let a = Analysis::new(text);
    match a.rename(pos, name) {
        Ok(r) => {
            print!("{}", r.new_text);
            ExitCode::SUCCESS
        }
        Err(atelier::RenameError::NotRenameable) => {
            eprintln!("rename: no renameable symbol at that position");
            ExitCode::FAILURE
        }
        Err(atelier::RenameError::InvalidName) => {
            eprintln!("rename: `{name}` is not a valid identifier");
            ExitCode::FAILURE
        }
        Err(atelier::RenameError::Conflict) => {
            eprintln!("rename: refused, renaming to `{name}` would change name resolution");
            ExitCode::FAILURE
        }
    }
}

fn cmd_run(a: &Analysis) {
    let errors = a.diagnostics().iter().filter(|d| {
        matches!(
            d.kind,
            atelier::DiagKind::ParseError | atelier::DiagKind::UnexpectedEof
        )
    });
    if errors.clone().count() > 0 {
        eprintln!("cannot run: the program has parse errors");
        for d in errors {
            let lc = span::offset_to_linecol(a.text(), d.span.start);
            eprintln!("  {}:{}: {}", lc.line, lc.col, d.message);
        }
        return;
    }
    match eval::run(a.program()) {
        Ok(out) => {
            for line in out.lines {
                println!("{line}");
            }
        }
        Err(e) => {
            let lc = span::offset_to_linecol(a.text(), e.span.start);
            eprintln!("runtime error at {}:{}: {}", lc.line, lc.col, e.message);
        }
    }
}

fn cmd_demo() {
    let src = "\
fn fib(n) {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

let count = 10;
let result = fib(count);
result;
";
    println!("=== source ===");
    println!("{src}");

    let mut a = Analysis::new(src);

    println!("=== diagnostics (clean program) ===");
    cmd_analyze(&a);

    println!("\n=== go to definition (the `fib` call inside the body) ===");
    let inner_fib = src.find("fib(n - 1)").unwrap() as u32;
    cmd_def(&a, inner_fib);

    println!("\n=== find references (the `count` binding) ===");
    let count_pos = src.find("count").unwrap() as u32;
    cmd_refs(&a, count_pos);

    println!("\n=== hover (the `fib` function) ===");
    cmd_hover(&a, src.find("fib").unwrap() as u32);

    println!("\n=== run ===");
    cmd_run(&a);

    println!("\n=== incremental edit (change count from 10 to 15, reusing untouched units) ===");
    let pos = a.text().find("10").unwrap() as u32;
    a.edit(pos, pos + 2, "15");
    cmd_run(&a);

    println!("\n=== rename (rename `fib` to `fibonacci`, all sites, safely) ===");
    match a.rename(a.text().find("fib").unwrap() as u32, "fibonacci") {
        Ok(r) => print!("{}", r.new_text),
        Err(e) => println!("rename failed: {e:?}"),
    }

    println!("\n=== diagnostics (a program with mistakes) ===");
    let broken = "let a = 1;\nlet a = 2;\nlet c = missing + 1\n";
    println!("{broken}");
    let b = Analysis::new(broken);
    cmd_analyze(&b);
}
