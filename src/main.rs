// xore compiler driver
//
// Pipeline:  .xr source → Lexer → Parser → AST → Codegen → native binary
//
// Usage:
//   xore <file.xr>            — compile to native binary (same dir)
//   xore --tokens <file.xr>   — dump token stream
//   xore --ast    <file.xr>   — dump parsed AST
//   xore --ir     <file.xr>   — dump generated IR (LLVM text)
//   xore --help

use std::{env, fs, path::Path, process};

use xore_lang::ast::*;
use xore_lang::codegen::Codegen;
use xore_lang::{lex, parse, KeywordGroup, NumBase, TokenKind};

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.as_slice() {
        [_, flag] if flag == "--help" => print_help(),

        [_, flag, path] if flag == "--tokens" => {
            run_tokens(&read_file(path));
        }
        [_, flag, path] if flag == "--ast" => {
            run_ast(&read_file(path), path);
        }
        [_, flag, path] if flag == "--ir" => {
            run_ir(&read_file(path), path);
        }
        [_, path] if !path.starts_with('-') => {
            run_compile(&read_file(path), path);
        }
        // No args → run bundled demo
        [_] => {
            run_ast(DEMO_XR, "<demo>");
        }
        _ => {
            eprintln!("error: unknown arguments. run `xore --help`");
            process::exit(1);
        }
    }
}

fn print_help() {
    println!("xore compiler v0.1.0");
    println!();
    println!("USAGE:");
    println!("  xore <file.xr>           compile to native binary");
    println!("  xore --tokens <file.xr>  dump token stream");
    println!("  xore --ast    <file.xr>  dump parsed AST");
    println!("  xore --ir     <file.xr>  dump IR (codegen output)");
    println!();
    println!("SOURCE FILES:  .xr");
    println!("OUTPUT:        native ELF binary (Linux x86-64)");
}

// ─── Pipeline stages ─────────────────────────────────────────────────────────

fn run_tokens(src: &str) {
    let (tokens, errors) = lex(src);
    println!("{:<6} {:<5} {:<22} {}", "LINE", "COL", "KIND", "VALUE");
    println!("{}", "─".repeat(62));
    for tok in &tokens {
        if tok.kind == TokenKind::Eof { break; }
        println!("{:<6} {:<5} {:<22} {}",
            tok.span.line,
            tok.span.col,
            kind_label(&tok.kind),
            kind_value(&tok.kind),
        );
    }
    if !errors.is_empty() {
        eprintln!("\n{} lex error(s):", errors.len());
        for e in &errors { eprintln!("  ✗ {e}"); }
        process::exit(1);
    }
}

fn run_ast(src: &str, path: &str) {
    let (program, lex_errs, parse_errs) = parse(src);
    println!("── AST: {path} ({} items) ─────────────────────────", program.items.len());
    for item in &program.items { print_item(item, 0); }
    report_errors(&lex_errs, &parse_errs);
}

fn run_ir(src: &str, path: &str) {
    let (program, lex_errs, parse_errs) = parse(src);
    report_errors(&lex_errs, &parse_errs);
    let mut cg = Codegen::new(path);
    let ir = cg.emit_program(&program);
    println!("{ir}");
}

fn run_compile(src: &str, path: &str) {
    let (program, lex_errs, parse_errs) = parse(src);
    report_errors(&lex_errs, &parse_errs);

    // Generate IR
    let mut cg = Codegen::new(path);
    let ir = cg.emit_program(&program);

    // Write .ll file next to source
    let stem = Path::new(path).file_stem()
        .and_then(|s| s.to_str()).unwrap_or("out");
    let ll_path = format!("{stem}.ll");
    let obj_path = format!("{stem}.o");
    let bin_path = stem.to_string();

    fs::write(&ll_path, &ir).unwrap_or_else(|e| fatal(&format!("write {ll_path}: {e}")));
    println!("  wrote  {ll_path}");

    // Try to compile via LLVM toolchain if available
    let llc = which("llc").or_else(|| which("llc-17")).or_else(|| which("llc-16"));
    let clang = which("clang").or_else(|| which("clang-17")).or_else(|| which("clang-16"));

    match (llc, clang) {
        (Some(llc_bin), Some(clang_bin)) => {
            // llc → obj
            let status = process::Command::new(&llc_bin)
                .args(["-filetype=obj", "-o", &obj_path, &ll_path])
                .status().expect("llc failed");
            if !status.success() { fatal("llc: compilation failed"); }

            // clang → link
            let status = process::Command::new(&clang_bin)
                .args([&obj_path, "-o", &bin_path, "-no-pie"])
                .status().expect("clang link failed");
            if !status.success() { fatal("clang: link failed"); }

            let _ = fs::remove_file(&obj_path);
            println!("  linked {bin_path}");
            println!("✓ compiled ok → ./{bin_path}");
        }
        _ => {
            // LLVM not installed — give the user the IR so they can compile manually
            println!("  note: LLVM toolchain not found");
            println!("  to compile manually:");
            println!("    llc -filetype=obj -o {obj_path} {ll_path}");
            println!("    clang {obj_path} -o {bin_path} -no-pie");
            println!("  or:  clang -x ir {ll_path} -o {bin_path} -no-pie");
        }
    }
}

