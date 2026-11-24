//! Translation of row-level calculated-field expressions between Davilize's
//! DuckDB dialect and Tableau or DAX formulas.
//!
//! Each formula is lexed and parsed into a flat tree (calls with argument lists,
//! parentheses, and everything else as words), then rendered for the target
//! dialect. Nothing here evaluates anything: aggregates, table calculations, LOD
//! expressions, parameters and unknown functions are refused with a reason, which
//! callers turn into a warning.

use std::collections::HashMap;

/// The result of a translation: the new text, or why it cannot be translated.
pub type Translation = Result<String, String>;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Str(String),
    Field(Vec<String>),
    Num(String),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    Comma,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Str(String),
    Field(Vec<String>),
    Num(String),
    Word(String),
    Paren(Vec<Node>),
    Call { name: String, args: Vec<Vec<Node>> },
}

// ---------------------------------------------------------------- lexers

fn lex_error(message: &str) -> String {
    message.to_string()
}

fn read_number(chars: &[char], i: &mut usize) -> String {
    let start = *i;
    while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '.') {
        *i += 1;
    }
    if *i < chars.len() && (chars[*i] == 'e' || chars[*i] == 'E') {
        let mut j = *i + 1;
        if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
            j += 1;
        }
        if j < chars.len() && chars[j].is_ascii_digit() {
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            *i = j;
        }
    }
    chars[start..*i].iter().collect()
}

fn read_ident(chars: &[char], i: &mut usize) -> String {
    let start = *i;
    while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_') {
        *i += 1;
    }
    chars[start..*i].iter().collect()
}

/// Reads `open ... close` where a doubled close character is an escaped one.
fn read_delimited(chars: &[char], i: &mut usize, close: char) -> Result<String, String> {
    *i += 1;
    let mut out = String::new();
    loop {
        let Some(&c) = chars.get(*i) else {
            return Err(lex_error("unterminated quoted text"));
        };
        *i += 1;
        if c == close {
            if chars.get(*i) == Some(&close) {
                out.push(close);
                *i += 1;
            } else {
                return Ok(out);
            }
        } else {
            out.push(c);
        }
    }
}

/// Reads a string where a backslash also escapes the next character (Tableau).
fn read_backslash_string(chars: &[char], i: &mut usize) -> Result<String, String> {
    let close = chars[*i];
    *i += 1;
    let mut out = String::new();
    loop {
        let Some(&c) = chars.get(*i) else {
            return Err(lex_error("unterminated string"));
        };
        *i += 1;
        if c == '\\' {
            if let Some(&next) = chars.get(*i) {
                out.push(next);
                *i += 1;
            }
        } else if c == close {
            return Ok(out);
        } else {
            out.push(c);
        }
    }
}

const OPS: &[&str] = &[
    "==", "!=", "<>", "<=", ">=", "||", "&&", "//", "**", "+", "-", "*", "/", "%", "^", "<", ">",
    "=", "&", "{", "}", ":", ";",
];

fn read_op(chars: &[char], i: &mut usize) -> Result<Tok, String> {
    for op in OPS {
        let n = op.chars().count();
        if chars.len() >= *i + n && chars[*i..*i + n].iter().collect::<String>() == *op {
            *i += n;
            return Ok(Tok::Op((*op).to_string()));
        }
    }
    Err(format!("unexpected character '{}'", chars[*i]))
}

fn lex_tableau(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '"' || c == '\'' {
            toks.push(Tok::Str(read_backslash_string(&chars, &mut i)?));
        } else if c == '[' {
            let mut parts = vec![read_delimited(&chars, &mut i, ']')?];
            while chars.get(i) == Some(&'.') && chars.get(i + 1) == Some(&'[') {
                i += 1;
                parts.push(read_delimited(&chars, &mut i, ']')?);
            }
            toks.push(Tok::Field(parts));
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            toks.push(Tok::Num(read_number(&chars, &mut i)));
        } else if c.is_alphabetic() || c == '_' {
            toks.push(Tok::Ident(read_ident(&chars, &mut i)));
        } else if c == '(' {
            toks.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            toks.push(Tok::RParen);
            i += 1;
        } else if c == ',' {
            toks.push(Tok::Comma);
            i += 1;
        } else {
            toks.push(read_op(&chars, &mut i)?);
        }
    }
    Ok(toks)
}

fn lex_duck(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' {
            toks.push(Tok::Str(read_delimited(&chars, &mut i, '\'')?));
        } else if c == '"' {
            toks.push(Tok::Field(vec![read_delimited(&chars, &mut i, '"')?]));
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            toks.push(Tok::Num(read_number(&chars, &mut i)));
        } else if c.is_alphabetic() || c == '_' {
            toks.push(Tok::Ident(read_ident(&chars, &mut i)));
        } else if c == '(' {
            toks.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            toks.push(Tok::RParen);
            i += 1;
        } else if c == ',' {
            toks.push(Tok::Comma);
            i += 1;
        } else {
            toks.push(read_op(&chars, &mut i)?);
        }
    }
    Ok(toks)
}

fn lex_dax(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '"' {
            toks.push(Tok::Str(read_delimited(&chars, &mut i, '"')?));
        } else if c == '\'' {
            let table = read_delimited(&chars, &mut i, '\'')?;
            if chars.get(i) == Some(&'[') {
                let column = read_delimited(&chars, &mut i, ']')?;
                toks.push(Tok::Field(vec![table, column]));
            } else {
                return Err(lex_error(
                    "a quoted table name must be followed by [column]",
                ));
            }
        } else if c == '[' {
            toks.push(Tok::Field(vec![read_delimited(&chars, &mut i, ']')?]));
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            toks.push(Tok::Num(read_number(&chars, &mut i)));
        } else if c.is_alphabetic() || c == '_' {
            let name = read_ident(&chars, &mut i);
            if chars.get(i) == Some(&'[') {
                let column = read_delimited(&chars, &mut i, ']')?;
                toks.push(Tok::Field(vec![name, column]));
            } else {
                toks.push(Tok::Ident(name));
            }
        } else if c == '(' {
            toks.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            toks.push(Tok::RParen);
            i += 1;
        } else if c == ',' {
            toks.push(Tok::Comma);
            i += 1;
        } else {
            toks.push(read_op(&chars, &mut i)?);
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------- parser

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    keywords: &'a [&'a str],
}

