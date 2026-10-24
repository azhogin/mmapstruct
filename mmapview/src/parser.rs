use crate::tokenizer::Token;

#[derive(Debug, Clone)]
pub enum Expr<'a> {
    /// Root path: `"/sections"` → Path(["sections"])
    Path(Vec<Segment<'a>>),
    /// Pipe: `a | b`
    Pipe(Box<Expr<'a>>, Box<Expr<'a>>),
    /// Filter: `?id>10` or `?id>1&name~s`
    Filter(FilterExpr<'a>),
    /// Projection: `{id, name}`
    Projection(Vec<&'a str>),
    /// Index: `[0]`
    Index(usize),
    /// All: `[]`
    All,
    /// Slice: `[0:10]`
    Slice(usize, usize),
    /// Select call: `select(.name~sensor)`, `select(.x>1&y<10)`
    Select { filter: FilterExpr<'a> },
    /// Aggregate: `count()`, `avg(.field)`, `min(.field)`, `max(.field)`
    Aggregate { func: AggregateFunc<'a> },
}

/// Aggregation functions.
#[derive(Debug, Clone)]
pub enum AggregateFunc<'a> {
    Count,
    Avg { field: &'a str },
    Min { field: &'a str },
    Max { field: &'a str },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    StartsWith,
    EndsWith,
}

#[derive(Debug, Clone, Copy)]
pub enum FilterValue<'a> {
    Int(i64),
    Float(f64),
    Str(&'a str),
}

#[derive(Debug, Clone)]
pub enum Segment<'a> {
    Field(&'a str),
    Filter(FilterExpr<'a>),
    Index(usize),
    All,
    Slice(usize, usize),
}

#[derive(Debug, Clone)]
pub enum FilterExpr<'a> {
    Simple {
        field: &'a str,
        op: FilterOp,
        value: FilterValue<'a>,
    },
    And(Box<FilterExpr<'a>>, Box<FilterExpr<'a>>),
    Or(Box<FilterExpr<'a>>, Box<FilterExpr<'a>>),
}

impl<'a> FilterExpr<'a> {
    /// Convenience constructor for a simple field-op-value filter.
    pub fn simple(field: &'a str, op: FilterOp, value: FilterValue<'a>) -> Self {
        FilterExpr::Simple { field, op, value }
    }
}

/// Parse a token stream into an expression.
pub fn parse<'a>(tokens: &[Token<'a>]) -> Result<Expr<'a>, String> {
    parse_pipe(tokens, 0).map(|(expr, _)| expr)
}

fn parse_pipe<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(Expr<'a>, usize), String> {
    let (left, pos) = parse_path(tokens, pos)?;

    if pos < tokens.len() && tokens[pos] == Token::Pipe {
        let (right, pos) = parse_pipe(tokens, pos + 1)?;
        Ok((Expr::Pipe(Box::new(left), Box::new(right)), pos))
    } else {
        Ok((left, pos))
    }
}

fn parse_path<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(Expr<'a>, usize), String> {
    if pos >= tokens.len() {
        return Err("unexpected end of input".into());
    }

    // Function calls: select(...), avg(...), min(...), max(...), count()
    if let Token::Ident(name) = &tokens[pos] {
        if *name == "select" && tokens.get(pos + 1) == Some(&Token::LParen) {
            return parse_select(tokens, pos);
        }
        if tokens.get(pos + 1) == Some(&Token::LParen) {
            match *name {
                "avg" | "min" | "max" => return parse_aggregate(tokens, pos),
                "count" => return parse_aggregate(tokens, pos),
                _ => {}
            }
        }
    }

    match &tokens[pos] {
        Token::Slash => parse_path_after_slash(tokens, pos + 1),
        Token::Ident(_) => {
            let (field, pos) = expect_ident(tokens, pos)?;
            parse_trailing(tokens, pos, vec![Segment::Field(field)])
        }
        Token::LBracket => {
            // select(...) wrapped in [ ]
            let (expr, pos) = parse_select_or_filter(tokens, pos + 1)?;
            Ok((expr, pos))
        }
        _ => Err(format!("expected path or filter at token {}", pos)),
    }
}

fn parse_path_after_slash<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(Expr<'a>, usize), String> {
    if pos >= tokens.len() {
        // Bare `/` — return empty path (root value)
        return Ok((Expr::Path(vec![]), pos));
    }
    match &tokens[pos] {
        Token::Ident(_name) => {
            let (field, pos) = expect_ident(tokens, pos)?;
            parse_trailing(tokens, pos, vec![Segment::Field(field)])
        }
        Token::LBracket => {
            // Starting bracket right after slash: e.g. /[?id>10] or /[]
            parse_bracket(tokens, pos)
        }
        Token::Pipe => {
            // Pipe after bare slash: `" / | count()"` — return empty root path
            Ok((Expr::Path(vec![]), pos))
        }
        _ => Err(format!(
            "expected field name or `[` after `/`, got {:?}",
            tokens[pos]
        )),
    }
}

fn parse_trailing<'a>(
    tokens: &[Token<'a>],
    pos: usize,
    mut segs: Vec<Segment<'a>>,
) -> Result<(Expr<'a>, usize), String> {
    if pos >= tokens.len() {
        return Ok((Expr::Path(segs), pos));
    }

    match &tokens[pos] {
        Token::Slash => {
            // Nested path via /: sections/records
            let (rest, pos) = parse_trailing_field(tokens, pos + 1)?;
            segs.push(rest);
            parse_trailing(tokens, pos, segs)
        }
        Token::Dot => {
            // Nested field via . (e.g., select(.name~sensor))
            let (rest, pos) = parse_trailing_field(tokens, pos + 1)?;
            segs.push(rest);
            parse_trailing(tokens, pos, segs)
        }
        Token::LBracket => {
            let (seg, pos) = parse_bracket_segment(tokens, pos)?;
            segs.push(seg);
            parse_trailing(tokens, pos, segs)
        }
        _ => Ok((Expr::Path(segs), pos)),
    }
}

fn parse_trailing_field<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(Segment<'a>, usize), String> {
    match tokens.get(pos) {
        Some(Token::Ident(name)) => {
            let name = *name;
            Ok((Segment::Field(name), pos + 1))
        }
        Some(tok) => Err(format!("expected field name, got {:?}", tok)),
        None => Err("expected field name after `/` or `.`".into()),
    }
}

fn parse_bracket<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(Expr<'a>, usize), String> {
    let (seg, pos) = parse_bracket_segment(tokens, pos)?;
    let path = match seg {
        Segment::Filter(f) => Expr::Filter(f),
        Segment::Index(i) => Expr::Index(i),
        Segment::All => Expr::All,
        Segment::Slice(s, e) => Expr::Slice(s, e),
        Segment::Field(_) => return Err("field in bracket? this shouldn't happen".into()),
    };
    Ok((path, pos))
}

fn parse_bracket_segment<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(Segment<'a>, usize), String> {
    expect_token(tokens, pos, &Token::LBracket)?;
    let pos = pos + 1;

    if pos >= tokens.len() {
        return Err("unexpected end in bracket expression".into());
    }

    let result = match &tokens[pos] {
        Token::Question => {
            // Filter: [?field op value]
            let (filter, pos) = parse_filter(tokens, pos + 1)?;
            let pos = expect_token(tokens, pos, &Token::RBracket)?;
            (Segment::Filter(filter), pos)
        }
        Token::Int(n) if pos + 2 < tokens.len() && tokens[pos + 1] == Token::Colon => {
            // Slice: [start:end]
            let start = *n as usize;
            let end = match &tokens[pos + 2] {
                Token::Int(m) => *m as usize,
                _ => return Err("expected integer after `:` in slice".into()),
            };
            let pos = expect_token(tokens, pos + 3, &Token::RBracket)?;
            (Segment::Slice(start, end), pos)
        }
        Token::Int(n) => {
            // Index: [n]
            let idx = *n as usize;
            let pos = expect_token(tokens, pos + 1, &Token::RBracket)?;
            (Segment::Index(idx), pos)
        }
        Token::RBracket => {
            // All: []
            (Segment::All, pos + 1)
        }
        _ => return Err(format!("unexpected token {:?} in bracket", tokens[pos])),
    };

    Ok(result)
}

fn parse_filter<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(FilterExpr<'a>, usize), String> {
    // FilterExpr = OrExpr
    parse_or_expr(tokens, pos)
}

/// Parse `/points[?x>1|x<10]` — pipe-separated alternatives.
fn parse_or_expr<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(FilterExpr<'a>, usize), String> {
    let (left, pos) = parse_and_expr(tokens, pos)?;
    if pos < tokens.len() && tokens[pos] == Token::Pipe {
        let (right, pos) = parse_or_expr(tokens, pos + 1)?;
        Ok((FilterExpr::Or(Box::new(left), Box::new(right)), pos))
    } else {
        Ok((left, pos))
    }
}

/// Parse `/points[?x>1&x<10]` — ampersand-separated conjunctions.
fn parse_and_expr<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(FilterExpr<'a>, usize), String> {
    let (left, pos) = parse_simple_filter(tokens, pos)?;
    if pos < tokens.len() && tokens[pos] == Token::Ampersand {
        let (right, pos) = parse_and_expr(tokens, pos + 1)?;
        Ok((FilterExpr::And(Box::new(left), Box::new(right)), pos))
    } else {
        Ok((left, pos))
    }
}

/// Parse a single `ident op value` condition.
fn parse_simple_filter<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(FilterExpr<'a>, usize), String> {
    let (field, pos) = expect_filter_field(tokens, pos)?;
    let (op, pos) = expect_filter_op(tokens, pos)?;
    let (value, pos) = expect_filter_value(tokens, pos)?;
    Ok((FilterExpr::simple(field, op, value), pos))
}

fn parse_select_or_filter<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(Expr<'a>, usize), String> {
    match &tokens[pos] {
        Token::Question => {
            let (filter, pos) = parse_filter(tokens, pos + 1)?;
            let pos = expect_token(tokens, pos, &Token::RBracket)?;
            Ok((Expr::Filter(filter), pos))
        }
        _ => Err(format!("expected `?` after `[`, got {:?}", tokens[pos])),
    }
}

/// Parse `select(.field op value)` with optional surrounding brackets.
/// Supports `select(.x>1&y<10)` and `select(.x>1|y<0)`.
fn parse_select<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(Expr<'a>, usize), String> {
    // pos points to Ident("select")
    let pos = pos + 1; // skip "select"
    let pos = expect_token(tokens, pos, &Token::LParen)?; // skip (

    let (filter, pos) = parse_select_body(tokens, pos)?;
    let pos = expect_token(tokens, pos, &Token::RParen)?; // skip )
    Ok((Expr::Select { filter }, pos))
}

/// Parse the inside of `select(...)` — starts with `.field op value`, may continue with `&` or `|`.
fn parse_select_body<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(FilterExpr<'a>, usize), String> {
    let pos = expect_token(tokens, pos, &Token::Dot)?;
    let (field, pos) = expect_filter_field(tokens, pos)?;
    let (op, pos) = expect_filter_op(tokens, pos)?;
    let (value, pos) = expect_filter_value(tokens, pos)?;
    let mut filter = FilterExpr::simple(field, op, value);

    // Check for AND/OR continuation
    if pos < tokens.len() {
        match tokens[pos] {
            Token::Ampersand => {
                let (right, pos) = parse_select_body(tokens, pos + 1)?;
                filter = FilterExpr::And(Box::new(filter), Box::new(right));
                Ok((filter, pos))
            }
            Token::Pipe => {
                let (right, pos) = parse_select_body(tokens, pos + 1)?;
                filter = FilterExpr::Or(Box::new(filter), Box::new(right));
                Ok((filter, pos))
            }
            _ => Ok((filter, pos)),
        }
    } else {
        Ok((filter, pos))
    }
}

/// Parse `avg(.field)`, `min(.field)`, `max(.field)`, or `count()`.
fn parse_aggregate<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(Expr<'a>, usize), String> {
    let name = match &tokens[pos] {
        Token::Ident(n) => *n,
        _ => return Err("expected aggregate function name".into()),
    };
    let pos = pos + 1; // skip function name
    let pos = expect_token(tokens, pos, &Token::LParen)?;

    match name {
        "count" => {
            let pos = expect_token(tokens, pos, &Token::RParen)?;
            Ok((
                Expr::Aggregate {
                    func: AggregateFunc::Count,
                },
                pos,
            ))
        }
        "avg" | "min" | "max" => {
            let pos = expect_token(tokens, pos, &Token::Dot)?;
            let (field, pos) = expect_ident(tokens, pos)?;
            let pos = expect_token(tokens, pos, &Token::RParen)?;
            let func = match name {
                "avg" => AggregateFunc::Avg { field },
                "min" => AggregateFunc::Min { field },
                "max" => AggregateFunc::Max { field },
                _ => unreachable!(),
            };
            Ok((Expr::Aggregate { func }, pos))
        }
        _ => Err(format!("unknown aggregate function `{}`", name)),
    }
}

fn expect_ident<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(&'a str, usize), String> {
    match tokens.get(pos) {
        Some(Token::Ident(name)) => Ok((name, pos + 1)),
        Some(tok) => Err(format!("expected identifier, got {:?}", tok)),
        None => Err("expected identifier, got end of input".into()),
    }
}

/// Parse a filter field name: either a plain identifier or the `$variant`
/// pseudo-field (tokenized as `Dollar` + `Ident("variant")`).
fn expect_filter_field<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(&'a str, usize), String> {
    if tokens.get(pos) == Some(&Token::Dollar) {
        return match tokens.get(pos + 1) {
            Some(Token::Ident(name)) if *name == "variant" => Ok(("$variant", pos + 2)),
            Some(Token::Ident(name)) => Err(format!(
                "unknown pseudo-field `${name}` (only `$variant` is supported)"
            )),
            Some(tok) => Err(format!("expected identifier after `$`, got {:?}", tok)),
            None => Err("expected identifier after `$`, got end of input".into()),
        };
    }
    expect_ident(tokens, pos)
}

fn expect_filter_op<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(FilterOp, usize), String> {
    match tokens.get(pos) {
        Some(Token::Eq) => Ok((FilterOp::Eq, pos + 1)),
        Some(Token::Ne) => Ok((FilterOp::Ne, pos + 1)),
        Some(Token::Gt) => Ok((FilterOp::Gt, pos + 1)),
        Some(Token::Ge) => Ok((FilterOp::Ge, pos + 1)),
        Some(Token::Lt) => Ok((FilterOp::Lt, pos + 1)),
        Some(Token::Le) => Ok((FilterOp::Le, pos + 1)),
        Some(Token::Tilde) => Ok((FilterOp::Contains, pos + 1)),
        Some(Token::Caret) => Ok((FilterOp::StartsWith, pos + 1)),
        Some(Token::Dollar) => Ok((FilterOp::EndsWith, pos + 1)),
        Some(tok) => Err(format!("expected filter operator, got {:?}", tok)),
        None => Err("expected filter operator, got end of input".into()),
    }
}

fn expect_filter_value<'a>(
    tokens: &[Token<'a>],
    pos: usize,
) -> Result<(FilterValue<'a>, usize), String> {
    match tokens.get(pos) {
        Some(Token::Int(n)) => Ok((FilterValue::Int(*n), pos + 1)),
        Some(Token::Float(n)) => Ok((FilterValue::Float(*n), pos + 1)),
        Some(Token::Str(s)) => Ok((FilterValue::Str(s), pos + 1)),
        Some(Token::Ident(s)) => Ok((FilterValue::Str(s), pos + 1)),
        Some(tok) => Err(format!("expected filter value, got {:?}", tok)),
        None => Err("expected filter value, got end of input".into()),
    }
}

fn expect_token<'a>(
    tokens: &[Token<'a>],
    pos: usize,
    expected: &Token<'a>,
) -> Result<usize, String> {
    match tokens.get(pos) {
        Some(tok) if tok == expected => Ok(pos + 1),
        Some(tok) => Err(format!("expected {:?}, got {:?}", expected, tok)),
        None => Err(format!("expected {:?}, got end of input", expected)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::tokenize;

    #[test]
    fn test_parse_field_path() {
        let t = tokenize("/sections").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(e, Expr::Path(ref segs) if segs.len() == 1));
    }

    #[test]
    fn test_parse_filter() {
        let t = tokenize("/sections[?id>10]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                assert_eq!(segs.len(), 2);
                assert!(matches!(segs[0], Segment::Field("sections")));
                assert!(matches!(
                    segs[1],
                    Segment::Filter(FilterExpr::Simple {
                        field: "id",
                        op: FilterOp::Gt,
                        value: FilterValue::Int(10)
                    })
                ));
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_select_standalone() {
        let t = tokenize("select(.name~sensor)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Select {
                filter: FilterExpr::Simple {
                    field: "name",
                    op: FilterOp::Contains,
                    value: FilterValue::Str("sensor")
                }
            }
        ));
    }

    #[test]
    fn test_parse_dollar_variant_filter() {
        let t = tokenize("/events[?$variant=Probe]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                assert!(matches!(
                    segs[1],
                    Segment::Filter(FilterExpr::Simple {
                        field: "$variant",
                        op: FilterOp::Eq,
                        value: FilterValue::Str("Probe")
                    })
                ));
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_dollar_variant_quoted_full_name() {
        let t = tokenize(r#"/events[?$variant="Event::Probe"]"#).unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Path(segs)
                if matches!(
                    segs[1],
                    Segment::Filter(FilterExpr::Simple {
                        field: "$variant",
                        op: FilterOp::Eq,
                        value: FilterValue::Str("Event::Probe")
                    })
                )
        ));
    }

    #[test]
    fn test_parse_dollar_variant_select() {
        let t = tokenize("select(.$variant=Key&.id>1)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Select {
                filter: FilterExpr::And(left, right)
            }
            if matches!(left.as_ref(), FilterExpr::Simple {
                field: "$variant",
                op: FilterOp::Eq,
                value: FilterValue::Str("Key")
            }) && matches!(right.as_ref(), FilterExpr::Simple {
                field: "id",
                op: FilterOp::Gt,
                value: FilterValue::Int(1)
            })
        ));
    }

    #[test]
    fn test_parse_dollar_unknown_pseudo_field_errors() {
        let t = tokenize("/events[?$foo=1]").unwrap();
        assert!(parse(&t).is_err());
    }

    #[test]
    fn test_parse_select_after_pipe() {
        let t = tokenize("/sections | select(.id>10)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(&e, Expr::Pipe(left, right)
            if matches!(left.as_ref(), Expr::Path(_))
            && matches!(right.as_ref(), Expr::Select { filter: FilterExpr::Simple { field: "id", op: FilterOp::Gt, value: FilterValue::Int(10) } })));
    }

    #[test]
    fn test_parse_select_eq() {
        let t = tokenize("select(.name=target)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Select {
                filter: FilterExpr::Simple {
                    field: "name",
                    op: FilterOp::Eq,
                    value: FilterValue::Str("target")
                }
            }
        ));
    }

    #[test]
    fn test_parse_root_slash() {
        let t = tokenize("/").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(&e, Expr::Path(segs) if segs.is_empty()));
    }

    #[test]
    fn test_parse_negative_float_filter() {
        let t = tokenize("/a[?x>-1.5]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                assert_eq!(segs.len(), 2);
                assert!(matches!(segs[0], Segment::Field("a")));
                assert!(
                    matches!(segs[1], Segment::Filter(FilterExpr::Simple { field: "x", op: FilterOp::Gt, value: FilterValue::Float(v) }) if (v - (-1.5)).abs() < f64::EPSILON)
                );
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_leading_dot_float_filter() {
        let t = tokenize("/a[?x>.5]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                assert!(
                    matches!(segs[1], Segment::Filter(FilterExpr::Simple { field: "x", op: FilterOp::Gt, value: FilterValue::Float(v) }) if (v - 0.5).abs() < f64::EPSILON)
                );
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_and_filter() {
        let t = tokenize("/a[?x>1&y<10]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                assert_eq!(segs.len(), 2);
                assert!(matches!(segs[1], Segment::Filter(FilterExpr::And(_, _))));
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_or_filter() {
        let t = tokenize("/a[?x>1|x<0]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                assert!(matches!(segs[1], Segment::Filter(FilterExpr::Or(_, _))));
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_and_or_combined() {
        let t = tokenize("/a[?x>1&y<10|z=5]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                // Should parse as Or(And(x>1, y<10), z=5) — & binds tighter
                assert!(matches!(segs[1], Segment::Filter(FilterExpr::Or(_, _))));
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_and_or_precedence() {
        let t = tokenize("/a[?x>1|y<10&z=5]").unwrap();
        let e = parse(&t).unwrap();
        match e {
            Expr::Path(ref segs) => {
                // Should parse as Or(x>1, And(y<10, z=5)) — & binds tighter
                assert!(matches!(segs[1], Segment::Filter(FilterExpr::Or(_, _))));
                if let Segment::Filter(FilterExpr::Or(left, right)) = &segs[1] {
                    assert!(matches!(left.as_ref(), FilterExpr::Simple { .. }));
                    assert!(matches!(right.as_ref(), FilterExpr::And(_, _)));
                }
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn test_parse_count() {
        let t = tokenize("count()").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Aggregate {
                func: AggregateFunc::Count
            }
        ));
    }

    #[test]
    fn test_parse_avg() {
        let t = tokenize("avg(.price)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Aggregate {
                func: AggregateFunc::Avg { field: "price" }
            }
        ));
    }

    #[test]
    fn test_parse_min() {
        let t = tokenize("min(.x)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Aggregate {
                func: AggregateFunc::Min { field: "x" }
            }
        ));
    }

    #[test]
    fn test_parse_max() {
        let t = tokenize("max(.y)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(
            &e,
            Expr::Aggregate {
                func: AggregateFunc::Max { field: "y" }
            }
        ));
    }

    #[test]
    fn test_parse_pipe_count() {
        let t = tokenize("/items | count()").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(&e, Expr::Pipe(_, _)));
        if let Expr::Pipe(left, right) = &e {
            assert!(matches!(left.as_ref(), Expr::Path(_)));
            assert!(matches!(
                right.as_ref(),
                Expr::Aggregate {
                    func: AggregateFunc::Count
                }
            ));
        }
    }

    #[test]
    fn test_parse_pipe_avg() {
        let t = tokenize("/items | avg(.price)").unwrap();
        let e = parse(&t).unwrap();
        assert!(matches!(&e, Expr::Pipe(_, _)));
        if let Expr::Pipe(_left, right) = &e {
            assert!(matches!(
                right.as_ref(),
                Expr::Aggregate {
                    func: AggregateFunc::Avg { field: "price" }
                }
            ));
        }
    }
}