fn report_errors(lex_errs: &[xore_lang::LexError], parse_errs: &[xore_lang::ParseError]) {
    let total = lex_errs.len() + parse_errs.len();
    if total > 0 {
        for e in lex_errs   { eprintln!("  lex:   {e}"); }
        for e in parse_errs { eprintln!("  parse: {e}"); }
        process::exit(1);
    }
}

fn which(name: &str) -> Option<String> {
    process::Command::new("which").arg(name).output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn fatal(msg: &str) -> ! {
    eprintln!("error: {msg}");
    process::exit(1);
}

fn read_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read `{path}`: {e}");
        process::exit(1);
    })
}

// ─── AST printer ─────────────────────────────────────────────────────────────

fn ind(d: usize) -> String { "  ".repeat(d) }

fn print_item(item: &Item, d: usize) {
    match item {
        Item::Function(f) => print_fn(f, d),
        Item::Struct(s)   => print_struct(s, d),
        Item::Enum(e)     => print_enum(e, d),
        Item::Import(i)   => println!("{}import {}", ind(d), i.path.join(".")),
        Item::Use(u)      => println!("{}use {}", ind(d), u.path.join(".")),
        Item::Mod(m) => {
            println!("{}mod {} {{", ind(d), m.name);
            for i in &m.items { print_item(i, d + 1); }
            println!("{}}}", ind(d));
        }
    }
}

fn print_fn(f: &FnDecl, d: usize) {
    let vis = vis_str(&f.vis);
    let ret = f.ret_ty.as_ref().map(|t| format!(" -> {}", type_str(t))).unwrap_or_default();
    let params: Vec<String> = f.params.iter()
        .map(|p| format!("{}: {}", p.name, type_str(&p.ty))).collect();
    println!("{}{}fn {}({}){} {{", ind(d), vis, f.name, params.join(", "), ret);
    print_block(&f.body, d + 1);
    println!("{}}}", ind(d));
}

fn print_struct(s: &StructDecl, d: usize) {
    println!("{}{}struct {} {{", ind(d), vis_str(&s.vis), s.name);
    for field in &s.fields {
        println!("{}  {}: {},", ind(d), field.name, type_str(&field.ty));
    }
    println!("{}}}", ind(d));
}

fn print_enum(e: &EnumDecl, d: usize) {
    println!("{}{}enum {} {{", ind(d), vis_str(&e.vis), e.name);
    for v in &e.variants {
        if v.fields.is_empty() {
            println!("{}  {},", ind(d), v.name);
        } else {
            let fs: Vec<String> = v.fields.iter().map(type_str).collect();
            println!("{}  {}({}),", ind(d), v.name, fs.join(", "));
        }
    }
    println!("{}}}", ind(d));
}

fn print_block(block: &Block, d: usize) {
    for stmt in &block.stmts { print_stmt(stmt, d); }
}

fn print_stmt(stmt: &Stmt, d: usize) {
    let pad = ind(d);
    match stmt {
        Stmt::Let(l) => {
            let m   = if l.mutable { "mut " } else { "" };
            let ty  = l.ty.as_ref().map(|t| format!(": {}", type_str(t))).unwrap_or_default();
            let ini = l.init.as_ref().map(|e| format!(" = {}", expr_str(e))).unwrap_or_default();
            println!("{pad}let {m}{}{ty}{ini};", l.name);
        }
        Stmt::Return(r) => {
            let v = r.value.as_ref().map(|e| format!(" {}", expr_str(e))).unwrap_or_default();
            println!("{pad}return{v};");
        }
        Stmt::Expr(e)       => println!("{pad}{};", expr_str(e)),
        Stmt::If(i)         => print_if(i, d),
        Stmt::While(w)      => {
            println!("{pad}while {} {{", expr_str(&w.cond));
            print_block(&w.body, d + 1);
            println!("{pad}}}");
        }
        Stmt::For(f)        => {
            println!("{pad}for in {} range ({}) {{", f.var, expr_str(&f.limit));
            print_block(&f.body, d + 1);
            println!("{pad}}}");
        }
        Stmt::End(e)        => match &e.cond {
            None    => println!("{pad}end();"),
            Some(c) => println!("{pad}end() if {};", expr_str(c)),
        },
        Stmt::FnDecl(f)     => print_fn(f, d),
        Stmt::EnumDecl(e)   => print_enum(e, d),
        Stmt::StructDecl(s) => print_struct(s, d),
    }
}

