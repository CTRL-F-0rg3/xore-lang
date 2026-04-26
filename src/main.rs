// xore — compiler and build system
//
// Commands:
//   xore new <name> [--lib] [--osdev]   create new project
//   xore build [--release] [--elf]      compile project
//   xore run   [--release]              build + execute
//   xore check                          parse + typecheck only
//   xore clean                          remove build/
//   xore version                        print compiler version
//   xore --tokens <file.xre>             dump token stream
//   xore --ast    <file.xre>             dump AST
//   xore --ir     <file.xre>             dump LLVM IR

use std::{env, fs, path::{Path, PathBuf}, process};

use xore_lang::ast::*;
use xore_lang::codegen::Codegen;
use xore_lang::libx::LibxWriter;
use xore_lang::project::{
    collect_sources, scaffold_project, Manifest, OptLevel, ProjectType, SourceUnit, Target,
};
use xore_lang::typeck::TypeChecker;
use xore_lang::{lex, parse, KeywordGroup, NumBase, TokenKind};

const VERSION: &str = "0.1.0";

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.as_slice() {
        // ── Dev / debug commands (single file, no project) ────────────────
        [_, flag, path] if flag == "--tokens" => run_tokens(&read_file(path)),
        [_, flag, path] if flag == "--ast"    => run_ast(&read_file(path), path),
        [_, flag, path] if flag == "--ir"     => run_ir(&read_file(path), path),

        // ── Project commands ──────────────────────────────────────────────
        [_, cmd] if cmd == "version" => cmd_version(),
        [_, cmd] if cmd == "clean"   => cmd_clean(),
        [_, cmd] if cmd == "check"   => cmd_check(),

        [_, cmd, name] if cmd == "new" => {
            cmd_new(name, ProjectType::Bin);
        }
        [_, cmd, name, flag] if cmd == "new" && flag == "--lib" => {
            cmd_new(name, ProjectType::Lib);
        }
        [_, cmd, name, flag] if cmd == "new" && flag == "--osdev" => {
            cmd_new(name, ProjectType::Osdev);
        }

        [_, cmd] if cmd == "build" => { cmd_build(false, false); }
        [_, cmd, flag] if cmd == "build" && flag == "--release" => { cmd_build(true, false); }
        [_, cmd, flag] if cmd == "build" && flag == "--elf" => { cmd_build(false, true); }
        [_, cmd, f1, f2] if cmd == "build" && f1 == "--release" && f2 == "--elf" => {
            cmd_build(true, true);
        }

        [_, cmd] if cmd == "run" => cmd_run(false),
        [_, cmd, flag] if cmd == "run" && flag == "--release" => cmd_run(true),

        // ── No args → show help ───────────────────────────────────────────
        [_] => print_help(),
        _   => { eprintln!("error: unknown command. run `xore` for help."); process::exit(1); }
    }
}

// ─── Help ─────────────────────────────────────────────────────────────────────

fn print_help() {
    println!("xore {} — Xore language compiler and build system", VERSION);
    println!();
    println!("USAGE:");
    println!("  xore new <name>              create new binary project");
    println!("  xore new <name> --lib        create new library project");
    println!("  xore new <name> --osdev      create new bare-metal project");
    println!();
    println!("  xore build                   compile project (debug)");
    println!("  xore build --release         compile project (optimised)");
    println!("  xore build --elf             compile to bare-metal ELF");
    println!("  xore run                     build + run");
    println!("  xore run   --release         build (release) + run");
    println!("  xore check                   parse + check without compiling");
    println!("  xore clean                   remove build/ directory");
    println!("  xore version                 print version");
    println!();
    println!("DEBUG:");
    println!("  xore --tokens <file.xre>      dump token stream");
    println!("  xore --ast    <file.xre>      dump AST");
    println!("  xore --ir     <file.xre>      dump generated LLVM IR");
    println!();
    println!("SOURCE FILES:");
    println!("  .xre    regular source file");
    println!("  .xrs   module spec  (interface — critical modules require both)");
    println!("  .xrb   module body  (implementation — critical modules require both)");
    println!();
    println!("OUTPUT FORMATS:");
    println!("  native binary   (xore build)");
    println!("  .libx           (xore build, type = lib)");
    println!("  .elf            (xore build --elf, type = osdev)");
}

