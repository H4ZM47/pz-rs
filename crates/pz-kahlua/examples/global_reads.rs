//! List the global variables a Lua file reads (GETGLOBAL), with lines.
//! Usage: global_reads <file.lua> [name]

use pz_kahlua::compiler::{compile, getarg_bx, OP_GETGLOBAL};
use pz_kahlua::vm::Prototype;

fn walk(p: &Prototype, filter: Option<&str>) {
    for (pc, &i) in p.code.iter().enumerate() {
        if i & 63 == OP_GETGLOBAL {
            if let Some(name) = p.constants[getarg_bx(i) as usize].as_str() {
                if filter.is_none_or(|f| f == name) {
                    println!("{}:{} {}", p.name.as_deref().unwrap_or("?"), p.lines[pc], name);
                }
            }
        }
    }
    for c in &p.prototypes {
        walk(c, filter);
    }
}

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("file");
    let filter = a.next();
    let src = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    let p = compile(&src, Some("chunk"), None, None).expect("compile");
    walk(&p, filter.as_deref());
}
