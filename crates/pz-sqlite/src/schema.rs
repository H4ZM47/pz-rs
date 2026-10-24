//! Just enough of `CREATE TABLE` to map column names to record positions.
//!
//! A table's record holds its columns in declaration order. A column declared
//! `INTEGER PRIMARY KEY` (not `DESC` in the column form) is an alias for the
//! rowid and is stored as NULL in the record ("2.6. Storage Of The SQL
//! Database Schema", "ROWID tables" in the SQLite documentation).
//! `ALTER TABLE ... ADD` rewrites the stored SQL with the new column
//! appended, so it is parsed like any other column.

use crate::{Error, Result};

/// A column of a rowid table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// Declared type as written (may be empty).
    pub decl_type: String,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDef {
    pub columns: Vec<Column>,
    /// Index of the column that aliases the rowid.
    pub rowid_alias: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// Bare word or quoted identifier, unquoted.
    Word(String, bool),
    /// String literal.
    Str(String),
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
        Tok::Punct(_) => None,
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
    for def in defs {
        let Some(first) = def.first() else {
            return Err(unsupported("empty column definition"));
        };
        if CONSTRAINT_STARTS.iter().any(|k| is_kw(first, k)) {
            // Table constraint: only PRIMARY KEY (...) matters here.
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
                Tok::Word(w, _) | Tok::Str(w) => w.clone(),
                Tok::Punct(p) => p.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        let cons = &rest[type_end..];
        if cons.iter().any(|t| is_kw(t, "GENERATED") || is_kw(t, "AS")) {
            return Err(unsupported("generated column"));
        }
        if let Some(p) = cons.iter().position(|t| is_kw(t, "PRIMARY")) {
            let desc = cons.get(p + 2).is_some_and(|t| is_kw(t, "DESC"));
            if decl_type.eq_ignore_ascii_case("INTEGER") && !desc {
                alias = Some(columns.len());
            }
        }
        columns.push(Column { name, decl_type });
    }
    if let Some(pk) = table_pk {
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
    Ok(TableDef {
        columns,
        rowid_alias: alias,
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
