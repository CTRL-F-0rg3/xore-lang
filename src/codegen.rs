// Xore → LLVM IR code generator.
//
// This module walks the AST and emits LLVM IR text format (.ll).
// The IR can then be compiled to native machine code via:
//
//   llc -filetype=obj out.ll -o out.o
//   clang out.o -o out -no-pie
//
// Or in one step:
//   clang -x ir out.ll -o out -no-pie
//
// Why LLVM IR?
//   - Gives us full optimisation pipeline for free (O0..O3, LTO)
//   - Targets x86-64, ARM64, RISC-V, WebAssembly with zero extra work
//   - Same approach used by Zig, Swift, Rust (rustc uses it via LLVM)
//   - We can later swap to direct x86-64 emission for a self-hosted backend

use crate::ast::*;

// ─── LLVM types ──────────────────────────────────────────────────────────────

/// Maps a Xore type name to its LLVM IR type string.
fn llvm_type(ty: &TypeExpr) -> &'static str {
    match ty {
        TypeExpr::Named(name, _) => match name.as_str() {
            "i8"   | "u8"   => "i8",
            "i16"  | "u16"  => "i16",
            "i32"  | "u32"  => "i32",
            "i64"  | "u64"  => "i64",
            "f32"           => "float",
            "f64"           => "double",
            "bool"          => "i1",
            "void"          => "void",
            "char"          => "i32",
            // Everything else (String, Self, etc.) → opaque pointer for now
            _               => "ptr",
        },
        TypeExpr::Void(_)         => "void",
        TypeExpr::Ref(_, _)       => "ptr",
        TypeExpr::MutRef(_, _)    => "ptr",
        TypeExpr::Pointer(_, _)   => "ptr",
        TypeExpr::Slice(_, _)     => "ptr",
        TypeExpr::Array(_, _, _)  => "ptr",
        TypeExpr::ErrorUnion(_, _)=> "ptr",
        TypeExpr::FnPtr { .. }    => "ptr",
        TypeExpr::Infer(_)        => "i64",   // default inference: 64-bit int
    }
}

#[allow(dead_code)]
fn llvm_type_default(name: &str) -> &'static str {
    match name {
        "i8"|"u8"   => "i8",  "i16"|"u16" => "i16",
        "i32"|"u32" => "i32", "i64"|"u64" => "i64",
        "f32"       => "float", "f64" => "double",
        "bool"      => "i1",    "void" => "void",
        _           => "ptr",
    }
}

// ─── Codegen context ─────────────────────────────────────────────────────────

pub struct Codegen {
    /// Output IR buffer.
    out: String,
    /// Module name (derived from source filename).
    module: String,
    /// Counter for unnamed temporaries: %0, %1, %2 …
    tmp: u32,
    /// Counter for basic-block labels.
    label: u32,
    /// String literal pool: (content, global_name)
    strings: Vec<(String, String)>,
    /// Global string counter.
    str_cnt: u32,
}