fn cmd_version() {
    println!("xore {VERSION}");
    println!("target: x86_64-linux | x86_64-windows | bare-metal");
    println!("formats: .xre .xrs .xrb → native binary | .libx | .elf");
}

// ─── xore new ────────────────────────────────────────────────────────────────

fn cmd_new(name: &str, ty: ProjectType) {
    let root = PathBuf::from(name);
    if root.exists() {
        fatal(&format!("directory `{name}` already exists"));
    }
    fs::create_dir_all(&root).unwrap_or_else(|e| fatal(&format!("mkdir: {e}")));

    scaffold_project(name, &root, ty.clone())
        .unwrap_or_else(|e| fatal(&e.to_string()));

    let type_label = match ty {
        ProjectType::Bin   => "binary",
        ProjectType::Lib   => "library",
        ProjectType::Osdev => "osdev (bare-metal)",
    };
    println!("  created  {name}/");
    println!("  created  {name}/xore.project");
    println!("  created  {name}/src/main.xre");
    println!("  created  {name}/.gitignore");
    println!();
    println!("✓ new {type_label} project `{name}`");
    println!();
    println!("  cd {name}");
    println!("  xore run");
}

// ─── xore check ──────────────────────────────────────────────────────────────

fn cmd_check() {
    let manifest = load_manifest();
    let src_dir  = manifest.root.join("src");

    println!("  checking {} v{}", manifest.name, manifest.version);

    let units = collect_sources(&src_dir)
        .unwrap_or_else(|e| fatal(&e.to_string()));

    let mut total_errors = 0usize;

    for unit in &units {
        match unit {
            SourceUnit::Single(path) => {
                let errors = check_file(path);
                total_errors += errors;
            }
            SourceUnit::Module { spec, body, name } => {
                println!("  module   {name}  (.xrs + .xrb)");
                total_errors += check_file(spec);
                total_errors += check_file(body);
            }
        }
    }

    if total_errors == 0 {
        println!("✓ check ok — 0 errors");
    } else {
        eprintln!("✗ check failed — {total_errors} error(s)");
        process::exit(1);
    }
}

fn check_file(path: &Path) -> usize {
    let src = read_file(path.to_str().unwrap_or("?"));
    let (prog, lex_errs, parse_errs) = parse(&src);

    let mut n = lex_errs.len() + parse_errs.len();
    for e in &lex_errs   { eprintln!("  lex   {}:{e}", path.display()); }
    for e in &parse_errs { eprintln!("  parse {}:{e}", path.display()); }

    // Type checker pass
    let mut tc = TypeChecker::new();
    tc.check_program(&prog);
    for e in &tc.errors { eprintln!("  type  {}:{e}", path.display()); }
    n += tc.errors.len();
    n
}

// ─── xore build ──────────────────────────────────────────────────────────────

fn cmd_build(release: bool, force_elf: bool) -> PathBuf {
    let mut manifest = load_manifest();

    // --release overrides manifest
    if release { manifest.optimize = OptLevel::Release; }
    // --elf overrides type
    if force_elf { manifest.ty = ProjectType::Osdev; }

    let src_dir   = manifest.root.join("src");
    let build_dir = manifest.build_dir();
    let opt_label = if release { "release" } else { "debug" };

    println!("  compiling {} v{} [{}]", manifest.name, manifest.version, opt_label);

    fs::create_dir_all(&build_dir).unwrap_or_else(|e| fatal(&format!("mkdir build: {e}")));

    let units = collect_sources(&src_dir)
        .unwrap_or_else(|e| fatal(&e.to_string()));

    let mut obj_files: Vec<PathBuf> = Vec::new();

    for unit in &units {
        match unit {
            SourceUnit::Single(path) => {
                if let Some(obj) = compile_unit(path, &build_dir, &manifest) {
                    obj_files.push(obj);
                }
            }
            SourceUnit::Module { spec, body, name } => {
                // For critical modules: check spec matches body signatures,
                // then compile body.
                println!("  module   {name}  (.xrs + .xrb)");
                validate_module_pair(spec, body);
                if let Some(obj) = compile_unit(body, &build_dir, &manifest) {
                    obj_files.push(obj);
                }
            }
        }
    }

    let output = manifest.output_path();
    link_objects(&obj_files, &output, &manifest);

    // For library projects — also package as .libx
    if manifest.ty == ProjectType::Lib {
        package_libx(&obj_files, &manifest);
    }

    println!("✓ built {} → {}", manifest.name, output.display());
    output
}

