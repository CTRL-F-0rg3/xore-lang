// Xore type checker — semantic analysis pass.
//
// Runs after parsing, before codegen.  Resolves types, checks assignments,
// validates function signatures, and produces a typed AST (TypedProgram).
//
// Design: two-pass
//   1. Collect pass  — gather all top-level function / struct / enum signatures
//   2. Check pass    — walk every function body, infer and verify types

use std::collections::HashMap;
use std::fmt;

use crate::ast::*;
use crate::token::Span;

// ─── Xore types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum XoreType {
    // Primitives
    I8, I16, I32, I64,
    U8, U16, U32, U64,
    F32, F64,
    Bool,
    Void,
    Char,
    // Compound
    Str,                              // &str  (string slice)
    String,                           // String (heap)
    Ptr(Box<XoreType>),               // *T
    Ref(Box<XoreType>),               // &T
    MutRef(Box<XoreType>),            // &mut T
    Slice(Box<XoreType>),             // [T]
    Array(Box<XoreType>, u64),        // [T; N]
    Struct(String),                   // named struct
    Enum(String),                     // named enum
    FnPtr(Vec<XoreType>, Box<XoreType>),
    // Special
    Unknown,                          // not yet resolved
    Never,                            // ! — diverging (return / end)
}

impl XoreType {
    pub fn from_type_expr(te: &TypeExpr) -> Self {
        match te {
            TypeExpr::Named(name, _) => Self::from_name(name),
            TypeExpr::Ref(inner, _)     => Self::Ref(Box::new(Self::from_type_expr(inner))),
            TypeExpr::MutRef(inner, _)  => Self::MutRef(Box::new(Self::from_type_expr(inner))),
            TypeExpr::Pointer(inner, _) => Self::Ptr(Box::new(Self::from_type_expr(inner))),
            TypeExpr::Slice(inner, _)   => Self::Slice(Box::new(Self::from_type_expr(inner))),
            TypeExpr::Array(inner, _, _)=> Self::Slice(Box::new(Self::from_type_expr(inner))),
            TypeExpr::ErrorUnion(inner, _) => Self::from_type_expr(inner),
            TypeExpr::Void(_)           => Self::Void,
            TypeExpr::Infer(_)          => Self::Unknown,
            TypeExpr::FnPtr { params, ret, .. } => Self::FnPtr(
                params.iter().map(Self::from_type_expr).collect(),
                Box::new(Self::from_type_expr(ret)),
            ),
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "i8"      => Self::I8,   "i16"  => Self::I16,
            "i32"     => Self::I32,  "i64"  => Self::I64,
            "u8"      => Self::U8,   "u16"  => Self::U16,
            "u32"     => Self::U32,  "u64"  => Self::U64,
            "f32"     => Self::F32,  "f64"  => Self::F64,
            "bool"    => Self::Bool,
            "void"    => Self::Void,
            "char"    => Self::Char,
            "str"     => Self::Str,
            "String"  => Self::String,
            name      => Self::Struct(name.to_string()),
        }
    }

