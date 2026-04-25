/// Byte offset span inside the source string.
/// `start` is inclusive, `end` is exclusive — same convention as Rust ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

impl Span {
    #[inline]
    pub fn new(start: usize, end: usize, line: u32, col: u32) -> Self {
        Self { start, end, line, col }
    }

    /// Returns the byte length of the spanned region.
    #[inline]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Returns the source slice covered by this span.
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }
}

// ─── Keyword kinds ──────────────────────────────────────────────────────────

/// Every keyword is kept as its own variant so the parser can match on it
/// directly without going through a string comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    // ── Module / function ──────────────────────────────────────────────────
    Fn,
    Pub,
    Export,
    Import,
    Mod,
    Use,

    // ── Variables ──────────────────────────────────────────────────────────
    Let,
    Mut,
    Const,
    Static,

    // ── Control flow ───────────────────────────────────────────────────────
    If,
    Else,
    Match,
    Switch,
    Loop,
    While,
    For,
    Break,
    Continue,
    Return,

    // ── Error handling ─────────────────────────────────────────────────────
    Try,
    Error,

    // ── Type system ────────────────────────────────────────────────────────
    Struct,
    Enum,
    Union,
    Type,
    Trait,

    // ── Safety / lifetime ──────────────────────────────────────────────────
    Unsafe,
    Defer,

    // ── Memory / allocation ────────────────────────────────────────────────
    Alloc,
    Free,
    Realloc,
    Heap,
    Stack,
    Ptr,
    Slice,
    Array,
    Align,
    Packed,
    Noalias,

    // ── ASM / inline assembly ──────────────────────────────────────────────
    Asm,
    InlineAsm,
    Syscall,
    Interrupt,
    Naked,
    Volatile,

    // ── Module system ──────────────────────────────────────────────────────
    Module,
    LoadModule,
    Link,
    Dynamic,
    Extern,
    Global,
    ThreadLocal,

    // ── Comptime / meta ────────────────────────────────────────────────────
    Comptime,
    Inline,
    Noinline,
    Unreachable,
    Panic,

    // ── Primitive types ────────────────────────────────────────────────────
    I8,  U8,
    I16, U16,
    I32, U32,
    I64, U64,
    F32, F64,
    Usize, Isize,
    Bool,
    Void,
    Char,
    Anytype,

    // ── Boolean / null literals that are keywords ──────────────────────────
    True,
    False,
    Null,

    // ── Xore-specific ──────────────────────────────────────────────────────
    Public,
    Private,
    End,
    Range,
    In,
    SelfType,   // Self (type-level)
    SelfValue,  // self (value-level)
}

impl Keyword {
    /// Returns the canonical source representation of this keyword.
    pub fn as_str(self) -> &'static str {
        use Keyword::*;
        match self {
            Fn => "fn", Pub => "pub", Export => "export", Import => "import",
            Mod => "mod", Use => "use",
            Let => "let", Mut => "mut", Const => "const", Static => "static",
            If => "if", Else => "else", Match => "match", Switch => "switch",
            Loop => "loop", While => "while", For => "for",
            Break => "break", Continue => "continue", Return => "return",
            Try => "try", Error => "error",
            Struct => "struct", Enum => "enum", Union => "union",
            Type => "type", Trait => "trait",
            Unsafe => "unsafe", Defer => "defer",
            Alloc => "alloc", Free => "free", Realloc => "realloc",
            Heap => "heap", Stack => "stack", Ptr => "ptr",
            Slice => "slice", Array => "array", Align => "align",
            Packed => "packed", Noalias => "noalias",
            Asm => "asm", InlineAsm => "inline_asm", Syscall => "syscall",
            Interrupt => "interrupt", Naked => "naked", Volatile => "volatile",
            Module => "module", LoadModule => "load_module", Link => "link",
            Dynamic => "dynamic", Extern => "extern", Global => "global",
            ThreadLocal => "thread_local",
            Comptime => "comptime", Inline => "inline", Noinline => "noinline",
            Unreachable => "unreachable", Panic => "panic",
            I8 => "i8", U8 => "u8", I16 => "i16", U16 => "u16",
            I32 => "i32", U32 => "u32", I64 => "i64", U64 => "u64",
            F32 => "f32", F64 => "f64",
            Usize => "usize", Isize => "isize",
            Bool => "bool", Void => "void", Char => "char", Anytype => "anytype",
            True => "True", False => "False", Null => "None",
            Public => "public", Private => "private",
            End => "end", Range => "range", In => "in",
            SelfType => "Self", SelfValue => "self",
        }
    }

    /// Classify a keyword into one of the high-level groups used for
    /// diagnostics and the playground colour scheme.
    pub fn group(self) -> KeywordGroup {
        use Keyword::*;
        match self {
            Fn | Pub | Export | Import | Mod | Use |
            Let | Mut | Const | Static |
            If | Else | Match | Switch | Loop | While | For |
            Break | Continue | Return |
            Try | Error |
            Struct | Enum | Union | Type | Trait |
            Unsafe | Defer => KeywordGroup::Core,

            Alloc | Free | Realloc | Heap | Stack | Ptr |
            Slice | Array | Align | Packed | Noalias => KeywordGroup::Memory,

            Asm | InlineAsm | Syscall | Interrupt |
            Naked | Volatile => KeywordGroup::Asm,

            Module | LoadModule | Link | Dynamic |
            Extern | Global | ThreadLocal => KeywordGroup::Module,

            Comptime | Inline | Noinline |
            Unreachable | Panic => KeywordGroup::Meta,

            I8 | U8 | I16 | U16 | I32 | U32 | I64 | U64 |
            F32 | F64 | Usize | Isize |
            Bool | Void | Char | Anytype |
            True | False | Null => KeywordGroup::Type,

            Public | Private | End | Range | In |
            SelfType | SelfValue => KeywordGroup::Core,
        }
    }
}

