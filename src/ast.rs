// AST (Abstract Syntax Tree) node definitions for Xore.
//
// Every node carries a `Span` so error messages can point at the exact
// source location.  Nodes are kept as plain structs / enums — no arena
// allocation yet, just `Box<>` for recursion.

use crate::token::Span;

// ─── Top-level ───────────────────────────────────────────────────────────────

/// A fully parsed `.xre` source file.
#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
    pub span:  Span,
}

/// Any top-level declaration.
#[derive(Debug, Clone)]
pub enum Item {
    Function(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Import(ImportDecl),
    Use(UseDecl),
    Mod(ModDecl),
}

// ─── Visibility ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Public,
    Private,  // default when no keyword is written
}

// ─── Type expressions ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// Named type: `i32`, `String`, `Point`, `Self`
    Named(String, Span),
    /// Reference: `&T`, `&str`
    Ref(Box<TypeExpr>, Span),
    /// Mutable reference: `&mut T`
    MutRef(Box<TypeExpr>, Span),
    /// Pointer: `*T`
    Pointer(Box<TypeExpr>, Span),
    /// Slice: `[T]`
    Slice(Box<TypeExpr>, Span),
    /// Fixed array: `[T; N]`
    Array(Box<TypeExpr>, Box<Expr>, Span),
    /// Result / error union: `!T`
    ErrorUnion(Box<TypeExpr>, Span),
    /// Function pointer: `fn(A, B) -> R`
    FnPtr { params: Vec<TypeExpr>, ret: Box<TypeExpr>, span: Span },
    /// Infer type from context (`_`)
    Infer(Span),
    /// `void` — no value
    Void(Span),
}

// ─── Function declarations ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub vis:      Visibility,
    pub exported: bool,      // @export annotation present
    pub name:     String,
    pub params:   Vec<Param>,
    pub ret_ty:   Option<TypeExpr>,
    pub body:     Block,
    pub span:     Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name:  String,
    pub ty:    TypeExpr,
    pub span:  Span,
}

// ─── Struct / Enum ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub vis:    Visibility,
    pub name:   String,
    pub fields: Vec<StructField>,
    pub span:   Span,
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub ty:   TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub vis:      Visibility,
    pub name:     String,
    pub variants: Vec<EnumVariant>,
    pub span:     Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name:   String,
    /// Optional tuple payload: `Blue(u8, u8, u8)`
    pub fields: Vec<TypeExpr>,
    pub span:   Span,
}

// ─── Module system ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ModDecl {
    pub name:  String,
    pub items: Vec<Item>,
    pub span:  Span,
}

// ─── Statements ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span:  Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `let [mut] name [: Type] = expr;`
    Let(LetStmt),
    /// `expr;` or `expr` (as expression-statement)
    Expr(Expr),
    /// `return [expr];`
    Return(ReturnStmt),
    /// `if [cond] { … } [else { … }]`
    If(IfStmt),
    /// `while cond { … }`
    While(WhileStmt),
    /// `for in name range (n) { … }`
    For(ForStmt),
    /// `end() [if cond];`  — Xore-specific early-exit
    End(EndStmt),
    /// `match expr { pattern => { block }, ... }`
    Match(MatchStmt),
    /// `switch expr { case val: { block } default: { block } }`
    Switch(SwitchStmt),
    /// Inner function definition: `fn name(…) -> T { … }`
    FnDecl(FnDecl),
    /// Inner enum definition: `enum Name { … }`
    EnumDecl(EnumDecl),
    /// Inner struct definition
    StructDecl(StructDecl),
}

// ── Match ─────────────────────────────────────────────────────────────────────

/// `match expr { pattern => { body }, ... }`
#[derive(Debug, Clone)]
pub struct MatchStmt {
    pub subject: Expr,
    pub arms:    Vec<MatchArm>,
    pub span:    Span,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body:    Block,
    pub span:    Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    /// Literal: `42`, `"hello"`, `True`, `False`, `None`
    Lit(Expr),
    /// Enum variant: `Color.Red` or just `Red`
    Variant(String, Option<String>),
    /// Binding: `x` — captures matched value
    Bind(String),
    /// Wildcard: `_`
    Wildcard,
    /// Range: `0..10`
    Range(Expr, Expr),
}

// ── Switch ────────────────────────────────────────────────────────────────────