/// Compile one source file: .xre or .xrb → LLVM IR → object file.
fn compile_unit(path: &Path, build_dir: &Path, manifest: &Manifest) -> Option<PathBuf> {
    let src = read_file(path.to_str().unwrap_or("?"));
    let (program, lex_errs, parse_errs) = parse(&src);

    if !lex_errs.is_empty() || !parse_errs.is_empty() {
        for e in &lex_errs   { eprintln!("  lex   {}:{e}", path.display()); }
        for e in &parse_errs { eprintln!("  parse {}:{e}", path.display()); }
        process::exit(1);
    }

    // Type checker
    let mut tc = TypeChecker::new();
    tc.check_program(&program);
    if !tc.errors.is_empty() {
        for e in &tc.errors { eprintln!("  type  {}:{e}", path.display()); }
        process::exit(1);
    }

    let stem = path.file_stem()?.to_str()?;
    let ll_path  = build_dir.join(format!("{stem}.ll"));
    let obj_path = build_dir.join(format!("{stem}.o"));

    // Generate LLVM IR
    let mut cg = Codegen::new(path.to_str().unwrap_or("?"));
    let ir = cg.emit_program_with_target(&program, manifest.target.llvm_triple());

    fs::write(&ll_path, &ir).unwrap_or_else(|e| fatal(&format!("write IR: {e}")));
    println!("  emit     {}", ll_path.display());

    // Compile IR → object
    let opt = manifest.optimize.llvm_opt_flag();
    let llc = find_tool(&["llc", "llc-17", "llc-16", "llc-15"]);

    match llc {
        Some(llc_bin) => {
            let arch = match manifest.target {
                Target::BareMetal => vec!["-mtriple=x86_64-unknown-none"],
                _                 => vec![],
            };
            let status = process::Command::new(&llc_bin)
                .arg("-filetype=obj")
                .arg(opt)
                .args(&arch)
                .arg("-o").arg(&obj_path)
                .arg(&ll_path)
                .status()
                .unwrap_or_else(|e| fatal(&format!("llc: {e}")));
            if !status.success() { fatal("llc: compilation failed"); }
            println!("  compile  {}", obj_path.display());
            Some(obj_path)
        }
        None => {
            println!("  note: llc not found — IR written, skipping native compile");
            println!("        install LLVM: apt install llvm  (or brew install llvm)");
            None
        }
    }
}

