pub mod error;
pub mod keywords;
pub mod lexer;
pub mod token;

pub use error::LexError;
pub use lexer::Lexer;
pub use token::{
    Delim, Keyword, KeywordGroup, NumBase, NumSuffix, Op, Punct, Span, Token, TokenKind,
};

/// Convenience: lex an entire source string and return `(tokens, errors)`.
///
/// ```rust
/// use xore_lexer::lex;
///
/// let (tokens, errors) = lex("fn main() void { return; }");
/// assert!(errors.is_empty());
/// ```
pub fn lex(src: &str) -> (Vec<Token>, Vec<LexError>) {
    Lexer::new(src).tokenize()
}