/// `switch expr { case val: { body } case val2: { body } default: { body } }`
#[derive(Debug, Clone)]
pub struct SwitchStmt {
    pub subject:  Expr,
    pub cases:    Vec<SwitchCase>,
    pub default:  Option<Block>,
    pub span:     Span,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub value: Expr,
    pub body:  Block,
    pub span:  Span,
}

#[derive(Debug, Clone)]
pub struct LetStmt {
    pub mutable: bool,
    pub name:    String,
    pub ty:      Option<TypeExpr>,
    pub init:    Option<Expr>,
    pub span:    Span,
}

#[derive(Debug, Clone)]
pub struct ReturnStmt {
    pub value: Option<Expr>,
    pub span:  Span,
}

/// `if [cond] { … } else { … }`
/// Xore uses square brackets for the condition: `if [x == y] { … }`
#[derive(Debug, Clone)]
pub struct IfStmt {
    pub cond:      Expr,
    pub then_body: Block,
    pub else_body: Option<Box<ElseBranch>>,
    pub span:      Span,
}

#[derive(Debug, Clone)]
pub enum ElseBranch {
    Block(Block),
    If(IfStmt),
}

#[derive(Debug, Clone)]
pub struct WhileStmt {
    pub cond: Expr,
    pub body: Block,
    pub span: Span,
}

/// `for in <ident> range (<expr>) { … }`
#[derive(Debug, Clone)]
pub struct ForStmt {
    pub var:   String,
    pub limit: Expr,
    pub body:  Block,
    pub span:  Span,
}

/// `end()` — break out of current loop / block.
/// Can be conditional: `end() if <cond>;`
#[derive(Debug, Clone)]
pub struct EndStmt {
    pub cond: Option<Expr>,
    pub span: Span,
}

// ─── Expressions ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    // ── Literals ─────────────────────────────────────────────────────────
    IntLit    { value: u128, span: Span },
    FloatLit  { value: f64,  span: Span },
    StrLit    { value: String, span: Span },
    BoolLit   { value: bool, span: Span },
    NoneLit   { span: Span },

    // ── Identifiers / paths ───────────────────────────────────────────────
    Ident     { name: String, span: Span },
    /// `a.b` or `a.b.c` — field access / method chain
    Field     { object: Box<Expr>, field: String, span: Span },
    /// `"".{}.x` — Xore format-string chain  `{value}.field`
    FmtChain  { parts: Vec<FmtPart>, span: Span },

    // ── Calls ─────────────────────────────────────────────────────────────
    /// Regular function call: `foo(a, b)`
    Call      { callee: Box<Expr>, args: Vec<Expr>, span: Span },
    /// Macro call: `println!("…")`
    MacroCall { name: String, args: Vec<Expr>, span: Span },
    /// Type constructor: `i32(23)`, `bool(False)`, `String.new()`
    TypeCall  { ty: Box<TypeExpr>, args: Vec<Expr>, span: Span },

    // ── Operators ─────────────────────────────────────────────────────────
    BinOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span: Span },
    UnOp  { op: UnOp,  operand: Box<Expr>,             span: Span },
    Assign { target: Box<Expr>, op: AssignOp, value: Box<Expr>, span: Span },

    // ── Grouping / casts ──────────────────────────────────────────────────
    Paren   { inner: Box<Expr>, span: Span },
    /// `[expr]` — square-bracket group used in `if` conditions
    Bracket { inner: Box<Expr>, span: Span },
}

/// One segment of a `"".{}.field` format chain.
#[derive(Debug, Clone)]
pub enum FmtPart {
    /// Literal string piece
    Str(String),
    /// `{expr}` interpolation hole
    Hole(Expr),
    /// `.field` access on the chain
    Field(String),
}

// ─── Operators ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    // arithmetic
    Add, Sub, Mul, Div, Rem,
    // bitwise
    BitAnd, BitOr, BitXor, Shl, Shr,
    // logical
    And, Or,
    // comparison
    Eq, Ne, Lt, Gt, Le, Ge,
    // range
    Range, RangeInclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,    // -x
    Not,    // !x
    Deref,  // *x
    Ref,    // &x
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,       // =
    AddAssign,    // +=
    SubAssign,    // -=
    MulAssign,    // *=
    DivAssign,    // /=
    RemAssign,    // %=
    AndAssign,    // &=
    OrAssign,     // |=
    XorAssign,    // ^=
    ShlAssign,    // <<=
    ShrAssign,    // >>=
}