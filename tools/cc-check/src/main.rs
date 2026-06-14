//! Cyclomatic-complexity gate for Rust, the sibling of `scripts/cc-check.js`.
//!
//! A function's complexity starts at 1 and increments for each branching node:
//!   `if` / `while` / `for` / `loop` / `match` / `?` (try) and each `&&` / `||`.
//! A `match` counts as a single decision (+1) regardless of arm count, keeping
//! idiomatic exhaustive matches viable while still flagging genuinely tangled
//! control flow. Nested functions and closures are scored independently.
//!
//! Usage:
//!   cc-check [--max <n>] [path ...]
//! Paths may be files or directories (directories are walked for `*.rs`).
//! With no paths, scans `crates/` and `tools/`. Exit code 1 if any function
//! exceeds `--max` (default 5, i.e. the project's "cc < 6" rule). `CC_MAX`
//! overrides the default.

use std::path::{Path, PathBuf};
use std::process::exit;

use syn::visit::{self, Visit};
use syn::{BinOp, Expr};
use walkdir::WalkDir;

struct Config {
    max: u32,
    paths: Vec<String>,
}

fn parse_args() -> Config {
    let default_max = std::env::var("CC_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let mut cfg = Config {
        max: default_max,
        paths: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        take_arg(&mut cfg, &arg, &mut args);
    }
    if cfg.paths.is_empty() {
        cfg.paths = vec!["crates".into(), "tools".into()];
    }
    cfg
}

fn take_arg(cfg: &mut Config, arg: &str, rest: &mut impl Iterator<Item = String>) {
    if arg == "--max" {
        if let Some(v) = rest.next().and_then(|v| v.parse().ok()) {
            cfg.max = v;
        }
    } else {
        cfg.paths.push(arg.to_string());
    }
}

fn is_rust(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "rs")
}

fn collect_files(paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw in paths {
        push_path(&mut out, Path::new(raw));
    }
    out
}

fn push_path(out: &mut Vec<PathBuf>, path: &Path) {
    if path.is_dir() {
        push_dir(out, path);
    } else if is_rust(path) {
        out.push(path.to_path_buf());
    }
}

fn push_dir(out: &mut Vec<PathBuf>, dir: &Path) {
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        let p = entry.path();
        if p.is_file() && is_rust(p) {
            out.push(p.to_path_buf());
        }
    }
}

struct Report {
    name: String,
    line: usize,
    complexity: u32,
}

#[derive(Default)]
struct Walker {
    reports: Vec<Report>,
    current: u32,
}

impl Walker {
    fn enter(&mut self, name: String, line: usize, body: impl FnOnce(&mut Self)) {
        let saved = self.current;
        self.current = 1;
        body(self);
        self.reports.push(Report {
            name,
            line,
            complexity: self.current,
        });
        self.current = saved;
    }
}

fn branch_inc(expr: &Expr) -> u32 {
    match expr {
        Expr::If(_)
        | Expr::While(_)
        | Expr::ForLoop(_)
        | Expr::Loop(_)
        | Expr::Match(_)
        | Expr::Try(_) => 1,
        Expr::Binary(b) => bin_inc(&b.op),
        _ => 0,
    }
}

fn bin_inc(op: &BinOp) -> u32 {
    u32::from(matches!(op, BinOp::And(_) | BinOp::Or(_)))
}

fn line_of(span: proc_macro2::Span) -> usize {
    span.start().line
}

impl<'ast> Visit<'ast> for Walker {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let name = node.sig.ident.to_string();
        let line = line_of(node.sig.ident.span());
        self.enter(name, line, |w| visit::visit_block(w, &node.block));
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let name = node.sig.ident.to_string();
        let line = line_of(node.sig.ident.span());
        self.enter(name, line, |w| visit::visit_block(w, &node.block));
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Expr::Closure(closure) = expr {
            let line = line_of(closure.or1_token.span);
            self.enter("(closure)".into(), line, |w| w.visit_expr(&closure.body));
            return;
        }
        self.current += branch_inc(expr);
        visit::visit_expr(self, expr);
    }
}

fn check_file(file: &Path, max: u32) -> u32 {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => return parse_failure(file, &e.to_string()),
    };
    let ast = match syn::parse_file(&text) {
        Ok(a) => a,
        Err(e) => return parse_failure(file, &e.to_string()),
    };
    let mut walker = Walker::default();
    walker.visit_file(&ast);
    report_violations(file, &walker.reports, max)
}

fn parse_failure(file: &Path, msg: &str) -> u32 {
    eprintln!("cc-check: failed to parse {}: {msg}", file.display());
    0
}

fn report_violations(file: &Path, reports: &[Report], max: u32) -> u32 {
    let mut violations = 0;
    for r in reports {
        if r.complexity > max {
            violations += 1;
            eprintln!(
                "{}:{}  {} has a cyclomatic complexity of {} (max {max})",
                file.display(),
                r.line,
                r.name,
                r.complexity,
            );
        }
    }
    violations
}

fn main() {
    let cfg = parse_args();
    let files = collect_files(&cfg.paths);
    let violations: u32 = files.iter().map(|f| check_file(f, cfg.max)).sum();
    if violations > 0 {
        eprintln!(
            "\nx Complexity gate: {violations} function(s) above {}. Refactor first.",
            cfg.max
        );
        exit(1);
    }
}