impl Parser<'_> {
    fn seq(&mut self, depth: usize) -> Result<Vec<Node>, String> {
        if depth > 100 {
            return Err("expression is nested too deeply".into());
        }
        let mut nodes = Vec::new();
        while let Some(tok) = self.toks.get(self.pos).cloned() {
            match tok {
                Tok::RParen | Tok::Comma => break,
                Tok::LParen => {
                    self.pos += 1;
                    let inner = self.seq(depth + 1)?;
                    self.expect_close()?;
                    nodes.push(Node::Paren(inner));
                }
                Tok::Str(s) => {
                    self.pos += 1;
                    nodes.push(Node::Str(s));
                }
                Tok::Field(f) => {
                    self.pos += 1;
                    nodes.push(Node::Field(f));
                }
                Tok::Num(n) => {
                    self.pos += 1;
                    nodes.push(Node::Num(n));
                }
                Tok::Op(o) => {
                    self.pos += 1;
                    nodes.push(Node::Word(o));
                }
                Tok::Ident(name) => {
                    self.pos += 1;
                    let is_keyword = self.keywords.iter().any(|k| k.eq_ignore_ascii_case(&name));
                    if !is_keyword && self.toks.get(self.pos) == Some(&Tok::LParen) {
                        self.pos += 1;
                        let mut args = Vec::new();
                        if self.toks.get(self.pos) == Some(&Tok::RParen) {
                            self.pos += 1;
                        } else {
                            loop {
                                args.push(self.seq(depth + 1)?);
                                match self.toks.get(self.pos) {
                                    Some(Tok::Comma) => self.pos += 1,
                                    Some(Tok::RParen) => {
                                        self.pos += 1;
                                        break;
                                    }
                                    _ => return Err("unbalanced parentheses".into()),
                                }
                            }
                        }
                        nodes.push(Node::Call { name, args });
                    } else {
                        nodes.push(Node::Word(name));
                    }
                }
            }
        }
        Ok(nodes)
    }

    fn expect_close(&mut self) -> Result<(), String> {
        if self.toks.get(self.pos) == Some(&Tok::RParen) {
            self.pos += 1;
            Ok(())
        } else {
            Err("unbalanced parentheses".into())
        }
    }
}

fn parse(toks: Vec<Tok>, keywords: &[&str]) -> Result<Vec<Node>, String> {
    let mut parser = Parser {
        toks,
        pos: 0,
        keywords,
    };
    let nodes = parser.seq(0)?;
    if parser.pos < parser.toks.len() {
        return Err("unbalanced parentheses".into());
    }
    Ok(nodes)
}

const TABLEAU_KEYWORDS: &[&str] = &[
    "IF", "ELSEIF", "THEN", "ELSE", "END", "CASE", "WHEN", "AND", "OR", "NOT", "IN",
];
const DUCK_KEYWORDS: &[&str] = &[
    "CASE", "WHEN", "THEN", "ELSE", "END", "AND", "OR", "NOT", "IN", "IS", "LIKE", "ILIKE",
    "BETWEEN",
];

// ---------------------------------------------------------------- helpers

fn quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn word_is(node: &Node, text: &str) -> bool {
    matches!(node, Node::Word(w) if w.eq_ignore_ascii_case(text))
}

fn join_pieces(pieces: Vec<String>) -> String {
    pieces.join(" ")
}

fn args_text(
    args: &[Vec<Node>],
    render: &dyn Fn(&[Node]) -> Translation,
) -> Result<Vec<String>, String> {
    args.iter().map(|a| render(a)).collect()
}

fn arity(name: &str, args: &[String], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min || args.len() > max {
        Err(format!("{name} has the wrong number of arguments"))
    } else {
        Ok(())
    }
}

const TABLEAU_AGGREGATES: &[&str] = &[
    "SUM",
    "AVG",
    "COUNT",
    "COUNTD",
    "MEDIAN",
    "ATTR",
    "STDEV",
    "STDEVP",
    "VAR",
    "VARP",
    "TOTAL",
    "PERCENTILE",
    "CORR",
    "COVAR",
    "COLLECT",
    "WINDOW_SUM",
    "WINDOW_AVG",
    "WINDOW_MIN",
    "WINDOW_MAX",
    "WINDOW_COUNT",
    "WINDOW_MEDIAN",
    "RUNNING_SUM",
    "RUNNING_AVG",
    "RUNNING_MIN",
    "RUNNING_MAX",
    "RUNNING_COUNT",
    "RANK",
    "RANK_DENSE",
    "RANK_UNIQUE",
    "RANK_MODIFIED",
    "RANK_PERCENTILE",
    "LOOKUP",
    "INDEX",
    "FIRST",
    "LAST",
    "SIZE",
    "PREVIOUS_VALUE",
    "FIXED",
    "INCLUDE",
    "EXCLUDE",
];

const DAX_AGGREGATES: &[&str] = &[
    "SUM",
    "AVERAGE",
    "COUNT",
    "COUNTA",
    "COUNTROWS",
    "DISTINCTCOUNT",
    "MIN",
    "MAX",
    "CALCULATE",
    "SUMX",
    "AVERAGEX",
    "MINX",
    "MAXX",
    "COUNTX",
    "FILTER",
    "ALL",
    "ALLEXCEPT",
    "VALUES",
    "DISTINCT",
    "EARLIER",
    "RANKX",
    "TOTALYTD",
    "SAMEPERIODLASTYEAR",
    "DATESYTD",
    "LOOKUPVALUE",
    "SELECTEDVALUE",
    "RELATEDTABLE",
    "SUMMARIZE",
    "ADDCOLUMNS",
    "TOPN",
];

