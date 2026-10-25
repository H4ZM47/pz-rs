//! Just enough of `CREATE TABLE` to map column names to record positions.
//!
//! A table's record holds its columns in declaration order. A column declared
//! `INTEGER PRIMARY KEY` (not `DESC` in the column form) is an alias for the
//! rowid and is stored as NULL in the record ("2.6. Storage Of The SQL
//! Database Schema", "ROWID tables" in the SQLite documentation).
//! `ALTER TABLE ... ADD` rewrites the stored SQL with the new column
//! appended, so it is parsed like any other column.

use crate::{Error, Result};

/// A literal `DEFAULT` value.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    /// An expression or `CURRENT_*` default, which this crate does not
    /// evaluate.
    Unsupported,
}

/// A column of a rowid table.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    /// Declared type as written (may be empty).
    pub decl_type: String,
    /// `DEFAULT` value; `None` when there is no `DEFAULT` clause (NULL).
    pub default: Option<Literal>,
    pub not_null: bool,
    /// `PRIMARY KEY` or `UNIQUE` in the column definition.
    pub primary_key: bool,
    pub unique: bool,
}

/// Column affinity ("3.1. Determination Of Column Affinity").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

impl Column {
    /// The value of the column in a record that does not have it, and of
    /// an `INSERT` that leaves it out: the default with the column
    /// affinity applied; NULL without a default (or for one this crate
    /// cannot evaluate).
    pub fn default_value(&self) -> crate::Value {
        use crate::Value;
        let v = match &self.default {
            None | Some(Literal::Null | Literal::Unsupported) => return Value::Null,
            Some(Literal::Integer(i)) => Value::Integer(*i),
            Some(Literal::Real(r)) => Value::Real(*r),
            Some(Literal::Text(s)) => Value::Text(s.clone()),
            Some(Literal::Blob(b)) => Value::Blob(b.clone()),
        };
        let v = crate::record::apply_affinity(v, self.affinity());
        // A REAL column reads integral values back as reals.
        match v {
            Value::Integer(i) if self.affinity() == Affinity::Real => Value::Real(i as f64),
            v => v,
        }
    }

    pub fn affinity(&self) -> Affinity {
        let t = self.decl_type.to_ascii_uppercase();
        if t.contains("INT") {
            Affinity::Integer
        } else if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
            Affinity::Text
        } else if t.contains("BLOB") || t.is_empty() {
            Affinity::Blob
        } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
            Affinity::Real
        } else {
            Affinity::Numeric
        }
    }
}

/// Parsed column layout of a `CREATE TABLE` statement.
#[derive(Clone, Debug, PartialEq)]
pub struct TableDef {
    pub columns: Vec<Column>,
    /// Index of the column that aliases the rowid.
    pub rowid_alias: Option<usize>,
    /// A `UNIQUE` or non-rowid `PRIMARY KEY` constraint, which SQLite
    /// backs with an automatic index.
    pub autoindex: bool,
    /// `INTEGER PRIMARY KEY AUTOINCREMENT`: new rowids also pass the
    /// largest ever used, kept in `sqlite_sequence`.
    pub autoincrement: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// Bare word or quoted identifier, unquoted.
    Word(String, bool),
    /// String literal.
    Str(String),
    /// Numeric literal as written.
    Num(String),
    Punct(char),
}

