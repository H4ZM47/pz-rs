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