// ------------------------------------------------------- Tableau -> DuckDB

/// Maps Tableau column names (such as `Calculation_123`) to Davilize field names.
pub type NameMap = HashMap<String, String>;

/// Translates a Tableau calculation into a DuckDB calculated-field expression.
pub fn tableau_to_duckdb(formula: &str, names: &NameMap) -> Translation {
    let nodes = parse(lex_tableau(formula)?, TABLEAU_KEYWORDS)?;
    if nodes.is_empty() {
        return Err("the formula is empty".into());
    }
    t2d(&nodes, names)
}

fn t2d(nodes: &[Node], names: &NameMap) -> Translation {
    let mut pieces: Vec<String> = Vec::new();
    for node in nodes {
        let piece = match node {
            Node::Str(s) => quote_str(s),
            Node::Num(n) => n.clone(),
            Node::Field(parts) => {
                if parts.len() > 1 && parts[0].eq_ignore_ascii_case("Parameters") {
                    return Err("uses a Tableau parameter, which calculated fields cannot".into());
                }
                let raw = parts.last().cloned().unwrap_or_default();
                quote_ident(names.get(&raw).unwrap_or(&raw))
            }
            Node::Paren(inner) => format!("({})", t2d(inner, names)?),
            Node::Word(w) => match w.to_ascii_uppercase().as_str() {
                "IF" => "CASE WHEN".to_string(),
                "ELSEIF" => "WHEN".to_string(),
                "==" => "=".to_string(),
                "{" | "}" => return Err("uses a level-of-detail expression".into()),
                "&&" | "||" | "&" | ":" | ";" => {
                    return Err(format!("uses unsupported operator {w}"));
                }
                up @ ("AND" | "OR" | "NOT" | "THEN" | "ELSE" | "END" | "CASE" | "WHEN" | "IN"
                | "NULL" | "TRUE" | "FALSE") => up.to_string(),
                _ => w.clone(),
            },
            Node::Call { name, args } => {
                let rendered = args_text(args, &|a| t2d(a, names))?;
                tableau_call(name, &rendered)?
            }
        };
        pieces.push(piece);
    }
    concat_fix(&mut pieces);
    Ok(join_pieces(pieces))
}

/// Tableau uses `+` for text; once a neighbour is a string literal, use `||`.
fn concat_fix(pieces: &mut [String]) {
    for i in 1..pieces.len().saturating_sub(1) {
        if pieces[i] == "+" {
            let text_side = |p: &String| p.starts_with('\'') && p.ends_with('\'') && p.len() >= 2;
            if text_side(&pieces[i - 1]) || text_side(&pieces[i + 1]) {
                pieces[i] = "||".to_string();
            }
        }
    }
}

fn tableau_call(name: &str, a: &[String]) -> Translation {
    let up = name.to_ascii_uppercase();
    let simple = |target: &str, min: usize, max: usize| -> Translation {
        arity(name, a, min, max)?;
        Ok(format!("{target}({})", a.join(", ")))
    };
    match up.as_str() {
        "LEN" => simple("length", 1, 1),
        "UPPER" => simple("upper", 1, 1),
        "LOWER" => simple("lower", 1, 1),
        "TRIM" => simple("trim", 1, 1),
        "LTRIM" => simple("ltrim", 1, 1),
        "RTRIM" => simple("rtrim", 1, 1),
        "LEFT" => simple("left", 2, 2),
        "RIGHT" => simple("right", 2, 2),
        "REPLACE" => simple("replace", 3, 3),
        "MID" => simple("substring", 2, 3),
        "CONTAINS" => simple("contains", 2, 2),
        "STARTSWITH" => simple("starts_with", 2, 2),
        "ENDSWITH" => simple("ends_with", 2, 2),
        "ABS" => simple("abs", 1, 1),
        "ROUND" => simple("round", 1, 2),
        "CEILING" => simple("ceil", 1, 1),
        "FLOOR" => simple("floor", 1, 1),
        "SQRT" => simple("sqrt", 1, 1),
        "POWER" => simple("power", 2, 2),
        "EXP" => simple("exp", 1, 1),
        "LN" => simple("ln", 1, 1),
        "SIGN" => simple("sign", 1, 1),
        "PI" => simple("pi", 0, 0),
        "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" | "DEGREES" | "RADIANS" | "ATAN2" => {
            Ok(format!("{}({})", up.to_ascii_lowercase(), a.join(", ")))
        }
        "YEAR" | "MONTH" | "DAY" | "HOUR" | "MINUTE" | "SECOND" | "QUARTER" | "WEEK" => {
            Ok(format!("{}({})", up.to_ascii_lowercase(), a.join(", ")))
        }
        "IFNULL" => simple("ifnull", 2, 2),
        "IIF" => {
            arity(name, a, 3, 4)?;
            Ok(format!("iif({}, {}, {})", a[0], a[1], a[2]))
        }
        "DATEDIFF" => simple("datediff", 3, 4),
        "DATETRUNC" => simple("date_trunc", 2, 3),
        "ZN" => {
            arity(name, a, 1, 1)?;
            Ok(format!("coalesce({}, 0)", a[0]))
        }
        "ISNULL" => {
            arity(name, a, 1, 1)?;
            Ok(format!("({} IS NULL)", a[0]))
        }
        "STR" => {
            arity(name, a, 1, 1)?;
            Ok(format!("CAST({} AS VARCHAR)", a[0]))
        }
        "INT" => {
            arity(name, a, 1, 1)?;
            Ok(format!("CAST(trunc({}) AS BIGINT)", a[0]))
        }
        "FLOAT" => {
            arity(name, a, 1, 1)?;
            Ok(format!("CAST({} AS DOUBLE)", a[0]))
        }
        "DATE" => {
            arity(name, a, 1, 1)?;
            Ok(format!("CAST({} AS DATE)", a[0]))
        }
        "SQUARE" => {
            arity(name, a, 1, 1)?;
            Ok(format!("power({}, 2)", a[0]))
        }
        "DIV" => {
            arity(name, a, 2, 2)?;
            Ok(format!("({} // {})", a[0], a[1]))
        }
        "LOG" => {
            arity(name, a, 1, 2)?;
            if a.len() == 1 {
                Ok(format!("log10({})", a[0]))
            } else {
                Ok(format!("(ln({}) / ln({}))", a[0], a[1]))
            }
        }
        "MAX" | "MIN" if a.len() == 2 => Ok(format!(
            "{}({}, {})",
            if up == "MAX" { "greatest" } else { "least" },
            a[0],
            a[1]
        )),
        "TODAY" | "NOW" => Err("uses the clock, which calculated fields do not allow".into()),
        _ if up == "MAX" || up == "MIN" || TABLEAU_AGGREGATES.contains(&up.as_str()) => {
            Err("uses an aggregate, level-of-detail or table calculation".into())
        }
        _ => Err(format!("uses unsupported function {name}")),
    }
}