impl Codegen {
    pub fn new(source_path: &str) -> Self {
        Self {
            out: String::new(),
            module: source_path.to_string(),
            tmp: 0,
            label: 0,
            strings: Vec::new(),
            str_cnt: 0,
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    fn emit(&mut self, line: &str) {
        self.out.push_str(line);
        self.out.push('\n');
    }

    fn tmp(&mut self) -> String {
        let n = self.tmp;
        self.tmp += 1;
        format!("%t{n}")
    }

    fn label(&mut self, prefix: &str) -> String {
        let n = self.label;
        self.label += 1;
        format!("{prefix}{n}")
    }

    /// Intern a string literal and return a pointer expression to it.
    fn intern_string(&mut self, s: &str) -> String {
        // Check if already interned.
        for (content, name) in &self.strings {
            if content == s {
                return format!("ptr @{name}");
            }
        }
        let name = format!(".str{}", self.str_cnt);
        self.str_cnt += 1;
        self.strings.push((s.to_string(), name.clone()));
        format!("ptr @{name}")
    }

    fn reset_locals(&mut self) {
        self.tmp = 0;
        self.label = 0;
    }

    // ── Top-level emission ────────────────────────────────────────────────

    /// Walk the whole program and return the complete IR text.
    pub fn emit_program(&mut self, program: &Program) -> String {
        // LLVM module header
        self.emit(&format!("; Xore compiler v0.1.0 — module: {}", self.module));
        self.emit(&format!("source_filename = \"{}\"", self.module));
        self.emit("target datalayout = \"e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128\"");
        self.emit("target triple = \"x86_64-unknown-linux-gnu\"");
        self.emit("");

        // Declare external libc functions we may need
        self.emit("; ── external declarations ─────────────────────────────────");
        self.emit("declare i32 @printf(ptr noundef, ...)");
        self.emit("declare i32 @puts(ptr noundef)");
        self.emit("declare ptr @malloc(i64 noundef)");
        self.emit("declare void @free(ptr noundef)");
        self.emit("declare void @exit(i32 noundef)");
        self.emit("");

        // Collect all items
        let mut fns: Vec<&FnDecl>     = Vec::new();
        let mut structs: Vec<&StructDecl> = Vec::new();
        let mut enums: Vec<&EnumDecl>   = Vec::new();

        for item in &program.items {
            match item {
                Item::Function(f) => fns.push(f),
                Item::Struct(s)   => structs.push(s),
                Item::Enum(e)     => enums.push(e),
                _ => {}
            }
        }

        // Emit struct type declarations
        if !structs.is_empty() {
            self.emit("; ── struct types ──────────────────────────────────────────");
            for s in &structs { self.emit_struct_type(s); }
            self.emit("");
        }

        // Emit enum type comments (enums become i64 tags in IR)
        if !enums.is_empty() {
            self.emit("; ── enum tags (i64) ───────────────────────────────────────");
            for e in &enums { self.emit_enum_constants(e); }
            self.emit("");
        }

        // Emit function definitions
        self.emit("; ── function definitions ──────────────────────────────────");
        for f in &fns { self.emit_fn(f); }

        // Emit entry point trampoline if `main` exists and is not already `@main`
        let has_main = fns.iter().any(|f| f.name == "main");
        if has_main {
            self.emit_main_trampoline();
        }

        // Flush interned string globals
        if !self.strings.is_empty() {
            self.emit("; ── string literals ───────────────────────────────────────");
            let strings = self.strings.clone();
            for (content, name) in &strings {
                // Escape content for LLVM
                let escaped = llvm_escape_string(content);
                let len = content.len() + 1; // +1 for null terminator
                self.emit(&format!(
                    "@{name} = private unnamed_addr constant [{len} x i8] c\"{escaped}\\00\", align 1"
                ));
            }
        }

        self.out.clone()
    }

    // ── Struct types ──────────────────────────────────────────────────────

    fn emit_struct_type(&mut self, s: &StructDecl) {
        let fields: Vec<String> = s.fields.iter()
            .map(|f| llvm_type(&f.ty).to_string())
            .collect();
        self.emit(&format!("%struct.{} = type {{ {} }}", s.name, fields.join(", ")));
    }

    // ── Enum constants ────────────────────────────────────────────────────

    fn emit_enum_constants(&mut self, e: &EnumDecl) {
        self.emit(&format!("; enum {}", e.name));
        for (i, v) in e.variants.iter().enumerate() {
            self.emit(&format!(
                "@{}.{} = private unnamed_addr constant i64 {i}, align 8",
                e.name, v.name
            ));
        }
    }

    // ── Functions ─────────────────────────────────────────────────────────

    fn emit_fn(&mut self, f: &FnDecl) {
        self.reset_locals();

        let ret_ty = f.ret_ty.as_ref()
            .map(|t| llvm_type(t).to_string())
            .unwrap_or_else(|| "void".to_string());

        // Build param list
        let params: Vec<String> = f.params.iter()
            .map(|p| format!("{} noundef %{}", llvm_type(&p.ty), p.name))
            .collect();

        // Linkage: public → external, private → internal
        let linkage = match f.vis {
            Visibility::Public  => "",
            Visibility::Private => "internal ",
        };

        self.emit(&format!(
            "define {linkage}{ret_ty} @{}({}) {{",
            f.name, params.join(", ")
        ));
        self.emit("entry:");

        // Allocate space for parameters (alloca pattern for mutable access)
        for p in &f.params {
            let ty = llvm_type(&p.ty);
            self.emit(&format!("  %{}.addr = alloca {ty}, align 8", p.name));
            self.emit(&format!("  store {ty} %{}, ptr %{}.addr, align 8", p.name, p.name));
        }

        // Emit body
        let mut env = FnEnv::new(&f.params);
        self.emit_block(&f.body, &mut env, &ret_ty);

        // Ensure function ends with a terminator
        if !self.out.trim_end().ends_with(':')
            && !self.out.trim_end().ends_with("ret void")
            && !self.out.trim_end().ends_with('}')
        {
            if ret_ty == "void" {
                self.emit("  ret void");
            } else {
                self.emit(&format!("  ret {ret_ty} 0"));
            }
        }

        self.emit("}");
        self.emit("");
    }

    /// Emit `@main` entry point that calls `@main_xore` (Xore's `main`).
    fn emit_main_trampoline(&mut self) {
        self.emit("; ── libc entry point ───────────────────────────────────────");
        self.emit("define i32 @main(i32 %argc, ptr %argv) {");
        self.emit("entry:");
        self.emit("  call void @main()");
        self.emit("  ret i32 0");
        self.emit("}");
        self.emit("");
    }

    // ── Block + statements ────────────────────────────────────────────────

    fn emit_block(&mut self, block: &Block, env: &mut FnEnv, ret_ty: &str) {
        for stmt in &block.stmts {
            self.emit_stmt(stmt, env, ret_ty);
        }
    }

    fn emit_stmt(&mut self, stmt: &Stmt, env: &mut FnEnv, ret_ty: &str) {
        match stmt {
            Stmt::Let(l)        => self.emit_let(l, env),
            Stmt::Return(r)     => self.emit_return(r, env, ret_ty),
            Stmt::Expr(e)       => { self.emit_expr(e, env); }
            Stmt::If(i)         => self.emit_if(i, env, ret_ty),
            Stmt::While(w)      => self.emit_while(w, env, ret_ty),
            Stmt::For(f)        => self.emit_for(f, env, ret_ty),
            Stmt::End(e)        => self.emit_end(e, env),
            Stmt::FnDecl(f)     => self.emit_fn(f),   // nested fn → hoist
            Stmt::EnumDecl(e)   => self.emit_enum_constants(e),
            Stmt::StructDecl(s) => self.emit_struct_type(s),
        }
    }

    // ── let ───────────────────────────────────────────────────────────────

    fn emit_let(&mut self, l: &LetStmt, env: &mut FnEnv) {
        // Determine LLVM type
        let ty = l.ty.as_ref()
            .map(|t| llvm_type(t).to_string())
            .unwrap_or_else(|| "i64".to_string());  // default: i64

        // Allocate stack slot
        self.emit(&format!("  %{} = alloca {ty}, align 8", l.name));
        env.declare(&l.name, &ty);

        // Store initialiser if present
        if let Some(init) = &l.init {
            let val = self.emit_expr(init, env);
            self.emit(&format!("  store {ty} {val}, ptr %{}, align 8", l.name));
        }
    }

    // ── return ────────────────────────────────────────────────────────────

    fn emit_return(&mut self, r: &ReturnStmt, env: &mut FnEnv, ret_ty: &str) {
        match &r.value {
            None => self.emit("  ret void"),
            Some(e) => {
                let val = self.emit_expr(e, env);
                self.emit(&format!("  ret {ret_ty} {val}"));
            }
        }
    }

    // ── if ────────────────────────────────────────────────────────────────

    fn emit_if(&mut self, i: &IfStmt, env: &mut FnEnv, ret_ty: &str) {
        let then_lbl = self.label("then");
        let else_lbl = self.label("else");
        let end_lbl  = self.label("ifend");

        // Condition
        let cond_val = self.emit_expr(&i.cond, env);
        // Truncate to i1 if needed
        let cond_i1 = self.tmp();
        self.emit(&format!("  {cond_i1} = trunc i64 {cond_val} to i1"));
        self.emit(&format!("  br i1 {cond_i1}, label %{then_lbl}, label %{else_lbl}"));

        // Then branch
        self.emit(&format!("{then_lbl}:"));
        self.emit_block(&i.then_body, env, ret_ty);
        self.emit(&format!("  br label %{end_lbl}"));

        // Else branch
        self.emit(&format!("{else_lbl}:"));
        match &i.else_body {
            None => {}
            Some(b) => match b.as_ref() {
                ElseBranch::Block(bl) => self.emit_block(bl, env, ret_ty),
                ElseBranch::If(i2)    => self.emit_if(i2, env, ret_ty),
            }
        }
        self.emit(&format!("  br label %{end_lbl}"));

        self.emit(&format!("{end_lbl}:"));
    }

    // ── while ─────────────────────────────────────────────────────────────

    fn emit_while(&mut self, w: &WhileStmt, env: &mut FnEnv, ret_ty: &str) {
        let cond_lbl = self.label("while_cond");
        let body_lbl = self.label("while_body");
        let end_lbl  = self.label("while_end");

        env.push_loop(&end_lbl);

        self.emit(&format!("  br label %{cond_lbl}"));
        self.emit(&format!("{cond_lbl}:"));
        let cond_val = self.emit_expr(&w.cond, env);
        let cond_i1 = self.tmp();
        self.emit(&format!("  {cond_i1} = trunc i64 {cond_val} to i1"));
        self.emit(&format!("  br i1 {cond_i1}, label %{body_lbl}, label %{end_lbl}"));

        self.emit(&format!("{body_lbl}:"));
        self.emit_block(&w.body, env, ret_ty);
        self.emit(&format!("  br label %{cond_lbl}"));

        self.emit(&format!("{end_lbl}:"));
        env.pop_loop();
    }

    // ── for in range ──────────────────────────────────────────────────────

    fn emit_for(&mut self, f: &ForStmt, env: &mut FnEnv, ret_ty: &str) {
        let _init_lbl = self.label("for_init");
        let cond_lbl = self.label("for_cond");
        let body_lbl = self.label("for_body");
        let end_lbl  = self.label("for_end");

        env.push_loop(&end_lbl);

        // Allocate loop variable
        self.emit(&format!("  %{} = alloca i64, align 8", f.var));
        env.declare(&f.var, "i64");
        self.emit(&format!("  store i64 0, ptr %{}, align 8", f.var));

        self.emit(&format!("  br label %{cond_lbl}"));

        // Condition: i < limit
        self.emit(&format!("{cond_lbl}:"));
        let limit_val = self.emit_expr(&f.limit, env);
        let cur = self.tmp();
        self.emit(&format!("  {cur} = load i64, ptr %{}, align 8", f.var));
        let cond = self.tmp();
        self.emit(&format!("  {cond} = icmp slt i64 {cur}, {limit_val}"));
        self.emit(&format!("  br i1 {cond}, label %{body_lbl}, label %{end_lbl}"));

        // Body
        self.emit(&format!("{body_lbl}:"));
        self.emit_block(&f.body, env, ret_ty);
        // Increment
        let cur2 = self.tmp();
        self.emit(&format!("  {cur2} = load i64, ptr %{}, align 8", f.var));
        let next = self.tmp();
        self.emit(&format!("  {next} = add nsw i64 {cur2}, 1"));
        self.emit(&format!("  store i64 {next}, ptr %{}, align 8", f.var));
        self.emit(&format!("  br label %{cond_lbl}"));

        self.emit(&format!("{end_lbl}:"));
        env.pop_loop();
    }

    // ── end (break) ───────────────────────────────────────────────────────

    fn emit_end(&mut self, e: &EndStmt, env: &mut FnEnv) {
        let break_lbl = env.current_break().unwrap_or("_dead").to_string();
        match &e.cond {
            None => {
                self.emit(&format!("  br label %{break_lbl}"));
            }
            Some(cond_expr) => {
                let after = self.label("after_end");
                let cond_val = self.emit_expr(cond_expr, env);
                let cond_i1 = self.tmp();
                self.emit(&format!("  {cond_i1} = trunc i64 {cond_val} to i1"));
                self.emit(&format!("  br i1 {cond_i1}, label %{break_lbl}, label %{after}"));
                self.emit(&format!("{after}:"));
            }
        }
    }

    // ── Expressions ───────────────────────────────────────────────────────

    /// Emit code for `e` and return the LLVM value string (e.g. `%t3` or `42`).
    fn emit_expr(&mut self, e: &Expr, env: &mut FnEnv) -> String {
        match e {
            // ── Literals ─────────────────────────────────────────────────
            Expr::IntLit { value, .. } => format!("{value}"),
            Expr::FloatLit { value, .. } => format!("{value:.6e}"),
            Expr::BoolLit { value, .. } => if *value { "1".into() } else { "0".into() },
            Expr::NoneLit { .. } => "null".into(),

            Expr::StrLit { value, .. } => {
                // Intern the string and return a pointer to it.
                self.intern_string(value)
            }

            // ── Identifier — load from alloca ─────────────────────────────
            Expr::Ident { name, .. } => {
                if let Some(ty) = env.lookup(name) {
                    let tmp = self.tmp();
                    self.emit(&format!("  {tmp} = load {ty}, ptr %{name}, align 8"));
                    tmp
                } else {
                    // Unknown identifier — emit 0 and continue (resolver will catch it)
                    format!("0 ; unknown `{name}`")
                }
            }

            // ── Field access ──────────────────────────────────────────────
            Expr::Field { object, field, .. } => {
                let _obj_val = self.emit_expr(object, env);
                // Full GEP would require type info; emit a stub for now.
                let tmp = self.tmp();
                self.emit(&format!("  {tmp} = ; field .{field} (stub)"));
                self.emit(&format!("  {tmp} = add i64 0, 0  ; .{field}"));
                tmp
            }

            // ── Binary operations ─────────────────────────────────────────
            Expr::BinOp { op, lhs, rhs, .. } => {
                let l = self.emit_expr(lhs, env);
                let r = self.emit_expr(rhs, env);
                let tmp = self.tmp();
                let instr = binop_instr(*op);
                self.emit(&format!("  {tmp} = {instr} i64 {l}, {r}"));
                tmp
            }

            // ── Unary operations ──────────────────────────────────────────
            Expr::UnOp { op, operand, .. } => {
                let v = self.emit_expr(operand, env);
                let tmp = self.tmp();
                match op {
                    UnOp::Neg   => self.emit(&format!("  {tmp} = sub i64 0, {v}")),
                    UnOp::Not   => self.emit(&format!("  {tmp} = xor i64 {v}, -1")),
                    UnOp::Ref   => self.emit(&format!("  {tmp} = add i64 {v}, 0  ; ref")),
                    UnOp::Deref => self.emit(&format!("  {tmp} = load i64, ptr {v}, align 8")),
                }
                tmp
            }

            // ── Assignment ────────────────────────────────────────────────
            Expr::Assign { target, op, value, .. } => {
                let rhs_val = self.emit_expr(value, env);
                if let Expr::Ident { name, .. } = target.as_ref() {
                    let ty = env.lookup(name).unwrap_or("i64").to_string();
                    let stored = if *op == AssignOp::Assign {
                        rhs_val.clone()
                    } else {
                        // Load current, apply op, store result
                        let cur = self.tmp();
                        self.emit(&format!("  {cur} = load {ty}, ptr %{name}, align 8"));
                        let tmp = self.tmp();
                        let instr = assign_op_instr(*op);
                        self.emit(&format!("  {tmp} = {instr} {ty} {cur}, {rhs_val}"));
                        tmp
                    };
                    self.emit(&format!("  store {ty} {stored}, ptr %{name}, align 8"));
                    stored
                } else {
                    rhs_val
                }
            }

            // ── Macro call: println! ──────────────────────────────────────
            Expr::MacroCall { name, args, .. } => {
                match name.as_str() {
                    "println" | "print" => self.emit_println(args, env),
                    _ => {
                        // Unknown macro — evaluate args for side effects, return 0
                        for a in args { self.emit_expr(a, env); }
                        "0".into()
                    }
                }
            }

            // ── Function call ─────────────────────────────────────────────
            Expr::Call { callee, args, .. } => {
                let fn_name = match callee.as_ref() {
                    Expr::Ident { name, .. } => name.clone(),
                    Expr::Field { object, field, .. } => {
                        // Method call: object.method(args)
                        let _obj = self.emit_expr(object, env);
                        field.clone()
                    }
                    _ => {
                        let v = self.emit_expr(callee, env);
                        format!("({v})")
                    }
                };
                let arg_vals: Vec<String> = args.iter()
                    .map(|a| format!("i64 {}", self.emit_expr(a, env)))
                    .collect();
                let tmp = self.tmp();
                self.emit(&format!("  {tmp} = call i64 @{fn_name}({})", arg_vals.join(", ")));
                tmp
            }

            // ── Type constructor: i32(x), bool(True) ─────────────────────
            Expr::TypeCall { ty, args, .. } => {
                if let Some(a) = args.first() {
                    let val = self.emit_expr(a, env);
                    let dst_ty = llvm_type(ty);
                    let tmp = self.tmp();
                    // Cast / truncate / extend as needed
                    self.emit(&format!("  {tmp} = trunc i64 {val} to {dst_ty}"));
                    let ext = self.tmp();
                    self.emit(&format!("  {ext} = sext {dst_ty} {tmp} to i64"));
                    ext
                } else {
                    "0".into()
                }
            }

            // ── Grouping ─────────────────────────────────────────────────
            Expr::Paren   { inner, .. } => self.emit_expr(inner, env),
            Expr::Bracket { inner, .. } => self.emit_expr(inner, env),

            // ── Format chain stub ─────────────────────────────────────────
            Expr::FmtChain { parts, .. } => {
                for p in parts {
                    if let FmtPart::Hole(e) = p { self.emit_expr(e, env); }
                }
                "0".into()
            }
        }
    }

    // ── println! helper ───────────────────────────────────────────────────

    fn emit_println(&mut self, args: &[Expr], env: &mut FnEnv) -> String {
        match args.first() {
            None => {
                // println!() → puts("")
                let empty = self.intern_string("");
                self.emit(&format!("  call i32 @puts({empty})"));
            }
            Some(Expr::StrLit { value, .. }) => {
                // println!("literal") → puts(@.strN)
                let ptr = self.intern_string(value);
                self.emit(&format!("  call i32 @puts({ptr})"));
            }
            Some(Expr::NoneLit { .. }) => {
                let ptr = self.intern_string("None");
                self.emit(&format!("  call i32 @puts({ptr})"));
            }
            Some(other) => {
                // println!(expr) → printf("%lld\n", val)
                let fmt = self.intern_string("%lld\n");
                let val = self.emit_expr(other, env);
                self.emit(&format!("  call i32 (ptr, ...) @printf({fmt}, i64 {val})"));
            }
        }
        "0".into()
    }
}

// ─── Per-function environment (symbol table) ─────────────────────────────────

pub struct FnEnv {
    /// Stack of (name → llvm_type) scopes.
    scopes: Vec<Vec<(String, String)>>,
    /// Stack of break-target labels for end() / break.
    loop_stack: Vec<String>,
}

impl FnEnv {
    pub fn new(params: &[Param]) -> Self {
        let mut env = Self { scopes: vec![Vec::new()], loop_stack: Vec::new() };
        for p in params {
            let ty = llvm_type(&p.ty).to_string();
            env.declare(&p.name, &ty);
        }
        env
    }

    pub fn declare(&mut self, name: &str, ty: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), ty.to_string()));
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&str> {
        for scope in self.scopes.iter().rev() {
            for (n, ty) in scope.iter().rev() {
                if n == name { return Some(ty); }
            }
        }
        None
    }

