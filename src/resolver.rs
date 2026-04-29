// Cross-file symbol resolver for Xore.
//
// Pass 1 (collect)  — scan ALL .xre/.xrb files → build SymbolTable
// Pass 2 (resolve)  — for each file being compiled, emit LLVM `declare`
//                     for every function called but defined elsewhere.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::fs;

use crate::ast::*;
use crate::parse;

// ─── Symbol kinds ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FnSymbol {
    pub name:     String,
    pub params:   Vec<ParamSig>,
    pub ret_ty:   String,       // LLVM type string
    pub exported: bool,
    pub source:   PathBuf,
}

#[derive(Debug, Clone)]
pub struct ParamSig {
    pub name: String,
    pub ty:   String,
}

#[derive(Debug, Clone)]
pub struct StructSymbol {
    pub name:   String,
    pub fields: Vec<(String, String)>,
    pub source: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EnumSymbol {
    pub name:     String,
    pub variants: Vec<String>,
    pub source:   PathBuf,
}

// ─── Symbol table ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    pub fns:     HashMap<String, FnSymbol>,
    pub structs: HashMap<String, StructSymbol>,
    pub enums:   HashMap<String, EnumSymbol>,
}

impl SymbolTable {
    pub fn new() -> Self { Self::default() }

    /// LLVM `declare` line for a function defined in another file.
    pub fn declare_fn(&self, name: &str) -> Option<String> {
        let sym = self.fns.get(name)?;
        let params: Vec<String> = sym.params.iter()
            .map(|p| format!("{} noundef", p.ty))
            .collect();
        Some(format!("declare {} @{}({})", sym.ret_ty, sym.name, params.join(", ")))
    }

    /// Collect all top-level symbols from a parsed program.
    pub fn collect_from(&mut self, program: &Program, source: &Path) {
        for item in &program.items {
            self.collect_item(item, source);
        }
    }

    fn collect_item(&mut self, item: &Item, source: &Path) {
        match item {
            Item::Function(f) => {
                let params = f.params.iter().map(|p| ParamSig {
                    name: p.name.clone(),
                    ty:   type_to_llvm(&p.ty),
                }).collect();
                let ret_ty = f.ret_ty.as_ref()
                    .map(|t| type_to_llvm(t))
                    .unwrap_or_else(|| "void".into());
                self.fns.insert(f.name.clone(), FnSymbol {
                    name: f.name.clone(), params, ret_ty,
                    exported: f.exported, source: source.to_path_buf(),
                });
            }
            Item::Struct(s) => {
                let fields = s.fields.iter()
                    .map(|f| (f.name.clone(), type_to_llvm(&f.ty)))
                    .collect();
                self.structs.insert(s.name.clone(), StructSymbol {
                    name: s.name.clone(), fields, source: source.to_path_buf(),
                });
            }
            Item::Enum(e) => {
                let variants = e.variants.iter().map(|v| v.name.clone()).collect();
                self.enums.insert(e.name.clone(), EnumSymbol {
                    name: e.name.clone(), variants, source: source.to_path_buf(),
                });
            }
            Item::Mod(m) => m.items.iter().for_each(|i| self.collect_item(i, source)),
            _ => {}
        }
    }
}

// ─── Project-wide symbol scan ─────────────────────────────────────────────────

/// Parse every .xre/.xrb in `src_dir` and return a full SymbolTable.
/// Called once before any file is compiled.
pub fn collect_project_symbols(src_dir: &Path) -> SymbolTable {
    let mut table = SymbolTable::new();
    let Ok(entries) = fs::read_dir(src_dir) else { return table; };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext  = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "xre" && ext != "xrb" { continue; }
        let Ok(src) = fs::read_to_string(&path) else { continue; };
        let (program, _, _) = parse(&src);
        table.collect_from(&program, &path);
    }
    table
}

// ─── Per-file external reference detection ────────────────────────────────────

/// Walk a program and return every function name that is called but NOT
/// defined locally.  The result is used to emit `declare` statements.
pub fn collect_external_refs(program: &Program, local_fns: &HashSet<String>) -> Vec<String> {
    let mut calls: Vec<String> = Vec::new();
    for item in &program.items {
        calls_in_item(item, &mut calls);
    }
    let mut seen = HashSet::new();
    calls.into_iter()
        .filter(|n| !local_fns.contains(n) && !is_builtin(n) && seen.insert(n.clone()))
        .collect()
}

fn is_builtin(n: &str) -> bool {
    matches!(n, "printf"|"puts"|"malloc"|"free"|"exit"|"main"|"xore_main")
}

/// Collect all locally-defined function names in a program.
pub fn local_fn_names(program: &Program) -> HashSet<String> {
    let mut names = HashSet::new();
    for item in &program.items {
        if let Item::Function(f) = item { names.insert(f.name.clone()); }
    }
    names
}

// ── AST walkers ───────────────────────────────────────────────────────────────

fn calls_in_item(item: &Item, out: &mut Vec<String>) {
    match item {
        Item::Function(f) => calls_in_block(&f.body, out),
        Item::Mod(m)      => m.items.iter().for_each(|i| calls_in_item(i, out)),
        _ => {}
    }
}

fn calls_in_block(block: &Block, out: &mut Vec<String>) {
    block.stmts.iter().for_each(|s| calls_in_stmt(s, out));
}