/// When a Tableau calculation is exactly `AGG([field])`, returns the aggregate
/// and the field, so it can become a measure on a visual.
pub fn simple_tableau_aggregate(formula: &str) -> Option<(dv_core::Aggregate, String)> {
    let nodes = parse(lex_tableau(formula).ok()?, TABLEAU_KEYWORDS).ok()?;
    let [Node::Call { name, args }] = nodes.as_slice() else {
        return None;
    };
    let [arg] = args.as_slice() else {
        return None;
    };
    let [Node::Field(parts)] = arg.as_slice() else {
        return None;
    };
    let agg = match name.to_ascii_uppercase().as_str() {
        "SUM" => dv_core::Aggregate::Sum,
        "AVG" => dv_core::Aggregate::Avg,
        "MIN" => dv_core::Aggregate::Min,
        "MAX" => dv_core::Aggregate::Max,
        "COUNT" => dv_core::Aggregate::Count,
        "COUNTD" => dv_core::Aggregate::CountDistinct,
        _ => return None,
    };
    Some((agg, parts.last()?.clone()))
}

/// The Tableau column names a formula refers to (for ordering calculations).
pub fn tableau_references(formula: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(toks) = lex_tableau(formula) {
        for tok in toks {
            if let Tok::Field(parts) = tok
                && let Some(last) = parts.last()
            {
                out.push(last.clone());
            }
        }
    }
    out
}

// ------------------------------------------------------- DuckDB -> Tableau

/// Translates a DuckDB calculated-field expression into a Tableau calculation.
pub fn duckdb_to_tableau(expression: &str) -> Translation {
    let nodes = parse(lex_duck(expression)?, DUCK_KEYWORDS)?;
    d2t(&nodes)
}

fn tableau_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn tableau_field(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

fn d2t(nodes: &[Node]) -> Translation {
    let mut pieces: Vec<String> = Vec::new();
    let mut case_stack: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        let node = &nodes[i];
        i += 1;
        let piece = match node {
            Node::Str(s) => tableau_string(s),
            Node::Num(n) => n.clone(),
            Node::Field(parts) => tableau_field(parts.last().map_or("", String::as_str)),
            Node::Paren(inner) => format!("({})", d2t(inner)?),
            Node::Call { name, args } => {
                if name.eq_ignore_ascii_case("cast") {
                    tableau_cast(args)?
                } else {
                    let rendered = args_text(args, &d2t)?;
                    duck_call_to_tableau(name, &rendered)?
                }
            }
            Node::Word(w) => {
                let up = w.to_ascii_uppercase();
                match up.as_str() {
                    "CASE" => {
                        let when_form = nodes.get(i).is_some_and(|n| word_is(n, "WHEN"));
                        case_stack.push(when_form);
                        if when_form {
                            i += 1;
                            "IF".to_string()
                        } else {
                            "CASE".to_string()
                        }
                    }
                    "WHEN" => {
                        if case_stack.last() == Some(&true) {
                            "ELSEIF".to_string()
                        } else {
                            "WHEN".to_string()
                        }
                    }
                    "END" => {
                        case_stack.pop();
                        "END".to_string()
                    }
                    "THEN" | "ELSE" | "AND" | "OR" | "IN" | "NULL" | "TRUE" | "FALSE" => up,
                    "NOT" => "NOT".to_string(),
                    "IS" => {
                        let negated = nodes.get(i).is_some_and(|n| word_is(n, "NOT"));
                        let null_at = if negated { i + 1 } else { i };
                        if !nodes.get(null_at).is_some_and(|n| word_is(n, "NULL")) {
                            return Err("uses IS with something other than NULL".into());
                        }
                        i = null_at + 1;
                        let subject = pieces.pop().ok_or("IS NULL without a value")?;
                        if negated {
                            format!("NOT ISNULL({subject})")
                        } else {
                            format!("ISNULL({subject})")
                        }
                    }
                    "||" => "+".to_string(),
                    "<>" | "!=" => "!=".to_string(),
                    "=" | "==" => "=".to_string(),
                    "+" | "-" | "*" | "/" | "<" | ">" | "<=" | ">=" | "^" | "%" => w.clone(),
                    "LIKE" | "ILIKE" | "BETWEEN" => {
                        return Err(format!("uses {up}, which Tableau formulas do not have"));
                    }
                    "//" | "**" | "&&" | "&" => {
                        return Err(format!("uses operator {w}, which has no Tableau form"));
                    }
                    _ if w.chars().all(|c| c.is_alphanumeric() || c == '_') => tableau_field(w),
                    _ => return Err(format!("uses unsupported syntax '{w}'")),
                }
            }
        };
        pieces.push(piece);
    }
    Ok(join_pieces(pieces))
}

