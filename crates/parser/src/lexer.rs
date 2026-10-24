use crate::error::{Span, SyntaxError};

pub const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "GROUP", "BY", "HAVING", "ORDER", "LIMIT", "JOIN", "UNION", "AS",
    "AND", "OR", "NOT", "IS", "IN", "NULL", "TRUE", "FALSE", "LIKE", "ILIKE", "BETWEEN", "EXISTS",
];

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Keyword(&'static str),
    Ident { name: String, quoted: bool },
    Number(String),
    String(String),
    Param { name: String, ty: String },
    Star,
    Plus,
    Minus,
    Slash,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    LParen,
    RParen,
    Comma,
    Semicolon,
    Dot,
    Eof,
}

impl TokenKind {
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Keyword(k) => k.to_string(),
            TokenKind::Ident { name, .. } => format!("identifier `{name}`"),
            TokenKind::Number(n) => format!("number `{n}`"),
            TokenKind::String(v) => format!("string '{v}'"),
            TokenKind::Param { name, ty } => format!("parameter {{{name}:{ty}}}"),
            TokenKind::Star => "`*`".into(),
            TokenKind::Plus => "`+`".into(),
            TokenKind::Minus => "`-`".into(),
            TokenKind::Slash => "`/`".into(),
            TokenKind::Eq => "`=`".into(),
            TokenKind::NotEq => "`!=`".into(),
            TokenKind::Lt => "`<`".into(),
            TokenKind::LtEq => "`<=`".into(),
            TokenKind::Gt => "`>`".into(),
            TokenKind::GtEq => "`>=`".into(),
            TokenKind::LParen => "`(`".into(),
            TokenKind::RParen => "`)`".into(),
            TokenKind::Comma => "`,`".into(),
            TokenKind::Semicolon => "`;`".into(),
            TokenKind::Dot => "`.`".into(),
            TokenKind::Eof => "end of input".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, SyntaxError> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'=' | b'!' | b'<' | b'>' => {
                let next = bytes.get(i + 1).copied();
                let (kind, len) = match (c, next) {
                    (b'=', _) => (TokenKind::Eq, 1),
                    (b'!', Some(b'=')) => (TokenKind::NotEq, 2),
                    (b'<', Some(b'=')) => (TokenKind::LtEq, 2),
                    (b'<', Some(b'>')) => (TokenKind::NotEq, 2),
                    (b'<', _) => (TokenKind::Lt, 1),
                    (b'>', Some(b'=')) => (TokenKind::GtEq, 2),
                    (b'>', _) => (TokenKind::Gt, 1),
                    _ => {
                        return Err(SyntaxError::new(
                            "unexpected character `!`",
                            Span(start, start + 1),
                        ));
                    }
                };
                i += len;
                tokens.push(Token {
                    kind,
                    span: Span(start, i),
                });
            }
            b'-' | b'+' | b'/' => {
                let kind = match c {
                    b'-' => TokenKind::Minus,
                    b'+' => TokenKind::Plus,
                    _ => TokenKind::Slash,
                };
                i += 1;
                tokens.push(Token {
                    kind,
                    span: Span(start, i),
                });
            }
            b'*' | b'(' | b')' | b',' | b';' | b'.' => {
                let kind = match c {
                    b'*' => TokenKind::Star,
                    b'(' => TokenKind::LParen,
                    b')' => TokenKind::RParen,
                    b',' => TokenKind::Comma,
                    b';' => TokenKind::Semicolon,
                    _ => TokenKind::Dot,
                };
                i += 1;
                tokens.push(Token {
                    kind,
                    span: Span(start, i),
                });
            }
            b'"' | b'`' => {
                let quote = c;
                i += 1;
                let body_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(SyntaxError::new(
                        "unterminated quoted identifier",
                        Span(start, bytes.len()),
                    ));
                }
                let name = &src[body_start..i];
                i += 1;
                if name.is_empty() {
                    return Err(SyntaxError::new("empty quoted identifier", Span(start, i)));
                }
                tokens.push(Token {
                    kind: TokenKind::Ident {
                        name: name.to_string(),
                        quoted: true,
                    },
                    span: Span(start, i),
                });
            }
            b'0'..=b'9' => {
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Number(src[start..i].to_string()),
                    span: Span(start, i),
                });
            }
            b'\'' => {
                i += 1;
                let mut value = String::new();
                let unterminated =
                    || SyntaxError::new("unterminated string literal", Span(start, bytes.len()));
                loop {
                    let Some(ch) = char_at(src, i) else {
                        return Err(unterminated());
                    };
                    match ch {
                        '\\' => {
                            let Some(escaped) = char_at(src, i + 1) else {
                                return Err(unterminated());
                            };
                            value.push(escaped);
                            i += 1 + escaped.len_utf8();
                        }
                        '\'' if bytes.get(i + 1) == Some(&b'\'') => {
                            value.push('\'');
                            i += 2;
                        }
                        '\'' => {
                            i += 1;
                            break;
                        }
                        _ => {
                            value.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::String(value),
                    span: Span(start, i),
                });
            }
            b'{' => {
                let (kind, end) = param(src, start)?;
                i = end;
                tokens.push(Token {
                    kind,
                    span: Span(start, i),
                });
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &src[start..i];
                let upper = word.to_ascii_uppercase();
                let kind = match KEYWORDS.iter().find(|k| **k == upper) {
                    Some(k) => TokenKind::Keyword(k),
                    None => TokenKind::Ident {
                        name: word.to_string(),
                        quoted: false,
                    },
                };
                tokens.push(Token {
                    kind,
                    span: Span(start, i),
                });
            }
            _ => {
                // `i` is always on a char boundary here; the fallback is never used.
                let ch = char_at(src, i).unwrap_or(char::REPLACEMENT_CHARACTER);
                return Err(SyntaxError::new(
                    format!("unexpected character `{ch}`"),
                    Span(start, start + ch.len_utf8()),
                ));
            }
        }
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span(src.len(), src.len()),
    });
    Ok(tokens)
}