/// Validate that a .xrs spec and .xrb body are consistent.
/// Currently checks that every @export in .xrb has a declaration in .xrs.
/// Full type-checking will be added in the semantic analysis pass.
fn validate_module_pair(spec_path: &Path, body_path: &Path) {
    let spec_src = read_file(spec_path.to_str().unwrap_or("?"));
    let body_src = read_file(body_path.to_str().unwrap_or("?"));

    let (spec_prog, _, _)  = parse(&spec_src);
    let (body_prog, _, _)  = parse(&body_src);

    // Collect @export names from body
    let body_exports: Vec<&str> = body_prog.items.iter()
        .filter_map(|item| match item {
            Item::Function(f) if f.exported => Some(f.name.as_str()),
            _ => None,
        })
        .collect();

    // Collect declared names from spec
    let spec_decls: Vec<&str> = spec_prog.items.iter()
        .filter_map(|item| match item {
            Item::Function(f) => Some(f.name.as_str()),
            Item::Struct(s)   => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    // Warn about missing spec entries (non-fatal for now — full resolver later)
    for name in &body_exports {
        if !spec_decls.contains(name) {
            eprintln!("  warning: @export fn `{name}` in {} has no declaration in {}",
                body_path.display(), spec_path.display());
        }
    }
}

/// Link object files into final output.
fn link_objects(objs: &[PathBuf], output: &Path, manifest: &Manifest) {
    if objs.is_empty() {
        println!("  note: no object files to link (LLVM not installed?)");
        return;
    }

    let clang = find_tool(&["clang", "clang-17", "clang-16", "clang-15", "gcc"]);

    match clang {
        Some(linker) => {
            let mut cmd = process::Command::new(&linker);
            for obj in objs { cmd.arg(obj); }
            cmd.arg("-o").arg(output);

            match manifest.ty {
                ProjectType::Osdev => {
                    // Bare-metal: no libc, no standard startup
                    cmd.args(["-nostdlib", "-static", "-Wl,-e,_start"]);
                }
                ProjectType::Lib => {
                    cmd.arg("-shared");
                }
                ProjectType::Bin => {
                    cmd.arg("-no-pie");
                }
            }

            match manifest.optimize {
                OptLevel::Release => { cmd.arg("-O3"); }
                OptLevel::Debug   => { cmd.arg("-g"); }
            }

            let status = cmd.status()
                .unwrap_or_else(|e| fatal(&format!("linker: {e}")));
            if !status.success() { fatal("link failed"); }
        }
        None => {
            println!("  note: no linker found (clang/gcc) — objects built but not linked");
            println!("        install: apt install clang  (or brew install llvm)");
        }
    }
}

/// Package compiled objects + type info into a .libx archive.
fn package_libx(objs: &[PathBuf], manifest: &Manifest) {
    let libx_path = manifest.build_dir()
        .join(format!("{}.libx", manifest.name));

    let mut writer = LibxWriter::new(&manifest.name, manifest.target.llvm_triple());

    // Add all object files
    for obj in objs {
        if let Err(e) = writer.add_object_file(obj) {
            eprintln!("  warning: libx: could not add {}: {e}", obj.display());
        }
    }

    // Collect exported signatures from source files
    let src_dir = manifest.root.join("src");
    if let Ok(units) = collect_sources(&src_dir) {
        for unit in &units {
            let path = match unit {
                SourceUnit::Single(p)       => p.clone(),
                SourceUnit::Module { body, .. } => body.clone(),
            };
            let src = fs::read_to_string(&path).unwrap_or_default();
            let (prog, _, _) = parse(&src);

            // Collect @export functions → add to SIGS section
            for item in &prog.items {
                if let Item::Function(f) = item {
                    if f.exported {
                        let params: Vec<String> = f.params.iter()
                            .map(|p| format!("{}", p.name))
                            .collect();
                        let ret = f.ret_ty.as_ref()
                            .map(|t| match t {
                                TypeExpr::Named(n, _) => n.clone(),
                                TypeExpr::Void(_)     => "void".into(),
                                _ => "ptr".into(),
                            })
                            .unwrap_or_else(|| "void".into());
                        writer.add_sig(xore_lang::libx::FnSig {
                            name: f.name.clone(), params, ret,
                        });
                    }
                }
                // Collect structs and enums
                if let Item::Struct(s) = item {
                    let fields: Vec<(&str, &str)> = s.fields.iter()
                        .map(|f| (f.name.as_str(), "i64"))  // simplified type for now
                        .collect();
                    writer.add_struct(&s.name, &fields);
                }
                if let Item::Enum(e) = item {
                    let variants: Vec<(&str, u64)> = e.variants.iter()
                        .enumerate()
                        .map(|(i, v)| (v.name.as_str(), i as u64))
                        .collect();
                    writer.add_enum(&e.name, &variants);
                }
            }
        }
    }

    match writer.write(&libx_path) {
        Ok(()) => println!("  packaged {}", libx_path.display()),
        Err(e) => eprintln!("  warning: libx packaging failed: {e}"),
    }
}

// ─── xore run ────────────────────────────────────────────────────────────────

fn cmd_run(release: bool) {
    let output = cmd_build(release, false);

    if !output.exists() {
        println!("  note: binary not produced (missing LLVM toolchain)");
        println!("        run `xore --ir src/main.xre` to see the generated IR");
        return;
    }

    println!("  running  {}", output.display());
    println!("{}", "─".repeat(50));

    let status = process::Command::new(&output)
        .status()
        .unwrap_or_else(|e| fatal(&format!("run: {e}")));

    println!("{}", "─".repeat(50));
    process::exit(status.code().unwrap_or(0));
}

// ─── xore clean ──────────────────────────────────────────────────────────────

fn cmd_clean() {
    let manifest = load_manifest();
    let build_dir = manifest.build_dir();
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir)
            .unwrap_or_else(|e| fatal(&format!("clean: {e}")));
        println!("  removed  {}", build_dir.display());
    } else {
        println!("  nothing to clean");
    }
    println!("✓ clean");
}