fn cast_target(args: &[Vec<Node>]) -> Result<(Vec<Node>, String), String> {
    let [arg] = args else {
        return Err("CAST has the wrong number of arguments".into());
    };
    let at = arg
        .iter()
        .rposition(|n| word_is(n, "AS"))
        .ok_or("CAST without AS")?;
    let ty = arg[at + 1..]
        .iter()
        .map(|n| match n {
            Node::Word(w) => w.to_ascii_uppercase(),
            Node::Call { name, .. } => name.to_ascii_uppercase(),
            _ => String::new(),
        })
        .collect::<String>();
    Ok((arg[..at].to_vec(), ty))
}

fn tableau_cast(args: &[Vec<Node>]) -> Translation {
    let (value, ty) = cast_target(args)?;
    let inner = d2t(&value)?;
    let func = match ty.as_str() {
        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "INT" => "INT",
        "FLOAT" | "REAL" | "DOUBLE" | "DECIMAL" | "NUMERIC" => "FLOAT",
        "VARCHAR" | "TEXT" | "STRING" | "CHAR" => "STR",
        "DATE" => "DATE",
        "TIMESTAMP" => "DATETIME",
        other => {
            return Err(format!(
                "casts to {other}, which Tableau has no function for"
            ));
        }
    };
    Ok(format!("{func}({inner})"))
}

fn duck_call_to_tableau(name: &str, a: &[String]) -> Translation {
    let low = name.to_ascii_lowercase();
    let same = |target: &str, min: usize, max: usize| -> Translation {
        arity(name, a, min, max)?;
        Ok(format!("{target}({})", a.join(", ")))
    };
    match low.as_str() {
        "length" | "char_length" => same("LEN", 1, 1),
        "upper" | "lower" | "trim" | "ltrim" | "rtrim" | "abs" | "sqrt" | "exp" | "ln" | "sign"
        | "floor" => same(&low.to_ascii_uppercase(), 1, 1),
        "left" | "right" | "contains" | "power" | "atan2" => same(&low.to_ascii_uppercase(), 2, 2),
        "pow" => same("POWER", 2, 2),
        "replace" => same("REPLACE", 3, 3),
        "round" => same("ROUND", 1, 2),
        "ceil" | "ceiling" => same("CEILING", 1, 1),
        "starts_with" => same("STARTSWITH", 2, 2),
        "ends_with" => same("ENDSWITH", 2, 2),
        "substring" | "substr" => same("MID", 2, 3),
        "log10" => same("LOG", 1, 1),
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "degrees" | "radians" | "pi" => {
            same(&low.to_ascii_uppercase(), 0, 1)
        }
        "year" | "month" | "day" | "hour" | "minute" | "second" | "quarter" | "week" => {
            same(&low.to_ascii_uppercase(), 1, 1)
        }
        "ifnull" => same("IFNULL", 2, 2),
        "coalesce" => {
            arity(name, a, 1, usize::MAX)?;
            let mut out = a[a.len() - 1].clone();
            for item in a[..a.len() - 1].iter().rev() {
                out = format!("IFNULL({item}, {out})");
            }
            Ok(out)
        }
        "nullif" => {
            arity(name, a, 2, 2)?;
            Ok(format!("IIF({} = {}, NULL, {})", a[0], a[1], a[0]))
        }
        "if" | "iif" => same("IIF", 3, 3),
        "greatest" => same("MAX", 2, 2),
        "least" => same("MIN", 2, 2),
        "date_trunc" => same("DATETRUNC", 2, 2),
        "datediff" | "date_diff" => same("DATEDIFF", 3, 3),
        "mod" => {
            arity(name, a, 2, 2)?;
            Ok(format!("({} % {})", a[0], a[1]))
        }
        "concat" => {
            arity(name, a, 1, usize::MAX)?;
            Ok(format!("({})", a.join(" + ")))
        }
        _ => Err(format!(
            "uses function {name}, which has no Tableau equivalent"
        )),
    }
}

// ------------------------------------------------------------ DAX -> DuckDB

/// Translates a DAX calculated-column formula into a DuckDB expression.
pub fn dax_to_duckdb(formula: &str) -> Translation {
    let nodes = parse(lex_dax(formula)?, &[])?;
    if nodes.is_empty() {
        return Err("the formula is empty".into());
    }
    x2d(&nodes)
}

fn x2d(nodes: &[Node]) -> Translation {
    let mut pieces: Vec<String> = Vec::new();
    for node in nodes {
        let piece = match node {
            Node::Str(s) => quote_str(s),
            Node::Num(n) => n.clone(),
            Node::Field(parts) => quote_ident(parts.last().map_or("", String::as_str)),
            Node::Paren(inner) => format!("({})", x2d(inner)?),
            Node::Word(w) => match w.to_ascii_uppercase().as_str() {
                "&" => "||".to_string(),
                "&&" => "AND".to_string(),
                "||" => "OR".to_string(),
                "IN" => return Err("uses IN, which has no direct translation".into()),
                "VAR" | "RETURN" | "{" | "}" => {
                    return Err("uses variables or table constructors".into());
                }
                "+" | "-" | "*" | "/" | "^" | "=" | "<>" | "<" | ">" | "<=" | ">=" => w.clone(),
                _ => return Err(format!("uses unsupported syntax '{w}'")),
            },
            Node::Call { name, args } => {
                let rendered = args_text(args, &x2d)?;
                dax_call(name, args, &rendered)?
            }
        };
        pieces.push(piece);
    }
    Ok(join_pieces(pieces))
}