/// High-level category of a keyword — used for diagnostics and colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordGroup {
    Core,
    Memory,
    Asm,
    Module,
    Meta,
    Type,
}

// ─── Integer / float suffixes ────────────────────────────────────────────────

/// Explicit numeric suffix on an integer or float literal, e.g. `42u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumSuffix {
    I8, U8, I16, U16, I32, U32, I64, U64, Usize, Isize,
    F32, F64,
}

impl NumSuffix {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "i8" => Some(Self::I8),   "u8"  => Some(Self::U8),
            "i16"=> Some(Self::I16),  "u16" => Some(Self::U16),
            "i32"=> Some(Self::I32),  "u32" => Some(Self::U32),
            "i64"=> Some(Self::I64),  "u64" => Some(Self::U64),
            "usize"  => Some(Self::Usize),
            "isize"  => Some(Self::Isize),
            "f32"=> Some(Self::F32),  "f64" => Some(Self::F64),
            _ => None,
        }
    }
}

// ─── Numeric base ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumBase {
    Decimal,
    Hex,
    Octal,
    Binary,
}

// ─── Operator / punctuation kinds ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    // ── Arithmetic ─────────────────────────────────────────────────────────
    Plus, Minus, Star, Slash, Percent,
    // ── Bitwise ────────────────────────────────────────────────────────────
    Amp, Pipe, Caret, Tilde, Shl, Shr,
    // ── Logical ────────────────────────────────────────────────────────────
    And, Or, Bang,
    // ── Comparison ─────────────────────────────────────────────────────────
    Eq, Ne, Lt, Gt, Le, Ge,
    // ── Assignment (compound) ──────────────────────────────────────────────
    Assign,
    PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    AmpEq, PipeEq, CaretEq, ShlEq, ShrEq,
    // ── Arrows / path ──────────────────────────────────────────────────────
    Arrow,       // ->
    FatArrow,    // =>
    ColonColon,  // ::
    // ── Range ──────────────────────────────────────────────────────────────
    DotDot,      // ..
    DotDotDot,   // ...
    // ── Error propagation ──────────────────────────────────────────────────
    Question,    // ?
    Bang2,       // ! (already Bang above, reuse)
    // ── Misc ───────────────────────────────────────────────────────────────
    At,          // @
    Hash,        // #
    Dollar,      // $
    Dot,         // .
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delim {
    OpenParen, CloseParen,
    OpenBrace, CloseBrace,
    OpenBracket, CloseBracket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Punct {
    Semicolon,
    Comma,
    Colon,
}

// ─── Main token kind ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ── Keywords ───────────────────────────────────────────────────────────
    Kw(Keyword),

    // ── Annotations: @comptime, @inline, @export … ─────────────────────────
    Annotation(String),

    // ── Identifiers ────────────────────────────────────────────────────────
    Ident(String),

    // ── Literals ───────────────────────────────────────────────────────────
    /// Raw integer value + original base + optional type suffix.
    IntLit {
        value: u128,
        base: NumBase,
        suffix: Option<NumSuffix>,
    },
    /// Parsed float value + optional suffix.
    FloatLit {
        value: f64,
        suffix: Option<NumSuffix>,
    },
    /// Decoded string contents (escape sequences resolved).
    StrLit(String),
    /// Raw source bytes for multi-line / raw strings: `r#"…"#`.
    RawStrLit(String),
    /// Decoded char value.
    CharLit(char),

    // ── Operators ──────────────────────────────────────────────────────────
    Op(Op),

    // ── Delimiters ─────────────────────────────────────────────────────────
    Delim(Delim),

    // ── Punctuation ────────────────────────────────────────────────────────
    Punct(Punct),

    // ── Comments (kept so a formatter / IDE can round-trip the source) ─────
    LineComment(String),
    BlockComment(String),

    // ── Special ────────────────────────────────────────────────────────────
    /// Produced when the lexer encounters a byte it cannot classify.
    Unknown(char),
    /// Synthetic end-of-file sentinel — always the last token in the stream.
    Eof,
}

impl TokenKind {
    /// Human-readable name, used in error messages.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Kw(_)          => "keyword",
            Self::Annotation(_)  => "annotation",
            Self::Ident(_)       => "identifier",
            Self::IntLit { .. }  => "integer literal",
            Self::FloatLit { .. }=> "float literal",
            Self::StrLit(_)      => "string literal",
            Self::RawStrLit(_)   => "raw string literal",
            Self::CharLit(_)     => "char literal",
            Self::Op(_)          => "operator",
            Self::Delim(_)       => "delimiter",
            Self::Punct(_)       => "punctuation",
            Self::LineComment(_) => "line comment",
            Self::BlockComment(_)=> "block comment",
            Self::Unknown(_)     => "unknown character",
            Self::Eof            => "end of file",
        }
    }
}

// ─── Full token ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn eof(pos: usize, line: u32, col: u32) -> Self {
        Self {
            kind: TokenKind::Eof,
            span: Span::new(pos, pos, line, col),
        }
    }

    /// Convenience: returns true when this token matches a specific keyword.
    pub fn is_kw(&self, kw: Keyword) -> bool {
        matches!(&self.kind, TokenKind::Kw(k) if *k == kw)
    }
}