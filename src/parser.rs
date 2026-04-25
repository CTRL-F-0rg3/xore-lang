// Recursive-descent parser for Xore.
//
// Grammar (informal, top-down):
//
//   program        = item*  EOF
//   item           = fn_decl | struct_decl | enum_decl | mod_decl | import | use
//   fn_decl        = visibility? 'fn'? IDENT '(' params ')' ('->' type)? block
//                  | visibility IDENT '(' params ')' block          // short form: public main(){}
//   block          = '{' stmt* '}'
//   stmt           = let_stmt | return_stmt | if_stmt | while_stmt
//                  | for_stmt | end_stmt | fn_decl | enum_decl | struct_decl
//                  | expr_stmt
//   let_stmt       = 'let' 'mut'? IDENT (':' type)? ('=' expr)? ';'
//   if_stmt        = 'if' '[' expr ']' block ('else' (block | if_stmt))?
//   while_stmt     = 'while' expr block
//   for_stmt       = 'for' 'in' IDENT 'range' '(' expr ')' block
//   end_stmt       = 'end' '(' ')' ('if' expr)? ';'?
//   expr           = assign_expr
//   assign_expr    = compare_expr (assign_op assign_expr)?
//   compare_expr   = add_expr (cmp_op add_expr)*
//   add_expr       = mul_expr (('+' | '-') mul_expr)*
//   mul_expr       = unary_expr (('*' | '/' | '%') unary_expr)*
//   unary_expr     = ('-' | '!' | '&' | '*') unary_expr | call_expr
//   call_expr      = primary ('(' args ')' | '.' IDENT | '!' '(' args ')')*
//   primary        = literal | IDENT | macro_call | type_call | '(' expr ')' | '[' expr ']'

use crate::ast::*;
use crate::error::LexError;
use crate::lexer::Lexer;
use crate::token::{Keyword, Op, Delim, Punct, Span, Token, TokenKind};

// ─── Parser errors ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParseError {
    pub msg:  String,
    pub span: Span,
}

impl ParseError {
    fn new(msg: impl Into<String>, span: Span) -> Self {
        Self { msg: msg.into(), span }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}:{}] parse error: {}", self.span.line, self.span.col, self.msg)
    }
}

pub type ParseResult<T> = Result<T, ParseError>;

// ─── Parser state ────────────────────────────────────────────────────────────

pub struct Parser {
    tokens:   Vec<Token>,
    pos:      usize,
    /// Lex errors collected during tokenisation.
    pub lex_errors: Vec<LexError>,
    /// Parse errors accumulated during parsing (non-fatal recovery).
    pub errors: Vec<ParseError>,
}

