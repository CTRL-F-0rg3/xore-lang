use std::iter::Peekable;
use std::str::Chars;

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
    I32, I64, I128, I256,
    String, SString, Bool, // Poprawione na CamelCase żeby nie było warningów
    Public, Private, 
    Fn, Enum, Struct,
    If, Else, Switch, Case, Default, 
    For, While, Return, Let, Mut,

    Add, Minus, Star, Slash, Percent,
    Assign, Eq, NotEq, Lt, Gt, Lte, Gte,
    And, Or, Not,

    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Comma, Dot, Colon, Semicolon, Arrow,

    Identifier(String),
    IntLiteral(i128),
    FloatLiteral(f64),
    StrLiteral(String),
    BoolLiteral(bool),
    Error(LexerError), // To było brakujące!
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
            '+' => Token::Add,
            '*' => Token::Star,
            '/' => Token::Slash,
            '%' => Token::Percent,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            ',' => Token::Comma,
            '.' => Token::Dot,
            ':' => Token::Colon,
            ';' => Token::Semicolon,

            '-' => if self.consume_if('>') { Token::Arrow } else { Token::Minus },
            '=' => if self.consume_if('=') { Token::Eq } else { Token::Assign },
            '!' => if self.consume_if('=') { Token::NotEq } else { Token::Not },
            '<' => if self.consume_if('=') { Token::Lte } else { Token::Lt },
            '>' => if self.consume_if('=') { Token::Gte } else { Token::Gt },
            '&' => if self.consume_if('&') { Token::And } else { Token::Error(LexerError::UnexpectedChar('&', pos)) },
            '|' => if self.consume_if('|') { Token::Or } else { Token::Error(LexerError::UnexpectedChar('|', pos)) },

            '"' => self.lex_string(pos),

            c if c.is_ascii_alphabetic() || c == '_' => self.lex_identifier(c),
            c if c.is_ascii_digit() => self.lex_number(c, pos),

            _ => Token::Error(LexerError::UnexpectedChar(ch, pos)),
        }
    }

    fn lex_identifier(&mut self, first: char) -> Token {
        let mut id = first.to_string();
        while let Some(&c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                id.push(self.next_char().unwrap());
            } else { break; }
        }

        match id.as_str() {
            "i32" => Token::I32,
            "i64" => Token::I64,
            "i128" => Token::I128,
            "i256" => Token::I256,
            "String" => Token::String,
            "str" => Token::SString,
            "bool" => Token::Bool,
            "public" => Token::Public,
            "private" => Token::Private,
            "fn" => Token::Fn,
            "enum" => Token::Enum,
            "struct" => Token::Struct,
            "if" => Token::If,
            "else" => Token::Else,
            "switch" => Token::Switch,
            "case" => Token::Case,
            "default" => Token::Default,
            "for" => Token::For,
            "while" => Token::While,
            "return" => Token::Return,
            "let" => Token::Let,
            "mut" => Token::Mut,
            "true" => Token::BoolLiteral(true),
            "false" => Token::BoolLiteral(false),
            _ => Token::Identifier(id),
        }
    }

    fn lex_number(&mut self, first: char, pos: Pos) -> Token {
        let mut s = first.to_string();
        let mut is_float = false;
        while let Some(&c) = self.peek_char() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' { s.push(self.next_char().unwrap()); } else { self.next_char(); }
            } else if c == '.' && !is_float {
                is_float = true;
                s.push(self.next_char().unwrap());
            } else { break; }
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