fn print_if(i: &IfStmt, d: usize) {
    let pad = ind(d);
    println!("{pad}if [{}] {{", expr_str(&i.cond));
    print_block(&i.then_body, d + 1);
    match &i.else_body {
        None => println!("{pad}}}"),
        Some(b) => match b.as_ref() {
            ElseBranch::Block(bl) => {
                println!("{pad}}} else {{");
                print_block(bl, d + 1);
                println!("{pad}}}");
            }
            ElseBranch::If(i2) => {
                println!("{pad}}} else");
                print_if(i2, d);
            }
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn vis_str(v: &Visibility) -> &'static str {
    match v { Visibility::Public => "public ", Visibility::Private => "" }
}

fn expr_str(e: &Expr) -> String {
    match e {
        Expr::IntLit   { value, .. }       => value.to_string(),
        Expr::FloatLit { value, .. }       => format!("{value}"),
        Expr::StrLit   { value, .. }       => format!("{value:?}"),
        Expr::BoolLit  { value, .. }       => if *value { "True".into() } else { "False".into() },
        Expr::NoneLit  { .. }              => "None".into(),
        Expr::Ident    { name, .. }        => name.clone(),
        Expr::Field { object, field, .. }  => format!("{}.{}", expr_str(object), field),
        Expr::FmtChain { parts, .. }       => parts.iter().map(|p| match p {
            FmtPart::Str(s)  => format!("{s:?}"),
            FmtPart::Hole(e) => format!("{{{}}}", expr_str(e)),
            FmtPart::Field(f)=> format!(".{f}"),
        }).collect::<Vec<_>>().join(""),
        Expr::Call { callee, args, .. } => {
            let a: Vec<_> = args.iter().map(expr_str).collect();
            format!("{}({})", expr_str(callee), a.join(", "))
        }
        Expr::MacroCall { name, args, .. } => {
            let a: Vec<_> = args.iter().map(expr_str).collect();
            format!("{name}!({})", a.join(", "))
        }
        Expr::TypeCall { ty, args, .. } => {
            let a: Vec<_> = args.iter().map(expr_str).collect();
            format!("{}({})", type_str(ty), a.join(", "))
        }
        Expr::BinOp { op, lhs, rhs, .. }  =>
            format!("{} {} {}", expr_str(lhs), binop_str(*op), expr_str(rhs)),
        Expr::UnOp  { op, operand, .. }   =>
            format!("{}{}", unop_str(*op), expr_str(operand)),
        Expr::Assign { target, op, value, .. } =>
            format!("{} {} {}", expr_str(target), assign_op_str(*op), expr_str(value)),
        Expr::Paren   { inner, .. }        => format!("({})", expr_str(inner)),
        Expr::Bracket { inner, .. }        => format!("[{}]", expr_str(inner)),
    }
}

fn type_str(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named(n, _)       => n.clone(),
        TypeExpr::Ref(t, _)         => format!("&{}", type_str(t)),
        TypeExpr::MutRef(t, _)      => format!("&mut {}", type_str(t)),
        TypeExpr::Pointer(t, _)     => format!("*{}", type_str(t)),
        TypeExpr::Slice(t, _)       => format!("[{}]", type_str(t)),
        TypeExpr::Array(t, n, _)    => format!("[{}; {}]", type_str(t), expr_str(n)),
        TypeExpr::ErrorUnion(t, _)  => format!("!{}", type_str(t)),
        TypeExpr::FnPtr { params, ret, .. } => {
            let ps: Vec<_> = params.iter().map(type_str).collect();
            format!("fn({}) -> {}", ps.join(", "), type_str(ret))
        }
        TypeExpr::Infer(_) => "_".into(),
        TypeExpr::Void(_)  => "void".into(),
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",    BinOp::Sub => "-",   BinOp::Mul => "*",
        BinOp::Div => "/",    BinOp::Rem => "%",
        BinOp::BitAnd => "&", BinOp::BitOr => "|", BinOp::BitXor => "^",
        BinOp::Shl => "<<",   BinOp::Shr => ">>",
        BinOp::And => "&&",   BinOp::Or  => "||",
        BinOp::Eq  => "==",   BinOp::Ne  => "!=",
        BinOp::Lt  => "<",    BinOp::Gt  => ">",
        BinOp::Le  => "<=",   BinOp::Ge  => ">=",
        BinOp::Range => "..", BinOp::RangeInclusive => "..=",
    }
}

fn unop_str(op: UnOp) -> &'static str {
    match op { UnOp::Neg => "-", UnOp::Not => "!", UnOp::Ref => "&", UnOp::Deref => "*" }
}

