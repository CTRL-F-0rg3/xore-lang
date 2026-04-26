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
        self.emit_program_with_target(program, "x86_64-unknown-linux-gnu")
    }

    /// Same as emit_program but with an explicit LLVM target triple.
    pub fn emit_program_with_target(&mut self, program: &Program, triple: &str) -> String {
        // LLVM module header
        self.emit(&format!("; Xore compiler v0.1.0 — module: {}", self.module));
        self.emit(&format!("source_filename = \"{}\"", self.module));
        self.emit("target datalayout = \"e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128\"");
        self.emit(&format!("target triple = \"{triple}\""));
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

        // Linkage rules:
        //   @export           → external (visible to linker / other .libx files)
        //   public (no export)→ external (visible within same binary)
        //   private           → internal (dead-code-eliminated if unused)
        let linkage = if f.exported {
            ""          // default external — visible to linker
        } else {
            match f.vis {
                Visibility::Public  => "",
                Visibility::Private => "internal ",
            }
        };

        // Xore's `main` is renamed to `xore_main` to avoid clashing with the
        // libc entry-point `@main` we emit as a trampoline below.
        let ir_name = if f.name == "main" { "xore_main".to_string() } else { f.name.clone() };

        self.emit(&format!(
            "define {linkage}{ret_ty} @{}({}) {{",
            ir_name, params.join(", ")
        ));
        self.emit("entry:");

        // Allocate stack slots for parameters so they can be re-assigned.
        // The incoming SSA value %name is stored into %name.slot, and all
        // subsequent loads use the slot pointer.
        for p in &f.params {
            let ty = llvm_type(&p.ty);
            self.emit(&format!("  %{}.slot = alloca {ty}, align 8", p.name));
            self.emit(&format!("  store {ty} %{}, ptr %{}.slot, align 8", p.name, p.name));
        }

        // Build a local env that maps param names → (slot_ptr_name, type)
        let mut env = FnEnv::new_with_slots(&f.params);
        self.emit_block(&f.body, &mut env, &ret_ty);

        // Ensure function ends with a terminator.
        // Check the last non-empty line of output for this function.
        let last = self.out.lines().rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let has_term = last.trim_start().starts_with("ret ")
            || last.trim_start().starts_with("br ")
            || last.trim_start().starts_with("unreachable");
        if !has_term {
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
        self.emit("  call void @xore_main()");
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
            Stmt::Match(m)      => self.emit_match(m, env, ret_ty),
            Stmt::Switch(s)     => self.emit_switch(s, env, ret_ty),
            Stmt::FnDecl(f)     => self.emit_fn(f),
            Stmt::EnumDecl(e)   => self.emit_enum_constants(e),
            Stmt::StructDecl(s) => self.emit_struct_type(s),
        }
    }

    // ── let ───────────────────────────────────────────────────────────────

    fn emit_let(&mut self, l: &LetStmt, env: &mut FnEnv) {
        // Determine LLVM type — prefer declared type, then infer from init
        let declared = l.ty.as_ref().map(|t| llvm_type(t).to_string());
        let init_ty  = l.init.as_ref().map(|e| infer_expr_type(e, env).to_string());
        let ty = declared.or(init_ty).unwrap_or_else(|| "i64".to_string());

        self.emit(&format!("  %{} = alloca {ty}, align 8", l.name));
        env.declare(&l.name, &l.name, &ty);

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
                let val     = self.emit_expr(e, env);
                let val_ty  = infer_expr_type(e, env);
                // Cast to declared return type if they differ
                let final_val = if val_ty.as_str() != ret_ty && ret_ty != "void" {
                    let cast = self.tmp();
                    let instr = pick_cast(&val_ty, ret_ty);
                    self.emit(&format!("  {cast} = {instr} {val_ty} {val} to {ret_ty}"));
                    cast
                } else {
                    val
                };
                self.emit(&format!("  ret {ret_ty} {final_val}"));
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
        let cond_ty  = infer_expr_type(&i.cond, env);
        let cond_i1  = to_bool_cond(self, cond_val, &cond_ty);
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
        let cond_ty  = infer_expr_type(&w.cond, env);
        let cond_i1  = to_bool_cond(self, cond_val, &cond_ty);
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
        env.declare(&f.var, &f.var, "i64");
        self.emit(&format!("  store i64 0, ptr %{}, align 8", f.var));

        self.emit(&format!("  br label %{cond_lbl}"));

        // Condition: i < limit
        self.emit(&format!("{cond_lbl}:"));
        let limit_val = self.emit_expr(&f.limit, env);
        let cur = self.tmp();
        let var_slot = f.var.clone();
        self.emit(&format!("  {cur} = load i64, ptr %{var_slot}, align 8"));
        let cond = self.tmp();
        self.emit(&format!("  {cond} = icmp slt i64 {cur}, {limit_val}"));
        self.emit(&format!("  br i1 {cond}, label %{body_lbl}, label %{end_lbl}"));

        // Body
        self.emit(&format!("{body_lbl}:"));
        self.emit_block(&f.body, env, ret_ty);
        // Increment
        let cur2 = self.tmp();
        self.emit(&format!("  {cur2} = load i64, ptr %{var_slot}, align 8"));
        let next = self.tmp();
        self.emit(&format!("  {next} = add nsw i64 {cur2}, 1"));
        self.emit(&format!("  store i64 {next}, ptr %{var_slot}, align 8"));
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
                let after    = self.label("after_end");
                let cond_val = self.emit_expr(cond_expr, env);
                let cond_ty  = infer_expr_type(cond_expr, env);
                let cond_i1  = to_bool_cond(self, cond_val, &cond_ty);
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
                if let Some((slot, ty)) = env.lookup(name) {
                    let tmp = self.tmp();
                    self.emit(&format!("  {tmp} = load {ty}, ptr %{slot}, align 8"));
                    tmp
                } else {
                    format!("0 ; unknown `{name}`")
                }
            }

            // ── Field access ──────────────────────────────────────────────
            // a.b  — could be struct field or method call like String.new()
            Expr::Field { object, field, .. } => {
                // Special case: String.new() called as field access on type name
                if let Expr::Ident { name, .. } = object.as_ref() {
                    if name == "String" && field == "new" {
                        // Return a small heap-allocated buffer as a String placeholder
                        // Full String type will be in stdlib; for now allocate 64 bytes
                        let tmp = self.tmp();
                        self.emit(&format!("  {tmp} = call ptr @malloc(i64 64)"));
                        // Zero-terminate immediately so it's a valid C string
                        self.emit(&format!("  store i8 0, ptr {tmp}, align 1"));
                        return tmp;
                    }
                }
                // General struct field access: obj.field
                // We need the struct pointer in obj_val, then GEP to the field.
                let obj_val = self.emit_expr(object, env);
                // For now emit a GEP with field index 0 as stub — full struct
                // layout resolution happens in the type checker / lowering pass.
                let tmp = self.tmp();
                self.emit(&format!("  {tmp} = getelementptr i8, ptr {obj_val}, i64 0 ; .{field}"));
                tmp
            }

            // ── Binary operations ─────────────────────────────────────────
            Expr::BinOp { op, lhs, rhs, .. } => {
                let l = self.emit_expr(lhs, env);
                let r = self.emit_expr(rhs, env);
                let tmp = self.tmp();
                // Determine operand type from environment context
                let ty = infer_expr_type(lhs, env);
                let instr = binop_instr(*op, &ty);
                // Comparison ops return i1 — keep as i1 (no zext)
                match op {
                    BinOp::Eq | BinOp::Ne | BinOp::Lt |
                    BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        self.emit(&format!("  {tmp} = {instr} {ty} {l}, {r}"));
                        tmp
                    }
                    _ => {
                        self.emit(&format!("  {tmp} = {instr} {ty} {l}, {r}"));
                        tmp
                    }
                }
            }

            // ── Unary operations ──────────────────────────────────────────
            Expr::UnOp { op, operand, .. } => {
                let v = self.emit_expr(operand, env);
                let ty = infer_expr_type(operand, env);
                let tmp = self.tmp();
                match op {
                    UnOp::Neg   => self.emit(&format!("  {tmp} = sub {ty} 0, {v}")),
                    UnOp::Not   => self.emit(&format!("  {tmp} = xor {ty} {v}, -1")),
                    UnOp::Ref   => self.emit(&format!("  {tmp} = add {ty} {v}, 0  ; ref")),
                    UnOp::Deref => self.emit(&format!("  {tmp} = load {ty}, ptr {v}, align 8")),
                }
                tmp
            }

            // ── Assignment ────────────────────────────────────────────────
            Expr::Assign { target, op, value, .. } => {
                let rhs_val = self.emit_expr(value, env);
                if let Expr::Ident { name, .. } = target.as_ref() {
                    let (slot, ty) = env.lookup(name)
                        .map(|(s,t)| (s.to_string(), t.to_string()))
                        .unwrap_or_else(|| (name.to_string(), "i64".to_string()));
                    let stored = if *op == AssignOp::Assign {
                        rhs_val.clone()
                    } else {
                        let cur = self.tmp();
                        self.emit(&format!("  {cur} = load {ty}, ptr %{slot}, align 8"));
                        let tmp = self.tmp();
                        let instr = assign_op_instr(*op);
                        self.emit(&format!("  {tmp} = {instr} {ty} {cur}, {rhs_val}"));
                        tmp
                    };
                    self.emit(&format!("  store {ty} {stored}, ptr %{slot}, align 8"));
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
                    Expr::Ident { name, .. } => {
                        if name == "main" { "xore_main".to_string() } else { name.clone() }
                    }
                    Expr::Field { object, field, .. } => {
                        // Type.method() style — e.g. String.new()
                        if let Expr::Ident { name: type_name, .. } = object.as_ref() {
                            if type_name == "String" && field == "new" {
                                // String.new() → malloc(64)
                                let tmp = self.tmp();
                                self.emit(&format!("  {tmp} = call ptr @malloc(i64 64)"));
                                self.emit(&format!("  store i8 0, ptr {tmp}, align 1"));
                                return tmp;
                            }
                            format!("{type_name}_{field}")
                        } else {
                            field.clone()
                        }
                    }
                    _ => {
                        let v = self.emit_expr(callee, env);
                        format!("({v})")
                    }
                };
                let arg_vals: Vec<String> = args.iter()
                    .map(|a| {
                        let ty = infer_expr_type(a, env);
                        let v  = self.emit_expr(a, env);
                        format!("{ty} {v}")
                    })
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
                let empty = self.intern_string("");
                self.emit(&format!("  call i32 @puts({empty})"));
            }
            Some(Expr::StrLit { value, .. }) => {
                let ptr = self.intern_string(value);
                self.emit(&format!("  call i32 @puts({ptr})"));
            }
            Some(Expr::NoneLit { .. }) => {
                let ptr = self.intern_string("None");
                self.emit(&format!("  call i32 @puts({ptr})"));
            }
            Some(other) => {
                let ty  = infer_expr_type(other, env);
                let val = self.emit_expr(other, env);
                if ty == "ptr" {
                    self.emit(&format!("  call i32 @puts(ptr {val})"));
                } else if ty == "float" || ty == "double" {
                    let fmt = self.intern_string("%g\n");
                    let dval = if ty == "float" {
                        let e = self.tmp();
                        self.emit(&format!("  {e} = fpext float {val} to double"));
                        e
                    } else { val };
                    self.emit(&format!("  call i32 (ptr, ...) @printf({fmt}, double {dval})"));
                } else {
                    let fmt = self.intern_string("%lld\n");
                    // Extend to i64 if narrower
                    let i64val = if ty != "i64" {
                        let e = self.tmp();
                        let cast = pick_cast(&ty, "i64");
                        if cast == "add" {
                            // same type effectively — just use val
                            val.clone()
                        } else {
                            self.emit(&format!("  {e} = {cast} {ty} {val} to i64"));
                            e
                        }
                    } else { val };
                    self.emit(&format!("  call i32 (ptr, ...) @printf({fmt}, i64 {i64val})"));
                }
            }
        }
        "0".into()
    }

    // ── match ─────────────────────────────────────────────────────────────
    // Compiled as a chain of comparisons — full pattern matching needs
    // a dedicated lowering pass, this is a solid first implementation.

    fn emit_match(&mut self, m: &MatchStmt, env: &mut FnEnv, ret_ty: &str) {
        let end_lbl = self.label("match_end");
        let subject_val = self.emit_expr(&m.subject, env);
        let subject_ty  = infer_expr_type(&m.subject, env);

        for (i, arm) in m.arms.iter().enumerate() {
            let body_lbl = self.label("match_arm");
            let next_lbl = if i == m.arms.len() - 1 {
                end_lbl.clone()
            } else {
                self.label("match_next")
            };

            match &arm.pattern {
                Pattern::Wildcard | Pattern::Bind(_) => {
                    // Always matches — unconditional jump to body
                    self.emit(&format!("  br label %{body_lbl}"));
                    self.emit(&format!("{body_lbl}:"));
                    self.emit_block(&arm.body, env, ret_ty);
                    self.emit(&format!("  br label %{end_lbl}"));
                }
                Pattern::Lit(expr) => {
                    let cmp_val = self.emit_expr(expr, env);
                    let cond    = self.tmp();
                    self.emit(&format!("  {cond} = icmp eq {subject_ty} {subject_val}, {cmp_val}"));
                    self.emit(&format!("  br i1 {cond}, label %{body_lbl}, label %{next_lbl}"));
                    self.emit(&format!("{body_lbl}:"));
                    self.emit_block(&arm.body, env, ret_ty);
                    self.emit(&format!("  br label %{end_lbl}"));
                    if i < m.arms.len() - 1 {
                        self.emit(&format!("{next_lbl}:"));
                    }
                }
                Pattern::Variant(enum_name, variant) => {
                    // Compare subject (i64 tag) against variant tag constant
                    let tag_name = variant.as_deref().unwrap_or(enum_name.as_str());
                    let tag_ptr  = format!("ptr @{enum_name}.{tag_name}");
                    let tag_val  = self.tmp();
                    self.emit(&format!("  {tag_val} = load i64, {tag_ptr}, align 8"));
                    let cond = self.tmp();
                    self.emit(&format!("  {cond} = icmp eq i64 {subject_val}, {tag_val}"));
                    self.emit(&format!("  br i1 {cond}, label %{body_lbl}, label %{next_lbl}"));
                    self.emit(&format!("{body_lbl}:"));
                    self.emit_block(&arm.body, env, ret_ty);
                    self.emit(&format!("  br label %{end_lbl}"));
                    if i < m.arms.len() - 1 {
                        self.emit(&format!("{next_lbl}:"));
                    }
                }
                Pattern::Range(lo, hi) => {
                    let lo_val = self.emit_expr(lo, env);
                    let hi_val = self.emit_expr(hi, env);
                    let c1 = self.tmp();
                    let c2 = self.tmp();
                    let ok = self.tmp();
                    self.emit(&format!("  {c1} = icmp sge {subject_ty} {subject_val}, {lo_val}"));
                    self.emit(&format!("  {c2} = icmp slt {subject_ty} {subject_val}, {hi_val}"));
                    self.emit(&format!("  {ok} = and i1 {c1}, {c2}"));
                    self.emit(&format!("  br i1 {ok}, label %{body_lbl}, label %{next_lbl}"));
                    self.emit(&format!("{body_lbl}:"));
                    self.emit_block(&arm.body, env, ret_ty);
                    self.emit(&format!("  br label %{end_lbl}"));
                    if i < m.arms.len() - 1 {
                        self.emit(&format!("{next_lbl}:"));
                    }
                }
            }
        }
        self.emit(&format!("{end_lbl}:"));
    }

    // ── switch ────────────────────────────────────────────────────────────
    // Compiles to LLVM `switch` instruction — O(1) dispatch for integers.

    fn emit_switch(&mut self, s: &SwitchStmt, env: &mut FnEnv, ret_ty: &str) {
        let default_lbl = self.label("switch_default");
        let end_lbl     = self.label("switch_end");
        let subject_val = self.emit_expr(&s.subject, env);
        let subject_ty  = infer_expr_type(&s.subject, env);

        // Collect case labels first
        let case_lbls: Vec<String> = s.cases.iter()
            .map(|_| self.label("switch_case"))
            .collect();

        // Emit the switch instruction
        let mut sw = format!("  switch {subject_ty} {subject_val}, label %{default_lbl} [\n");
        for (case, lbl) in s.cases.iter().zip(case_lbls.iter()) {
            let val = match &case.value {
                Expr::IntLit { value, .. }  => value.to_string(),
                Expr::BoolLit { value, .. } => if *value { "1".to_string() } else { "0".to_string() },
                _ => "0".to_string(),
            };
            sw.push_str(&format!("    {subject_ty} {val}, label %{lbl}\n"));
        }
        sw.push_str("  ]");
        self.emit(&sw);

        // Emit each case body
        for (case, lbl) in s.cases.iter().zip(case_lbls.iter()) {
            self.emit(&format!("{lbl}:"));
            self.emit_block(&case.body, env, ret_ty);
            self.emit(&format!("  br label %{end_lbl}"));
        }

        // Default body
        self.emit(&format!("{default_lbl}:"));
        if let Some(def_body) = &s.default {
            self.emit_block(def_body, env, ret_ty);
        }
        self.emit(&format!("  br label %{end_lbl}"));

        self.emit(&format!("{end_lbl}:"));
    }
}

