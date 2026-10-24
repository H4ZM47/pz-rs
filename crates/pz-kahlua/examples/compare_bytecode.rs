//! Compare this compiler's output with bytecode dumped by the original
//! compiler (a local oracle run that is not part of this repository).
//! Usage: compare_bytecode <lua-dir> <oracle-dir>

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")) {
            out.push(p);
        }
    }
}

fn main() {
    let mut a = std::env::args().skip(1);
    let root = PathBuf::from(a.next().expect("lua dir"));
    let oracle = PathBuf::from(a.next().expect("oracle dir"));
    let mut files = Vec::new();
    walk(&root, &mut files);
    let (mut same, mut diff) = (0, 0);
    for f in &files {
        let src = String::from_utf8_lossy(&std::fs::read(f).unwrap()).into_owned();
        let name = f.file_name().unwrap().to_string_lossy();
        let mine = pz_kahlua::compiler::compile(&src, Some(&name), None, None).expect("compile").dump();
        let rel = f.strip_prefix(&root).unwrap();
        let theirs = std::fs::read(oracle.join(format!("{}.luac", rel.display()))).unwrap();
        if mine == theirs {
            same += 1;
        } else {
            diff += 1;
            let at = mine.iter().zip(&theirs).position(|(x, y)| x != y).unwrap_or(mine.len().min(theirs.len()));
            println!("DIFF {} (first difference at byte {at}, sizes {} vs {})", rel.display(), mine.len(), theirs.len());
        }
    }
    println!("{same} identical, {diff} different");
}