// ─── Debug commands (single-file, no project needed) ─────────────────────────

fn run_tokens(src: &str) {
    let (tokens, errors) = lex(src);
    println!("{:<6} {:<5} {:<22} {}", "LINE", "COL", "KIND", "VALUE");
    println!("{}", "─".repeat(62));
    for tok in &tokens {
        if tok.kind == TokenKind::Eof { break; }
        println!("{:<6} {:<5} {:<22} {}",
            tok.span.line, tok.span.col, kind_label(&tok.kind), kind_value(&tok.kind));
    }
    if !errors.is_empty() {
        for e in &errors { eprintln!("✗ {e}"); }
        process::exit(1);
    }
}

fn run_ast(src: &str, path: &str) {
    let (program, lex_errs, parse_errs) = parse(src);
    println!("── AST: {path} ({} item(s)) ──────────────", program.items.len());
    for item in &program.items { print_item(item, 0); }
    finish(&lex_errs, &parse_errs);
}

fn run_ir(src: &str, path: &str) {
    let (program, lex_errs, parse_errs) = parse(src);
    finish(&lex_errs, &parse_errs);
    let mut cg = Codegen::new(path);
    println!("{}", cg.emit_program(&program));
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn load_manifest() -> Manifest {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Manifest::load(&cwd).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        eprintln!("hint:  run `xore new <name>` to create a project");
        process::exit(1);
    })
}

fn find_tool(candidates: &[&str]) -> Option<String> {
    for name in candidates {
        let out = process::Command::new("which").arg(name).output().ok()?;
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() { return Some(s); }
        }
    }
    None
}

fn finish(lex_errs: &[xore_lang::LexError], parse_errs: &[xore_lang::ParseError]) {
    let n = lex_errs.len() + parse_errs.len();
    if n > 0 {
        for e in lex_errs   { eprintln!("  lex:   {e}"); }
        for e in parse_errs { eprintln!("  parse: {e}"); }
        process::exit(1);
    }
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
        Item::Use(u)      => println!("{}use {}",    ind(d), u.path.join(".")),
        Item::Mod(m) => {
            println!("{}mod {} {{", ind(d), m.name);
            for i in &m.items { print_item(i, d + 1); }
            println!("{}}}", ind(d));
        }
    }
}

fn print_fn(f: &FnDecl, d: usize) {
    let exp = if f.exported { "@export " } else { "" };
    let vis = vis_str(&f.vis);
    let ret = f.ret_ty.as_ref().map(|t| format!(" -> {}", type_str(t))).unwrap_or_default();
    let params: Vec<String> = f.params.iter()
        .map(|p| format!("{}: {}", p.name, type_str(&p.ty))).collect();
    println!("{}{}{}fn {}({}){} {{", ind(d), exp, vis, f.name, params.join(", "), ret);
    print_block(&f.body, d + 1);
    println!("{}}}", ind(d));
}

fn print_struct(s: &StructDecl, d: usize) {
    println!("{}{}struct {} {{", ind(d), vis_str(&s.vis), s.name);
    for f in &s.fields { println!("{}  {}: {},", ind(d), f.name, type_str(&f.ty)); }
    println!("{}}}", ind(d));
}

fn print_enum(e: &EnumDecl, d: usize) {
    println!("{}{}enum {} {{", ind(d), vis_str(&e.vis), e.name);
    for v in &e.variants {
        if v.fields.is_empty() { println!("{}  {},", ind(d), v.name); }
        else {
            let fs: Vec<String> = v.fields.iter().map(type_str).collect();
            println!("{}  {}({}),", ind(d), v.name, fs.join(", "));
        }
    }
    println!("{}}}", ind(d));
}

fn print_block(b: &Block, d: usize) { for s in &b.stmts { print_stmt(s, d); } }