fn char_at(src: &str, i: usize) -> Option<char> {
    src.get(i..)?.chars().next()
}

fn param(src: &str, start: usize) -> Result<(TokenKind, usize), SyntaxError> {
    let bytes = src.as_bytes();
    let err = |msg: &str, end: usize| Err(SyntaxError::new(msg, Span(start, end)));
    let mut i = start + 1;
    let name_start = i;
    if i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
    }
    let name = &src[name_start..i];
    if name.is_empty() {
        return err(
            "expected a parameter name after `{` (as in {id:UUID})",
            i.max(start + 1),
        );
    }
    if bytes.get(i) != Some(&b':') {
        return err(
            "expected `:` and a type after the parameter name (as in {id:UUID})",
            i,
        );
    }
    i += 1;
    let ty_start = i;
    let (mut depth, mut quoted) = (0usize, false);
    loop {
        match bytes.get(i) {
            None => return err("unterminated parameter; expected `}`", bytes.len()),
            Some(b'\\') if quoted => i += 1,
            Some(b'\'') => quoted = !quoted,
            Some(b'(') if !quoted => depth += 1,
            Some(b')') if !quoted => depth = depth.saturating_sub(1),
            Some(b'}') if !quoted && depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    let ty = src[ty_start..i].trim();
    if ty.is_empty() {
        return err("expected a type after `:` (as in {id:UUID})", i + 1);
    }
    Ok((
        TokenKind::Param {
            name: name.to_string(),
            ty: ty.to_string(),
        },
        i + 1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str) -> TokenKind {
        TokenKind::Ident {
            name: name.into(),
            quoted: false,
        }
    }

    fn quoted(name: &str) -> TokenKind {
        TokenKind::Ident {
            name: name.into(),
            quoted: true,
        }
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn keywords_are_case_insensitive_and_idents_keep_case() {
        assert_eq!(
            kinds("sElEcT * FrOm Events"),
            [
                TokenKind::Keyword("SELECT"),
                TokenKind::Star,
                TokenKind::Keyword("FROM"),
                ident("Events"),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn quoted_identifiers_and_comments() {
        assert_eq!(
            kinds("select -- all of it\n * from \"select\";"),
            [
                TokenKind::Keyword("SELECT"),
                TokenKind::Star,
                TokenKind::Keyword("FROM"),
                quoted("select"),
                TokenKind::Semicolon,
                TokenKind::Eof
            ]
        );
        assert_eq!(kinds("`my table`")[0], quoted("my table"));
        assert_eq!(kinds("final ALL")[..2], [ident("final"), ident("ALL")]);
    }

    #[test]
    fn operators() {
        assert_eq!(
            kinds("a = 1 != 2 <> 3 < <= > >= - -- comment\n+ /"),
            [
                ident("a"),
                TokenKind::Eq,
                TokenKind::Number("1".into()),
                TokenKind::NotEq,
                TokenKind::Number("2".into()),
                TokenKind::NotEq,
                TokenKind::Number("3".into()),
                TokenKind::Lt,
                TokenKind::LtEq,
                TokenKind::Gt,
                TokenKind::GtEq,
                TokenKind::Minus,
                TokenKind::Plus,
                TokenKind::Slash,
                TokenKind::Eof
            ]
        );
        assert_eq!(
            kinds("is NULL true")[..3],
            [
                TokenKind::Keyword("IS"),
                TokenKind::Keyword("NULL"),
                TokenKind::Keyword("TRUE")
            ]
        );
        assert!(tokenize("a ! b").is_err());
    }

    #[test]
    fn literals() {
        assert_eq!(
            kinds("1 2.5 'it''s' 'a\\'b'"),
            [
                TokenKind::Number("1".into()),
                TokenKind::Number("2.5".into()),
                TokenKind::String("it's".into()),
                TokenKind::String("a'b".into()),
                TokenKind::Eof
            ]
        );
        assert!(tokenize("'open").is_err());
    }

    #[test]
    fn errors_have_positions() {
        let e = tokenize("select # from x").unwrap_err();
        assert_eq!(e.span, Span(7, 8));
        assert!(tokenize("select * from \"oops").is_err());
    }

    #[test]
    fn parameters() {
        assert_eq!(
            kinds("id = {id:UUID}"),
            [
                ident("id"),
                TokenKind::Eq,
                TokenKind::Param {
                    name: "id".into(),
                    ty: "UUID".into()
                },
                TokenKind::Eof
            ]
        );
        assert_eq!(
            kinds("{t: DateTime64(3, 'U}C') }")[0],
            TokenKind::Param {
                name: "t".into(),
                ty: "DateTime64(3, 'U}C')".into()
            }
        );
        let err = |src: &str| {
            let e = tokenize(src).unwrap_err();
            (e.message, e.span.0, e.span.1)
        };
        assert_eq!(
            err("a = {:UUID}"),
            (
                "expected a parameter name after `{` (as in {id:UUID})".into(),
                4,
                5
            )
        );
        assert_eq!(
            err("a = {id UUID}"),
            (
                "expected `:` and a type after the parameter name (as in {id:UUID})".into(),
                4,
                7
            )
        );
        assert_eq!(
            err("a = {id:}"),
            ("expected a type after `:` (as in {id:UUID})".into(), 4, 9)
        );
        assert_eq!(
            err("a = {id:UUID"),
            ("unterminated parameter; expected `}`".into(), 4, 12)
        );
    }
}
