//! The Java and Kahlua behaviour the definition loaders depend on: how a
//! Lua value prints (`Object.toString`), the number parsers, `String.split`,
//! the `KahluaTableImpl` getters, and the iteration order of
//! `java.util.HashMap<String, _>`.

use indexmap::IndexMap;
use pz_kahlua::{TableRef, Value};

pub use pz_world::java::{java_equals_ignore_case as eq_ignore_case, java_trim};

/// An exception the original loader would throw. It aborts the load, as it
/// does in the original.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaException(pub String);

impl std::fmt::Display for JavaException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JavaException {}

pub type JResult<T> = Result<T, JavaException>;

/// `String.valueOf(o)` of a Kahlua value (`Double.toString` for numbers).
pub fn to_string(v: &Value) -> String {
    pz_kahlua::exec::java_value_of(v)
}

/// `value.toString().trim()`.
pub fn to_string_trim(v: &Value) -> String {
    java_trim(&to_string(v)).to_string()
}

/// `Float.parseFloat(s)`.
pub fn parse_float(s: &str) -> JResult<f32> {
    pz_world::java::parse_float(s)
        .ok_or_else(|| JavaException(format!("NumberFormatException: For input string: \"{s}\"")))
}

/// `Float.valueOf(s).intValue()`.
pub fn float_int(s: &str) -> JResult<i32> {
    parse_float(s).map(|f| f as i32)
}

/// `Integer.parseInt(s)`.
pub fn parse_int(s: &str) -> JResult<i32> {
    pz_world::java::parse_int(s)
        .ok_or_else(|| JavaException(format!("NumberFormatException: For input string: \"{s}\"")))
}

/// `Boolean.parseBoolean(s)`.
pub fn parse_bool(s: &str) -> bool {
    pz_world::java::parse_boolean(s)
}

/// `new ArrayList<>(Arrays.asList(s.split(",")))`: trailing empty strings
/// are dropped when the separator occurs.
pub fn split_comma(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = s.split(',').map(str::to_string).collect();
    if parts.len() > 1 {
        while parts.last().is_some_and(String::is_empty) {
            parts.pop();
        }
    }
    parts
}

/// `String.toLowerCase()`.
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `(KahluaTableImpl) v`.
pub fn as_table(v: &Value) -> JResult<TableRef> {
    match v {
        Value::Table(t) => Ok(t.clone()),
        Value::Nil => Err(JavaException("NullPointerException".into())),
        other => Err(JavaException(format!(
            "ClassCastException: {} cannot be cast to KahluaTableImpl",
            to_string(other)
        ))),
    }
}

/// `(String) v`: `None` for nil.
pub fn as_string(v: &Value) -> JResult<Option<String>> {
    match v {
        Value::Nil => Ok(None),
        Value::Str(s) => Ok(Some(s.to_string())),
        other => Err(JavaException(format!(
            "ClassCastException: {} cannot be cast to String",
            to_string(other)
        ))),
    }
}

/// `KahluaTableImpl.rawgetStr(key)`.
pub fn rawget_str(t: &TableRef, key: &str) -> JResult<Option<String>> {
    as_string(&t.rawget(&Value::str(key)))
}

/// `KahluaTableImpl.rawgetInt(key)`: `-1` unless the value is a number.
pub fn rawget_int(t: &TableRef, key: &str) -> i32 {
    match t.rawget(&Value::str(key)) {
        Value::Num(n) => n as i32,
        _ => -1,
    }
}

/// `KahluaTableImpl.iterator()`: the entries in insertion order.
pub fn entries(t: &TableRef) -> Vec<(Value, Value)> {
    t.entries()
}

/// A `java.util.HashMap<String, V>` that is only ever added to: lookups by
/// key, and iteration in the original's bucket order.
#[derive(Clone, Debug, PartialEq)]
pub struct JHashMap<V> {
    /// Entries in first-insertion order (`put` of a present key keeps its
    /// position and replaces the value, as it does in the original).
    map: IndexMap<String, V>,
}

impl<V> Default for JHashMap<V> {
    fn default() -> Self {
        JHashMap {
            map: IndexMap::new(),
        }
    }
}

impl<V> JHashMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `put(key, value)`.
    pub fn insert(&mut self, key: String, value: V) {
        self.map.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.map.get_mut(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Indices into the insertion order, in iteration order.
    fn order(&self) -> Vec<usize> {
        let keys: Vec<&str> = self.map.keys().map(String::as_str).collect();
        let ordered = pz_world::java::hashmap_order(&keys);
        ordered
            .iter()
            .map(|k| self.map.get_index_of(*k).expect("key"))
            .collect()
    }

    /// `keySet()` in iteration order.
    pub fn keys(&self) -> Vec<String> {
        self.order()
            .into_iter()
            .map(|i| self.map.get_index(i).expect("index").0.clone())
            .collect()
    }

    /// `entrySet()` in iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.order()
            .into_iter()
            .map(move |i| self.map.get_index(i).expect("index"))
    }

    /// `values()` in iteration order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
}

impl<V> FromIterator<(String, V)> for JHashMap<V> {
    fn from_iter<T: IntoIterator<Item = (String, V)>>(iter: T) -> Self {
        let mut m = JHashMap::new();
        for (k, v) in iter {
            m.insert(k, v);
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_like_java() {
        assert_eq!(split_comma("a,b"), vec!["a", "b"]);
        assert_eq!(split_comma("a,,"), vec!["a"]);
        assert_eq!(split_comma(""), vec![""]);
        assert_eq!(split_comma(","), Vec::<String>::new());
        assert_eq!(split_comma(",a"), vec!["", "a"]);
    }

    #[test]
    fn hashmap_iteration_order() {
        // java.util.HashMap: "b"(98) before "a"? Both hash to their code
        // point; buckets 97 & 15 = 1 and 98 & 15 = 2.
        let mut m = JHashMap::new();
        m.insert("b".to_string(), 1);
        m.insert("a".to_string(), 2);
        m.insert("b".to_string(), 3);
        assert_eq!(m.keys(), vec!["a", "b"]);
        assert_eq!(m.get("b"), Some(&3));
    }

    #[test]
    fn numbers_print_like_java() {
        assert_eq!(to_string(&Value::Num(180.0)), "180.0");
        assert_eq!(float_int("180.0"), Ok(180));
        assert!(parse_float("abc").is_err());
    }
}
