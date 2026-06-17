#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    /// Field name (e.g., `sections`)
    Ident(&'a str),
    /// `/` path separator or root
    Slash,
    /// `.` path separator
    Dot,
    /// `[` bracket open
    LBracket,
    /// `]` bracket close
    RBracket,
    /// `|` pipe
    Pipe,
    /// `{` projection open
    LBrace,
    /// `}` projection close
    RBrace,
    /// `,` comma
    Comma,
    /// `?` filter indicator
    Question,
    /// `=` equality
    Eq,
    /// `!=` not equal
    Ne,
    /// `>` greater than
    Gt,
    /// `>=` greater or equal
    Ge,
    /// `<` less than
    Lt,
    /// `<=` less or equal
    Le,
    /// `&` ampersand (AND in filters)
    Ampersand,
    /// `~` string contains
    Tilde,
    /// `^` string starts with
    Caret,
    /// `$` string ends with
    Dollar,
    /// `:` colon (slice)
    Colon,
    /// `(` open paren
    LParen,
    /// `)` close paren
    RParen,
    /// Integer literal (e.g., `42`)
    Int(i64),
    /// Float literal (e.g., `2.5`)
    Float(f64),
    /// String literal (e.g., `"hello"`)
    Str(&'a str),
}

pub fn tokenize(input: &str) -> Result<Vec<Token<'_>>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        match ch {
            '/' => tokens.push(Token::Slash),
            '.' => {
                // Could be a path separator or a leading-dot float (.5)
                if let Some((_j, c)) = chars.peek() {
                    if c.is_ascii_digit() {
                        // Leading-dot float: consume as "0.xxx"
                        let start = i;
                        let mut end = i + 1;
                        loop {
                            match chars.peek() {
                                Some((k, cc)) if cc.is_ascii_digit() => {
                                    end = *k + 1;
                                    chars.next();
                                }
                                _ => break,
                            }
                        }
                        let s = format!("0{}", &input[start..end]);
                        let n: f64 = s
                            .parse()
                            .map_err(|e| format!("bad float `{}`: {}", &input[start..end], e))?;
                        tokens.push(Token::Float(n));
                    } else {
                        tokens.push(Token::Dot);
                    }
                } else {
                    tokens.push(Token::Dot);
                }
            }
            '[' => tokens.push(Token::LBracket),
            ']' => tokens.push(Token::RBracket),
            '|' => tokens.push(Token::Pipe),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            ',' => tokens.push(Token::Comma),
            '?' => tokens.push(Token::Question),
            '=' => {
                if chars.peek().is_some_and(|(_, c)| *c == '=') {
                    chars.next(); // skip second =
                }
                tokens.push(Token::Eq);
            }
            '!' => {
                if chars.peek().is_some_and(|(_, c)| *c == '=') {
                    chars.next();
                    tokens.push(Token::Ne);
                } else {
                    return Err(format!("unexpected `!` at position {}", i));
                }
            }
            '>' => {
                if chars.peek().is_some_and(|(_, c)| *c == '=') {
                    chars.next();
                    tokens.push(Token::Ge);
                } else {
                    tokens.push(Token::Gt);
                }
            }
            '<' => {
                if chars.peek().is_some_and(|(_, c)| *c == '=') {
                    chars.next();
                    tokens.push(Token::Le);
                } else {
                    tokens.push(Token::Lt);
                }
            }
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            ':' => tokens.push(Token::Colon),
            '&' => tokens.push(Token::Ampersand),
            '~' => tokens.push(Token::Tilde),
            '^' => tokens.push(Token::Caret),
            '$' => tokens.push(Token::Dollar),
            '"' => {
                // String literal
                let start = i + 1;
                let mut end = start;
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((j, '\\')) => {
                            // skip escaped character
                            chars.next();
                            end = j + 2;
                        }
                        Some((_, _)) => end += 1,
                        None => {
                            return Err(format!("unterminated string literal at position {}", i))
                        }
                    }
                }
                tokens.push(Token::Str(&input[start..end]));
            }
            ch if ch.is_ascii_digit()
                || (ch == '-'
                    && chars
                        .peek()
                        .is_some_and(|(_, c)| c.is_ascii_digit() || *c == '.')) =>
            {
                let start = i;
                let mut end = i + 1;
                let mut is_float = false;
                loop {
                    match chars.peek() {
                        Some((j, c)) if c.is_ascii_digit() => {
                            end = *j + 1;
                            chars.next();
                        }
                        Some((j, '.')) if !is_float => {
                            // Check if followed by a digit (float literal)
                            let after_dot = input[*j + 1..].chars().next();
                            if after_dot.is_some_and(|c| c.is_ascii_digit()) {
                                is_float = true;
                                end = *j + 1;
                                chars.next(); // consume '.'
                            } else {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                if is_float {
                    let n: f64 = input[start..end]
                        .parse()
                        .map_err(|e| format!("bad float `{}`: {}", &input[start..end], e))?;
                    tokens.push(Token::Float(n));
                } else {
                    let n: i64 = input[start..end]
                        .parse()
                        .map_err(|e| format!("bad integer `{}`: {}", &input[start..end], e))?;
                    tokens.push(Token::Int(n));
                }
            }
            ch if ch.is_ascii_alphabetic() || ch == '_' || ch == '-' => {
                let start = i;
                let mut end = i + 1;
                while let Some((j, c)) = chars.peek() {
                    if c.is_ascii_alphanumeric() || *c == '_' || *c == '-' {
                        end = *j + 1;
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(&input[start..end]));
            }
            ch if ch.is_whitespace() => {}
            _ => return Err(format!("unexpected character `{}` at position {}", ch, i)),
        }
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_path() {
        let t = tokenize("/sections").unwrap();
        assert_eq!(t, vec![Token::Slash, Token::Ident("sections")]);
    }

    #[test]
    fn test_filter() {
        let t = tokenize("/sections[?id>10]").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Slash,
                Token::Ident("sections"),
                Token::LBracket,
                Token::Question,
                Token::Ident("id"),
                Token::Gt,
                Token::Int(10),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn test_string_contains() {
        let t = tokenize("/sections[?name~sensor]").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Slash,
                Token::Ident("sections"),
                Token::LBracket,
                Token::Question,
                Token::Ident("name"),
                Token::Tilde,
                Token::Ident("sensor"),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn test_pipe() {
        let t = tokenize("/sections | select(.name~sensor)").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Slash,
                Token::Ident("sections"),
                Token::Pipe,
                Token::Ident("select"),
                Token::LParen,
                Token::Dot,
                Token::Ident("name"),
                Token::Tilde,
                Token::Ident("sensor"),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn test_float_literal() {
        let t = tokenize("/points[?x>2.5]").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Slash,
                Token::Ident("points"),
                Token::LBracket,
                Token::Question,
                Token::Ident("x"),
                Token::Gt,
                Token::Float(2.5),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn test_float_edge() {
        // 0.5 and .5 — both should be floats
        let t = tokenize("/a[?b<0.5]").unwrap();
        assert_eq!(
            t[..6],
            [
                Token::Slash,
                Token::Ident("a"),
                Token::LBracket,
                Token::Question,
                Token::Ident("b"),
                Token::Lt,
            ]
        );
        assert_eq!(t[6], Token::Float(0.5));
        assert_eq!(t[7], Token::RBracket);
    }

    #[test]
    fn test_leading_dot_float() {
        // .5 alone should be Float(0.5), not Dot + Int(5)
        let t = tokenize("/a[?b>.5]").unwrap();
        assert_eq!(
            t[..6],
            [
                Token::Slash,
                Token::Ident("a"),
                Token::LBracket,
                Token::Question,
                Token::Ident("b"),
                Token::Gt,
            ]
        );
        assert_eq!(t[6], Token::Float(0.5));
        assert_eq!(t[7], Token::RBracket);
    }

    #[test]
    fn test_negative_float() {
        let t = tokenize("/a[?x>-1.5]").unwrap();
        assert_eq!(t[5], Token::Gt);
        assert_eq!(t[6], Token::Float(-1.5));
        assert_eq!(t[7], Token::RBracket);
    }

    #[test]
    fn test_dot_separator_not_float() {
        // . followed by non-digit is still a path separator
        let t = tokenize("select(.x>1)").unwrap();
        assert_eq!(t[1], Token::LParen);
        assert_eq!(t[2], Token::Dot);
        assert_eq!(t[3], Token::Ident("x"));
    }

    #[test]
    fn test_ampersand_token() {
        let t = tokenize("[?x>1&y<10]").unwrap();
        assert_eq!(t[5], Token::Ampersand);
    }

    #[test]
    fn test_pipe_in_filter() {
        // | inside [? ] is OR, not pipe expression
        let t = tokenize("[?x>1|x<0]").unwrap();
        assert_eq!(t[5], Token::Pipe);
    }
}
