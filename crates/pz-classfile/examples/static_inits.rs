//! Prints the evaluated static initializer of a class in a jar:
//! `cargo run -p pz-classfile --example static_inits -- <jar> <a/b/Class>`.

fn main() {
    let mut a = std::env::args().skip(1);
    let (jar, class) = (a.next().expect("jar"), a.next().expect("class"));
    let jar = pz_classfile::Jar::open(jar).expect("open jar");
    let cf = pz_classfile::load_class(&jar, &class).expect("class");
    for s in pz_classfile::static_inits(&cf).expect("clinit") {
        println!("{}.{} = {:?}", s.class, s.field, s.value);
    }
}
