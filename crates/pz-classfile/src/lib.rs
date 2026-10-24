//! Reads constants out of the installed game's `projectzomboid.jar` without
//! a JVM: a zip reader, a class-file parser, and a symbolic evaluator for
//! static initializers. The engine uses it for built-in ids that the
//! original defines in code (registry keys, item keys, sound keys); the
//! values are read from the user's install at runtime and never shipped.

pub mod class;
pub mod jar;
pub mod static_init;

pub use class::ClassFile;
pub use jar::Jar;
pub use static_init::{static_inits, StaticInit, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Io(String),
    NotAZip,
    NotAClass,
    Truncated,
    Inflate,
    BadUtf8,
    BadIndex(u16),
    Missing(String),
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Parses `class` (`a/b/C`, or `a/b/C$Inner`) from a jar.
pub fn load_class(jar: &Jar, class: &str) -> Result<ClassFile> {
    ClassFile::parse(&jar.read(&format!("{class}.class"))?)
}

/// `values()` names of enum `class`, in declaration order.
pub fn enum_constants(jar: &Jar, class: &str) -> Result<Vec<String>> {
    Ok(load_class(jar, class)?.enum_constants().into_iter().map(str::to_string).collect())
}

/// Static fields of `class` that end up holding a string: compile-time
/// `static final String` constants, then fields its static initializer
/// assigns a string literal, each in declaration/execution order.
pub fn static_strings(jar: &Jar, class: &str) -> Result<Vec<(String, String)>> {
    let cf = load_class(jar, class)?;
    let mut out: Vec<(String, String)> =
        cf.static_final_strings().into_iter().map(|(n, v)| (n.to_string(), v.to_string())).collect();
    for init in static_inits(&cf)? {
        if let Value::Str(v) = init.value {
            if init.class == cf.this_class && !out.iter().any(|(n, _)| *n == init.field) {
                out.push((init.field, v));
            }
        }
    }
    Ok(out)
}

/// For each static field of `class` initialized with `new <class>(...)` (an
/// enum constant, or a key class built by its own constructor), the field
/// name and the constructor arguments. For enums these start with the
/// implicit name and ordinal.
pub fn self_constructed_constants(jar: &Jar, class: &str) -> Result<Vec<(String, Vec<Value>)>> {
    let cf = load_class(jar, class)?;
    Ok(static_inits(&cf)?
        .into_iter()
        .filter_map(|i| match i.value {
            Value::New { class: c, args } if c == cf.this_class && i.class == cf.this_class => Some((i.field, args)),
            _ => None,
        })
        .collect())
}

/// The value a static initializer stores into `class.field`, if any.
pub fn static_value(jar: &Jar, class: &str, field: &str) -> Result<Option<Value>> {
    let cf = load_class(jar, class)?;
    Ok(static_inits(&cf)?.into_iter().rev().find(|i| i.class == cf.this_class && i.field == field).map(|i| i.value))
}
