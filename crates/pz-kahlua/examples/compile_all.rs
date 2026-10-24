//! Compile every `.lua` file under a directory and report failures.
//! Usage: cargo run -p pz-kahlua --example compile_all -- <dir>

use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
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
    let dir = std::env::args().nth(1).expect("directory");
    let mut files = Vec::new();
    walk(Path::new(&dir), &mut files);
    let (mut ok, mut bad) = (0, 0);
    let start = std::time::Instant::now();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let src = String::from_utf8_lossy(&bytes);
        let name = f.file_name().unwrap().to_string_lossy();
        match pz_kahlua::compiler::compile(&src, Some(&name), Some(&name), f.to_str()) {
            Ok(_) => ok += 1,
            Err(e) => {
                bad += 1;
                println!("FAIL {}: {}", f.display(), e);
            }
        }
    }
    println!("{ok} compiled, {bad} failed, {:?}", start.elapsed());
}