fn calls_in_stmt(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Let(l)       => { l.init.iter().for_each(|e| calls_in_expr(e, out)); }
        Stmt::Return(r)    => { r.value.iter().for_each(|e| calls_in_expr(e, out)); }
        Stmt::Expr(e)      => calls_in_expr(e, out),
        Stmt::If(i)        => {
            calls_in_expr(&i.cond, out);
            calls_in_block(&i.then_body, out);
            if let Some(els) = &i.else_body {
                match els.as_ref() {
                    ElseBranch::Block(b) => calls_in_block(b, out),
                    ElseBranch::If(i2)   => calls_in_stmt(&Stmt::If(i2.clone()), out),
                }
            }
        }
        Stmt::While(w)     => { calls_in_expr(&w.cond, out); calls_in_block(&w.body, out); }
        Stmt::For(f)       => { calls_in_expr(&f.limit, out); calls_in_block(&f.body, out); }
        Stmt::End(e)       => { e.cond.iter().for_each(|c| calls_in_expr(c, out)); }
        Stmt::Match(m)     => {
            calls_in_expr(&m.subject, out);
            m.arms.iter().for_each(|a| calls_in_block(&a.body, out));
        }
        Stmt::Switch(s)    => {
            calls_in_expr(&s.subject, out);
            s.cases.iter().for_each(|c| calls_in_block(&c.body, out));
            s.default.iter().for_each(|d| calls_in_block(d, out));
        }
        Stmt::FnDecl(f)    => calls_in_block(&f.body, out),
        _ => {}
    }
}

fn calls_in_expr(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident { name, .. } = callee.as_ref() {
                out.push(name.clone());
            }
            args.iter().for_each(|a| calls_in_expr(a, out));
        }
        Expr::MacroCall { args, .. }       => args.iter().for_each(|a| calls_in_expr(a, out)),
        Expr::BinOp { lhs, rhs, .. }       => { calls_in_expr(lhs, out); calls_in_expr(rhs, out); }
        Expr::UnOp { operand, .. }         => calls_in_expr(operand, out),
        Expr::Assign { target, value, .. } => { calls_in_expr(target, out); calls_in_expr(value, out); }
        Expr::Paren   { inner, .. }        => calls_in_expr(inner, out),
        Expr::Bracket { inner, .. }        => calls_in_expr(inner, out),
        Expr::Field   { object, .. }       => calls_in_expr(object, out),
        Expr::TypeCall { args, .. }        => args.iter().for_each(|a| calls_in_expr(a, out)),
        _ => {}
    }
}

// ─── Type expression → LLVM type ─────────────────────────────────────────────

pub fn type_to_llvm(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Named(name, _) => match name.as_str() {
            "i8"|"u8"   => "i8",
            "i16"|"u16" => "i16",
            "i32"|"u32" => "i32",
            "i64"|"u64" => "i64",
            "f32"       => "float",
            "f64"       => "double",
            "bool"      => "i1",
            "void"      => "void",
            "char"      => "i32",
            "usize"|"isize" => "i64",
            _           => "ptr",
        }.into(),
        TypeExpr::Void(_)         => "void".into(),
        TypeExpr::Ref(_,_) | TypeExpr::MutRef(_,_) |
        TypeExpr::Pointer(_,_)    => "ptr".into(),
        TypeExpr::Slice(_,_) | TypeExpr::Array(_,_,_) => "ptr".into(),
        TypeExpr::ErrorUnion(t,_) => type_to_llvm(t),
        TypeExpr::FnPtr{..}       => "ptr".into(),
        TypeExpr::Infer(_)        => "i64".into(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn collects_export_fn() {
        let src = "@export fn add(a: i32, b: i32) -> i32 { return a + b; }";
        let (prog, _, _) = parse(src);
        let mut t = SymbolTable::new();
        t.collect_from(&prog, Path::new("math.xre"));
        let sym = t.fns.get("add").unwrap();
        assert_eq!(sym.ret_ty, "i32");
        assert_eq!(sym.params.len(), 2);
        assert!(sym.exported);
    }

    #[test]
    fn declare_fn_format() {
        let src = "@export fn mul(a: i64, b: i64) -> i64 { return a * b; }";
        let (prog, _, _) = parse(src);
        let mut t = SymbolTable::new();
        t.collect_from(&prog, Path::new("math.xre"));
        let decl = t.declare_fn("mul").unwrap();
        assert!(decl.starts_with("declare i64 @mul("), "got: {decl}");
    }

    #[test]
    fn external_refs_detected() {
        let src = "public main() { println!(add(1, 2)); println!(mul(3, 4)); }";
        let (prog, _, _) = parse(src);
        let locals: HashSet<String> = ["main".into()].into();
        let refs = collect_external_refs(&prog, &locals);
        assert!(refs.contains(&"add".to_string()));
        assert!(refs.contains(&"mul".to_string()));
    }

    #[test]
    fn local_fns_excluded() {
        let src = "fn local() -> i64 { return 1; } public main() { println!(local()); }";
        let (prog, _, _) = parse(src);
        let locals = local_fn_names(&prog);
        let refs = collect_external_refs(&prog, &locals);
        assert!(!refs.contains(&"local".to_string()));
    }

    #[test]
    fn nested_call_in_for_detected() {
        let src = "public main() { for in i range(5) { println!(helper(i)); } }";
        let (prog, _, _) = parse(src);
        let locals: HashSet<String> = ["main".into()].into();
        let refs = collect_external_refs(&prog, &locals);
        assert!(refs.contains(&"helper".to_string()));
    }

    #[test]
    fn collects_struct_and_enum() {
        let src = "struct Pt { x: i32, y: i32, } enum Dir { N, S, }";
        let (prog, _, _) = parse(src);
        let mut t = SymbolTable::new();
        t.collect_from(&prog, Path::new("t.xre"));
        assert!(t.structs.contains_key("Pt"));
        assert!(t.enums.contains_key("Dir"));
    }
}
