//! Prints the constant-pool references of the named methods in bytecode order.

fn cstr(cf: &pz_classfile::ClassFile, idx: u16) -> String {
    use pz_classfile::class::Constant as C;
    match cf.constant(idx) {
        Ok(C::String(u)) => format!("{:?}", cf.utf8(*u).unwrap_or("?")),
        Ok(C::Integer(v)) => format!("int {v}"),
        Ok(C::Float(v)) => format!("float {v}"),
        Ok(C::Class(_)) => format!("class {}", cf.class_name(idx).unwrap_or("?")),
        Ok(C::InvokeDynamic(_, nt)) => match cf.constant(*nt) {
            Ok(C::NameAndType(n, d)) => format!("indy {} {}", cf.utf8(*n).unwrap_or("?"), cf.utf8(*d).unwrap_or("?")),
            _ => "indy ?".into(),
        },
        Ok(c) => format!("{c:?}"),
        Err(_) => "?".into(),
    }
}
fn main() {
    let mut a = std::env::args().skip(1);
    let (jar, class, prefix) = (a.next().unwrap(), a.next().unwrap(), a.next().unwrap());
    let jar = pz_classfile::Jar::open(jar).unwrap();
    let cf = pz_classfile::load_class(&jar, &class).unwrap();
    for m in cf.methods.iter().filter(|m| prefix.split(',').any(|p| m.name == p)) {
        println!("== {} {} access={:#x}", m.name, m.descriptor, m.access);
        let Some(code) = &m.code else { continue };
        let mut i = 0;
        while i < code.len() {
            let op = code[i];
            let u16at = |k: usize| u16::from_be_bytes([code[k], code[k + 1]]);
            let len = match op {
                0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
                0x11 | 0x13 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb2..=0xb8 | 0xbb | 0xbd | 0xc0 | 0xc1 | 0xc6 | 0xc7 => 3,
                0xb9 | 0xba | 0xc8 | 0xc9 => 5,
                0xc5 => 4,
                _ => 1,
            };
            match op {
                0x12 => println!("  ldc {}", cstr(&cf, code[i + 1] as u16)),
                0xba => println!("  {}", cstr(&cf, u16at(i + 1))),
                0x13 | 0xb2..=0xb9 | 0xbb | 0xc0 | 0xc1 => {
                    let idx = u16at(i + 1);
                    match cf.member_ref(idx) {
                        Ok(r) => println!("  {op:#x} {}.{} {}", r.class, r.name, r.descriptor),
                        Err(_) => println!("  {op:#x} {}", cstr(&cf, idx)),
                    }
                }
                0x10 => println!("  bipush {}", code[i + 1] as i8),
                0x11 => println!("  sipush {}", u16at(i + 1) as i16),
                0x02..=0x08 => println!("  iconst {}", op as i32 - 3),
                0xc6 | 0xc7 | 0x99..=0xa7 => println!("  @{i} branch {op:#x} -> {}", i as i32 + u16at(i + 1) as i16 as i32),
                0x1a..=0x2d => println!("  @{i} load{}", (op - 0x1a) % 4),
                0x15..=0x19 => println!("  @{i} load {}", code[i + 1]),
                0xac | 0xb1 | 0xb0 => println!("  @{i} return"),
                _ => {}
            }
            i += len;
        }
    }
}