    pub fn push_loop(&mut self, break_label: &str) {
        self.loop_stack.push(break_label.to_string());
    }

    pub fn pop_loop(&mut self) {
        self.loop_stack.pop();
    }

    pub fn current_break(&self) -> Option<&str> {
        self.loop_stack.last().map(|s| s.as_str())
    }
}

// ─── Instruction helpers ─────────────────────────────────────────────────────

fn binop_instr(op: BinOp) -> &'static str {
    match op {
        BinOp::Add    => "add nsw",
        BinOp::Sub    => "sub nsw",
        BinOp::Mul    => "mul nsw",
        BinOp::Div    => "sdiv",
        BinOp::Rem    => "srem",
        BinOp::BitAnd => "and",
        BinOp::BitOr  => "or",
        BinOp::BitXor => "xor",
        BinOp::Shl    => "shl",
        BinOp::Shr    => "ashr",
        BinOp::And    => "and",
        BinOp::Or     => "or",
        BinOp::Eq     => "icmp eq",
        BinOp::Ne     => "icmp ne",
        BinOp::Lt     => "icmp slt",
        BinOp::Gt     => "icmp sgt",
        BinOp::Le     => "icmp sle",
        BinOp::Ge     => "icmp sge",
        BinOp::Range | BinOp::RangeInclusive => "add nsw",
    }
}

fn assign_op_instr(op: AssignOp) -> &'static str {
    match op {
        AssignOp::AddAssign => "add nsw",
        AssignOp::SubAssign => "sub nsw",
        AssignOp::MulAssign => "mul nsw",
        AssignOp::DivAssign => "sdiv",
        AssignOp::RemAssign => "srem",
        AssignOp::AndAssign => "and",
        AssignOp::OrAssign  => "or",
        AssignOp::XorAssign => "xor",
        AssignOp::ShlAssign => "shl",
        AssignOp::ShrAssign => "ashr",
        AssignOp::Assign    => "add nsw",  // fallback, shouldn't reach
    }
}

/// Escape a string for LLVM IR constant syntax.
fn llvm_escape_string(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"'  => out.push_str("\\22"),
            b'\n' => out.push_str("\\0A"),
            b'\t' => out.push_str("\\09"),
            b'\r' => out.push_str("\\0D"),
            0x20..=0x7e => out.push(b as char),
            other => out.push_str(&format!("\\{other:02X}")),
        }
    }
    out
}