fn dax_call(name: &str, raw: &[Vec<Node>], a: &[String]) -> Translation {
    let up = name.to_ascii_uppercase();
    let simple = |target: &str, min: usize, max: usize| -> Translation {
        arity(name, a, min, max)?;
        Ok(format!("{target}({})", a.join(", ")))
    };
    match up.as_str() {
        "IF" => {
            arity(name, a, 2, 3)?;
            if a.len() == 3 {
                Ok(format!(
                    "CASE WHEN {} THEN {} ELSE {} END",
                    a[0], a[1], a[2]
                ))
            } else {
                Ok(format!("CASE WHEN {} THEN {} END", a[0], a[1]))
            }
        }
        "SWITCH" => {
            arity(name, a, 3, usize::MAX)?;
            let on_true = matches!(raw[0].as_slice(), [Node::Call { name, args }] if name.eq_ignore_ascii_case("TRUE") && args.is_empty());
            let rest = &a[1..];
            let has_else = rest.len() % 2 == 1;
            let pairs = rest.len() / 2;
            let mut out = if on_true {
                "CASE".to_string()
            } else {
                format!("CASE {}", a[0])
            };
            for p in 0..pairs {
                out.push_str(&format!(" WHEN {} THEN {}", rest[2 * p], rest[2 * p + 1]));
            }
            if has_else {
                out.push_str(&format!(" ELSE {}", rest[rest.len() - 1]));
            }
            out.push_str(" END");
            Ok(out)
        }
        "BLANK" => Ok("NULL".into()),
        "TRUE" => Ok("TRUE".into()),
        "FALSE" => Ok("FALSE".into()),
        "ISBLANK" => {
            arity(name, a, 1, 1)?;
            Ok(format!("({} IS NULL)", a[0]))
        }
        "NOT" => {
            arity(name, a, 1, 1)?;
            Ok(format!("(NOT ({}))", a[0]))
        }
        "AND" => {
            arity(name, a, 2, 2)?;
            Ok(format!("(({}) AND ({}))", a[0], a[1]))
        }
        "OR" => {
            arity(name, a, 2, 2)?;
            Ok(format!("(({}) OR ({}))", a[0], a[1]))
        }
        "DIVIDE" => {
            arity(name, a, 2, 3)?;
            let base = format!("(({}) / nullif(({}), 0))", a[0], a[1]);
            Ok(if a.len() == 3 {
                format!("coalesce({base}, {})", a[2])
            } else {
                base
            })
        }
        "LEN" => simple("length", 1, 1),
        "UPPER" => simple("upper", 1, 1),
        "LOWER" => simple("lower", 1, 1),
        "TRIM" => simple("trim", 1, 1),
        "LEFT" => simple("left", 1, 2),
        "RIGHT" => simple("right", 1, 2),
        "MID" => simple("substring", 3, 3),
        "SUBSTITUTE" => simple("replace", 3, 3),
        "ABS" => simple("abs", 1, 1),
        "ROUND" => simple("round", 2, 2),
        "FLOOR" => simple("floor", 1, 1),
        "CEILING" => simple("ceil", 1, 1),
        "SQRT" => simple("sqrt", 1, 1),
        "POWER" => simple("power", 2, 2),
        "EXP" => simple("exp", 1, 1),
        "LN" => simple("ln", 1, 1),
        "LOG10" => simple("log10", 1, 1),
        "SIGN" => simple("sign", 1, 1),
        "MOD" => simple("mod", 2, 2),
        "YEAR" | "MONTH" | "DAY" | "HOUR" | "MINUTE" | "SECOND" | "QUARTER" => {
            simple(&up.to_ascii_lowercase(), 1, 1)
        }
        "WEEKNUM" => simple("week", 1, 2),
        "COALESCE" => simple("coalesce", 1, usize::MAX),
        "INT" => {
            arity(name, a, 1, 1)?;
            Ok(format!("CAST(floor({}) AS BIGINT)", a[0]))
        }
        "CONCATENATE" => {
            arity(name, a, 2, 2)?;
            Ok(format!("({} || {})", a[0], a[1]))
        }
        "DATE" => simple("make_date", 3, 3),
        "RELATED" => {
            arity(name, a, 1, 1)?;
            Ok(a[0].clone())
        }
        "DATEDIFF" => {
            arity(name, a, 3, 3)?;
            let unit = match raw[2].as_slice() {
                [Node::Word(w)] => w.to_ascii_lowercase(),
                _ => return Err("DATEDIFF needs a unit such as DAY".into()),
            };
            Ok(format!("datediff('{unit}', {}, {})", a[0], a[1]))
        }
        "TODAY" | "NOW" | "UTCNOW" | "UTCTODAY" => {
            Err("uses the clock, which calculated fields do not allow".into())
        }
        _ if DAX_AGGREGATES.contains(&up.as_str()) => {
            Err("uses an aggregate, filter-context or table function".into())
        }
        _ => Err(format!("uses unsupported function {name}")),
    }
}

/// When a DAX measure is exactly `SUM(Table[Column])` (or AVERAGE, MIN, MAX, COUNT,
/// DISTINCTCOUNT), returns the aggregate and the column name.
pub fn simple_dax_aggregate(formula: &str) -> Option<(dv_core::Aggregate, String)> {
    let nodes = parse(lex_dax(formula).ok()?, &[]).ok()?;
    let [Node::Call { name, args }] = nodes.as_slice() else {
        return None;
    };
    let [arg] = args.as_slice() else {
        return None;
    };
    let [Node::Field(parts)] = arg.as_slice() else {
        return None;
    };
    let agg = match name.to_ascii_uppercase().as_str() {
        "SUM" => dv_core::Aggregate::Sum,
        "AVERAGE" => dv_core::Aggregate::Avg,
        "MIN" => dv_core::Aggregate::Min,
        "MAX" => dv_core::Aggregate::Max,
        "COUNT" | "COUNTA" => dv_core::Aggregate::Count,
        "DISTINCTCOUNT" => dv_core::Aggregate::CountDistinct,
        _ => return None,
    };
    Some((agg, parts.last()?.clone()))
}

