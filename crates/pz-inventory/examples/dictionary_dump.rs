//! Prints the tables of a save's `WorldDictionary.bin`:
//! `cargo run -p pz-inventory --example dictionary_dump -- <WorldDictionary.bin> [--items]`.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).expect("read");
    let d = pz_inventory::dictionary::WorldDictionary::load(&bytes).expect("parse");
    println!(
        "version={} items={} entities={} objects={} sprites={}",
        d.version,
        d.items.len(),
        d.entities.len(),
        d.object_names.len(),
        d.sprite_names.len()
    );
    for r in d.string_registers.iter().chain(d.script_registers.iter()) {
        println!(
            "register {} next={} entries={}",
            r.name,
            r.next_id,
            r.entries.len()
        );
    }
    if a.get(2).map(String::as_str) == Some("--items") {
        let mut ids: Vec<_> = d.items.keys().copied().collect();
        ids.sort();
        for id in ids {
            let i = &d.items[&id];
            println!(
                "{id}\t{}\t{}{}",
                i.full_type, i.obsolete as u8, i.removed as u8
            );
        }
    }
}