fn lex(sql: &str) -> Result<Vec<Tok>> {
    let err = || Error::Unsupported(format!("cannot parse schema SQL: {sql}"));
    let c: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '-' && c.get(i + 1) == Some(&'-') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if matches!(ch, '"' | '`' | '\'' | '[') {
            let close = if ch == '[' { ']' } else { ch };
            let mut s = String::new();
            i += 1;
            loop {
                let x = *c.get(i).ok_or_else(err)?;
                i += 1;
                if x == close {
                    // Doubled quote characters escape themselves.
                    if close != ']' && c.get(i) == Some(&close) {
                        s.push(close);
                        i += 1;
                        continue;
                    }
                    break;
                }
                s.push(x);
            }
            // A single-quoted name in a column position is accepted by
            // SQLite as an identifier; callers decide by position.
            out.push(if ch == '\'' {
                Tok::Str(s)
            } else {
                Tok::Word(s, true)
            });
        } else if ch.is_ascii_digit()
            || (ch == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            if c.get(i) == Some(&'.') {
                i += 1;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if matches!(c.get(i), Some('e' | 'E')) {
                let mut j = i + 1;
                if matches!(c.get(j), Some('+' | '-')) {
                    j += 1;
                }
                if c.get(j).is_some_and(char::is_ascii_digit) {
                    i = j;
                    while i < c.len() && c[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            out.push(Tok::Num(c[start..i].iter().collect()));
        } else if ch.is_alphanumeric() || ch == '_' || ch == '$' || !ch.is_ascii() {
            let start = i;
            while i < c.len()
                && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$' || !c[i].is_ascii())
            {
                i += 1;
            }
            out.push(Tok::Word(c[start..i].iter().collect(), false));
        } else {
            out.push(Tok::Punct(ch));
            i += 1;
        }
    }
    Ok(out)
}

fn is_kw(t: &Tok, kw: &str) -> bool {
    matches!(t, Tok::Word(w, false) if w.eq_ignore_ascii_case(kw))
}

/// Name of a token in identifier position.
fn ident(t: &Tok) -> Option<&str> {
    match t {
        Tok::Word(w, _) | Tok::Str(w) => Some(w),
        Tok::Num(_) | Tok::Punct(_) => None,
    }
}

/// Splits the tokens between the outermost parentheses at top-level commas.
fn split_defs(toks: &[Tok]) -> Option<(Vec<&[Tok]>, &[Tok])> {
    let open = toks.iter().position(|t| *t == Tok::Punct('('))?;
    let mut depth = 0;
    let mut start = open + 1;
    let mut defs = Vec::new();
    for (i, t) in toks.iter().enumerate().skip(open) {
        match t {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') => {
                depth -= 1;
                if depth == 0 {
                    defs.push(&toks[start..i]);
                    return Some((defs, &toks[i + 1..]));
                }
            }
            Tok::Punct(',') if depth == 1 => {
                defs.push(&toks[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    None
}

const CONSTRAINT_STARTS: [&str; 5] = ["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"];
const COLUMN_CONSTRAINTS: [&str; 12] = [
    "CONSTRAINT",
    "PRIMARY",
    "NOT",
    "NULL",
    "UNIQUE",
    "CHECK",
    "DEFAULT",
    "COLLATE",
    "REFERENCES",
    "GENERATED",
    "AS",
    "ON",
];

/// The literal after `DEFAULT` (`sqlite3ValueFromExpr` for constants).
fn default_literal(t: &[Tok]) -> Literal {
    let (neg, t) = match t.first() {
        Some(Tok::Punct('-')) => (true, &t[1..]),
        Some(Tok::Punct('+')) => (false, &t[1..]),
        _ => (false, t),
    };
    match t {
        [Tok::Num(n), ..] => {
            let pure_int = !n.contains(['.', 'e', 'E']);
            let text = if neg { format!("-{n}") } else { n.clone() };
            match text.parse::<i64>() {
                Ok(i) if pure_int => Literal::Integer(i),
                _ => text
                    .parse::<f64>()
                    .map_or(Literal::Unsupported, Literal::Real),
            }
        }
        _ if neg => Literal::Unsupported,
        [Tok::Str(s), ..] => Literal::Text(s.clone()),
        [Tok::Word(x, false), Tok::Str(h), ..] if x.eq_ignore_ascii_case("X") => {
            let b = h.as_bytes();
            if b.len() % 2 != 0 || !b.iter().all(u8::is_ascii_hexdigit) {
                return Literal::Unsupported;
            }
            let hex = |c: u8| (c as char).to_digit(16).unwrap() as u8;
            Literal::Blob(b.chunks(2).map(|p| hex(p[0]) << 4 | hex(p[1])).collect())
        }
        [w @ Tok::Word(_, false), ..] if is_kw(w, "NULL") => Literal::Null,
        [w @ Tok::Word(_, false), ..] if is_kw(w, "TRUE") => Literal::Integer(1),
        [w @ Tok::Word(_, false), ..] if is_kw(w, "FALSE") => Literal::Integer(0),
        _ => Literal::Unsupported,
    }
}

/// Parses `CREATE [TEMP] TABLE [IF NOT EXISTS] name (defs) [options]`.
pub fn parse_create_table(sql: &str) -> Result<TableDef> {
    let toks = lex(sql)?;
    let unsupported = |what: &str| Error::Unsupported(format!("{what}: {sql}"));
    let open = toks.iter().position(|t| *t == Tok::Punct('('));
    if toks[..open.unwrap_or(toks.len())]
        .iter()
        .any(|t| is_kw(t, "AS"))
    {
        return Err(unsupported("CREATE TABLE ... AS"));
    }
    let (defs, tail) = split_defs(&toks).ok_or_else(|| unsupported("no column list"))?;
    if tail.iter().any(|t| is_kw(t, "WITHOUT")) {
        return Err(unsupported("WITHOUT ROWID table"));
    }
    let mut columns = Vec::new();
    let mut alias = None;
    let mut table_pk: Option<Vec<String>> = None;
    let mut table_unique = false;
    let mut autoincrement = false;
    for def in defs {
        let Some(first) = def.first() else {
            return Err(unsupported("empty column definition"));
        };
        if CONSTRAINT_STARTS.iter().any(|k| is_kw(first, k)) {
            // Table constraint: only PRIMARY KEY (...) matters here.
            table_unique |= def.iter().any(|t| is_kw(t, "UNIQUE"));
            if let Some(p) = def.iter().position(|t| is_kw(t, "PRIMARY")) {
                if let Some((cols, _)) = split_defs(&def[p..]) {
                    let names = cols
                        .iter()
                        .filter_map(|c| c.first().and_then(ident).map(str::to_string));
                    table_pk = Some(names.collect());
                }
            }
            continue;
        }
        let name = ident(first)
            .ok_or_else(|| unsupported("bad column name"))?
            .to_string();
        let rest = &def[1..];
        let type_end = rest
            .iter()
            .position(|t| COLUMN_CONSTRAINTS.iter().any(|k| is_kw(t, k)))
            .unwrap_or(rest.len());
        let decl_type = rest[..type_end]
            .iter()
            .map(|t| match t {
                Tok::Word(w, _) | Tok::Str(w) | Tok::Num(w) => w.clone(),
                Tok::Punct(p) => p.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        let cons = &rest[type_end..];
        if cons.iter().any(|t| is_kw(t, "GENERATED") || is_kw(t, "AS")) {
            return Err(unsupported("generated column"));
        }
        let primary_key = cons.iter().any(|t| is_kw(t, "PRIMARY"));
        if let Some(p) = cons.iter().position(|t| is_kw(t, "PRIMARY")) {
            let desc = cons.get(p + 2).is_some_and(|t| is_kw(t, "DESC"));
            if decl_type.eq_ignore_ascii_case("INTEGER") && !desc {
                alias = Some(columns.len());
                autoincrement = cons.iter().any(|t| is_kw(t, "AUTOINCREMENT"));
            }
        }
        let not_null = cons
            .windows(2)
            .any(|w| is_kw(&w[0], "NOT") && is_kw(&w[1], "NULL"));
        let unique = cons.iter().any(|t| is_kw(t, "UNIQUE"));
        let default = cons
            .iter()
            .position(|t| is_kw(t, "DEFAULT"))
            .map(|p| default_literal(&cons[p + 1..]));
        columns.push(Column {
            name,
            decl_type,
            default,
            not_null,
            primary_key,
            unique,
        });
    }
    if let Some(pk) = &table_pk {
        if let [only] = pk.as_slice() {
            if let Some(i) = columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(only))
            {
                if columns[i].decl_type.eq_ignore_ascii_case("INTEGER") {
                    alias = Some(i);
                }
            }
        }
    }
    let autoindex = table_unique
        || columns
            .iter()
            .enumerate()
            .any(|(i, c)| c.unique || (c.primary_key && alias != Some(i)))
        || (table_pk.is_some() && alias.is_none());
    Ok(TableDef {
        columns,
        rowid_alias: alias,
        autoindex,
        autoincrement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_players() {
        let d = parse_create_table(
            "CREATE TABLE localPlayers (id INTEGER PRIMARY KEY NOT NULL,name STRING,wx INTEGER,\
             wy INTEGER,x FLOAT,y FLOAT,z FLOAT,worldversion INTEGER,data BLOB,isDead BOOLEAN)",
        )
        .unwrap();
        let names: Vec<_> = d.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "id",
                "name",
                "wx",
                "wy",
                "x",
                "y",
                "z",
                "worldversion",
                "data",
                "isDead"
            ]
        );
        assert_eq!(d.rowid_alias, Some(0));
        assert_eq!(d.columns[1].decl_type, "STRING");
        let aff: Vec<_> = d.columns.iter().map(Column::affinity).collect();
        use Affinity::*;
        assert_eq!(
            aff,
            [Integer, Numeric, Integer, Integer, Real, Real, Real, Integer, Blob, Numeric]
        );
    }

    #[test]
    fn altered_and_quoted() {
        let d = parse_create_table(
            "CREATE TABLE \"t\" (a integer, \"b c\" VARCHAR(10, 2) DEFAULT 'x,y', [d] TEXT, 'e' STRING)",
        )
        .unwrap();
        let names: Vec<_> = d.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b c", "d", "e"]);
        assert_eq!(d.columns[1].decl_type, "VARCHAR ( 10 , 2 )");
        assert_eq!(d.rowid_alias, None);
    }

    #[test]
    fn rowid_alias_forms() {
        let alias = |s| parse_create_table(s).unwrap().rowid_alias;
        assert_eq!(alias("CREATE TABLE t(a INT PRIMARY KEY, b)"), None);
        assert_eq!(alias("CREATE TABLE t(a, b INTEGER PRIMARY KEY DESC)"), None);
        assert_eq!(
            alias("CREATE TABLE t(a, b integer primary key asc)"),
            Some(1)
        );
        assert_eq!(
            alias("CREATE TABLE t(a, b INTEGER, PRIMARY KEY(b DESC))"),
            Some(1)
        );
        assert_eq!(
            alias("CREATE TABLE t(a INTEGER, b, PRIMARY KEY(a, b))"),
            None
        );
        assert!(parse_create_table("CREATE TABLE t(a PRIMARY KEY) WITHOUT ROWID").is_err());
        assert!(parse_create_table("CREATE TABLE t AS SELECT 1").is_err());
    }
}