// ─── Per-function environment (symbol table) ─────────────────────────────────

pub struct FnEnv {
    /// Stack of (xore_name → (slot_ptr_name, llvm_type)) scopes.
    /// For params:  slot = "%name.slot",  ty = "i32" etc.
    /// For locals:  slot = "%name",       ty = "i64" (alloca pointer)
    scopes: Vec<Vec<(String, String, String)>>,  // (name, slot, ty)
    /// Stack of break-target labels for end() / break.
    loop_stack: Vec<String>,
}

impl FnEnv {
    /// Create env for a function — params get .slot allocas.
    pub fn new_with_slots(params: &[Param]) -> Self {
        let mut env = Self { scopes: vec![Vec::new()], loop_stack: Vec::new() };
        for p in params {
            let ty = llvm_type(&p.ty).to_string();
            // The alloca slot is named %name.slot (created in emit_fn)
            env.scopes[0].push((p.name.clone(), format!("{}.slot", p.name), ty));
        }
        env
    }

    /// Declare a new local variable.  `slot` is the alloca name (without %).
    pub fn declare(&mut self, name: &str, slot: &str, ty: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), slot.to_string(), ty.to_string()));
        }
    }

    /// Look up (slot_name, llvm_type) for a variable.
    pub fn lookup(&self, name: &str) -> Option<(&str, &str)> {
        for scope in self.scopes.iter().rev() {
            for (n, slot, ty) in scope.iter().rev() {
                if n == name { return Some((slot.as_str(), ty.as_str())); }
            }
        }
        None
    }

    pub fn push_loop(&mut self, break_label: &str) {
        self.loop_stack.push(break_label.to_string());
    }

    pub fn pop_loop(&mut self) { self.loop_stack.pop(); }

    pub fn current_break(&self) -> Option<&str> {
        self.loop_stack.last().map(|s| s.as_str())
    }
}

