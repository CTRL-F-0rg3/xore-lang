use crate::error::{LexError, LexResult};
use crate::keywords::lookup;
use crate::token::{
    Delim, NumBase, NumSuffix, Op, Punct, Span, Token, TokenKind,
};

// ─── Cursor ──────────────────────────────────────────────────────────────────

/// A byte-level cursor over a UTF-8 source string.
///
/// We work with byte positions internally and only decode `char` values at the
/// point where we need them, which keeps hot paths allocation-free.
struct Cursor<'src> {
    src: &'src str,
    /// Current byte position (always on a valid UTF-8 char boundary).
    pos: usize,
    line: u32,
    col: u32,
}

impl<'src> Cursor<'src> {
    fn new(src: &'src str) -> Self {
        Self { src, pos: 0, line: 1, col: 1 }
    }

    // ── Fundamental primitives ─────────────────────────────────────────────

    /// Current byte position.
    #[inline]
    fn pos(&self) -> usize { self.pos }

    /// `(line, col)` at the current position.
    #[inline]
    fn location(&self) -> (u32, u32) { (self.line, self.col) }

    /// Peek at the next `char` without consuming it, or `'\0'` at EOF.
    #[inline]
    fn peek(&self) -> char {
        self.src[self.pos..].chars().next().unwrap_or('\0')
    }

    /// Peek at the char *after* the next one — cheap second look-ahead.
    #[inline]
    fn peek2(&self) -> char {
        let mut it = self.src[self.pos..].chars();
        it.next();
        it.next().unwrap_or('\0')
    }

    /// Consume and return the next `char`.  Returns `'\0'` at EOF (caller must
    /// guard with `!self.is_eof()` when needed).
    fn bump(&mut self) -> char {
        let ch = match self.src[self.pos..].chars().next() {
            Some(c) => c,
            None => return '\0',
        };
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    /// Consume the next char only if it equals `expected`.
    #[inline]
    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == expected {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Consume chars while `predicate` holds.
    fn eat_while(&mut self, mut predicate: impl FnMut(char) -> bool) {
        while !self.is_eof() && predicate(self.peek()) {
            self.bump();
        }
    }

    #[inline]
    fn is_eof(&self) -> bool { self.pos >= self.src.len() }

    /// Build a `Span` from a recorded start position to *now*.
    #[inline]
    fn span_from(&self, start: usize, line: u32, col: u32) -> Span {
        Span::new(start, self.pos, line, col)
    }

    /// The source slice between `start` and the current position.
    #[inline]
    fn slice_from(&self, start: usize) -> &str {
        &self.src[start..self.pos]
    }
}

// ─── Lexer ───────────────────────────────────────────────────────────────────

/// The Xore lexer.
///
/// Call [`Lexer::next_token`] in a loop until you receive [`TokenKind::Eof`],
/// or collect everything at once with [`Lexer::tokenize`].
///
/// Errors are *non-fatal*: when the lexer cannot make sense of a byte sequence
/// it emits an [`TokenKind::Unknown`] token and records the error in
/// [`Lexer::errors`] so the caller can report all problems in one pass.
pub struct Lexer<'src> {
    cursor: Cursor<'src>,
    /// Accumulated non-fatal lexer errors.
    pub errors: Vec<LexError>,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self {
            cursor: Cursor::new(src),
            errors: Vec::new(),
        }
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Lex and return the next token.  Never panics; always terminates.
    pub fn next_token(&mut self) -> Token {
        self.skip_trivia();

        let start = self.cursor.pos();
        let (line, col) = self.cursor.location();

        if self.cursor.is_eof() {
            return Token::eof(start, line, col);
        }

        let ch = self.cursor.bump();
        let kind = self.dispatch(ch, start, line, col);
        let span = self.cursor.span_from(start, line, col);
        Token::new(kind, span)
    }

    /// Consume the entire source and return a `Vec<Token>` (including the
    /// final `Eof` sentinel).
    pub fn tokenize(mut self) -> (Vec<Token>, Vec<LexError>) {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof { break; }
        }
        (tokens, self.errors)
    }

    // ── Dispatch ───────────────────────────────────────────────────────────