fn assign_op_str(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign    => "=",    AssignOp::AddAssign => "+=",
        AssignOp::SubAssign => "-=",   AssignOp::MulAssign => "*=",
        AssignOp::DivAssign => "/=",   AssignOp::RemAssign => "%=",
        AssignOp::AndAssign => "&=",   AssignOp::OrAssign  => "|=",
        AssignOp::XorAssign => "^=",   AssignOp::ShlAssign => "<<=",
        AssignOp::ShrAssign => ">>=",
    }
}

fn kind_label(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Kw(kw) => match kw.group() {
            KeywordGroup::Core   => "KW:CORE",
            KeywordGroup::Asm    => "KW:ASM",
            KeywordGroup::Memory => "KW:MEM",
            KeywordGroup::Module => "KW:MOD",
            KeywordGroup::Meta   => "KW:META",
            KeywordGroup::Type   => "KW:TYPE",
        }.into(),
        TokenKind::Annotation(_)   => "ANNOTATION".into(),
        TokenKind::Ident(_)        => "IDENT".into(),
        TokenKind::IntLit { .. }   => "INT_LIT".into(),
        TokenKind::FloatLit { .. } => "FLOAT_LIT".into(),
        TokenKind::StrLit(_)       => "STR_LIT".into(),
        TokenKind::RawStrLit(_)    => "RAW_STR_LIT".into(),
        TokenKind::CharLit(_)      => "CHAR_LIT".into(),
        TokenKind::Op(_)           => "OPERATOR".into(),
        TokenKind::Delim(_)        => "DELIM".into(),
        TokenKind::Punct(_)        => "PUNCT".into(),
        TokenKind::LineComment(_)  => "COMMENT:LINE".into(),
        TokenKind::BlockComment(_) => "COMMENT:BLOCK".into(),
        TokenKind::Unknown(_)      => "UNKNOWN".into(),
        TokenKind::Eof             => "EOF".into(),
    }
}

fn kind_value(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Kw(kw)               => kw.as_str().into(),
        TokenKind::Annotation(s)        => format!("@{s}"),
        TokenKind::Ident(s)             => s.clone(),
        TokenKind::IntLit { value, base, suffix } => {
            let p = match base {
                NumBase::Hex => "0x", NumBase::Octal => "0o",
                NumBase::Binary => "0b", NumBase::Decimal => "",
            };
            let s = suffix.map(|x| format!(" ({x:?})")).unwrap_or_default();
            format!("{p}{value}{s}")
        }
        TokenKind::FloatLit { value, .. } => format!("{value}"),
        TokenKind::StrLit(s)              => format!("{s:?}"),
        TokenKind::RawStrLit(s)           => format!("r#{s:?}"),
        TokenKind::CharLit(c)             => format!("'{c}'"),
        TokenKind::Op(o)                  => format!("{o:?}"),
        TokenKind::Delim(d)               => format!("{d:?}"),
        TokenKind::Punct(p)               => format!("{p:?}"),
        TokenKind::LineComment(s)         => format!("//{s}"),
        TokenKind::BlockComment(s)        => format!("/* {}… */", &s[..s.len().min(20)]),
        TokenKind::Unknown(c)             => format!("'{c}'"),
        TokenKind::Eof                    => "<EOF>".into(),
    }
}

// ─── Bundled demo source ──────────────────────────────────────────────────────

const DEMO_XR: &str = r##"
// Xore-lang demo — main.xr

public main(){
    println!("data");
    let x = String.new();
    let y = String.new();
    let mut z = bool(False);

    if [x == y]{
        z %= True;
    }
    else {
        println!(None);
    }

    while z {
        end() if y == i32(23);
    }

    for in i range (30){
        println!(y);
        end()
    }

    fn new() -> Self {
        enum data {
            SELF,
            HOST,
        }
    }
}

public fn add(a: i32, b: i32) -> i32 {
    return a + b;
}

struct Point {
    x: f32,
    y: f32,
}

enum Direction {
    North,
    South,
    East,
    West,
}
"##;