fn print_stmt(stmt: &Stmt, d: usize) {
    let p = ind(d);
    match stmt {
        Stmt::Let(l) => {
            let m   = if l.mutable { "mut " } else { "" };
            let ty  = l.ty.as_ref().map(|t| format!(": {}", type_str(t))).unwrap_or_default();
            let ini = l.init.as_ref().map(|e| format!(" = {}", expr_str(e))).unwrap_or_default();
            println!("{p}let {m}{}{ty}{ini};", l.name);
        }
        Stmt::Return(r) => {
            let v = r.value.as_ref().map(|e| format!(" {}", expr_str(e))).unwrap_or_default();
            println!("{p}return{v};");
        }
        Stmt::Expr(e)       => println!("{p}{};", expr_str(e)),
        Stmt::If(i)         => print_if(i, d),
        Stmt::While(w)      => {
            println!("{p}while {} {{", expr_str(&w.cond));
            print_block(&w.body, d + 1);
            println!("{p}}}");
        }
        Stmt::For(f)        => {
            println!("{p}for in {} range ({}) {{", f.var, expr_str(&f.limit));
            print_block(&f.body, d + 1);
            println!("{p}}}");
        }
        Stmt::End(e)        => match &e.cond {
            None    => println!("{p}end();"),
            Some(c) => println!("{p}end() if {};", expr_str(c)),
        },
        Stmt::Match(m)      => {
            println!("{p}match {} {{", expr_str(&m.subject));
            for arm in &m.arms {
                let pat = match &arm.pattern {
                    Pattern::Wildcard     => "_".to_string(),
                    Pattern::Bind(n)      => n.clone(),
                    Pattern::Lit(e)       => expr_str(e),
                    Pattern::Variant(e, Some(v)) => format!("{e}.{v}"),
                    Pattern::Variant(e, None)    => e.clone(),
                    Pattern::Range(lo, hi) => format!("{}..{}", expr_str(lo), expr_str(hi)),
                };
                println!("{p}  {pat} => {{");
                print_block(&arm.body, d + 2);
                println!("{p}  }}");
            }
            println!("{p}}}");
        }
        Stmt::Switch(s)     => {
            println!("{p}switch {} {{", expr_str(&s.subject));
            for case in &s.cases {
                println!("{p}  case {}: {{", expr_str(&case.value));
                print_block(&case.body, d + 2);
                println!("{p}  }}");
            }
            if let Some(def) = &s.default {
                println!("{p}  default: {{");
                print_block(def, d + 2);
                println!("{p}  }}");
            }
            println!("{p}}}");
        }
        Stmt::FnDecl(f)     => print_fn(f, d),
        Stmt::EnumDecl(e)   => print_enum(e, d),
        Stmt::StructDecl(s) => print_struct(s, d),
    }
}

fn print_if(i: &IfStmt, d: usize) {
    let p = ind(d);
    println!("{p}if [{}] {{", expr_str(&i.cond));
    print_block(&i.then_body, d + 1);
    match &i.else_body {
        None => println!("{p}}}"),
        Some(b) => match b.as_ref() {
            ElseBranch::Block(bl) => {
                println!("{p}}} else {{");
                print_block(bl, d + 1);
                println!("{p}}}");
            }
            ElseBranch::If(i2) => { println!("{p}}} else"); print_if(i2, d); }
        }
    }
}

fn vis_str(v: &Visibility) -> &'static str {
    match v { Visibility::Public => "public ", Visibility::Private => "" }
}

