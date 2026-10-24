//! The schema statements the game runs on its databases, parsed far enough
//! to carry them out and to store their text as SQLite stores it:
//!
//! * `CREATE TABLE [IF NOT EXISTS] name (...)`: stored as `CREATE TABLE `
//!   followed by the statement text from the name to the closing token
//!   (`sqlite3EndTable`);
//! * `CREATE [UNIQUE] INDEX [IF NOT EXISTS] name ON table (col, ...)`:
//!   stored as `CREATE INDEX ` and the text from the name to the end of
//!   the statement, without a trailing `;` (`sqlite3CreateIndex`);
//! * `ALTER TABLE name ADD [COLUMN] def`;
//! * `DROP TABLE [IF EXISTS] name`;
//! * `PRAGMA journal_mode=delete|truncate|persist` (per connection).

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
enum T {
    /// Identifier or keyword (unquoted text), quoted identifier.
    Word(String, bool),
    Str(String),
    Other(char),
}

#[derive(Clone, Debug)]
struct Tok {
    t: T,
    start: usize,
    end: usize,
}

fn lex(sql: &str) -> Result<Vec<Tok>> {
    let err = || Error::Sql(format!("unrecognized token in: {sql}"));
    let b: Vec<(usize, char)> = sql.char_indices().collect();
    let at = |i: usize| b.get(i).map(|x| x.1);
    let pos = |i: usize| b.get(i).map_or(sql.len(), |x| x.0);
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let ch = b[i].1;
        let start = pos(i);
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '-' && at(i + 1) == Some('-') {
            while i < b.len() && b[i].1 != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && at(i + 1) == Some('*') {
            i += 2;
            while i < b.len() && !(b[i].1 == '*' && at(i + 1) == Some('/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        let t = if matches!(ch, '"' | '`' | '\'' | '[') {
            let close = if ch == '[' { ']' } else { ch };
            let mut s = String::new();
            i += 1;
            loop {
                let x = at(i).ok_or_else(err)?;
                i += 1;
                if x == close {
                    if close != ']' && at(i) == Some(close) {
                        s.push(close);
                        i += 1;
                        continue;
                    }
                    break;
                }
                s.push(x);
            }
            if ch == '\'' {
                T::Str(s)
            } else {
                T::Word(s, true)
            }
        } else if ch.is_alphanumeric() || ch == '_' || ch == '$' || !ch.is_ascii() {
            let st = i;
            while i < b.len()
                && (b[i].1.is_alphanumeric()
                    || b[i].1 == '_'
                    || b[i].1 == '$'
                    || !b[i].1.is_ascii())
            {
                i += 1;
            }
            T::Word(b[st..i].iter().map(|x| x.1).collect(), false)
        } else {
            i += 1;
            T::Other(ch)
        };
        out.push(Tok {
            t,
            start,
            end: pos(i),
        });
    }
    Ok(out)
}

fn kw(t: &Tok, k: &str) -> bool {
    matches!(&t.t, T::Word(w, false) if w.eq_ignore_ascii_case(k))
}

fn name(t: Option<&Tok>) -> Option<String> {
    match t.map(|t| &t.t) {
        Some(T::Word(w, _) | T::Str(w)) => Some(w.clone()),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ddl {
    CreateTable {
        name: String,
        if_not_exists: bool,
        sql: String,
    },
    CreateIndex {
        name: String,
        table: String,
        columns: Vec<String>,
        unique: bool,
        if_not_exists: bool,
        sql: String,
    },
    AddColumn {
        table: String,
        /// Column name.
        column: String,
        /// The column definition as written.
        def: String,
    },
    DropTable {
        name: String,
        if_exists: bool,
    },
    /// `PRAGMA journal_mode=<mode>` (lower-cased mode).
    JournalMode(String),
}

pub(crate) fn parse(sql: &str) -> Result<Ddl> {
    let mut toks = lex(sql)?;
    while toks.last().is_some_and(|t| t.t == T::Other(';')) {
        toks.pop();
    }
    let syntax = || Error::Sql(format!("near \"{sql}\": syntax error"));
    let unsupported = || Error::Unsupported(format!("statement: {sql}"));
    let last_end = toks.last().map_or(0, |t| t.end);
    let mut i = 0;
    let next_kw = |i: &mut usize, k: &str| {
        if toks.get(*i).is_some_and(|t| kw(t, k)) {
            *i += 1;
            true
        } else {
            false
        }
    };
    let if_clause = |i: &mut usize, words: &[&str]| {
        if words
            .iter()
            .enumerate()
            .all(|(j, w)| toks.get(*i + j).is_some_and(|t| kw(t, w)))
        {
            *i += words.len();
            true
        } else {
            false
        }
    };
    if next_kw(&mut i, "PRAGMA") {
        let n = name(toks.get(i)).unwrap_or_default();
        let eq = toks.get(i + 1).is_some_and(|t| t.t == T::Other('='));
        return match (
            n.eq_ignore_ascii_case("journal_mode"),
            eq,
            name(toks.get(i + 2)),
        ) {
            (true, true, Some(m)) if i + 3 == toks.len() => {
                Ok(Ddl::JournalMode(m.to_ascii_lowercase()))
            }
            _ => Err(unsupported()),
        };
    }
    if next_kw(&mut i, "CREATE") {
        let unique = next_kw(&mut i, "UNIQUE");
        if !unique && next_kw(&mut i, "TABLE") {
            let if_not_exists = if_clause(&mut i, &["IF", "NOT", "EXISTS"]);
            let n = toks.get(i).ok_or_else(syntax)?;
            let nm = name(Some(n)).ok_or_else(syntax)?;
            if toks.get(i + 1).is_some_and(|t| t.t == T::Other('.')) {
                return Err(unsupported());
            }
            if toks.iter().skip(i).any(|t| kw(t, "AS"))
                && !toks.iter().any(|t| t.t == T::Other('('))
            {
                return Err(unsupported());
            }
            let sql = format!("CREATE TABLE {}", &sql[n.start..last_end]);
            return Ok(Ddl::CreateTable {
                name: nm,
                if_not_exists,
                sql,
            });
        }
        if next_kw(&mut i, "INDEX") {
            let if_not_exists = if_clause(&mut i, &["IF", "NOT", "EXISTS"]);
            let n = toks.get(i).ok_or_else(syntax)?;
            let nm = name(Some(n)).ok_or_else(syntax)?;
            let start = n.start;
            i += 1;
            if !next_kw(&mut i, "ON") {
                return Err(syntax());
            }
            let table = name(toks.get(i)).ok_or_else(syntax)?;
            i += 1;
            if toks.get(i).map(|t| &t.t) != Some(&T::Other('(')) {
                return Err(syntax());
            }
            i += 1;
            let mut columns = Vec::new();
            loop {
                let c = name(toks.get(i)).ok_or_else(syntax)?;
                columns.push(c);
                i += 1;
                if next_kw(&mut i, "ASC") {
                    // Ascending is the default order.
                }
                match toks.get(i).map(|t| &t.t) {
                    Some(T::Other(',')) => i += 1,
                    Some(T::Other(')')) => {
                        i += 1;
                        break;
                    }
                    // DESC, COLLATE, expressions.
                    _ => return Err(unsupported()),
                }
            }
            if i != toks.len() {
                // WHERE (partial index) or trailing text.
                return Err(unsupported());
            }
            let u = if unique { " UNIQUE" } else { "" };
            let sql = format!("CREATE{u} INDEX {}", &sql[start..last_end]);
            return Ok(Ddl::CreateIndex {
                name: nm,
                table,
                columns,
                unique,
                if_not_exists,
                sql,
            });
        }
        return Err(unsupported());
    }
    if next_kw(&mut i, "ALTER") {
        if !next_kw(&mut i, "TABLE") {
            return Err(syntax());
        }
        let table = name(toks.get(i)).ok_or_else(syntax)?;
        i += 1;
        if !next_kw(&mut i, "ADD") {
            return Err(unsupported());
        }
        next_kw(&mut i, "COLUMN");
        let c = toks.get(i).ok_or_else(syntax)?;
        let column = name(Some(c)).ok_or_else(syntax)?;
        // sqlite3AlterFinishAddColumn trims trailing ';' and spaces.
        let def = sql[c.start..]
            .trim_end_matches(|ch: char| ch == ';' || ch.is_ascii_whitespace())
            .to_string();
        return Ok(Ddl::AddColumn { table, column, def });
    }
    if next_kw(&mut i, "DROP") {
        if !next_kw(&mut i, "TABLE") {
            return Err(unsupported());
        }
        let if_exists = if_clause(&mut i, &["IF", "EXISTS"]);
        let nm = name(toks.get(i)).ok_or_else(syntax)?;
        if i + 1 != toks.len() {
            return Err(syntax());
        }
        return Ok(Ddl::DropTable {
            name: nm,
            if_exists,
        });
    }
    Err(unsupported())
}

/// Byte offset of the `)` closing the column list of a stored `CREATE
/// TABLE` (`Table.u.tab.addColOffset`).
pub(crate) fn add_col_offset(sql: &str) -> Result<usize> {
    let toks = lex(sql)?;
    let mut depth = 0;
    for t in &toks {
        match t.t {
            T::Other('(') => depth += 1,
            T::Other(')') => {
                depth -= 1;
                if depth == 0 {
                    return Ok(t.start);
                }
            }
            _ => {}
        }
    }
    Err(Error::Corrupt(format!("malformed schema SQL: {sql}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_text() {
        let d = parse("CREATE TABLE localPlayers (id   INTEGER PRIMARY KEY NOT NULL,name STRING);")
            .unwrap();
        assert_eq!(
            d,
            Ddl::CreateTable {
                name: "localPlayers".into(),
                if_not_exists: false,
                sql: "CREATE TABLE localPlayers (id   INTEGER PRIMARY KEY NOT NULL,name STRING)"
                    .into()
            }
        );
        let d = parse("create  table if not exists \"a b\"(x)").unwrap();
        assert_eq!(
            d,
            Ddl::CreateTable {
                name: "a b".into(),
                if_not_exists: true,
                sql: "CREATE TABLE \"a b\"(x)".into()
            }
        );
        let d = parse("CREATE INDEX ivwx ON vehicles (wx);").unwrap();
        assert_eq!(
            d,
            Ddl::CreateIndex {
                name: "ivwx".into(),
                table: "vehicles".into(),
                columns: vec!["wx".into()],
                unique: false,
                if_not_exists: false,
                sql: "CREATE INDEX ivwx ON vehicles (wx)".into()
            }
        );
        let d = parse("ALTER TABLE 'vehicles' ADD 'inMeta' BOOLEAN NULL DEFAULT FALSE ; ").unwrap();
        assert_eq!(
            d,
            Ddl::AddColumn {
                table: "vehicles".into(),
                column: "inMeta".into(),
                def: "'inMeta' BOOLEAN NULL DEFAULT FALSE".into()
            }
        );
        assert_eq!(
            parse("DROP TABLE localPlayers").unwrap(),
            Ddl::DropTable {
                name: "localPlayers".into(),
                if_exists: false
            }
        );
        assert_eq!(
            parse("PRAGMA JOURNAL_MODE=TRUNCATE;").unwrap(),
            Ddl::JournalMode("truncate".into())
        );
        assert!(matches!(
            parse("CREATE INDEX i ON t (a DESC)"),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(
            add_col_offset("CREATE TABLE t (a, b VARCHAR(3)) ").unwrap(),
            31
        );
    }
}