// ─── Instruction helpers ─────────────────────────────────────────────────────

fn binop_instr(op: BinOp, ty: &str) -> String {
    let is_float = ty == "float" || ty == "double";
    match op {
        BinOp::Add    => if is_float { "fadd".into() } else { "add nsw".into() },
        BinOp::Sub    => if is_float { "fsub".into() } else { "sub nsw".into() },
        BinOp::Mul    => if is_float { "fmul".into() } else { "mul nsw".into() },
        BinOp::Div    => if is_float { "fdiv".into() } else { "sdiv".into() },
        BinOp::Rem    => if is_float { "frem".into() } else { "srem".into() },
        BinOp::BitAnd => "and".into(),
        BinOp::BitOr  => "or".into(),
        BinOp::BitXor => "xor".into(),
        BinOp::Shl    => "shl".into(),
        BinOp::Shr    => "ashr".into(),
        BinOp::And    => "and".into(),
        BinOp::Or     => "or".into(),
        BinOp::Eq     => if is_float { "fcmp oeq".into() } else { "icmp eq".into() },
        BinOp::Ne     => if is_float { "fcmp one".into() } else { "icmp ne".into() },
        BinOp::Lt     => if is_float { "fcmp olt".into() } else { "icmp slt".into() },
        BinOp::Gt     => if is_float { "fcmp ogt".into() } else { "icmp sgt".into() },
        BinOp::Le     => if is_float { "fcmp ole".into() } else { "icmp sle".into() },
        BinOp::Ge     => if is_float { "fcmp oge".into() } else { "icmp sge".into() },
        BinOp::Range | BinOp::RangeInclusive => "add nsw".into(),
    }
}