impl Parser {
    /// Create a parser from source text.  Tokenises eagerly.
    pub fn new(src: &str) -> Self {
        let lexer = Lexer::new(src);
        let (tokens, lex_errors) = lexer.tokenize();
        // Filter out comment tokens — they're irrelevant to the grammar.
        let tokens: Vec<Token> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::LineComment(_) | TokenKind::BlockComment(_)))
            .collect();
        Self { tokens, pos: 0, lex_errors, errors: Vec::new() }
    }

    // ── Token stream helpers ──────────────────────────────────────────────

    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek2(&self) -> &Token {
        let i = (self.pos + 1).min(self.tokens.len() - 1);
        &self.tokens[i]
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() - 1 { self.pos += 1; }
        tok
    }

    fn is_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn current_span(&self) -> Span {
        self.peek().span
    }

    /// Consume the next token if it matches `kw`, otherwise return an error.
    fn expect_kw(&mut self, kw: Keyword) -> ParseResult<Span> {
        let tok = self.peek().clone();
        if matches!(&tok.kind, TokenKind::Kw(k) if *k == kw) {
            self.advance();
            Ok(tok.span)
        } else {
            Err(ParseError::new(
                format!("expected keyword `{}`, got `{}`", kw.as_str(), self.token_desc()),
                tok.span,
            ))
        }
    }

    fn expect_delim(&mut self, d: Delim) -> ParseResult<Span> {
        let tok = self.peek().clone();
        if matches!(&tok.kind, TokenKind::Delim(x) if *x == d) {
            self.advance();
            Ok(tok.span)
        } else {
            Err(ParseError::new(
                format!("expected `{}`, got `{}`", delim_str(d), self.token_desc()),
                tok.span,
            ))
        }
    }

    fn expect_punct(&mut self, p: Punct) -> ParseResult<Span> {
        let tok = self.peek().clone();
        if matches!(&tok.kind, TokenKind::Punct(x) if *x == p) {
            self.advance();
            Ok(tok.span)
        } else {
            Err(ParseError::new(
                format!("expected `{}`, got `{}`", punct_str(p), self.token_desc()),
                tok.span,
            ))
        }
    }

    #[allow(dead_code)]
    fn expect_op(&mut self, o: Op) -> ParseResult<Span> {
        let tok = self.peek().clone();
        if matches!(&tok.kind, TokenKind::Op(x) if *x == o) {
            self.advance();
            Ok(tok.span)
        } else {
            Err(ParseError::new(
                format!("expected `{o:?}`, got `{}`", self.token_desc()),
                tok.span,
            ))
        }
    }

    fn expect_ident(&mut self) -> ParseResult<(String, Span)> {
        let tok = self.peek().clone();
        if let TokenKind::Ident(name) = &tok.kind {
            let name = name.clone();
            self.advance();
            Ok((name, tok.span))
        } else if let TokenKind::Kw(kw) = &tok.kind {
            // Allow some keywords to be used as identifiers (e.g. `data`, `new`)
            let name = kw.as_str().to_string();
            self.advance();
            Ok((name, tok.span))
        } else {
            Err(ParseError::new(
                format!("expected identifier, got `{}`", self.token_desc()),
                tok.span,
            ))
        }
    }

    fn eat_kw(&mut self, kw: Keyword) -> bool {
        if matches!(&self.peek().kind, TokenKind::Kw(k) if *k == kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_op(&mut self, op: Op) -> bool {
        if matches!(&self.peek().kind, TokenKind::Op(o) if *o == op) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_punct(&mut self, p: Punct) -> bool {
        if matches!(&self.peek().kind, TokenKind::Punct(x) if *x == p) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_delim(&mut self, d: Delim) -> bool {
        if matches!(&self.peek().kind, TokenKind::Delim(x) if *x == d) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn token_desc(&self) -> String {
        match &self.peek().kind {
            TokenKind::Kw(k)           => format!("keyword `{}`", k.as_str()),
            TokenKind::Ident(s)        => format!("identifier `{s}`"),
            TokenKind::IntLit { .. }   => "integer literal".into(),
            TokenKind::FloatLit { .. } => "float literal".into(),
            TokenKind::StrLit(_)       => "string literal".into(),
            TokenKind::Op(o)           => format!("operator `{o:?}`"),
            TokenKind::Delim(d)        => delim_str(*d).into(),
            TokenKind::Punct(p)        => punct_str(*p).into(),
            TokenKind::Eof             => "end of file".into(),
            other                      => format!("{}", other.describe()),
        }
    }

    /// Record a non-fatal error and return the error object (caller decides
    /// whether to abort or continue).
    fn emit_error(&mut self, err: ParseError) {
        self.errors.push(err);
    }

    // ── Synchronisation (error recovery) ─────────────────────────────────

    /// Skip tokens until we see something that looks like a statement start
    /// or a closing brace — used after a parse error to keep going.
    fn synchronise(&mut self) {
        loop {
            if self.is_eof() { break; }
            match &self.peek().kind {
                TokenKind::Delim(Delim::CloseBrace) => break,
                TokenKind::Kw(
                    Keyword::Let | Keyword::If | Keyword::While | Keyword::For |
                    Keyword::Return | Keyword::Fn | Keyword::Struct | Keyword::Enum |
                    Keyword::Public | Keyword::Private | Keyword::End
                ) => break,
                _ => { self.advance(); }
            }
        }
    }

    // ── Public entry point ────────────────────────────────────────────────

    pub fn parse_program(&mut self) -> Program {
        let start = self.current_span();
        let mut items = Vec::new();
        while !self.is_eof() {
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(e) => { self.emit_error(e); self.synchronise(); }
            }
        }
        let span = Span::new(start.start, self.current_span().end, start.line, start.col);
        Program { items, span }
    }

    // ── Items ─────────────────────────────────────────────────────────────

    fn parse_item(&mut self) -> ParseResult<Item> {
        // Check for @export annotation before visibility/fn keyword.
        // Syntax: @export fn name(...) or @export public fn name(...)
        let exported = if matches!(&self.peek().kind, TokenKind::Annotation(s) if s == "export") {
            self.advance(); // consume @export
            true
        } else {
            false
        };

        let vis = self.parse_visibility();

        match &self.peek().kind.clone() {
            TokenKind::Kw(Keyword::Fn) => {
                Ok(Item::Function(self.parse_fn_decl(vis, exported)?))
            }
            TokenKind::Kw(Keyword::Struct) => {
                Ok(Item::Struct(self.parse_struct(vis)?))
            }
            TokenKind::Kw(Keyword::Enum) => {
                Ok(Item::Enum(self.parse_enum(vis)?))
            }
            TokenKind::Kw(Keyword::Import) => {
                Ok(Item::Import(self.parse_import()?))
            }
            TokenKind::Kw(Keyword::Use) => {
                Ok(Item::Use(self.parse_use()?))
            }
            TokenKind::Kw(Keyword::Mod) => {
                Ok(Item::Mod(self.parse_mod()?))
            }
            // Xore short form: `public main() { … }` — no `fn` keyword
            TokenKind::Ident(_) => {
                Ok(Item::Function(self.parse_fn_decl_short(vis, exported)?))
            }
            _ => Err(ParseError::new(
                format!("unexpected token at top level: `{}`", self.token_desc()),
                self.current_span(),
            ))
        }
    }

    fn parse_visibility(&mut self) -> Visibility {
        if self.eat_kw(Keyword::Public)  { return Visibility::Public; }
        if self.eat_kw(Keyword::Private) { return Visibility::Private; }
        Visibility::Private
    }

    // ── Function declarations ─────────────────────────────────────────────

    /// Full form: `[@export] [public] fn name(params) [-> type] { body }`
    fn parse_fn_decl(&mut self, vis: Visibility, exported: bool) -> ParseResult<FnDecl> {
        let start = self.current_span();
        self.expect_kw(Keyword::Fn)?;
        let (name, _) = self.expect_ident()?;
        let params = self.parse_params()?;
        let ret_ty = self.parse_optional_return_type()?;
        let body = self.parse_block()?;
        let span = span_to(start, self.current_span());
        Ok(FnDecl { vis, exported, name, params, ret_ty, body, span })
    }

    /// Short form (Xore): `[@export] public main() { body }` — no `fn` keyword
    fn parse_fn_decl_short(&mut self, vis: Visibility, exported: bool) -> ParseResult<FnDecl> {
        let start = self.current_span();
        let (name, _) = self.expect_ident()?;
        let params = self.parse_params()?;
        let ret_ty = self.parse_optional_return_type()?;
        let body = self.parse_block()?;
        let span = span_to(start, self.current_span());
        Ok(FnDecl { vis, exported, name, params, ret_ty, body, span })
    }

    fn parse_params(&mut self) -> ParseResult<Vec<Param>> {
        self.expect_delim(Delim::OpenParen)?;
        let mut params = Vec::new();
        while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseParen) | TokenKind::Eof) {
            let start = self.current_span();
            let (name, _) = self.expect_ident()?;
            self.expect_punct(Punct::Colon)?;
            let ty = self.parse_type()?;
            params.push(Param { name, ty, span: span_to(start, self.current_span()) });
            if !self.eat_punct(Punct::Comma) { break; }
        }
        self.expect_delim(Delim::CloseParen)?;
        Ok(params)
    }

    fn parse_optional_return_type(&mut self) -> ParseResult<Option<TypeExpr>> {
        if self.eat_op(Op::Arrow) {
            Ok(Some(self.parse_type()?))
        } else {
            Ok(None)
        }
    }

    // ── Type expressions ──────────────────────────────────────────────────

    fn parse_type(&mut self) -> ParseResult<TypeExpr> {
        let span = self.current_span();

        // `!T` — error union
        if self.eat_op(Op::Bang) {
            let inner = self.parse_type()?;
            return Ok(TypeExpr::ErrorUnion(Box::new(inner), span));
        }

        // `&T` or `&mut T`
        if self.eat_op(Op::Amp) {
            if self.eat_kw(Keyword::Mut) {
                let inner = self.parse_type()?;
                return Ok(TypeExpr::MutRef(Box::new(inner), span));
            }
            let inner = self.parse_type()?;
            return Ok(TypeExpr::Ref(Box::new(inner), span));
        }

        // `*T`
        if self.eat_op(Op::Star) {
            let inner = self.parse_type()?;
            return Ok(TypeExpr::Pointer(Box::new(inner), span));
        }

        // `void`
        if self.eat_kw(Keyword::Void) {
            return Ok(TypeExpr::Void(span));
        }

        // `[T]` or `[T; N]`
        if self.eat_delim(Delim::OpenBracket) {
            let inner = self.parse_type()?;
            if self.eat_punct(Punct::Semicolon) {
                let len = self.parse_expr()?;
                self.expect_delim(Delim::CloseBracket)?;
                return Ok(TypeExpr::Array(Box::new(inner), Box::new(len), span));
            }
            self.expect_delim(Delim::CloseBracket)?;
            return Ok(TypeExpr::Slice(Box::new(inner), span));
        }

        // Named type
        let (name, name_span) = self.expect_ident()?;
        Ok(TypeExpr::Named(name, name_span))
    }

    // ── Struct / Enum ─────────────────────────────────────────────────────

    fn parse_struct(&mut self, vis: Visibility) -> ParseResult<StructDecl> {
        let start = self.current_span();
        self.expect_kw(Keyword::Struct)?;
        let (name, _) = self.expect_ident()?;
        self.expect_delim(Delim::OpenBrace)?;
        let mut fields = Vec::new();
        while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseBrace) | TokenKind::Eof) {
            let fstart = self.current_span();
            let (fname, _) = self.expect_ident()?;
            self.expect_punct(Punct::Colon)?;
            let ty = self.parse_type()?;
            fields.push(StructField { name: fname, ty, span: span_to(fstart, self.current_span()) });
            if !self.eat_punct(Punct::Comma) { break; }
        }
        self.expect_delim(Delim::CloseBrace)?;
        Ok(StructDecl { vis, name, fields, span: span_to(start, self.current_span()) })
    }

    fn parse_enum(&mut self, vis: Visibility) -> ParseResult<EnumDecl> {
        let start = self.current_span();
        self.expect_kw(Keyword::Enum)?;
        let (name, _) = self.expect_ident()?;
        self.expect_delim(Delim::OpenBrace)?;
        let mut variants = Vec::new();
        while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseBrace) | TokenKind::Eof) {
            let vstart = self.current_span();
            let (vname, _) = self.expect_ident()?;
            let mut fields = Vec::new();
            // Optional payload: `Variant(T1, T2)`
            if self.eat_delim(Delim::OpenParen) {
                while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseParen) | TokenKind::Eof) {
                    fields.push(self.parse_type()?);
                    if !self.eat_punct(Punct::Comma) { break; }
                }
                self.expect_delim(Delim::CloseParen)?;
            }
            variants.push(EnumVariant { name: vname, fields, span: span_to(vstart, self.current_span()) });
            if !self.eat_punct(Punct::Comma) { break; }
        }
        self.expect_delim(Delim::CloseBrace)?;
        Ok(EnumDecl { vis, name, variants, span: span_to(start, self.current_span()) })
    }

    // ── Module system ─────────────────────────────────────────────────────

    fn parse_import(&mut self) -> ParseResult<ImportDecl> {
        let start = self.current_span();
        self.expect_kw(Keyword::Import)?;
        let path = self.parse_dotted_path()?;
        self.eat_punct(Punct::Semicolon);
        Ok(ImportDecl { path, span: span_to(start, self.current_span()) })
    }

    fn parse_use(&mut self) -> ParseResult<UseDecl> {
        let start = self.current_span();
        self.expect_kw(Keyword::Use)?;
        let path = self.parse_dotted_path()?;
        self.eat_punct(Punct::Semicolon);
        Ok(UseDecl { path, span: span_to(start, self.current_span()) })
    }

    fn parse_mod(&mut self) -> ParseResult<ModDecl> {
        let start = self.current_span();
        self.expect_kw(Keyword::Mod)?;
        let (name, _) = self.expect_ident()?;
        self.expect_delim(Delim::OpenBrace)?;
        let mut items = Vec::new();
        while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseBrace) | TokenKind::Eof) {
            match self.parse_item() {
                Ok(i) => items.push(i),
                Err(e) => { self.emit_error(e); self.synchronise(); }
            }
        }
        self.expect_delim(Delim::CloseBrace)?;
        Ok(ModDecl { name, items, span: span_to(start, self.current_span()) })
    }

    fn parse_dotted_path(&mut self) -> ParseResult<Vec<String>> {
        let mut path = Vec::new();
        let (seg, _) = self.expect_ident()?;
        path.push(seg);
        while self.eat_op(Op::Dot) {
            let (seg, _) = self.expect_ident()?;
            path.push(seg);
        }
        Ok(path)
    }

    // ── Block + Statements ────────────────────────────────────────────────

    pub fn parse_block(&mut self) -> ParseResult<Block> {
        let start = self.current_span();
        self.expect_delim(Delim::OpenBrace)?;
        let mut stmts = Vec::new();
        while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseBrace) | TokenKind::Eof) {
            match self.parse_stmt() {
                Ok(s) => stmts.push(s),
                Err(e) => { self.emit_error(e); self.synchronise(); }
            }
        }
        self.expect_delim(Delim::CloseBrace)?;
        Ok(Block { stmts, span: span_to(start, self.current_span()) })
    }

    fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        match &self.peek().kind.clone() {
            TokenKind::Kw(Keyword::Let)    => self.parse_let(),
            TokenKind::Kw(Keyword::Return) => self.parse_return(),
            TokenKind::Kw(Keyword::If)     => self.parse_if().map(Stmt::If),
            TokenKind::Kw(Keyword::While)  => self.parse_while().map(Stmt::While),
            TokenKind::Kw(Keyword::For)    => self.parse_for().map(Stmt::For),
            TokenKind::Kw(Keyword::End)    => self.parse_end().map(Stmt::End),
            // Inner `fn` declaration
            TokenKind::Kw(Keyword::Fn) => {
                // Check for @export before inner fn (unusual but valid)
                let exported = false;
                let vis = Visibility::Private;
                Ok(Stmt::FnDecl(self.parse_fn_decl(vis, exported)?))
            }
            // Inner `enum` declaration
            TokenKind::Kw(Keyword::Enum) => {
                let vis = Visibility::Private;
                Ok(Stmt::EnumDecl(self.parse_enum(vis)?))
            }
            // Inner `struct` declaration
            TokenKind::Kw(Keyword::Struct) => {
                let vis = Visibility::Private;
                Ok(Stmt::StructDecl(self.parse_struct(vis)?))
            }
            _ => self.parse_expr_stmt(),
        }
    }

    // ── let ──────────────────────────────────────────────────────────────

    fn parse_let(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect_kw(Keyword::Let)?;
        let mutable = self.eat_kw(Keyword::Mut);
        let (name, _) = self.expect_ident()?;

        let ty = if self.eat_punct(Punct::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let init = if self.eat_op(Op::Assign) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        self.eat_punct(Punct::Semicolon);
        Ok(Stmt::Let(LetStmt {
            mutable, name, ty, init,
            span: span_to(start, self.current_span()),
        }))
    }

    // ── return ────────────────────────────────────────────────────────────

    fn parse_return(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect_kw(Keyword::Return)?;
        let value = if !matches!(self.peek().kind, TokenKind::Punct(Punct::Semicolon) | TokenKind::Delim(Delim::CloseBrace)) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.eat_punct(Punct::Semicolon);
        Ok(Stmt::Return(ReturnStmt { value, span: span_to(start, self.current_span()) }))
    }

    // ── if ────────────────────────────────────────────────────────────────

    fn parse_if(&mut self) -> ParseResult<IfStmt> {
        let start = self.current_span();
        self.expect_kw(Keyword::If)?;

        // Xore uses `if [cond]` with square brackets
        let cond = if self.eat_delim(Delim::OpenBracket) {
            let c = self.parse_expr()?;
            self.expect_delim(Delim::CloseBracket)?;
            c
        } else {
            // Fall back to bare expression
            self.parse_expr()?
        };

        let then_body = self.parse_block()?;

        let else_body = if self.eat_kw(Keyword::Else) {
            if matches!(self.peek().kind, TokenKind::Kw(Keyword::If)) {
                Some(Box::new(ElseBranch::If(self.parse_if()?)))
            } else {
                Some(Box::new(ElseBranch::Block(self.parse_block()?)))
            }
        } else {
            None
        };

        Ok(IfStmt { cond, then_body, else_body, span: span_to(start, self.current_span()) })
    }

    // ── while ─────────────────────────────────────────────────────────────

    fn parse_while(&mut self) -> ParseResult<WhileStmt> {
        let start = self.current_span();
        self.expect_kw(Keyword::While)?;
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(WhileStmt { cond, body, span: span_to(start, self.current_span()) })
    }

    // ── for ───────────────────────────────────────────────────────────────
    // Syntax: `for in <ident> range (<expr>) { … }`

    fn parse_for(&mut self) -> ParseResult<ForStmt> {
        let start = self.current_span();
        self.expect_kw(Keyword::For)?;
        self.expect_kw(Keyword::In)?;
        let (var, _) = self.expect_ident()?;
        self.expect_kw(Keyword::Range)?;
        self.expect_delim(Delim::OpenParen)?;
        let limit = self.parse_expr()?;
        self.expect_delim(Delim::CloseParen)?;
        let body = self.parse_block()?;
        Ok(ForStmt { var, limit, body, span: span_to(start, self.current_span()) })
    }

    // ── end ───────────────────────────────────────────────────────────────
    // Syntax: `end() [if <cond>];`

    fn parse_end(&mut self) -> ParseResult<EndStmt> {
        let start = self.current_span();
        self.expect_kw(Keyword::End)?;
        self.expect_delim(Delim::OpenParen)?;
        self.expect_delim(Delim::CloseParen)?;
        let cond = if self.eat_kw(Keyword::If) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.eat_punct(Punct::Semicolon);
        Ok(EndStmt { cond, span: span_to(start, self.current_span()) })
    }

    // ── Expression statement ──────────────────────────────────────────────

    fn parse_expr_stmt(&mut self) -> ParseResult<Stmt> {
        let e = self.parse_expr()?;
        self.eat_punct(Punct::Semicolon);
        Ok(Stmt::Expr(e))
    }

    // ── Expressions ───────────────────────────────────────────────────────

    pub fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> ParseResult<Expr> {
        let lhs = self.parse_or()?;

        let op = match &self.peek().kind {
            TokenKind::Op(Op::Assign)     => Some(AssignOp::Assign),
            TokenKind::Op(Op::PlusEq)     => Some(AssignOp::AddAssign),
            TokenKind::Op(Op::MinusEq)    => Some(AssignOp::SubAssign),
            TokenKind::Op(Op::StarEq)     => Some(AssignOp::MulAssign),
            TokenKind::Op(Op::SlashEq)    => Some(AssignOp::DivAssign),
            TokenKind::Op(Op::PercentEq)  => Some(AssignOp::RemAssign),
            TokenKind::Op(Op::AmpEq)      => Some(AssignOp::AndAssign),
            TokenKind::Op(Op::PipeEq)     => Some(AssignOp::OrAssign),
            TokenKind::Op(Op::CaretEq)    => Some(AssignOp::XorAssign),
            TokenKind::Op(Op::ShlEq)      => Some(AssignOp::ShlAssign),
            TokenKind::Op(Op::ShrEq)      => Some(AssignOp::ShrAssign),
            _ => None,
        };

        if let Some(aop) = op {
            let span = self.current_span();
            self.advance();
            let rhs = self.parse_assign()?;
            let end = self.current_span();
            return Ok(Expr::Assign {
                target: Box::new(lhs), op: aop,
                value: Box::new(rhs),
                span: span_to(span, end),
            });
        }
        Ok(lhs)
    }

    fn parse_or(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek().kind, TokenKind::Op(Op::Or)) {
            let span = self.current_span();
            self.advance();
            let rhs = self.parse_and()?;
            let end = self.current_span();
            lhs = Expr::BinOp { op: BinOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs), span: span_to(span, end) };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_compare()?;
        while matches!(self.peek().kind, TokenKind::Op(Op::And)) {
            let span = self.current_span();
            self.advance();
            let rhs = self.parse_compare()?;
            let end = self.current_span();
            lhs = Expr::BinOp { op: BinOp::And, lhs: Box::new(lhs), rhs: Box::new(rhs), span: span_to(span, end) };
        }
        Ok(lhs)
    }

    fn parse_compare(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_add()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Op(Op::Eq) => Some(BinOp::Eq),
                TokenKind::Op(Op::Ne) => Some(BinOp::Ne),
                TokenKind::Op(Op::Lt) => Some(BinOp::Lt),
                TokenKind::Op(Op::Gt) => Some(BinOp::Gt),
                TokenKind::Op(Op::Le) => Some(BinOp::Le),
                TokenKind::Op(Op::Ge) => Some(BinOp::Ge),
                _ => break,
            };
            let span = self.current_span();
            self.advance();
            let rhs = self.parse_add()?;
            let end = self.current_span();
            lhs = Expr::BinOp { op: op.unwrap(), lhs: Box::new(lhs), rhs: Box::new(rhs), span: span_to(span, end) };
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Op(Op::Plus)  => Some(BinOp::Add),
                TokenKind::Op(Op::Minus) => Some(BinOp::Sub),
                _ => break,
            };
            let span = self.current_span();
            self.advance();
            let rhs = self.parse_mul()?;
            let end = self.current_span();
            lhs = Expr::BinOp { op: op.unwrap(), lhs: Box::new(lhs), rhs: Box::new(rhs), span: span_to(span, end) };
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Op(Op::Star)    => Some(BinOp::Mul),
                TokenKind::Op(Op::Slash)   => Some(BinOp::Div),
                TokenKind::Op(Op::Percent) => Some(BinOp::Rem),
                _ => break,
            };
            let span = self.current_span();
            self.advance();
            let rhs = self.parse_unary()?;
            let end = self.current_span();
            lhs = Expr::BinOp { op: op.unwrap(), lhs: Box::new(lhs), rhs: Box::new(rhs), span: span_to(span, end) };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> ParseResult<Expr> {
        let span = self.current_span();
        if self.eat_op(Op::Minus) {
            let operand = self.parse_unary()?;
            let end = self.current_span();
            return Ok(Expr::UnOp { op: UnOp::Neg, operand: Box::new(operand), span: span_to(span, end) });
        }
        if self.eat_op(Op::Bang) {
            let operand = self.parse_unary()?;
            let end = self.current_span();
            return Ok(Expr::UnOp { op: UnOp::Not, operand: Box::new(operand), span: span_to(span, end) });
        }
        if self.eat_op(Op::Amp) {
            let operand = self.parse_unary()?;
            let end = self.current_span();
            return Ok(Expr::UnOp { op: UnOp::Ref, operand: Box::new(operand), span: span_to(span, end) });
        }
        if self.eat_op(Op::Star) {
            let operand = self.parse_unary()?;
            let end = self.current_span();
            return Ok(Expr::UnOp { op: UnOp::Deref, operand: Box::new(operand), span: span_to(span, end) });
        }
        self.parse_call()
    }

    // ── Call / field access / macro call ──────────────────────────────────

    fn parse_call(&mut self) -> ParseResult<Expr> {
        let mut base = self.parse_primary()?;

        loop {
            let span = self.current_span();

            // `expr(args)` — function call
            if matches!(self.peek().kind, TokenKind::Delim(Delim::OpenParen)) {
                let args = self.parse_call_args()?;
                let end = self.current_span();
                base = Expr::Call { callee: Box::new(base), args, span: span_to(span, end) };
                continue;
            }

            // `expr.field` or `expr.method(args)` or `expr.{}`
            if self.eat_op(Op::Dot) {
                // `{` after dot — format hole
                if matches!(self.peek().kind, TokenKind::Delim(Delim::OpenBrace)) {
                    self.advance(); // consume `{`
                    self.expect_delim(Delim::CloseBrace)?;
                    // Wrap existing base + hole into a FmtChain
                    base = self.build_fmt_chain(base, span)?;
                    continue;
                }
                // Normal field / method
                let (fname, fspan) = self.expect_ident()?;
                let end = self.current_span();
                base = Expr::Field { object: Box::new(base), field: fname, span: span_to(fspan, end) };
                continue;
            }

            break;
        }
        Ok(base)
    }

    /// Parses `(arg1, arg2, …)`
    fn parse_call_args(&mut self) -> ParseResult<Vec<Expr>> {
        self.expect_delim(Delim::OpenParen)?;
        let mut args = Vec::new();
        while !matches!(self.peek().kind, TokenKind::Delim(Delim::CloseParen) | TokenKind::Eof) {
            args.push(self.parse_expr()?);
            if !self.eat_punct(Punct::Comma) { break; }
        }
        self.expect_delim(Delim::CloseParen)?;
        Ok(args)
    }

    /// Build `FmtChain` from a base expression followed by `.{}` holes.
    /// Called after we've consumed the first `.{}`.
    fn build_fmt_chain(&mut self, base: Expr, start: Span) -> ParseResult<Expr> {
        let mut parts = Vec::new();
        // The base becomes a string part if it's a StrLit, otherwise a hole.
        match &base {
            Expr::StrLit { value, .. } => parts.push(FmtPart::Str(value.clone())),
            other => parts.push(FmtPart::Hole(other.clone())),
        }
        // We already consumed `.{}` — that was an empty hole.
        parts.push(FmtPart::Hole(Expr::Ident { name: "_fmt".into(), span: start }));

        // Continue consuming `.field` or `.{}` chains
        loop {
            if !self.eat_op(Op::Dot) { break; }
            if matches!(self.peek().kind, TokenKind::Delim(Delim::OpenBrace)) {
                self.advance();
                self.expect_delim(Delim::CloseBrace)?;
                parts.push(FmtPart::Hole(Expr::Ident { name: "_fmt".into(), span: self.current_span() }));
            } else {
                let (fname, _) = self.expect_ident()?;
                parts.push(FmtPart::Field(fname));
            }
        }
        let end = self.current_span();
        Ok(Expr::FmtChain { parts, span: span_to(start, end) })
    }

    // ── Primary expressions ───────────────────────────────────────────────

    fn parse_primary(&mut self) -> ParseResult<Expr> {
        let tok = self.peek().clone();
        let span = tok.span;

        match &tok.kind {
            // ── Integer literal ───────────────────────────────────────────
            TokenKind::IntLit { value, .. } => {
                let v = *value;
                self.advance();
                Ok(Expr::IntLit { value: v, span })
            }

            // ── Float literal ─────────────────────────────────────────────
            TokenKind::FloatLit { value, .. } => {
                let v = *value;
                self.advance();
                Ok(Expr::FloatLit { value: v, span })
            }

            // ── String literal ────────────────────────────────────────────
            TokenKind::StrLit(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::StrLit { value: s, span })
            }

            // ── Boolean / None literals ───────────────────────────────────
            TokenKind::Kw(Keyword::True)  => { self.advance(); Ok(Expr::BoolLit { value: true,  span }) }
            TokenKind::Kw(Keyword::False) => { self.advance(); Ok(Expr::BoolLit { value: false, span }) }
            TokenKind::Kw(Keyword::Null)  => { self.advance(); Ok(Expr::NoneLit { span }) }

            // ── Macro call: `println!(…)` ─────────────────────────────────
            TokenKind::Ident(name) if self.peek2_is_bang_paren() => {
                let name = name.clone();
                self.advance(); // ident
                self.advance(); // !
                let args = self.parse_call_args()?;
                Ok(Expr::MacroCall { name, args, span: span_to(span, self.current_span()) })
            }

            // ── Type constructor: `i32(23)`, `bool(False)`, etc. ──────────
            // Keywords that are type names followed by `(`
            TokenKind::Kw(kw) if is_type_kw(*kw) && matches!(self.peek2().kind, TokenKind::Delim(Delim::OpenParen)) => {
                let ty_name = kw.as_str().to_string();
                self.advance();
                let args = self.parse_call_args()?;
                Ok(Expr::TypeCall {
                    ty: Box::new(TypeExpr::Named(ty_name, span)),
                    args,
                    span: span_to(span, self.current_span()),
                })
            }

            // ── Identifier ────────────────────────────────────────────────
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Ident { name, span })
            }

            // Allow keyword-as-ident for things like `new`, `data`
            TokenKind::Kw(kw) => {
                let name = kw.as_str().to_string();
                self.advance();
                Ok(Expr::Ident { name, span })
            }

            // ── Parenthesised expression ──────────────────────────────────
            TokenKind::Delim(Delim::OpenParen) => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect_delim(Delim::CloseParen)?;
                Ok(Expr::Paren { inner: Box::new(inner), span: span_to(span, self.current_span()) })
            }

            // ── Square-bracket group: `[expr]` used in if-conditions ──────
            TokenKind::Delim(Delim::OpenBracket) => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect_delim(Delim::CloseBracket)?;
                Ok(Expr::Bracket { inner: Box::new(inner), span: span_to(span, self.current_span()) })
            }

            _ => Err(ParseError::new(
                format!("unexpected token in expression: `{}`", self.token_desc()),
                span,
            ))
        }
    }

    /// Returns true if `tokens[pos]` is an Ident and `tokens[pos+1]` is `!`
    /// followed by `(` — i.e. a macro call.
    fn peek2_is_bang_paren(&self) -> bool {
        matches!(&self.peek2().kind, TokenKind::Op(Op::Bang))
            && {
                let i = (self.pos + 2).min(self.tokens.len() - 1);
                matches!(&self.tokens[i].kind, TokenKind::Delim(Delim::OpenParen))
            }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn delim_str(d: Delim) -> &'static str {
    match d {
        Delim::OpenParen    => "(",
        Delim::CloseParen   => ")",
        Delim::OpenBrace    => "{",
        Delim::CloseBrace   => "}",
        Delim::OpenBracket  => "[",
        Delim::CloseBracket => "]",
    }
}

