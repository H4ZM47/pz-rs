//! `java.io.File` / `java.net.URI` path arithmetic used by
//! `ZomboidFileSystem.getRelativeFile`.
//!
//! Paths are kept as strings, as in the original, because the file map's
//! keys and values are strings built from them.

use std::path::Path;

pub const SEP: char = std::path::MAIN_SEPARATOR;

/// `File` normalization: native separators, no repeated separators, no
/// trailing separator (except for a root).
pub fn normalize(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut prev_sep = false;
    for c in path.chars() {
        let c = if cfg!(windows) && c == '/' { '\\' } else { c };
        if c == SEP {
            // Windows keeps a leading "\\" for UNC paths.
            if prev_sep && !(cfg!(windows) && out.len() == 1) {
                continue;
            }
            prev_sep = true;
        } else {
            prev_sep = false;
        }
        out.push(c);
    }
    while out.len() > 1 && out.ends_with(SEP) && !(cfg!(windows) && out.len() == 3 && out.as_bytes()[1] == b':') {
        out.pop();
    }
    out
}

/// `new File(path).getAbsolutePath()` with `cwd` as `user.dir`.
pub fn absolute(path: &str, cwd: &str) -> String {
    let n = normalize(path);
    if Path::new(&n).is_absolute() {
        n
    } else if n.is_empty() {
        cwd.to_string()
    } else {
        format!("{}{SEP}{n}", cwd.trim_end_matches(SEP))
    }
}

/// `File.getCanonicalPath`, falling back to the absolute path like
/// `ZomboidFileSystem.getCanonicalFile` does.
pub fn canonical(path: &str, cwd: &str) -> String {
    let abs = absolute(path, cwd);
    match std::fs::canonicalize(&abs) {
        Ok(p) => strip_verbatim(&p.to_string_lossy()),
        Err(_) => abs,
    }
}

/// `canonicalize` on Windows yields `\\?\C:\...`; Java does not.
fn strip_verbatim(p: &str) -> String {
    if cfg!(windows) {
        if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = p.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    p.to_string()
}

/// The path component of `new File(abs).toURI()`: '/'-separated, rooted,
/// with a trailing '/' when the file is an existing directory.
pub fn uri_path(abs: &str) -> String {
    let mut p = abs.replace(SEP, "/");
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    if !p.ends_with('/') && Path::new(abs).is_dir() {
        p.push('/');
    }
    p
}

/// `URI.normalize` on an absolute path: drops "." segments and folds
/// "segment/.." pairs.
pub fn uri_normalize(path: &str) -> String {
    let trailing = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    let mut segs: Vec<&str> = Vec::new();
    for s in path.split('/').skip(1) {
        match s {
            "" | "." => {}
            ".." if segs.last().is_some_and(|l| *l != "..") => {
                segs.pop();
            }
            _ => segs.push(s),
        }
    }
    let mut out = String::from("/");
    out.push_str(&segs.join("/"));
    if trailing && !segs.is_empty() {
        out.push('/');
    }
    out
}

/// `URI.relativize(child)` for two file URIs: the child's path relative to
/// `base`, or `None` when `base` is not a prefix.
pub fn relativize(base: &str, child: &str) -> Option<String> {
    let mut bp = uri_normalize(base);
    let cp = uri_normalize(child);
    if bp == cp {
        return Some(String::new());
    }
    if !bp.ends_with('/') {
        bp.push('/');
    }
    cp.strip_prefix(&bp).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn file_rules() {
        assert_eq!(normalize("a//b/"), "a/b");
        assert_eq!(normalize("/"), "/");
        assert_eq!(absolute("media/x", "/base"), "/base/media/x");
        assert_eq!(absolute("/abs//y/", "/base"), "/abs/y");
    }

    #[test]
    fn uri_rules() {
        assert_eq!(uri_normalize("/a/./b/../c"), "/a/c");
        assert_eq!(uri_normalize("/a/b/.."), "/a/");
        assert_eq!(relativize("/base/", "/base/media/x.lua").as_deref(), Some("media/x.lua"));
        assert_eq!(relativize("/base", "/base/media/").as_deref(), Some("media/"));
        assert_eq!(relativize("/base/", "/basement/x"), None);
        assert_eq!(relativize("/base/", "/base/").as_deref(), Some(""));
    }
}
