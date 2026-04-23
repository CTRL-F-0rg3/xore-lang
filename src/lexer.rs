use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, PartialEq, Clone)]

pub enum Token {
    I32, I64, I128, I256,
    STRING, S_STRING, BOOL,


    PUBLIC, PRIVATE, 
    FN, ENUM, STRUCT,
    IF, ELSE, 
    SWITCH, CASE, DEFAULT, 
    FOR, WHILE, 
    RETURN, LET, MUT,

    ADD,      // +
    MINUS,    // -
    STAR,     // *
    SLASH,    // /
    PERCENT,  // %

    ASSIGN,   // =
    EQ,       // ==
    NOT_EQ,   // !=
    LT,       // <
    GT,       // >
    LTE,      // <=
    GTE,      // >=
    AND,      // &&
    OR,       // ||
    NOT,      // !

    L_PAREN,    // (
    R_PAREN,    // )
    L_BRACE,    // {
    R_BRACE,    // }
    L_BRACKET,  // [
    R_BRACKET,  // ]
    COMMA,      // ,
    DOT,        // .
    COLON,      // :
    SEMICOLON,  // ;
    ARROW,      // ->

    
    Identifier(String),
    IntLiteral(i128),
    StrLiteral(String),
    BoolLiteral(bool),

    EOF,
}



pub struct Lexer<'a> {
    input: Peekable<Chars<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { input: input.chars().peekable() }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();

        match self.input.next() {
            Some('+') => Token::ADD,
            Some('*') => Token::STAR,
            Some('/') => Token::SLASH,
            Some('%') => Token::PERCENT,
            Some(',') => Token::COMMA,
            Some('.') => Token::DOT,
            Some(':') => Token::COLON,
            Some(';') => Token::SEMICOLON,
            Some('(') => Token::L_PAREN,
            Some(')') => Token::R_PAREN,
            Some('{') => Token::L_BRACE,
            Some('}') => Token::R_BRACE,
            Some('[') => Token::L_BRACKET,
            Some(']') => Token::R_BRACKET,

            Some('-') => {
                if self.peek_is('>') { Token::ARROW } 
                else { Token::MINUS }
            }
            Some('=') => {
                if self.peek_is('=') { Token::EQ } 
                else { Token::ASSIGN }
            }
            Some('!') => {
                if self.peek_is('=') { Token::NOT_EQ } 
                else { Token::NOT }
            }
            Some('<') => {
                if self.peek_is('=') { Token::LTE } 
                else { Token::LT }
            }
            Some('>') => {
                if self.peek_is('=') { Token::GTE } 
                else { Token::GT }
            }
            Some('&') => {
                if self.peek_is('&') { Token::AND } 
                else { panic!("Expected &&") }
            }
            Some('|') => {
                if self.peek_is('|') { Token::OR } 
                else { panic!("Expected ||") }
            }

            Some('"') => self.lex_string(),

            Some(c) if c.is_alphabetic() || c == '_' => self.lex_identifier(c),
            Some(c) if c.is_numeric() => self.lex_number(c),

            None => Token::EOF,
            _ => panic!("Unexpected character"),
        }
    }

    fn lex_identifier(&mut self, first: char) -> Token {
        let mut id = first.to_string();
        while let Some(&c) = self.input.peek() {
            if c.is_alphanumeric() || c == '_' {
                id.push(self.input.next().unwrap());
            } else { break; }
        }

        match id.as_str() {
            "i32" => Token::I32,
            "i64" => Token::I64,
            "i128" => Token::I128,
            "i256" => Token::I256,
            "String" => Token::STRING,
            "str" => Token::S_STRING,
            "bool" => Token::BOOL,
            "public" => Token::PUBLIC,
            "private" => Token::PRIVATE,
            "fn" => Token::FN,
            "enum" => Token::ENUM,
            "struct" => Token::STRUCT,
            "if" => Token::IF,
            "else" => Token::ELSE,
            "switch" => Token::SWITCH,
            "case" => Token::CASE,
            "default" => Token::DEFAULT,
            "for" => Token::FOR,
            "while" => Token::WHILE,
            "return" => Token::RETURN,
            "let" => Token::LET,
            "mut" => Token::MUT,
            "true" => Token::BoolLiteral(true),
            "false" => Token::BoolLiteral(false),
            _ => Token::Identifier(id),
        }
    }

    fn lex_number(&mut self, first: char) -> Token {
        let mut num = first.to_string();
        while let Some(&c) = self.input.peek() {
            if c.is_numeric() { num.push(self.input.next().unwrap()); } 
            else { break; }
        }
        Token::IntLiteral(num.parse().unwrap())
    }

    fn lex_string(&mut self) -> Token {
        let mut s = String::new();
        while let Some(c) = self.input.next() {
            if c == '"' { break; }
            s.push(c);
        }
        Token::StrLiteral(s)
    }

    fn skip_whitespace(&mut self) {
        while let Some(&c) = self.input.peek() {
            if c.is_whitespace() { self.input.next(); } 
            else { break; }
        }
    }

    fn peek_is(&mut self, expected: char) -> bool {
        if self.input.peek() == Some(&expected) {
            self.input.next();
            true
        } else {
            false
        }
    }
}