fn expr_str(e: &Expr) -> String {
    match e {
        Expr::IntLit   { value, .. }  => value.to_string(),
        Expr::FloatLit { value, .. }  => format!("{value}"),
        Expr::StrLit   { value, .. }  => format!("{value:?}"),
        Expr::BoolLit  { value, .. }  => if *value { "True".into() } else { "False".into() },
        Expr::NoneLit  { .. }         => "None".into(),
        Expr::Ident    { name, .. }   => name.clone(),
        Expr::Field { object, field, .. } =>
            format!("{}.{}", expr_str(object), field),
        Expr::FmtChain { parts, .. }  => parts.iter().map(|p| match p {
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
        Expr::BinOp { op, lhs, rhs, .. } =>
            format!("{} {} {}", expr_str(lhs), binop_str(*op), expr_str(rhs)),
        Expr::UnOp  { op, operand, .. }  =>
            format!("{}{}", unop_str(*op), expr_str(operand)),
        Expr::Assign { target, op, value, .. } =>
            format!("{} {} {}", expr_str(target), assign_op_str(*op), expr_str(value)),
        Expr::Paren   { inner, .. } => format!("({})", expr_str(inner)),
        Expr::Bracket { inner, .. } => format!("[{}]", expr_str(inner)),
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
        BinOp::Add=>"+" ,BinOp::Sub=>"-" ,BinOp::Mul=>"*" ,BinOp::Div=>"/" ,BinOp::Rem=>"%",
        BinOp::BitAnd=>"&",BinOp::BitOr=>"|",BinOp::BitXor=>"^",BinOp::Shl=>"<<",BinOp::Shr=>">>",
        BinOp::And=>"&&",BinOp::Or=>"||",
        BinOp::Eq=>"==",BinOp::Ne=>"!=",BinOp::Lt=>"<",BinOp::Gt=>">",BinOp::Le=>"<=",BinOp::Ge=>">=",
        BinOp::Range=>"..",BinOp::RangeInclusive=>"..=",
    }
}

fn unop_str(op: UnOp) -> &'static str {
    match op { UnOp::Neg=>"-",UnOp::Not=>"!",UnOp::Ref=>"&",UnOp::Deref=>"*" }
}

fn assign_op_str(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign=>"=",AssignOp::AddAssign=>"+=",AssignOp::SubAssign=>"-=",
        AssignOp::MulAssign=>"*=",AssignOp::DivAssign=>"/=",AssignOp::RemAssign=>"%=",
        AssignOp::AndAssign=>"&=",AssignOp::OrAssign=>"|=",AssignOp::XorAssign=>"^=",
        AssignOp::ShlAssign=>"<<=",AssignOp::ShrAssign=>">>=",
    }
}

fn kind_label(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Kw(kw) => match kw.group() {
            KeywordGroup::Core=>"KW:CORE", KeywordGroup::Asm=>"KW:ASM",
            KeywordGroup::Memory=>"KW:MEM", KeywordGroup::Module=>"KW:MOD",
            KeywordGroup::Meta=>"KW:META", KeywordGroup::Type=>"KW:TYPE",
        }.into(),
        TokenKind::Annotation(_)=>"ANNOTATION".into(), TokenKind::Ident(_)=>"IDENT".into(),
        TokenKind::IntLit{..}=>"INT_LIT".into(), TokenKind::FloatLit{..}=>"FLOAT_LIT".into(),
        TokenKind::StrLit(_)=>"STR_LIT".into(), TokenKind::RawStrLit(_)=>"RAW_STR_LIT".into(),
        TokenKind::CharLit(_)=>"CHAR_LIT".into(), TokenKind::Op(_)=>"OPERATOR".into(),
        TokenKind::Delim(_)=>"DELIM".into(), TokenKind::Punct(_)=>"PUNCT".into(),
        TokenKind::LineComment(_)=>"COMMENT:LINE".into(),
        TokenKind::BlockComment(_)=>"COMMENT:BLOCK".into(),
        TokenKind::Unknown(_)=>"UNKNOWN".into(), TokenKind::Eof=>"EOF".into(),
    }
}

fn kind_value(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Kw(kw)               => kw.as_str().into(),
        TokenKind::Annotation(s)        => format!("@{s}"),
        TokenKind::Ident(s)             => s.clone(),
        TokenKind::IntLit { value, base, suffix } => {
            let p = match base {
                NumBase::Hex=>"0x", NumBase::Octal=>"0o",
                NumBase::Binary=>"0b", NumBase::Decimal=>"",
            };
            let s = suffix.map(|x| format!(" ({x:?})")).unwrap_or_default();
            format!("{p}{value}{s}")
        }
        TokenKind::FloatLit{value,..}   => format!("{value}"),
        TokenKind::StrLit(s)            => format!("{s:?}"),
        TokenKind::RawStrLit(s)         => format!("r#{s:?}"),
        TokenKind::CharLit(c)           => format!("'{c}'"),
        TokenKind::Op(o)                => format!("{o:?}"),
        TokenKind::Delim(d)             => format!("{d:?}"),
        TokenKind::Punct(p)             => format!("{p:?}"),
        TokenKind::LineComment(s)       => format!("//{s}"),
        TokenKind::BlockComment(s)      => format!("/* {}… */", &s[..s.len().min(20)]),
        TokenKind::Unknown(c)           => format!("'{c}'"),
        TokenKind::Eof                  => "<EOF>".into(),
    }
}