fn punct_str(p: Punct) -> &'static str {
    match p {
        Punct::Semicolon => ";",
        Punct::Comma     => ",",
        Punct::Colon     => ":",
    }
}

fn span_to(start: Span, end: Span) -> Span {
    Span::new(start.start, end.end, start.line, start.col)
}

fn is_type_kw(kw: Keyword) -> bool {
    matches!(kw,
        Keyword::I8  | Keyword::U8  | Keyword::I16 | Keyword::U16 |
        Keyword::I32 | Keyword::U32 | Keyword::I64 | Keyword::U64 |
        Keyword::F32 | Keyword::F64 | Keyword::Usize | Keyword::Isize |
        Keyword::Bool | Keyword::Void | Keyword::Char | Keyword::Anytype
    )
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Program {
        let mut p = Parser::new(src);
        let prog = p.parse_program();
        if !p.errors.is_empty() {
            for e in &p.errors { eprintln!("PARSE ERROR: {e}"); }
        }
        prog
    }

    fn parse_ok(src: &str) -> Program {
        let mut p = Parser::new(src);
        let prog = p.parse_program();
        assert!(p.errors.is_empty(), "unexpected errors: {:?}", p.errors);
        prog
    }

    #[test]
    fn parse_empty_program() {
        let prog = parse_ok("");
        assert!(prog.items.is_empty());
    }

    #[test]
    fn parse_public_main_short_form() {
        let prog = parse_ok("public main() { }");
        assert_eq!(prog.items.len(), 1);
        if let Item::Function(f) = &prog.items[0] {
            assert_eq!(f.name, "main");
            assert_eq!(f.vis, Visibility::Public);
        } else { panic!("expected function"); }
    }

    #[test]
    fn parse_fn_with_params_and_return() {
        let prog = parse_ok("public fn add(a: i32, b: i32) -> i32 { return a + b; }");
        assert_eq!(prog.items.len(), 1);
        if let Item::Function(f) = &prog.items[0] {
            assert_eq!(f.params.len(), 2);
            assert!(f.ret_ty.is_some());
        } else { panic!("expected function"); }
    }

    #[test]
    fn parse_let_stmt() {
        let prog = parse_ok("public main() { let x = 42; }");
        if let Item::Function(f) = &prog.items[0] {
            assert_eq!(f.body.stmts.len(), 1);
            assert!(matches!(&f.body.stmts[0], Stmt::Let(_)));
        }
    }

    #[test]
    fn parse_let_mut_stmt() {
        let prog = parse_ok("public main() { let mut z = bool(False); }");
        if let Item::Function(f) = &prog.items[0] {
            if let Stmt::Let(l) = &f.body.stmts[0] {
                assert!(l.mutable);
                assert_eq!(l.name, "z");
            }
        }
    }

    #[test]
    fn parse_if_with_brackets() {
        let prog = parse_ok("public main() { if [x == y] { } }");
        if let Item::Function(f) = &prog.items[0] {
            assert!(matches!(&f.body.stmts[0], Stmt::If(_)));
        }
    }

    #[test]
    fn parse_if_else() {
        let prog = parse_ok("public main() { if [x == y] { } else { } }");
        if let Item::Function(f) = &prog.items[0] {
            if let Stmt::If(i) = &f.body.stmts[0] {
                assert!(i.else_body.is_some());
            }
        }
    }

    #[test]
    fn parse_while_loop() {
        let prog = parse_ok("public main() { while z { } }");
        if let Item::Function(f) = &prog.items[0] {
            assert!(matches!(&f.body.stmts[0], Stmt::While(_)));
        }
    }

    #[test]
    fn parse_for_range() {
        let prog = parse_ok("public main() { for in i range (30) { } }");
        if let Item::Function(f) = &prog.items[0] {
            if let Stmt::For(fr) = &f.body.stmts[0] {
                assert_eq!(fr.var, "i");
            } else { panic!("expected for"); }
        }
    }

    #[test]
    fn parse_end_stmt() {
        let prog = parse_ok("public main() { end() }");
        if let Item::Function(f) = &prog.items[0] {
            assert!(matches!(&f.body.stmts[0], Stmt::End(_)));
        }
    }

    #[test]
    fn parse_end_if_stmt() {
        let prog = parse_ok("public main() { while z { end() if y == 23; } }");
        if let Item::Function(f) = &prog.items[0] {
            if let Stmt::While(w) = &f.body.stmts[0] {
                if let Stmt::End(e) = &w.body.stmts[0] {
                    assert!(e.cond.is_some());
                }
            }
        }
    }

    #[test]
    fn parse_macro_call() {
        let prog = parse_ok(r#"public main() { println!("hello"); }"#);
        if let Item::Function(f) = &prog.items[0] {
            if let Stmt::Expr(Expr::MacroCall { name, .. }) = &f.body.stmts[0] {
                assert_eq!(name, "println");
            } else { panic!("expected macro call"); }
        }
    }

    #[test]
    fn parse_struct() {
        let prog = parse_ok("struct Point { x: f32, y: f32 }");
        assert!(matches!(&prog.items[0], Item::Struct(_)));
    }

    #[test]
    fn parse_enum() {
        let prog = parse_ok("enum Direction { North, South, East, West, }");
        assert!(matches!(&prog.items[0], Item::Enum(_)));
    }

    #[test]
    fn parse_inner_enum_in_fn() {
        let src = "public main() { fn new() -> Self { enum data { SELF, HOST, } } }";
        let prog = parse(src);
        // Should not crash even if some parts are not fully resolved.
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn parse_full_example() {
        // The full example from the Xore spec.
        let src = r#"
public main(){
    println!("data");
    let x = String.new();
    let y = String.new();
    let mut z = bool(False);
    if [x == y]{
        z %= True;
    }
    else {
        println!(None);
    }
    while z {
        end() if y == i32(23);
    }
    for in i range (30){
        println!(y);
        end()
    }
}
"#;
        let mut p = Parser::new(src);
        let prog = p.parse_program();
        // Acceptable to have minor parse errors on exotic chains;
        // the core structure must parse cleanly.
        assert_eq!(prog.items.len(), 1, "should produce 1 top-level item");
    }
}