use crate::token::Span;
use std::fmt;

/// All errors the lexer can emit.  Each variant carries enough context to
/// produce a helpful diagnostic without requiring access to the source buffer.
#[derive(Debug, Clone, PartialEq)]
pub enum LexError {
    /// A string or char literal was opened but never closed before EOF / newline.
    UnterminatedString { span: Span },
    UnterminatedChar { span: Span },
    UnterminatedBlockComment { span: Span },
    UnterminatedRawString { span: Span },

    /// A char literal contained more than one character (or zero).
    InvalidCharLiteral { span: Span, reason: &'static str },

    /// An unknown escape sequence inside a string or char, e.g. `\q`.
    UnknownEscape { span: Span, ch: char },

    /// A `\uXXXX` / `\u{…}` escape contained an invalid Unicode codepoint.
    InvalidUnicodeEscape { span: Span },

    /// Numeric literal with an unexpected character, e.g. `0x_` or `0b2`.
    InvalidNumericLiteral { span: Span, msg: &'static str },

    /// A byte outside the expected encoding (non-ASCII where ASCII is required).
    UnexpectedByte { span: Span, byte: u8 },

    /// A character the lexer does not recognise at all.
    UnknownChar { span: Span, ch: char },
}

impl LexError {
    pub fn span(&self) -> Span {
        match self {
            Self::UnterminatedString       { span, .. } => *span,
            Self::UnterminatedChar         { span, .. } => *span,
            Self::UnterminatedBlockComment { span, .. } => *span,
            Self::UnterminatedRawString    { span, .. } => *span,
            Self::InvalidCharLiteral       { span, .. } => *span,
            Self::UnknownEscape            { span, .. } => *span,
            Self::InvalidUnicodeEscape     { span, .. } => *span,
            Self::InvalidNumericLiteral    { span, .. } => *span,
            Self::UnexpectedByte           { span, .. } => *span,
            Self::UnknownChar              { span, .. } => *span,
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.span();
        let loc = format!("{}:{}", s.line, s.col);
        match self {
            Self::UnterminatedString { .. } =>
                write!(f, "[{loc}] unterminated string literal"),
            Self::UnterminatedChar { .. } =>
                write!(f, "[{loc}] unterminated char literal"),
            Self::UnterminatedBlockComment { .. } =>
                write!(f, "[{loc}] unterminated block comment `/* … */`"),
            Self::UnterminatedRawString { .. } =>
                write!(f, "[{loc}] unterminated raw string literal"),
            Self::InvalidCharLiteral { reason, .. } =>
                write!(f, "[{loc}] invalid char literal: {reason}"),
            Self::UnknownEscape { ch, .. } =>
                write!(f, "[{loc}] unknown escape sequence `\\{ch}`"),
            Self::InvalidUnicodeEscape { .. } =>
                write!(f, "[{loc}] invalid unicode escape sequence"),
            Self::InvalidNumericLiteral { msg, .. } =>
                write!(f, "[{loc}] invalid numeric literal: {msg}"),
            Self::UnexpectedByte { byte, .. } =>
                write!(f, "[{loc}] unexpected byte 0x{byte:02X}"),
            Self::UnknownChar { ch, .. } =>
                write!(f, "[{loc}] unknown character `{ch}`"),
        }
    }
}

impl std::error::Error for LexError {}

/// A non-fatal diagnostic — emitted alongside a best-effort recovery token.
pub type LexResult<T> = Result<T, LexError>;