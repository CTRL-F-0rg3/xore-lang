pub mod ast;
pub mod codegen;
pub mod error;
pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod token;

pub use error::LexError;
pub use lexer::Lexer;
pub use parser::{ParseError, Parser};
pub use token::{
    Delim, Keyword, KeywordGroup, NumBase, NumSuffix, Op, Punct, Span, Token, TokenKind,
};

/// Convenience: lex an entire source string and return `(tokens, errors)`.
pub fn lex(src: &str) -> (Vec<Token>, Vec<LexError>) {
    Lexer::new(src).tokenize()
}

/// Convenience: parse a source string and return `(program, lex_errors, parse_errors)`.
pub fn parse(src: &str) -> (ast::Program, Vec<LexError>, Vec<ParseError>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program();
    (prog, p.lex_errors, p.errors)
}