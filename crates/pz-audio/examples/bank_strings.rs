//! Print a strings bank as `GUID<TAB>path` lines, sorted by GUID.
//!
//! `cargo run -p pz-audio --example bank_strings -- <Foo.strings.bank>`

fn main() {
    let path = std::env::args().nth(1).expect("strings bank path");
    let bank = pz_audio::BankFile::open(path.as_ref()).expect("open bank");
    let t = bank.strings().expect("no string table");
    let mut v: Vec<_> = t.by_guid.iter().collect();
    v.sort_by_key(|(g, _)| g.to_string());
    for (g, p) in v {
        println!("{g}\t{p}");
    }
}
