//! A hand written lexer for the Atelier language.
//!
//! [`lex`] turns a source string into a vector of [`Token`]s with absolute
//! spans. Whitespace and `//` line comments are skipped. Unknown bytes become a
//! single character [`TokenKind::Error`] so the parser can report them with a
//! precise span. [`lex_at`] lexes a substring while assigning absolute spans,
//! which the incremental analyser uses to re lex only the region around an edit.

use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Int,
    Ident,
    // keywords
    Let,
    Fn,
    If,
    Else,
    True,
    False,
    // delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    // operators
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Bang,
    // an unrecognised byte
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub text: String,
}

impl Token {
    pub fn shifted(&self, delta: i64) -> Token {
        Token {
            kind: self.kind,
            span: self.span.shifted(delta),
            text: self.text.clone(),
        }
    }
}

fn keyword(word: &str) -> Option<TokenKind> {
    Some(match word {
        "let" => TokenKind::Let,
        "fn" => TokenKind::Fn,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        _ => return None,
    })
}

fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

fn is_ident_continue(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

/// Lex a full source string into tokens (excluding any end marker).
pub fn lex(text: &str) -> Vec<Token> {
    lex_at(text, 0)
}

/// Lex `text` as if it began at absolute byte offset `base`. Spans are absolute.
pub fn lex_at(text: &str, base: u32) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        // whitespace
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }
        // line comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        let kind = if is_ident_start(b) {
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &text[start..i];
            keyword(word).unwrap_or(TokenKind::Ident)
        } else if b.is_ascii_digit() {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            TokenKind::Int
        } else {
            let two = |a: u8, c: u8| bytes[i] == a && i + 1 < bytes.len() && bytes[i + 1] == c;
            if two(b'=', b'=') {
                i += 2;
                TokenKind::EqEq
            } else if two(b'!', b'=') {
                i += 2;
                TokenKind::Ne
            } else if two(b'<', b'=') {
                i += 2;
                TokenKind::Le
            } else if two(b'>', b'=') {
                i += 2;
                TokenKind::Ge
            } else {
                let single = match b {
                    b'(' => Some(TokenKind::LParen),
                    b')' => Some(TokenKind::RParen),
                    b'{' => Some(TokenKind::LBrace),
                    b'}' => Some(TokenKind::RBrace),
                    b',' => Some(TokenKind::Comma),
                    b';' => Some(TokenKind::Semi),
                    b'=' => Some(TokenKind::Eq),
                    b'<' => Some(TokenKind::Lt),
                    b'>' => Some(TokenKind::Gt),
                    b'+' => Some(TokenKind::Plus),
                    b'-' => Some(TokenKind::Minus),
                    b'*' => Some(TokenKind::Star),
                    b'/' => Some(TokenKind::Slash),
                    b'!' => Some(TokenKind::Bang),
                    _ => None,
                };
                match single {
                    Some(k) => {
                        i += 1;
                        k
                    }
                    None => {
                        i += 1;
                        TokenKind::Error
                    }
                }
            }
        };
        out.push(Token {
            kind,
            span: Span::new(base + start as u32, base + i as u32),
            text: text[start..i].to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_keywords_and_idents() {
        let toks = lex("let x = 10;");
        let kinds: Vec<_> = toks.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Let,
                TokenKind::Ident,
                TokenKind::Eq,
                TokenKind::Int,
                TokenKind::Semi
            ]
        );
        assert_eq!(toks[1].text, "x");
        assert_eq!(toks[1].span, Span::new(4, 5));
    }

    #[test]
    fn skips_comments_and_whitespace() {
        let toks = lex("  a // comment\n b ");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].text, "a");
        assert_eq!(toks[1].text, "b");
        assert_eq!(toks[1].span.start, 16);
    }

    #[test]
    fn two_char_operators() {
        let toks = lex("a == b != c <= d >= e");
        let kinds: Vec<_> = toks.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&TokenKind::EqEq));
        assert!(kinds.contains(&TokenKind::Ne));
        assert!(kinds.contains(&TokenKind::Le));
        assert!(kinds.contains(&TokenKind::Ge));
    }

    #[test]
    fn lex_at_offsets() {
        let toks = lex_at("x", 5);
        assert_eq!(toks[0].span, Span::new(5, 6));
    }

    #[test]
    fn unknown_byte_is_error_token() {
        let toks = lex("a @ b");
        assert_eq!(toks[1].kind, TokenKind::Error);
        assert_eq!(toks[1].text, "@");
    }
}
