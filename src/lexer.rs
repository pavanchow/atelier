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

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
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
        // Decode the leading char so multi byte input never gets sliced across a
        // codepoint boundary. `i` only ever advances by whole chars or ASCII
        // bytes, so `text[i..]` always begins on a boundary here.
        let ch = text[i..].chars().next().unwrap();
        let kind = if is_ident_start(ch) {
            i += ch.len_utf8();
            while i < bytes.len() {
                let c = text[i..].chars().next().unwrap();
                if is_ident_continue(c) {
                    i += c.len_utf8();
                } else {
                    break;
                }
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
                if let Some(k) = single {
                    i += 1;
                    k
                } else {
                    // Consume the whole char, not one byte, so an Error token
                    // over a multi byte codepoint keeps well formed spans.
                    i += ch.len_utf8();
                    TokenKind::Error
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

    #[test]
    fn unicode_identifier_lexes_without_panic() {
        let toks = lex("let café = 1;");
        assert_eq!(toks[1].kind, TokenKind::Ident);
        assert_eq!(toks[1].text, "café");
        // `é` is two bytes, so the `=` sits two bytes past a naive ASCII count.
        assert_eq!(toks[2].kind, TokenKind::Eq);
    }

    #[test]
    fn non_ascii_symbol_is_a_single_error_token() {
        let toks = lex("a € b");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1].kind, TokenKind::Error);
        assert_eq!(toks[1].text, "€");
        assert_eq!(toks[1].span, Span::new(2, 5));
    }

    #[test]
    fn unicode_inside_comment_is_skipped() {
        let toks = lex("a // café ☕\nb");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].text, "a");
        assert_eq!(toks[1].text, "b");
    }
}