    /// LLVM IR type string for this Xore type.
    pub fn llvm(&self) -> &'static str {
        match self {
            Self::I8  | Self::U8  => "i8",
            Self::I16 | Self::U16 => "i16",
            Self::I32 | Self::U32 => "i32",
            Self::I64 | Self::U64 => "i64",
            Self::F32             => "float",
            Self::F64             => "double",
            Self::Bool            => "i1",
            Self::Void            => "void",
            Self::Char            => "i32",
            Self::Str | Self::String | Self::Ptr(_) |
            Self::Ref(_) | Self::MutRef(_) |
            Self::Slice(_) | Self::Array(_, _) => "ptr",
            Self::Struct(_) | Self::Enum(_)    => "ptr",
            Self::FnPtr(_, _)                  => "ptr",
            Self::Unknown | Self::Never        => "i64",
        }
    }

    /// True if this type is an integer (signed or unsigned).
    pub fn is_integer(&self) -> bool {
        matches!(self,
            Self::I8 | Self::I16 | Self::I32 | Self::I64 |
            Self::U8 | Self::U16 | Self::U32 | Self::U64 |
            Self::Bool | Self::Char
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    /// True if two types are compatible for assignment / comparison.
    pub fn compatible(&self, other: &XoreType) -> bool {
        if self == other { return true; }
        // Integer widening / coercion
        if self.is_integer() && other.is_integer() { return true; }
        if self.is_float()   && other.is_float()   { return true; }
        // Unknown is compatible with anything (not yet resolved)
        matches!(self, Self::Unknown) || matches!(other, Self::Unknown)
    }
}

impl fmt::Display for XoreType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I8  => write!(f, "i8"),   Self::I16 => write!(f, "i16"),
            Self::I32 => write!(f, "i32"),  Self::I64 => write!(f, "i64"),
            Self::U8  => write!(f, "u8"),   Self::U16 => write!(f, "u16"),
            Self::U32 => write!(f, "u32"),  Self::U64 => write!(f, "u64"),
            Self::F32 => write!(f, "f32"),  Self::F64 => write!(f, "f64"),
            Self::Bool   => write!(f, "bool"),
            Self::Void   => write!(f, "void"),
            Self::Char   => write!(f, "char"),
            Self::Str    => write!(f, "&str"),
            Self::String => write!(f, "String"),
            Self::Ptr(t)    => write!(f, "*{t}"),
            Self::Ref(t)    => write!(f, "&{t}"),
            Self::MutRef(t) => write!(f, "&mut {t}"),
            Self::Slice(t)  => write!(f, "[{t}]"),
            Self::Array(t, n) => write!(f, "[{t}; {n}]"),
            Self::Struct(n) | Self::Enum(n) => write!(f, "{n}"),
            Self::FnPtr(params, ret) => {
                let ps: Vec<String> = params.iter().map(|t| t.to_string()).collect();
                write!(f, "fn({}) -> {ret}", ps.join(", "))
            }
            Self::Unknown => write!(f, "_"),
            Self::Never   => write!(f, "!"),
        }
    }
}

// ─── Type errors ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TypeError {
    pub msg:  String,
    pub span: Span,
}

impl TypeError {
    fn new(msg: impl Into<String>, span: Span) -> Self {
        Self { msg: msg.into(), span }
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}:{}] type error: {}", self.span.line, self.span.col, self.msg)
    }
}

// ─── Function signature ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FnSig {
    pub params:  Vec<(String, XoreType)>,
    pub ret:     XoreType,
    pub exported: bool,
}

// ─── Struct layout ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StructLayout {
    pub fields: Vec<(String, XoreType)>,   // (field_name, type) in order
}

impl StructLayout {
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|(n, _)| n == name)
    }
    pub fn field_type(&self, name: &str) -> Option<&XoreType> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }
}

// ─── Enum layout ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EnumLayout {
    pub variants: Vec<(String, Vec<XoreType>)>,
}

impl EnumLayout {
    pub fn variant_tag(&self, name: &str) -> Option<u64> {
        self.variants.iter().position(|(n, _)| n == name).map(|i| i as u64)
    }
}

// ─── Type checker ────────────────────────────────────────────────────────────