// ------------------------------------------------------------ DuckDB -> DAX

/// Translates a DuckDB expression into a DAX calculated-column formula.
pub fn duckdb_to_dax(expression: &str) -> Translation {
    let nodes = parse(lex_duck(expression)?, DUCK_KEYWORDS)?;
    d2x(&nodes)
}

fn dax_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn dax_field(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

fn d2x(nodes: &[Node]) -> Translation {
    let mut pieces: Vec<String> = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        let node = &nodes[i];
        i += 1;
        let piece = match node {
            Node::Str(s) => dax_string(s),
            Node::Num(n) => n.clone(),
            Node::Field(parts) => dax_field(parts.last().map_or("", String::as_str)),
            Node::Paren(inner) => format!("({})", d2x(inner)?),
            Node::Call { name, args } => {
                if name.eq_ignore_ascii_case("cast") {
                    let (value, ty) = cast_target(args)?;
                    let inner = d2x(&value)?;
                    let target = match ty.as_str() {
                        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "INT" => {
                            "INTEGER"
                        }
                        "FLOAT" | "REAL" | "DOUBLE" | "DECIMAL" | "NUMERIC" => "DOUBLE",
                        "VARCHAR" | "TEXT" | "STRING" | "CHAR" => "STRING",
                        "DATE" | "TIMESTAMP" => "DATETIME",
                        "BOOLEAN" => "BOOLEAN",
                        other => return Err(format!("casts to {other}, which DAX cannot do")),
                    };
                    format!("CONVERT({inner}, {target})")
                } else {
                    let rendered = args_text(args, &d2x)?;
                    duck_call_to_dax(name, &rendered)?
                }
            }
            Node::Word(w) => {
                let up = w.to_ascii_uppercase();
                match up.as_str() {
                    "CASE" => {
                        let when_form = nodes.get(i).is_some_and(|n| word_is(n, "WHEN"));
                        if when_form {
                            i += 1;
                            "SWITCH(TRUE(),".to_string()
                        } else {
                            "SWITCH(".to_string()
                        }
                    }
                    "WHEN" | "THEN" | "ELSE" => ",".to_string(),
                    "END" => ")".to_string(),
                    "AND" => "&&".to_string(),
                    "OR" => "||".to_string(),
                    "NULL" => "BLANK()".to_string(),
                    "TRUE" => "TRUE()".to_string(),
                    "FALSE" => "FALSE()".to_string(),
                    "||" => "&".to_string(),
                    "<>" | "!=" => "<>".to_string(),
                    "=" | "==" => "=".to_string(),
                    "+" | "-" | "*" | "/" | "<" | ">" | "<=" | ">=" | "^" => w.clone(),
                    "IS" => {
                        let negated = nodes.get(i).is_some_and(|n| word_is(n, "NOT"));
                        let null_at = if negated { i + 1 } else { i };
                        if !nodes.get(null_at).is_some_and(|n| word_is(n, "NULL")) {
                            return Err("uses IS with something other than NULL".into());
                        }
                        i = null_at + 1;
                        let subject = pieces.pop().ok_or("IS NULL without a value")?;
                        if negated {
                            format!("NOT(ISBLANK({subject}))")
                        } else {
                            format!("ISBLANK({subject})")
                        }
                    }
                    "IN" => match nodes.get(i) {
                        Some(Node::Paren(inner)) => {
                            i += 1;
                            format!("IN {{ {} }}", d2x(inner)?)
                        }
                        _ => return Err("uses IN without a list".into()),
                    },
                    "NOT" => return Err("uses NOT, which has no operator form in DAX".into()),
                    "LIKE" | "ILIKE" | "BETWEEN" | "%" | "//" | "**" | "&&" | "&" => {
                        return Err(format!("uses {up}, which has no DAX operator form"));
                    }
                    _ if w.chars().all(|c| c.is_alphanumeric() || c == '_') => dax_field(w),
                    _ => return Err(format!("uses unsupported syntax '{w}'")),
                }
            }
        };
        pieces.push(piece);
    }
    // Commas and closing parentheses attach to what precedes them.
    let mut out = String::new();
    for piece in pieces {
        let attach = piece == "," || piece == ")" || out.ends_with('(') || out.is_empty();
        if !attach {
            out.push(' ');
        }
        out.push_str(&piece);
    }
    Ok(out)
}

