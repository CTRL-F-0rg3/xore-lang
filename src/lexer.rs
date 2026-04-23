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