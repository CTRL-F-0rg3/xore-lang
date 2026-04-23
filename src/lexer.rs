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



#[derive(Debug, PartialEq, Clone)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, PartialEq, Clone)]
pub enum LexerError {
    UnexpectedChar(char, Pos),
    UnterminatedString(Pos),
    InvalidNumber(Pos),
}

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    // ... Twoje poprzednie tokeny ...
    Error(LexerError),
    Identifier(String),
    IntLiteral(i128),
    FloatLiteral(f64),
    StrLiteral(String),
    EOF,
}

pub struct Lexer<'a> {
    input: Peekable<Chars<'a>>,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.chars().peekable(),
            line: 1,
            col: 1,
        }
    }

    fn next_char(&mut self) -> Option<char> {
        let ch = self.input.next()?;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn peek_char(&mut self) -> Option<&char> {
        self.input.peek()
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();

        let pos = Pos { line: self.line, col: self.col };
        
        let ch = match self.next_char() {
            Some(c) => c,
            None => return Token::EOF,
        };

        match ch {
            '+' => Token::ADD,
            '*' => Token::STAR,
            '/' => Token::SLASH,
            '%' => Token::PERCENT,
            '(' => Token::L_PAREN,
            ')' => Token::R_PAREN,
            '{' => Token::L_BRACE,
            '}' => Token::R_BRACE,
            ',' => Token::COMMA,
            ';' => Token::SEMICOLON,

            '-' => if self.consume_if('>') { Token::ARROW } else { Token::MINUS },
            '=' => if self.consume_if('=') { Token::EQ } else { Token::ASSIGN },
            '!' => if self.consume_if('=') { Token::NOT_EQ } else { Token::NOT },
            '<' => if self.consume_if('=') { Token::LTE } else { Token::LT },
            '>' => if self.consume_if('=') { Token::GTE } else { Token::GT },
            '&' => if self.consume_if('&') { Token::AND } else { Token::Error(LexerError::UnexpectedChar('&', pos)) },
            '|' => if self.consume_if('|') { Token::OR } else { Token::Error(LexerError::UnexpectedChar('|', pos)) },

            '"' => self.lex_string(pos),

            c if c.is_ascii_alphabetic() || c == '_' => self.lex_identifier(c),
            c if c.is_ascii_digit() => self.lex_number(c, pos),

            _ => Token::Error(LexerError::UnexpectedChar(ch, pos)),
        }
    }

    fn lex_number(&mut self, first: char, pos: Pos) -> Token {
        let mut s = first.to_string();
        let mut is_float = false;

        while let Some(&c) = self.peek_char() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' { s.push(c); }
                self.next_char();
            } else if c == '.' && !is_float {
                is_float = true;
                s.push(self.next_char().unwrap());
            } else {
                break;
            }
        }

        if is_float {
            s.parse::<f64>().map(Token::FloatLiteral).unwrap_or(Token::Error(LexerError::InvalidNumber(pos)))
        } else {
            s.parse::<i128>().map(Token::IntLiteral).unwrap_or(Token::Error(LexerError::InvalidNumber(pos)))
        }
    }

    fn lex_string(&mut self, start_pos: Pos) -> Token {
        let mut s = String::new();
        while let Some(ch) = self.next_char() {
            match ch {
                '"' => return Token::StrLiteral(s),
                '\\' => match self.next_char() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(c) => s.push(c),
                    None => return Token::Error(LexerError::UnterminatedString(start_pos)),
                },
                _ => s.push(ch),
            }
        }
        Token::Error(LexerError::UnterminatedString(start_pos))
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(&c) = self.peek_char() {
            if c.is_whitespace() {
                self.next_char();
            } else if c == '/' {
                // Podglądamy czy to komentarz bez zjadania '/'
                if self.nth_char_is(1, '/') {
                    while let Some(nc) = self.next_char() {
                        if nc == '\n' { break; }
                    }
                } else { break; }
            } else { break; }
        }
    }

    fn consume_if(&mut self, expected: char) -> bool {
        if self.peek_char() == Some(&expected) {
            self.next_char();
            true
        } else {
            false
        }
    }

    fn nth_char_is(&self, n: usize, expected: char) -> bool {
        self.input.clone().nth(n) == Some(expected)
    }
}