fn duck_call_to_dax(name: &str, a: &[String]) -> Translation {
    let low = name.to_ascii_lowercase();
    let same = |target: &str, min: usize, max: usize| -> Translation {
        arity(name, a, min, max)?;
        Ok(format!("{target}({})", a.join(", ")))
    };
    match low.as_str() {
        "length" | "char_length" => same("LEN", 1, 1),
        "upper" | "lower" | "trim" | "abs" | "sqrt" | "exp" | "ln" | "sign" | "floor" => {
            same(&low.to_ascii_uppercase(), 1, 1)
        }
        "left" | "right" => same(&low.to_ascii_uppercase(), 1, 2),
        "pow" | "power" => same("POWER", 2, 2),
        "round" => {
            if a.len() == 1 {
                Ok(format!("ROUND({}, 0)", a[0]))
            } else {
                same("ROUND", 2, 2)
            }
        }
        "ceil" | "ceiling" => same("CEILING", 1, 1),
        "log10" => same("LOG10", 1, 1),
        "mod" => same("MOD", 2, 2),
        "replace" => same("SUBSTITUTE", 3, 3),
        "substring" | "substr" => same("MID", 3, 3),
        "year" | "month" | "day" | "hour" | "minute" | "second" | "quarter" => {
            same(&low.to_ascii_uppercase(), 1, 1)
        }
        "coalesce" => same("COALESCE", 1, usize::MAX),
        "ifnull" => same("COALESCE", 2, 2),
        "if" | "iif" => same("IF", 3, 3),
        "nullif" => {
            arity(name, a, 2, 2)?;
            Ok(format!("IF({} = {}, BLANK(), {})", a[0], a[1], a[0]))
        }
        "concat" => {
            arity(name, a, 1, usize::MAX)?;
            Ok(format!("({})", a.join(" & ")))
        }
        "make_date" => same("DATE", 3, 3),
        _ => Err(format!("uses function {name}, which has no DAX equivalent")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> NameMap {
        NameMap::from([("Calculation_1".to_string(), "profit".to_string())])
    }

    #[test]
    fn tableau_if_becomes_case() {
        let out = tableau_to_duckdb(
            "IF [Sales] > 100 THEN \"big\" ELSEIF [Sales] > 10 THEN \"mid\" ELSE \"small\" END",
            &names(),
        )
        .unwrap();
        assert_eq!(
            out,
            "CASE WHEN \"Sales\" > 100 THEN 'big' WHEN \"Sales\" > 10 THEN 'mid' ELSE 'small' END"
        );
    }

    #[test]
    fn tableau_functions_fields_and_concat() {
        assert_eq!(
            tableau_to_duckdb("UPPER([Region]) + \"-\" + STR([Calculation_1])", &names()).unwrap(),
            "upper(\"Region\") || '-' || CAST(\"profit\" AS VARCHAR)"
        );
        assert_eq!(
            tableau_to_duckdb("ZN([A]) + ISNULL([B])", &names()).unwrap(),
            "coalesce(\"A\", 0) + (\"B\" IS NULL)"
        );
        assert_eq!(
            tableau_to_duckdb("MAX([A], [B])", &names()).unwrap(),
            "greatest(\"A\", \"B\")"
        );
    }

    #[test]
    fn tableau_refuses_what_calculated_fields_cannot_do() {
        for formula in [
            "SUM([Sales])",
            "{FIXED [Region] : SUM([Sales])}",
            "[Parameters].[Rate] * 2",
            "TODAY()",
            "RUNNING_SUM(SUM([Sales]))",
            "MAX([Sales])",
            "FOO([Sales])",
        ] {
            assert!(tableau_to_duckdb(formula, &names()).is_err(), "{formula}");
        }
    }

    #[test]
    fn simple_aggregates_are_recognised() {
        assert_eq!(
            simple_tableau_aggregate("SUM([Sales])"),
            Some((dv_core::Aggregate::Sum, "Sales".into()))
        );
        assert_eq!(
            simple_tableau_aggregate("COUNTD([Id])"),
            Some((dv_core::Aggregate::CountDistinct, "Id".into()))
        );
        assert_eq!(simple_tableau_aggregate("SUM([A]) / 2"), None);
    }

    #[test]
    fn duckdb_to_tableau_round_trips_common_shapes() {
        let duck = "CASE WHEN \"Sales\" > 100 THEN 'big' ELSE 'small' END";
        let tab = duckdb_to_tableau(duck).unwrap();
        assert_eq!(tab, "IF [Sales] > 100 THEN \"big\" ELSE \"small\" END");
        assert_eq!(tableau_to_duckdb(&tab, &NameMap::new()).unwrap(), duck);

        assert_eq!(duckdb_to_tableau("\"a\" || 'x'").unwrap(), "[a] + \"x\"");
        assert_eq!(
            duckdb_to_tableau("\"a\" IS NOT NULL AND length(\"b\") > 2").unwrap(),
            "NOT ISNULL([a]) AND LEN([b]) > 2"
        );
        assert_eq!(
            duckdb_to_tableau("CAST(\"a\" AS BIGINT)").unwrap(),
            "INT([a])"
        );
        assert!(duckdb_to_tableau("\"a\" LIKE 'x%'").is_err());
    }

    #[test]
    fn dax_if_switch_and_fields() {
        assert_eq!(
            dax_to_duckdb("IF(Sales[Amount] > 100, \"big\", \"small\")").unwrap(),
            "CASE WHEN \"Amount\" > 100 THEN 'big' ELSE 'small' END"
        );
        assert_eq!(
            dax_to_duckdb("SWITCH(TRUE(), [A] < 1, \"x\", [A] < 5, \"y\", \"z\")").unwrap(),
            "CASE WHEN \"A\" < 1 THEN 'x' WHEN \"A\" < 5 THEN 'y' ELSE 'z' END"
        );
        assert_eq!(
            dax_to_duckdb("'Sales Table'[First] & \" \" & [Last]").unwrap(),
            "\"First\" || ' ' || \"Last\""
        );
        assert_eq!(
            dax_to_duckdb("DIVIDE([A], [B])").unwrap(),
            "((\"A\") / nullif((\"B\"), 0))"
        );
        assert!(dax_to_duckdb("CALCULATE(SUM(Sales[Amount]))").is_err());
        assert!(dax_to_duckdb("TODAY()").is_err());
    }

    #[test]
    fn duckdb_to_dax_shapes() {
        assert_eq!(
            duckdb_to_dax("CASE WHEN \"a\" < 1 THEN 'x' ELSE 'y' END").unwrap(),
            "SWITCH(TRUE(), [a] < 1, \"x\", \"y\")"
        );
        assert_eq!(duckdb_to_dax("\"a\" || 'x'").unwrap(), "[a] & \"x\"");
        assert_eq!(duckdb_to_dax("\"a\" IS NULL").unwrap(), "ISBLANK([a])");
        assert_eq!(duckdb_to_dax("ceil(\"a\")").unwrap(), "CEILING([a])");
        assert!(duckdb_to_dax("\"a\" % 2").is_err());
    }
}
