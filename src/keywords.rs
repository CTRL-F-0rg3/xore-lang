use crate::token::Keyword;

/// Try to parse `word` as a Xore keyword.
/// Returns `None` for plain identifiers.
///
/// All 100+ keywords live here so the lexer never does string
/// comparisons anywhere else.
pub fn lookup(word: &str) -> Option<Keyword> {
    // Fast-path: most identifiers are longer than the longest keyword (11 chars).
    // "thread_local" is 12, "inline_asm" is 10, "load_module" is 11.
    if word.len() > 12 {
        return None;
    }

    match word {
        // ── Core: modules / functions ──────────────────────────────────────
        "fn"          => Some(Keyword::Fn),
        "pub"         => Some(Keyword::Pub),
        "export"      => Some(Keyword::Export),
        "import"      => Some(Keyword::Import),
        "mod"         => Some(Keyword::Mod),
        "use"         => Some(Keyword::Use),

        // ── Core: variables ────────────────────────────────────────────────
        "let"         => Some(Keyword::Let),
        "mut"         => Some(Keyword::Mut),
        "const"       => Some(Keyword::Const),
        "static"      => Some(Keyword::Static),

        // ── Core: control flow ─────────────────────────────────────────────
        "if"          => Some(Keyword::If),
        "else"        => Some(Keyword::Else),
        "match"       => Some(Keyword::Match),
        "switch"      => Some(Keyword::Switch),
        "loop"        => Some(Keyword::Loop),
        "while"       => Some(Keyword::While),
        "for"         => Some(Keyword::For),
        "break"       => Some(Keyword::Break),
        "continue"    => Some(Keyword::Continue),
        "return"      => Some(Keyword::Return),

        // ── Core: error handling ───────────────────────────────────────────
        "try"         => Some(Keyword::Try),
        "error"       => Some(Keyword::Error),

        // ── Core: type system ──────────────────────────────────────────────
        "struct"      => Some(Keyword::Struct),
        "enum"        => Some(Keyword::Enum),
        "union"       => Some(Keyword::Union),
        "type"        => Some(Keyword::Type),
        "trait"       => Some(Keyword::Trait),

        // ── Core: safety / lifetime ────────────────────────────────────────
        "unsafe"      => Some(Keyword::Unsafe),
        "defer"       => Some(Keyword::Defer),

        // ── Memory / allocation ────────────────────────────────────────────
        "alloc"       => Some(Keyword::Alloc),
        "free"        => Some(Keyword::Free),
        "realloc"     => Some(Keyword::Realloc),
        "heap"        => Some(Keyword::Heap),
        "stack"       => Some(Keyword::Stack),
        "ptr"         => Some(Keyword::Ptr),
        "slice"       => Some(Keyword::Slice),
        "array"       => Some(Keyword::Array),
        "align"       => Some(Keyword::Align),
        "packed"      => Some(Keyword::Packed),
        "noalias"     => Some(Keyword::Noalias),

        // ── ASM / inline assembly ──────────────────────────────────────────
        "asm"         => Some(Keyword::Asm),
        "inline_asm"  => Some(Keyword::InlineAsm),
        "syscall"     => Some(Keyword::Syscall),
        "interrupt"   => Some(Keyword::Interrupt),
        "naked"       => Some(Keyword::Naked),
        "volatile"    => Some(Keyword::Volatile),

        // ── Module system ──────────────────────────────────────────────────
        "module"      => Some(Keyword::Module),
        "load_module" => Some(Keyword::LoadModule),
        "link"        => Some(Keyword::Link),
        "dynamic"     => Some(Keyword::Dynamic),
        "extern"      => Some(Keyword::Extern),
        "global"      => Some(Keyword::Global),
        "thread_local"=> Some(Keyword::ThreadLocal),

        // ── Comptime / meta ────────────────────────────────────────────────
        "comptime"    => Some(Keyword::Comptime),
        "inline"      => Some(Keyword::Inline),
        "noinline"    => Some(Keyword::Noinline),
        "unreachable" => Some(Keyword::Unreachable),
        "panic"       => Some(Keyword::Panic),

        // ── Primitive types ────────────────────────────────────────────────
        "i8"          => Some(Keyword::I8),
        "u8"          => Some(Keyword::U8),
        "i16"         => Some(Keyword::I16),
        "u16"         => Some(Keyword::U16),
        "i32"         => Some(Keyword::I32),
        "u32"         => Some(Keyword::U32),
        "i64"         => Some(Keyword::I64),
        "u64"         => Some(Keyword::U64),
        "f32"         => Some(Keyword::F32),
        "f64"         => Some(Keyword::F64),
        "usize"       => Some(Keyword::Usize),
        "isize"       => Some(Keyword::Isize),
        "bool"        => Some(Keyword::Bool),
        "void"        => Some(Keyword::Void),
        "char"        => Some(Keyword::Char),
        "anytype"     => Some(Keyword::Anytype),

        // ── Boolean / null literals that are keywords ──────────────────────
        "true"        => Some(Keyword::True),
        "false"       => Some(Keyword::False),
        "null"        => Some(Keyword::Null),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_keywords_resolve() {
        assert_eq!(lookup("fn"),     Some(Keyword::Fn));
        assert_eq!(lookup("unsafe"), Some(Keyword::Unsafe));
        assert_eq!(lookup("match"),  Some(Keyword::Match));
        assert_eq!(lookup("defer"),  Some(Keyword::Defer));
    }

    #[test]
    fn asm_keywords_resolve() {
        assert_eq!(lookup("asm"),        Some(Keyword::Asm));
        assert_eq!(lookup("inline_asm"), Some(Keyword::InlineAsm));
        assert_eq!(lookup("syscall"),    Some(Keyword::Syscall));
        assert_eq!(lookup("volatile"),   Some(Keyword::Volatile));
        assert_eq!(lookup("naked"),      Some(Keyword::Naked));
    }

    #[test]
    fn type_keywords_resolve() {
        assert_eq!(lookup("i32"),  Some(Keyword::I32));
        assert_eq!(lookup("u64"),  Some(Keyword::U64));
        assert_eq!(lookup("f32"),  Some(Keyword::F32));
        assert_eq!(lookup("bool"), Some(Keyword::Bool));
        assert_eq!(lookup("void"), Some(Keyword::Void));
    }

    #[test]
    fn module_keywords_resolve() {
        assert_eq!(lookup("load_module"),  Some(Keyword::LoadModule));
        assert_eq!(lookup("thread_local"), Some(Keyword::ThreadLocal));
    }

    #[test]
    fn plain_identifier_returns_none() {
        assert_eq!(lookup("myVar"),    None);
        assert_eq!(lookup("_private"), None);
        assert_eq!(lookup("fn2"),      None);
        // Rust-reserved words that are NOT Xore keywords
        assert_eq!(lookup("impl"),     None);
        assert_eq!(lookup("where"),    None);
    }

    #[test]
    fn long_identifier_fast_path() {
        // Should hit the early-return, never panic.
        assert_eq!(lookup("this_identifier_is_very_long"), None);
    }
}
