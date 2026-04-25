pub mod ast;
pub mod codegen;
pub mod error;
pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod project;
pub mod token;

pub use error::LexError;
pub use lexer::Lexer;
pub use parser::{ParseError, Parser};
pub use token::{
    Delim, Keyword, KeywordGroup, NumBase, NumSuffix, Op, Punct, Span, Token, TokenKind,
};

/// Lex an entire source string → (tokens, errors).
pub fn lex(src: &str) -> (Vec<Token>, Vec<LexError>) {
    Lexer::new(src).tokenize()
}

/// Parse a source string → (program, lex_errors, parse_errors).
pub fn parse(src: &str) -> (ast::Program, Vec<LexError>, Vec<ParseError>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program();
    (prog, p.lex_errors, p.errors)
}