/// Best-effort type inference — returns an owned String so it doesn't
/// conflict with the mutable borrow of env needed by emit_expr.
fn infer_expr_type(e: &Expr, env: &FnEnv) -> String {
    match e {
        Expr::IntLit { .. }             => "i64".into(),
        Expr::FloatLit { .. }           => "double".into(),
        Expr::BoolLit { .. }            => "i1".into(),
        Expr::StrLit { .. }             => "ptr".into(),
        Expr::NoneLit { .. }            => "ptr".into(),
        Expr::Ident { name, .. }        => {
            env.lookup(name)
               .map(|(_, ty)| match ty {
                   "i8"    => "i8",   "i16"   => "i16",
                   "i32"   => "i32",  "i64"   => "i64",
                   "u8"    => "i8",   "u16"   => "i16",
                   "u32"   => "i32",  "u64"   => "i64",
                   "float" => "float","double" => "double",
                   "i1"    => "i1",   "ptr"   => "ptr",
                   _       => "i64",
               })
               .unwrap_or("i64")
               .to_string()
        }
        Expr::Field { object, .. } => {
            if let Expr::Ident { name, .. } = object.as_ref() {
                if name == "String" { return "ptr".into(); }
            }
            "ptr".into()
        }
        Expr::TypeCall { ty, .. }       => llvm_type(ty).to_string(),
        Expr::Call { callee, .. }               => {
            // Detect String.new() → ptr
            match callee.as_ref() {
                Expr::Field { object, field, .. } => {
                    if let Expr::Ident { name, .. } = object.as_ref() {
                        if name == "String" && field == "new" { return "ptr".into(); }
                    }
                    "i64".into()
                }
                _ => "i64".into(),
            }
        }
        Expr::MacroCall { .. }          => "i64".into(),
        Expr::BinOp { op, lhs, .. }    => match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt |
            BinOp::Gt | BinOp::Le | BinOp::Ge |
            BinOp::And | BinOp::Or     => "i1".into(),
            _                           => infer_expr_type(lhs, env),
        },
        Expr::UnOp { op, operand, .. } => match op {
            UnOp::Not => "i1".into(),
            UnOp::Ref => "ptr".into(),
            _         => infer_expr_type(operand, env),
        },
        Expr::Assign { target, .. }    => infer_expr_type(target, env),
        Expr::Paren   { inner, .. }    => infer_expr_type(inner, env),
        Expr::Bracket { inner, .. }    => infer_expr_type(inner, env),
        Expr::FmtChain { .. }          => "ptr".into(),
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

/// Convert an expression value to i1 for use as a branch condition.
/// Uses `icmp ne T val, 0` for integer types instead of `trunc` — this
/// correctly handles all even numbers (trunc i64 2 to i1 = 0, which is wrong).
fn to_bool_cond(out: &mut Codegen, val: String, ty: &str) -> String {
    if ty == "i1" {
        return val;
    }
    if ty == "float" || ty == "double" {
        let t = out.tmp();
        out.emit(&format!("  {t} = fcmp one {ty} {val}, 0.0"));
        return t;
    }
    // For all integer types: icmp ne ty val, 0
    let t = out.tmp();
    out.emit(&format!("  {t} = icmp ne {ty} {val}, 0"));
    t
}
fn pick_cast(from_ty: &str, to_ty: &str) -> &'static str {
    match (from_ty, to_ty) {
        // Widen int
        ("i1",  "i8")  | ("i1",  "i16") | ("i1",  "i32") | ("i1",  "i64") => "zext",
        ("i8",  "i16") | ("i8",  "i32") | ("i8",  "i64") => "sext",
        ("i16", "i32") | ("i16", "i64") => "sext",
        ("i32", "i64") => "sext",
        // Narrow int
        ("i64", "i32") | ("i64", "i16") | ("i64", "i8")  | ("i64", "i1")  => "trunc",
        ("i32", "i16") | ("i32", "i8")  | ("i32", "i1")  => "trunc",
        ("i16", "i8")  | ("i16", "i1")  => "trunc",
        ("i8",  "i1")  => "trunc",
        // Int ↔ float
        ("i32", "float") | ("i64", "float")  => "sitofp",
        ("i32", "double")| ("i64", "double") => "sitofp",
        ("float",  "i32") | ("float",  "i64") => "fptosi",
        ("double", "i32") | ("double", "i64") => "fptosi",
        ("float", "double") => "fpext",
        ("double", "float") => "fptrunc",
        // ptr ↔ int
        ("ptr", _)  => "ptrtoint",
        (_, "ptr")  => "inttoptr",
        // same type — no-op (use add with 0 as identity)
        _ => "add",
    }
}
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