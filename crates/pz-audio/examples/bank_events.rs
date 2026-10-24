//! Print each event's length (ms), distances and parameters, one line per
//! event, for comparison with the FMOD runtime.
//!
//! `cargo run -p pz-audio --example bank_events -- <Foo.strings.bank> <Foo.bank>...`

use pz_audio::BankFile;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let strings = BankFile::open(args[0].as_ref()).unwrap().strings().unwrap();
    for path in &args[1..] {
        let bank = BankFile::open(path.as_ref()).unwrap();
        let m = bank.model().unwrap();
        let mut lines = Vec::new();
        for e in m.events.values() {
            let Some(name) = strings.path(&e.id) else {
                continue;
            };
            let len = m.event_length_ms(e);
            let params: Vec<String> = m
                .event_parameters(e, &[])
                .iter()
                .map(|p| {
                    format!(
                        "{}:{}:{}:{}:{}:{}:{}:{}",
                        p.name, p.id.0, p.id.1, p.minimum, p.maximum, p.default, p.kind, p.flags
                    )
                })
                .collect();
            lines.push(format!(
                "{}\t{name}\t{len}\t{:.3}\t{:.3}\t{}",
                e.id,
                e.min_distance,
                e.max_distance,
                params.join(",")
            ));
        }
        lines.sort();
        for l in lines {
            println!("{l}");
        }
    }
}