    /// Main dispatch table — called after we've already consumed `ch`.
    fn dispatch(&mut self, ch: char, start: usize, line: u32, col: u32) -> TokenKind {
        match ch {
            // ── Whitespace (should have been skipped, but guard anyway) ────
            c if c.is_whitespace() => {
                let next = self.cursor.bump();
                self.dispatch(next, start, line, col)
            }

            // ── Line comment ───────────────────────────────────────────────
            '/' if self.cursor.peek() == '/' => self.lex_line_comment(),

            // ── Block comment ──────────────────────────────────────────────
            '/' if self.cursor.peek() == '*' => {
                self.cursor.bump(); // consume '*'
                self.lex_block_comment(start, line, col)
            }

            // ── Division / compound assign ─────────────────────────────────
            '/' => if self.cursor.eat('=') { TokenKind::Op(Op::SlashEq) }
                   else { TokenKind::Op(Op::Slash) },

            // ── Annotations: @name ─────────────────────────────────────────
            '@' => self.lex_annotation(),

            // ── String literals ────────────────────────────────────────────
            '"' => self.lex_string(start, line, col),

            // ── Raw string literals: r#"…"# ────────────────────────────────
            'r' if self.cursor.peek() == '#' || self.cursor.peek() == '"' => {
                self.lex_raw_string_or_ident('r', start, line, col)
            }

            // ── Char literals ──────────────────────────────────────────────
            '\'' => self.lex_char(start, line, col),

            // ── Numeric literals ───────────────────────────────────────────
            c if c.is_ascii_digit() => self.lex_number(c, start, line, col),

            // ── Identifiers and keywords ───────────────────────────────────
            c if is_ident_start(c) => self.lex_ident(c),

            // ── Delimiters ─────────────────────────────────────────────────
            '(' => TokenKind::Delim(Delim::OpenParen),
            ')' => TokenKind::Delim(Delim::CloseParen),
            '{' => TokenKind::Delim(Delim::OpenBrace),
            '}' => TokenKind::Delim(Delim::CloseBrace),
            '[' => TokenKind::Delim(Delim::OpenBracket),
            ']' => TokenKind::Delim(Delim::CloseBracket),

            // ── Punctuation ────────────────────────────────────────────────
            ';' => TokenKind::Punct(Punct::Semicolon),
            ',' => TokenKind::Punct(Punct::Comma),
            ':' => if self.cursor.eat(':') { TokenKind::Op(Op::ColonColon) }
                   else { TokenKind::Punct(Punct::Colon) },

            // ── Dot family ─────────────────────────────────────────────────
            '.' => match (self.cursor.peek(), self.cursor.peek2()) {
                ('.', '.') => { self.cursor.bump(); self.cursor.bump(); TokenKind::Op(Op::DotDotDot) }
                ('.', _)   => { self.cursor.bump(); TokenKind::Op(Op::DotDot) }
                _          => TokenKind::Op(Op::Dot),
            },

            // ── Arithmetic / compound assign ───────────────────────────────
            '+' => if self.cursor.eat('=') { TokenKind::Op(Op::PlusEq) }
                   else { TokenKind::Op(Op::Plus) },
            '-' => {
                if self.cursor.eat('>') { TokenKind::Op(Op::Arrow) }
                else if self.cursor.eat('=') { TokenKind::Op(Op::MinusEq) }
                else { TokenKind::Op(Op::Minus) }
            }
            '*' => if self.cursor.eat('=') { TokenKind::Op(Op::StarEq) }
                   else { TokenKind::Op(Op::Star) },
            '%' => if self.cursor.eat('=') { TokenKind::Op(Op::PercentEq) }
                   else { TokenKind::Op(Op::Percent) },

            // ── Bitwise / logical ──────────────────────────────────────────
            '&' => {
                if self.cursor.eat('&') { TokenKind::Op(Op::And) }
                else if self.cursor.eat('=') { TokenKind::Op(Op::AmpEq) }
                else { TokenKind::Op(Op::Amp) }
            }
            '|' => {
                if self.cursor.eat('|') { TokenKind::Op(Op::Or) }
                else if self.cursor.eat('=') { TokenKind::Op(Op::PipeEq) }
                else { TokenKind::Op(Op::Pipe) }
            }
            '^' => if self.cursor.eat('=') { TokenKind::Op(Op::CaretEq) }
                   else { TokenKind::Op(Op::Caret) },
            '~' => TokenKind::Op(Op::Tilde),

            // ── Shifts ─────────────────────────────────────────────────────
            '<' => {
                if self.cursor.eat('<') {
                    if self.cursor.eat('=') { TokenKind::Op(Op::ShlEq) }
                    else { TokenKind::Op(Op::Shl) }
                } else if self.cursor.eat('=') { TokenKind::Op(Op::Le) }
                else { TokenKind::Op(Op::Lt) }
            }
            '>' => {
                if self.cursor.eat('>') {
                    if self.cursor.eat('=') { TokenKind::Op(Op::ShrEq) }
                    else { TokenKind::Op(Op::Shr) }
                } else if self.cursor.eat('=') { TokenKind::Op(Op::Ge) }
                else { TokenKind::Op(Op::Gt) }
            }

            // ── Comparison / assignment ────────────────────────────────────
            '=' => {
                if self.cursor.eat('=') { TokenKind::Op(Op::Eq) }
                else if self.cursor.eat('>') { TokenKind::Op(Op::FatArrow) }
                else { TokenKind::Op(Op::Assign) }
            }
            '!' => if self.cursor.eat('=') { TokenKind::Op(Op::Ne) }
                   else { TokenKind::Op(Op::Bang) },

            // ── Misc ───────────────────────────────────────────────────────
            '?' => TokenKind::Op(Op::Question),
            '#' => TokenKind::Op(Op::Hash),
            '$' => TokenKind::Op(Op::Dollar),

            // ── Fallthrough: emit Unknown and record the error ─────────────
            c => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::UnknownChar { span, ch: c });
                TokenKind::Unknown(c)
            }
        }
    }

    // ── Trivia (whitespace + comments skipped silently) ────────────────────

    fn skip_trivia(&mut self) {
        // Only skip pure whitespace here.  Comments are emitted as tokens so
        // that formatters and IDEs can round-trip the source unchanged.
        self.cursor.eat_while(|c| c.is_whitespace());
    }

    // ── Comments ───────────────────────────────────────────────────────────

    /// Called after consuming the first `/` when `peek()=='/'`.
    /// Reads to end of line and returns the comment content (without `//`).
    fn lex_line_comment(&mut self) -> TokenKind {
        self.cursor.bump(); // consume the second '/'
        let start = self.cursor.pos();
        self.cursor.eat_while(|c| c != '\n');
        TokenKind::LineComment(self.cursor.slice_from(start).to_string())
    }

    /// Called after consuming `/*`.  Supports nested block comments.
    fn lex_block_comment(&mut self, start: usize, line: u32, col: u32) -> TokenKind {
        let content_start = self.cursor.pos();
        let mut depth: u32 = 1;

        loop {
            if self.cursor.is_eof() {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::UnterminatedBlockComment { span });
                break;
            }
            match (self.cursor.peek(), self.cursor.peek2()) {
                ('/', '*') => { self.cursor.bump(); self.cursor.bump(); depth += 1; }
                ('*', '/') => {
                    let end = self.cursor.pos();
                    self.cursor.bump(); self.cursor.bump();
                    return TokenKind::BlockComment(self.cursor.src[content_start..end].to_string());
                }
                _ => { self.cursor.bump(); }
            }
            if depth == 0 { break; }
        }
        // Unterminated — return what we have.
        TokenKind::BlockComment(self.cursor.slice_from(content_start).to_string())
    }

    // ── Annotations ────────────────────────────────────────────────────────

    /// Called after consuming `@`.
    fn lex_annotation(&mut self) -> TokenKind {
        let start = self.cursor.pos();
        self.cursor.eat_while(is_ident_continue);
        TokenKind::Annotation(self.cursor.slice_from(start).to_string())
    }

    // ── String literals ────────────────────────────────────────────────────

    /// Called after consuming the opening `"`.
    fn lex_string(&mut self, start: usize, line: u32, col: u32) -> TokenKind {
        let mut buf = String::new();
        loop {
            if self.cursor.is_eof() || self.cursor.peek() == '\n' {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::UnterminatedString { span });
                return TokenKind::StrLit(buf);
            }
            let ch = self.cursor.bump();
            match ch {
                '"' => return TokenKind::StrLit(buf),
                '\\' => {
                    let esc_start = self.cursor.pos() - 1;
                    let (line2, col2) = self.cursor.location();
                    match self.lex_escape(esc_start, line2, col2) {
                        Ok(c)  => buf.push(c),
                        Err(e) => { self.errors.push(e); }
                    }
                }
                c => buf.push(c),
            }
        }
    }

    /// Handles `r"…"` and `r#"…"#` raw string literals.
    /// The leading `r` has already been consumed.
    fn lex_raw_string_or_ident(&mut self, first: char, start: usize, line: u32, col: u32) -> TokenKind {
        // Count leading `#` characters.
        if self.cursor.peek() != '#' && self.cursor.peek() != '"' {
            return self.lex_ident(first);
        }

        let mut hashes: usize = 0;
        while self.cursor.eat('#') { hashes += 1; }

        if !self.cursor.eat('"') {
            // Not actually a raw string — treat as identifier starting with 'r'.
            return self.lex_ident(first);
        }

        let mut buf = String::new();
        loop {
            if self.cursor.is_eof() {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::UnterminatedRawString { span });
                return TokenKind::RawStrLit(buf);
            }
            let ch = self.cursor.bump();
            if ch == '"' {
                // Check for the right number of closing hashes.
                let mut closing = 0usize;
                let saved_pos = self.cursor.pos();
                let (saved_line, saved_col) = self.cursor.location();
                while self.cursor.eat('#') { closing += 1; }
                if closing == hashes {
                    return TokenKind::RawStrLit(buf);
                }
                // Not enough hashes — put them back into the buffer.
                buf.push('"');
                for _ in 0..closing { buf.push('#'); }
                // The cursor already advanced; that's fine — we just append.
                let _ = (saved_pos, saved_line, saved_col); // suppress unused warnings
            } else {
                buf.push(ch);
            }
        }
    }

    // ── Char literals ──────────────────────────────────────────────────────

    /// Called after consuming the opening `'`.
    fn lex_char(&mut self, start: usize, line: u32, col: u32) -> TokenKind {
        if self.cursor.is_eof() {
            let span = Span::new(start, self.cursor.pos(), line, col);
            self.errors.push(LexError::UnterminatedChar { span });
            return TokenKind::Unknown('\'');
        }

        let ch = self.cursor.bump();
        let value = if ch == '\\' {
            let esc_start = self.cursor.pos() - 1;
            let (el, ec) = self.cursor.location();
            match self.lex_escape(esc_start, el, ec) {
                Ok(c)  => c,
                Err(e) => { self.errors.push(e); '?' }
            }
        } else {
            ch
        };

        // Expect the closing quote.
        if !self.cursor.eat('\'') {
            if !self.cursor.is_eof() && self.cursor.peek() != '\n' {
                // Multi-char literal — consume the rest and report.
                self.cursor.eat_while(|c| c != '\'');
                self.cursor.eat('\'');
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::InvalidCharLiteral {
                    span,
                    reason: "char literal must contain exactly one codepoint",
                });
            } else {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::UnterminatedChar { span });
            }
        }

        TokenKind::CharLit(value)
    }

    // ── Escape sequences ───────────────────────────────────────────────────

    /// Decode a single escape sequence after the `\` has been consumed.
    fn lex_escape(&mut self, start: usize, line: u32, col: u32) -> LexResult<char> {
        let ch = self.cursor.bump();
        match ch {
            'n'  => Ok('\n'),
            'r'  => Ok('\r'),
            't'  => Ok('\t'),
            '0'  => Ok('\0'),
            '\\' => Ok('\\'),
            '\'' => Ok('\''),
            '"'  => Ok('"'),
            // Hex escape: \xNN
            'x'  => {
                let h1 = self.cursor.bump();
                let h2 = self.cursor.bump();
                let s = format!("{h1}{h2}");
                u8::from_str_radix(&s, 16)
                    .map(|b| b as char)
                    .map_err(|_| LexError::InvalidUnicodeEscape {
                        span: Span::new(start, self.cursor.pos(), line, col),
                    })
            }
            // Unicode escape: \u{NNNN}
            'u'  => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                if !self.cursor.eat('{') {
                    return Err(LexError::InvalidUnicodeEscape { span });
                }
                let esc_start = self.cursor.pos();
                self.cursor.eat_while(|c| c.is_ascii_hexdigit());
                let hex = self.cursor.slice_from(esc_start).to_owned();
                if !self.cursor.eat('}') {
                    return Err(LexError::InvalidUnicodeEscape {
                        span: Span::new(start, self.cursor.pos(), line, col),
                    });
                }
                u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or(LexError::InvalidUnicodeEscape { span })
            }
            c    => Err(LexError::UnknownEscape {
                span: Span::new(start, self.cursor.pos(), line, col),
                ch: c,
            }),
        }
    }

    // ── Numeric literals ───────────────────────────────────────────────────

    /// Called after consuming the first digit `first`.
    fn lex_number(&mut self, first: char, start: usize, line: u32, col: u32) -> TokenKind {
        // Detect base prefix: 0x, 0o, 0b
        if first == '0' {
            match self.cursor.peek() {
                'x' | 'X' => { self.cursor.bump(); return self.lex_hex(start, line, col); }
                'o' | 'O' => { self.cursor.bump(); return self.lex_octal(start, line, col); }
                'b' | 'B' => { self.cursor.bump(); return self.lex_binary(start, line, col); }
                _ => {}
            }
        }

        // Decimal integer (may become float).
        let mut s = first.to_string();
        self.eat_digits_into(&mut s, 10);

        let is_float = (self.cursor.peek() == '.' && self.cursor.peek2().is_ascii_digit())
            || self.cursor.peek() == 'e'
            || self.cursor.peek() == 'E';

        if is_float {
            return self.lex_float_tail(s, start, line, col);
        }

        // Optional integer suffix.
        let suffix = self.lex_num_suffix();
        let raw = s.replace('_', "");
        match raw.parse::<u128>() {
            Ok(value) => TokenKind::IntLit { value, base: NumBase::Decimal, suffix },
            Err(_)    => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::InvalidNumericLiteral { span, msg: "integer overflow" });
                TokenKind::IntLit { value: 0, base: NumBase::Decimal, suffix }
            }
        }
    }

    fn lex_hex(&mut self, start: usize, line: u32, col: u32) -> TokenKind {
        let digit_start = self.cursor.pos();
        self.cursor.eat_while(|c| c.is_ascii_hexdigit() || c == '_');
        let raw = self.cursor.slice_from(digit_start).replace('_', "");
        let suffix = self.lex_num_suffix();
        match u128::from_str_radix(&raw, 16) {
            Ok(value) => TokenKind::IntLit { value, base: NumBase::Hex, suffix },
            Err(_) => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::InvalidNumericLiteral { span, msg: "invalid hex literal" });
                TokenKind::IntLit { value: 0, base: NumBase::Hex, suffix }
            }
        }
    }

    fn lex_octal(&mut self, start: usize, line: u32, col: u32) -> TokenKind {
        let digit_start = self.cursor.pos();
        self.cursor.eat_while(|c| matches!(c, '0'..='7') || c == '_');
        let raw = self.cursor.slice_from(digit_start).replace('_', "");
        let suffix = self.lex_num_suffix();
        match u128::from_str_radix(&raw, 8) {
            Ok(value) => TokenKind::IntLit { value, base: NumBase::Octal, suffix },
            Err(_) => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::InvalidNumericLiteral { span, msg: "invalid octal literal" });
                TokenKind::IntLit { value: 0, base: NumBase::Octal, suffix }
            }
        }
    }

    fn lex_binary(&mut self, start: usize, line: u32, col: u32) -> TokenKind {
        let digit_start = self.cursor.pos();
        self.cursor.eat_while(|c| c == '0' || c == '1' || c == '_');
        let raw = self.cursor.slice_from(digit_start).replace('_', "");
        let suffix = self.lex_num_suffix();
        match u128::from_str_radix(&raw, 2) {
            Ok(value) => TokenKind::IntLit { value, base: NumBase::Binary, suffix },
            Err(_) => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::InvalidNumericLiteral { span, msg: "invalid binary literal" });
                TokenKind::IntLit { value: 0, base: NumBase::Binary, suffix }
            }
        }
    }

    /// Continue lexing a float after the integer part is in `int_part`.
    fn lex_float_tail(&mut self, mut s: String, start: usize, line: u32, col: u32) -> TokenKind {
        // Decimal fraction.
        if self.cursor.peek() == '.' && self.cursor.peek2().is_ascii_digit() {
            s.push(self.cursor.bump()); // '.'
            self.eat_digits_into(&mut s, 10);
        }
        // Exponent.
        if self.cursor.peek() == 'e' || self.cursor.peek() == 'E' {
            s.push(self.cursor.bump());
            if self.cursor.peek() == '+' || self.cursor.peek() == '-' {
                s.push(self.cursor.bump());
            }
            self.eat_digits_into(&mut s, 10);
        }
        let suffix = self.lex_num_suffix();
        let raw = s.replace('_', "");
        match raw.parse::<f64>() {
            Ok(value) => TokenKind::FloatLit { value, suffix },
            Err(_) => {
                let span = Span::new(start, self.cursor.pos(), line, col);
                self.errors.push(LexError::InvalidNumericLiteral { span, msg: "invalid float literal" });
                TokenKind::FloatLit { value: 0.0, suffix }
            }
        }
    }

    /// Consume digit chars (and underscores as separators) and append them.
    fn eat_digits_into(&mut self, buf: &mut String, _base: u32) {
        while !self.cursor.is_eof() {
            let c = self.cursor.peek();
            if c.is_ascii_digit() || c == '_' {
                buf.push(self.cursor.bump());
            } else {
                break;
            }
        }
    }

    /// Try to consume an explicit type suffix like `u32`, `f64`, `usize`.
    fn lex_num_suffix(&mut self) -> Option<NumSuffix> {
        // Only attempt if the next char looks like the start of a suffix.
        let p = self.cursor.peek();
        if p != 'i' && p != 'u' && p != 'f' { return None; }

        let saved_pos = self.cursor.pos();
        // We need to snapshot position to backtrack if it isn't a real suffix.
        // Because `Cursor` has no backtrack, we peek at the suffix characters
        // manually without advancing first.
        let rest = &self.cursor.src[self.cursor.pos()..];
        for &candidate in &["usize", "isize", "u128", "i128", "u64", "i64",
                             "u32", "i32", "u16", "i16", "u8", "i8", "f64", "f32"] {
            if rest.starts_with(candidate) {
                // Make sure the suffix isn't followed by an ident char.
                let after = rest[candidate.len()..].chars().next().unwrap_or('\0');
                if is_ident_continue(after) { continue; }
                // Advance the cursor past the suffix.
                for _ in 0..candidate.len() { self.cursor.bump(); }
                return NumSuffix::from_str(candidate);
            }
        }
        let _ = saved_pos; // no backtrack needed, we didn't move
        None
    }

    // ── Identifiers & keywords ─────────────────────────────────────────────

    /// Called after consuming the first character `first` of an identifier.
    fn lex_ident(&mut self, first: char) -> TokenKind {
        let mut buf = first.to_string();
        while !self.cursor.is_eof() && is_ident_continue(self.cursor.peek()) {
            buf.push(self.cursor.bump());
        }

        // Check keyword table.
        if let Some(kw) = lookup(&buf) {
            return TokenKind::Kw(kw);
        }

        // Plain identifier.
        TokenKind::Ident(buf)
    }
}

// ─── Character classification helpers ────────────────────────────────────────

/// Valid first character of an identifier: ASCII letter or underscore,
/// or any non-ASCII Unicode letter/number (XID_Start-ish).
#[inline]
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Valid continuation character of an identifier.
#[inline]
fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// ─── Iterator impl ───────────────────────────────────────────────────────────

impl<'src> Iterator for Lexer<'src> {
    type Item = Token;

    /// Stops yielding tokens after `Eof` (first call returns `Eof`, subsequent
    /// calls return `None`).
    fn next(&mut self) -> Option<Self::Item> {
        let tok = self.next_token();
        if tok.kind == TokenKind::Eof { None } else { Some(tok) }
    }
}