pub struct TypeChecker {
    pub errors:  Vec<TypeError>,
    pub fns:     HashMap<String, FnSig>,
    pub structs: HashMap<String, StructLayout>,
    pub enums:   HashMap<String, EnumLayout>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            errors:  Vec::new(),
            fns:     HashMap::new(),
            structs: HashMap::new(),
            enums:   HashMap::new(),
        }
    }

    // ── Entry point ───────────────────────────────────────────────────────

    pub fn check_program(&mut self, program: &Program) {
        // Pass 1 — collect all top-level declarations
        self.collect_program(program);
        // Pass 2 — check function bodies
        for item in &program.items {
            if let Item::Function(f) = item {
                self.check_fn(f);
            }
        }
    }

    // ── Pass 1: collect ───────────────────────────────────────────────────

    fn collect_program(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Function(f)  => self.collect_fn(f),
                Item::Struct(s)    => self.collect_struct(s),
                Item::Enum(e)      => self.collect_enum(e),
                Item::Mod(m)       => {
                    // Recurse into modules
                    for inner in &m.items {
                        if let Item::Function(f) = inner { self.collect_fn(f); }
                        if let Item::Struct(s)   = inner { self.collect_struct(s); }
                        if let Item::Enum(e)     = inner { self.collect_enum(e); }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_fn(&mut self, f: &FnDecl) {
        let params = f.params.iter()
            .map(|p| (p.name.clone(), XoreType::from_type_expr(&p.ty)))
            .collect();
        let ret = f.ret_ty.as_ref()
            .map(XoreType::from_type_expr)
            .unwrap_or(XoreType::Void);
        self.fns.insert(f.name.clone(), FnSig { params, ret, exported: f.exported });
    }

    fn collect_struct(&mut self, s: &StructDecl) {
        let fields = s.fields.iter()
            .map(|f| (f.name.clone(), XoreType::from_type_expr(&f.ty)))
            .collect();
        self.structs.insert(s.name.clone(), StructLayout { fields });
    }

    fn collect_enum(&mut self, e: &EnumDecl) {
        let variants = e.variants.iter()
            .map(|v| {
                let fields = v.fields.iter().map(XoreType::from_type_expr).collect();
                (v.name.clone(), fields)
            })
            .collect();
        self.enums.insert(e.name.clone(), EnumLayout { variants });
    }

    // ── Pass 2: check function bodies ─────────────────────────────────────

    fn check_fn(&mut self, f: &FnDecl) {
        let ret_ty = f.ret_ty.as_ref()
            .map(XoreType::from_type_expr)
            .unwrap_or(XoreType::Void);

        let mut scope = LocalScope::new();
        for p in &f.params {
            scope.declare(&p.name, XoreType::from_type_expr(&p.ty));
        }

        self.check_block(&f.body, &mut scope, &ret_ty);
    }

    fn check_block(&mut self, block: &Block, scope: &mut LocalScope, ret_ty: &XoreType) {
        scope.push();
        for stmt in &block.stmts {
            self.check_stmt(stmt, scope, ret_ty);
        }
        scope.pop();
    }

    fn check_stmt(&mut self, stmt: &Stmt, scope: &mut LocalScope, ret_ty: &XoreType) {
        match stmt {
            Stmt::Let(l)   => self.check_let(l, scope),
            Stmt::Return(r)=> self.check_return(r, scope, ret_ty),
            Stmt::Expr(e)  => { self.infer_expr(e, scope); }
            Stmt::If(i)    => self.check_if(i, scope, ret_ty),
            Stmt::While(w) => self.check_while(w, scope, ret_ty),
            Stmt::For(f)   => self.check_for(f, scope, ret_ty),
            Stmt::End(_)   => {}
            Stmt::Match(m) => {
                self.infer_expr(&m.subject, scope);
                for arm in &m.arms {
                    if let Pattern::Bind(name) = &arm.pattern {
                        scope.push();
                        scope.declare(name, XoreType::Unknown);
                        self.check_block(&arm.body, scope, ret_ty);
                        scope.pop();
                    } else {
                        self.check_block(&arm.body, scope, ret_ty);
                    }
                }
            }
            Stmt::Switch(s) => {
                self.infer_expr(&s.subject, scope);
                for case in &s.cases {
                    self.infer_expr(&case.value, scope);
                    self.check_block(&case.body, scope, ret_ty);
                }
                if let Some(def) = &s.default {
                    self.check_block(def, scope, ret_ty);
                }
            }
            Stmt::FnDecl(f)=> {
                self.collect_fn(f);
                self.check_fn(f);
            }
            Stmt::EnumDecl(e)   => self.collect_enum(e),
            Stmt::StructDecl(s) => self.collect_struct(s),
        }
    }

    fn check_let(&mut self, l: &LetStmt, scope: &mut LocalScope) {
        let declared_ty = l.ty.as_ref().map(XoreType::from_type_expr);
        let init_ty = l.init.as_ref().map(|e| self.infer_expr(e, scope));

        let ty = match (declared_ty, init_ty) {
            (Some(d), Some(i)) => {
                if !d.compatible(&i) {
                    self.errors.push(TypeError::new(
                        format!("type mismatch: declared `{d}`, initialiser is `{i}`"),
                        l.span,
                    ));
                }
                d
            }
            (Some(d), None) => d,
            (None, Some(i)) => i,
            (None, None)    => XoreType::Unknown,
        };
        scope.declare(&l.name, ty);
    }

    fn check_return(&mut self, r: &ReturnStmt, scope: &mut LocalScope, ret_ty: &XoreType) {
        match &r.value {
            None => {
                if *ret_ty != XoreType::Void {
                    self.errors.push(TypeError::new(
                        format!("empty return in function that returns `{ret_ty}`"),
                        r.span,
                    ));
                }
            }
            Some(e) => {
                let actual = self.infer_expr(e, scope);
                if !ret_ty.compatible(&actual) {
                    self.errors.push(TypeError::new(
                        format!("return type mismatch: expected `{ret_ty}`, got `{actual}`"),
                        r.span,
                    ));
                }
            }
        }
    }

    fn check_if(&mut self, i: &IfStmt, scope: &mut LocalScope, ret_ty: &XoreType) {
        let cond_ty = self.infer_expr(&i.cond, scope);
        if !cond_ty.compatible(&XoreType::Bool) && !cond_ty.is_integer() {
            self.errors.push(TypeError::new(
                format!("if condition must be bool or integer, got `{cond_ty}`"),
                i.span,
            ));
        }
        self.check_block(&i.then_body, scope, ret_ty);
        if let Some(els) = &i.else_body {
            match els.as_ref() {
                ElseBranch::Block(b) => self.check_block(b, scope, ret_ty),
                ElseBranch::If(i2)   => self.check_if(i2, scope, ret_ty),
            }
        }
    }

    fn check_while(&mut self, w: &WhileStmt, scope: &mut LocalScope, ret_ty: &XoreType) {
        self.infer_expr(&w.cond, scope);
        self.check_block(&w.body, scope, ret_ty);
    }

    fn check_for(&mut self, f: &ForStmt, scope: &mut LocalScope, ret_ty: &XoreType) {
        self.infer_expr(&f.limit, scope);
        scope.push();
        scope.declare(&f.var, XoreType::I64);
        self.check_block(&f.body, scope, ret_ty);
        scope.pop();
    }

    // ── Expression type inference ─────────────────────────────────────────

    pub fn infer_expr(&mut self, e: &Expr, scope: &LocalScope) -> XoreType {
        match e {
            Expr::IntLit { .. }   => XoreType::I64,
            Expr::FloatLit { .. } => XoreType::F64,
            Expr::StrLit { .. }   => XoreType::Str,
            Expr::BoolLit { .. }  => XoreType::Bool,
            Expr::NoneLit { .. }  => XoreType::Unknown,

            Expr::Ident { name, span } => {
                if let Some(ty) = scope.lookup(name) {
                    ty.clone()
                } else if self.fns.contains_key(name.as_str()) {
                    XoreType::Unknown  // function pointer — not yet modelled
                } else {
                    self.errors.push(TypeError::new(
                        format!("undefined variable `{name}`"),
                        *span,
                    ));
                    XoreType::Unknown
                }
            }

            Expr::Field { object, field, span } => {
                let obj_ty = self.infer_expr(object, scope);
                match &obj_ty {
                    XoreType::Struct(name) => {
                        if let Some(layout) = self.structs.get(name) {
                            if let Some(ft) = layout.field_type(field) {
                                ft.clone()
                            } else {
                                self.errors.push(TypeError::new(
                                    format!("struct `{name}` has no field `{field}`"),
                                    *span,
                                ));
                                XoreType::Unknown
                            }
                        } else {
                            XoreType::Unknown
                        }
                    }
                    // String method calls (e.g. String.new())
                    XoreType::String => XoreType::String,
                    _ => XoreType::Unknown,
                }
            }

            Expr::BinOp { op, lhs, rhs, span } => {
                let lt = self.infer_expr(lhs, scope);
                let rt = self.infer_expr(rhs, scope);
                if !lt.compatible(&rt) {
                    self.errors.push(TypeError::new(
                        format!("type mismatch in binary op: `{lt}` vs `{rt}`"),
                        *span,
                    ));
                }
                // Comparison ops return bool
                match op {
                    BinOp::Eq | BinOp::Ne | BinOp::Lt |
                    BinOp::Gt | BinOp::Le | BinOp::Ge |
                    BinOp::And | BinOp::Or => XoreType::Bool,
                    _ => lt,
                }
            }

            Expr::UnOp { op, operand, .. } => {
                let t = self.infer_expr(operand, scope);
                match op {
                    UnOp::Not => XoreType::Bool,
                    UnOp::Ref => XoreType::Ref(Box::new(t)),
                    UnOp::Deref => match t {
                        XoreType::Ref(inner) | XoreType::MutRef(inner) |
                        XoreType::Ptr(inner) => *inner,
                        _ => XoreType::Unknown,
                    },
                    UnOp::Neg => t,
                }
            }

            Expr::Assign { target, value, span, .. } => {
                let lt = self.infer_expr(target, scope);
                let rt = self.infer_expr(value, scope);
                if !lt.compatible(&rt) {
                    self.errors.push(TypeError::new(
                        format!("assignment type mismatch: `{lt}` = `{rt}`"),
                        *span,
                    ));
                }
                lt
            }

            Expr::Call { callee, args, span } => {
                let fn_name = match callee.as_ref() {
                    Expr::Ident { name, .. } => Some(name.clone()),
                    Expr::Field { field, .. } => Some(field.clone()),
                    _ => None,
                };
                if let Some(name) = fn_name {
                    if let Some(sig) = self.fns.get(&name).cloned() {
                        // Check arity
                        if args.len() != sig.params.len() {
                            self.errors.push(TypeError::new(
                                format!("function `{name}` expects {} args, got {}",
                                    sig.params.len(), args.len()),
                                *span,
                            ));
                        }
                        // Check arg types
                        for (arg, (_, param_ty)) in args.iter().zip(sig.params.iter()) {
                            let at = self.infer_expr(arg, scope);
                            if !param_ty.compatible(&at) {
                                self.errors.push(TypeError::new(
                                    format!("arg type mismatch: expected `{param_ty}`, got `{at}`"),
                                    *span,
                                ));
                            }
                        }
                        sig.ret.clone()
                    } else {
                        // Unknown function — infer args for side effects
                        for a in args { self.infer_expr(a, scope); }
                        XoreType::Unknown
                    }
                } else {
                    for a in args { self.infer_expr(a, scope); }
                    XoreType::Unknown
                }
            }

            Expr::MacroCall { args, .. } => {
                for a in args { self.infer_expr(a, scope); }
                XoreType::Void
            }

            Expr::TypeCall { ty, args, .. } => {
                for a in args { self.infer_expr(a, scope); }
                XoreType::from_type_expr(ty)
            }

            Expr::Paren   { inner, .. } => self.infer_expr(inner, scope),
            Expr::Bracket { inner, .. } => self.infer_expr(inner, scope),

            Expr::FmtChain { parts, .. } => {
                for p in parts {
                    if let FmtPart::Hole(e) = p { self.infer_expr(e, scope); }
                }
                XoreType::Str
            }
        }
    }
}

// ─── Local scope (variable → type mapping) ───────────────────────────────────

pub struct LocalScope {
    stack: Vec<HashMap<String, XoreType>>,
}

impl LocalScope {
    pub fn new() -> Self {
        Self { stack: vec![HashMap::new()] }
    }

    pub fn push(&mut self) {
        self.stack.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        if self.stack.len() > 1 { self.stack.pop(); }
    }

    pub fn declare(&mut self, name: &str, ty: XoreType) {
        if let Some(top) = self.stack.last_mut() {
            top.insert(name.to_string(), ty);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&XoreType> {
        for scope in self.stack.iter().rev() {
            if let Some(ty) = scope.get(name) { return Some(ty); }
        }
        None
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn check(src: &str) -> Vec<TypeError> {
        let (prog, _, _) = parse(src);
        let mut tc = TypeChecker::new();
        tc.check_program(&prog);
        tc.errors
    }

    fn check_ok(src: &str) {
        let errs = check(src);
        assert!(errs.is_empty(), "unexpected type errors: {errs:?}");
    }

    #[test]
    fn simple_fn_ok() {
        check_ok("public fn add(a: i32, b: i32) -> i32 { return a + b; }");
    }

    #[test]
    fn let_type_mismatch() {
        // i32 variable assigned a float — should error
        let errs = check("public main() { let x: i32 = 3.14; }");
        assert!(!errs.is_empty(), "expected type error");
    }

    #[test]
    fn undefined_variable() {
        let errs = check("public main() { let x = y + 1; }");
        assert!(!errs.is_empty());
    }

    #[test]
    fn return_type_mismatch() {
        let errs = check("public fn f() -> i32 { return; }");
        assert!(!errs.is_empty());
    }

    #[test]
    fn correct_return() {
        check_ok("public fn f() -> i32 { return 42; }");
    }

    #[test]
    fn struct_field_access_ok() {
        let src = "struct Point { x: i32, y: i32 } public fn get_x(p: Point) -> i32 { return 0; }";
        check_ok(src);
    }

    #[test]
    fn for_loop_var_declared() {
        check_ok("public main() { for in i range (10) { println!(i); } }");
    }

    #[test]
    fn nested_fn_ok() {
        check_ok("public main() { fn helper() -> i32 { return 1; } println!(helper()); }");
    }
}