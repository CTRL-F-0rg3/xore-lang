use std::io::{self, Read};
use xore_lexer::{lex, TokenKind, KeywordGroup};

fn main() {
    // Read source from stdin if piped, otherwise use the built-in demo.
    let source = {
        let mut buf = String::new();
        if atty::is_atty() {
            buf = DEMO_SOURCE.to_string();
        } else {
            io::stdin().read_to_string(&mut buf).expect("failed to read stdin");
        }
        buf
    };

    let (tokens, errors) = lex(&source);

    // ── Print header ──────────────────────────────────────────────────────
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│                   Xore Lexer  v0.1.0                       │");
    println!("└─────────────────────────────────────────────────────────────┘");
    println!();

    // ── Print tokens ──────────────────────────────────────────────────────
    println!("{:<6} {:<5} {:<22} {}", "LINE", "COL", "TYPE", "VALUE");
    println!("{}", "─".repeat(60));

    for tok in &tokens {
        if tok.kind == TokenKind::Eof { break; }

        let type_str = token_type_label(&tok.kind);
        let value_str = token_value(&tok.kind);

        println!(
            "{:<6} {:<5} {:<22} {}",
            tok.span.line, tok.span.col, type_str, value_str,
        );
    }

    println!("{}", "─".repeat(60));
    println!(
        "  {} tokens  |  {} errors",
        tokens.iter().filter(|t| t.kind != TokenKind::Eof).count(),
        errors.len()
    );

    // ── Print errors ──────────────────────────────────────────────────────
    if !errors.is_empty() {
        println!();
        println!("ERRORS:");
        for e in &errors {
            println!("  ✗ {e}");
        }
    }
}

fn token_type_label(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Kw(kw) => {
            let group = match kw.group() {
                KeywordGroup::Core   => "KW:CORE",
                KeywordGroup::Asm    => "KW:ASM",
                KeywordGroup::Memory => "KW:MEM",
                KeywordGroup::Module => "KW:MOD",
                KeywordGroup::Meta   => "KW:META",
                KeywordGroup::Type   => "KW:TYPE",
            };
            group.to_string()
        }
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

fn token_value(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Kw(kw)              => kw.as_str().to_string(),
        TokenKind::Annotation(s)       => format!("@{s}"),
        TokenKind::Ident(s)            => s.clone(),
        TokenKind::IntLit { value, base, suffix } => {
            let prefix: &str = match base {
                xore_lexer::NumBase::Decimal => "",
                xore_lexer::NumBase::Hex     => "0x",
                xore_lexer::NumBase::Octal   => "0o",
                xore_lexer::NumBase::Binary  => "0b",
            };
            let suf = suffix.map(|s| format!(" ({s:?})")).unwrap_or_default();
            format!("{prefix}{value}{suf}")
        }
        TokenKind::FloatLit { value, suffix } => {
            let suf = suffix.map(|s| format!(" ({s:?})")).unwrap_or_default();
            format!("{value}{suf}")
        }
        TokenKind::StrLit(s)           => format!("{s:?}"),
        TokenKind::RawStrLit(s)        => format!("r#{s:?}#"),
        TokenKind::CharLit(c)          => format!("'{c}'"),
        TokenKind::Op(op)              => format!("{op:?}"),
        TokenKind::Delim(d)            => format!("{d:?}"),
        TokenKind::Punct(p)            => format!("{p:?}"),
        TokenKind::LineComment(s)      => format!("//{s}"),
        TokenKind::BlockComment(s)     => {
            let preview = if s.len() > 30 { &s[..30] } else { s };
            format!("/* {preview} */")
        }
        TokenKind::Unknown(c)          => format!("'{c}'"),
        TokenKind::Eof                 => "<EOF>".into(),
    }
}

// ── atty shim ─────────────────────────────────────────────────────────────────
// Avoid pulling in a crate just for the demo; hard-code to "is_atty = true" so
// the demo source always runs when executed directly.
mod atty {
    pub fn is_atty() -> bool { true }
}

// ─── Demo source ──────────────────────────────────────────────────────────────
const DEMO_SOURCE: &str = r##"
// ── Xore-lang demo ──────────────────────────────────────────────────────────

export fn add(a: i32, b: i32) i32 {
    return a + b;
}

pub fn factorial(n: u64) u64 {
    if n == 0u64 { return 1u64; }
    return n * factorial(n - 1u64);
}

/* Unsafe block with inline ASM + syscall */
unsafe fn write_stdout(buf: ptr, len: usize) !void {
    asm volatile("syscall"
        : : "a"(1u64), "D"(1u64), "S"(buf), "d"(len)
        : "memory", "rcx", "r11"
    );
}

naked fn _start() void {
    asm("xor rbp, rbp\ncall main\nmov rax, 60\nsyscall\n");
}

mod io {
    pub fn print(s: &str) void { /* impl */ }
    pub fn read_line() !str  { /* impl */ }
}

load_module("net");
import net.tcp;

struct Point { x: f32, y: f32 }
enum  Color  { Red, Green, Blue(u8, u8, u8) }
union Value  { i: i64, f: f64, p: ptr }

trait Drawable {
    fn draw(self: &Self) void;
}

@comptime
fn zero_array(comptime T: anytype, n: usize) [T; n] {
    let mut arr: [T; n] = [0; n];
    defer { alloc.free(arr.ptr); }
    return arr;
}

const HEX: u32   = 0xDEAD_BEEF;
const OCT: u32   = 0o777;
const BIN: u8    = 0b1010_1010;
const PI:  f64   = 3.141_592_653f64;
const MSG: &str  = "Hello, \u{1F600} world!\n";
const RAW: &str  = r#"no \n escapes here"#;
let esc_char: char = '\t';
"##;
