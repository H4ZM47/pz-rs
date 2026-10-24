//! Local parity probe: dumps the population tables loaded from an
//! installed game (and invented edge-case files) in the format of the
//! local reference oracle. Usage: `appearance_probe <game dir> <edge dir>`.

use pz_characters::population::*;
use pz_core::random::outfit_rng;
use pz_fs::GameFs;

fn q(s: Option<&str>) -> String {
    s.map_or("null".into(), |s| format!("\"{s}\""))
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.join(", "))
}

fn hair(tag: &str, h: &HairStyle) {
    let trim: String = h.trim_choices.iter().map(|s| format!("{},", q(Some(s)))).collect();
    let alt: String =
        h.alternate.iter().map(|a| format!("{}:{},", q(a.category.as_deref()), q(a.style.as_deref()))).collect();
    println!(
        "{tag} name={} model={} texture={} level={} grow={} attached={} noChoose={} trim=[{trim}] alt=[{alt}] valid={}",
        q(Some(&h.name)),
        q(h.model.as_deref()),
        q(Some(&h.texture)),
        h.level,
        h.grow_reference,
        h.attached_hair,
        h.no_choose,
        h.is_valid()
    );
}

fn dump_hair(h: Option<&HairStyles>) {
    let Some(h) = h else { return println!("hairStyles null") };
    h.male.iter().for_each(|s| hair("male", s));
    h.female.iter().for_each(|s| hair("female", s));
}

fn dump_beard(b: Option<&BeardStyles>) {
    let Some(b) = b else { return println!("beardStyles null") };
    for s in &b.styles {
        let trim: String = s.trim_choices.iter().map(|s| format!("{},", q(Some(s)))).collect();
        println!(
            "beard name={} model={} texture={} level={} grow={} trim=[{trim}] valid={}",
            q(Some(&s.name)),
            q(s.model.as_deref()),
            q(Some(&s.texture)),
            s.level,
            s.grow_reference,
            s.is_valid()
        );
    }
}

fn dump_voice(v: Option<&VoiceStyles>) {
    let Some(v) = v else { return println!("voiceStyles null") };
    for s in &v.styles {
        println!(
            "voice name={} prefix={} type={} body={} valid={}",
            q(Some(&s.name)),
            q(Some(&s.prefix)),
            s.voice_type,
            s.body_type_default,
            s.is_valid()
        );
    }
}

fn dump_decals(d: Option<&ClothingDecals>) {
    let Some(d) = d else { return println!("clothingDecals null") };
    for g in &d.groups {
        let decals: String = g.decals.iter().map(|s| format!("{},", q(Some(s)))).collect();
        let groups: String = g.groups.iter().map(|s| format!("{},", q(Some(s)))).collect();
        println!("group name={} decals=[{decals}] groups=[{groups}]", q(g.name.as_deref()));
    }
}

fn decal(tag: &str, d: Option<&ClothingDecal>) {
    let Some(d) = d else { return println!("{tag} null") };
    println!(
        "{tag} name={} texture={} x={} y={} w={} h={} valid={}",
        q(d.name.as_deref()),
        q(d.texture.as_deref()),
        d.x,
        d.y,
        d.width,
        d.height,
        d.is_valid()
    );
}

fn nm(s: Option<&HairStyle>) -> String {
    s.map_or("null".into(), |s| s.name.clone())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (game, edge) = (&args[1], &args[2]);
    println!("== edge");
    println!("-- hair1");
    dump_hair(HairStyles::parse(&format!("{edge}/hair1.xml")).as_ref());
    println!("-- hair2");
    dump_hair(HairStyles::parse(&format!("{edge}/hair2.xml")).as_ref());
    for f in ["wrongroot", "malformed", "empty"] {
        println!("-- {f}");
        dump_hair(HairStyles::parse(&format!("{edge}/{f}.xml")).as_ref());
    }
    println!("-- missing");
    dump_hair(HairStyles::parse(&format!("{edge}/nothere.xml")).as_ref());
    println!("-- beard1");
    dump_beard(BeardStyles::parse(&format!("{edge}/beard1.xml")).as_ref());
    println!("-- voice1");
    dump_voice(VoiceStyles::parse(&format!("{edge}/voice1.xml")).as_ref());
    println!("-- decals1");
    dump_decals(ClothingDecals::parse(&format!("{edge}/decals1.xml")).as_ref());
    for f in ["decal1", "decal2"] {
        println!("-- {f}");
        let d = std::fs::read(format!("{edge}/{f}.xml")).ok().and_then(|b| ClothingDecal::from_xml(&b).ok());
        decal("decal", d.as_ref());
    }

    let mut fs = GameFs::new(game);
    let mut pop = Population::default();
    pop.init(&mut fs);
    println!("== real");
    dump_hair(pop.hair_styles.as_ref());
    dump_beard(pop.beard_styles.as_ref());
    dump_voice(pop.voice_styles.as_ref());
    let cd = pop.clothing_decals.as_ref().unwrap();
    dump_decals(Some(cd));
    for g in &cd.groups {
        let mut all = Vec::new();
        g.get_decals(cd, &mut all);
        println!("resolved {} {}", g.name.as_deref().unwrap_or("null"), list(&all));
        for n in &all {
            decal("decal", cd.get_decal(&fs, n).as_deref());
        }
    }
    decal("decal-missing", cd.get_decal(&fs, "NoSuchDecal").as_deref());
    let p = &pop.pop_template;
    let mut sk = String::from("skins");
    for l in [
        &p.male_skins,
        &p.female_skins,
        &p.male_skins_zombie1,
        &p.female_skins_zombie1,
        &p.male_skins_zombie2,
        &p.female_skins_zombie2,
        &p.male_skins_zombie3,
        &p.female_skins_zombie3,
        &p.skeleton_male_skins_zombie,
        &p.skeleton_female_skins_zombie,
        &p.cow_skins,
        &p.rat_skins,
    ] {
        sk.push(' ');
        sk.push_str(&list(l));
    }
    println!("{sk}");

    println!("== rng");
    outfit_rng::set_seed(12345);
    let mut sb = String::new();
    for _ in 0..20 {
        sb += &format!("{},", outfit_rng::next(1000));
    }
    for _ in 0..10 {
        sb += &format!("{},", outfit_rng::next_float(0.0, 100.0).to_bits() as i32);
    }
    for _ in 0..10 {
        sb += &format!("{},", outfit_rng::next_range(-5, 17));
    }
    sb += &outfit_rng::get_seed().to_string();
    println!("{sb}");
    for seed in [0i64, 1, -1, 42, i64::MIN] {
        outfit_rng::set_seed(seed);
        let mut sb = format!("seed {seed}:");
        for g in &cd.groups {
            sb += &format!("{},", cd.get_random_decal(g.name.as_deref().unwrap_or("")).as_deref().unwrap_or("null"));
        }
        sb += cd.get_random_decal("TShirtAll").as_deref().unwrap_or("null");
        sb += cd.get_random_decal("nope").as_deref().unwrap_or("null");
        println!("{sb}");
    }

    println!("== hairdefs");
    let d = &mut pop.hair_outfit_definitions;
    d.dirty = false;
    d.haircut_definition.push(HaircutDefinition::new("Mohawk".into(), 0, "Punk,Bandit"));
    let def = |o: &str, a: Option<&str>, b: Option<&str>, c: Option<&str>, e: Option<&str>, f: Option<&str>| {
        HaircutOutfitDefinition::new(o.into(), a, b, c, e, f).unwrap()
    };
    d.outfit_definition.push(def("Punk", Some("Mohawk:60;Bald:10"), None, None, Some("Full:50;null:20"), Some("0.1,0.2,0.3:50")));
    d.outfit_definition.push(def("Prisoner", None, Some("Bob:100"), Some("CrewCut:70;random:10"), None, None));
    d.outfit_definition.push(def("Prisoner", Some("Bald:100"), None, None, None, None));
    d.outfit_definition.push(def("Office", Some("null:30"), None, None, None, None));
    let hs = pop.hair_styles.as_ref().unwrap();
    let bs = pop.beard_styles.as_ref().unwrap();
    let d = &pop.hair_outfit_definitions;
    for outfit in [None, Some(""), Some("Punk"), Some("Prisoner"), Some("Office"), Some("Bandit"), Some("None")] {
        for seed in 0..25i64 {
            outfit_rng::set_seed(seed * 7919 + 3);
            let m = hs.get_random_male_style(d, outfit, 0.0);
            let f = hs.get_random_female_style(d, outfit, 0.0);
            let b = bs.get_random_style(d, outfit).unwrap_or_default();
            let h = d.get_random_haircut(outfit, &hs.male, 0.0);
            println!("outfit {} seed {seed} m={m} f={f} b={b} h={h} next={}", q(outfit), outfit_rng::next(1000000));
        }
    }
    println!("== colors");
    let common: [[f32; 3]; 3] = [[0.1, 0.2, 0.3], [0.5, 0.4, 0.3], [0.9, 0.8, 0.7]];
    for outfit in [None, Some("Punk"), Some("Office")] {
        for seed in 0..20i64 {
            outfit_rng::set_seed(seed * 31 + 1);
            let c = d.get_random_haircut_color(outfit, &common).unwrap();
            let s = random_skin_color();
            let b = |v: [f32; 3]| format!("{},{},{}", v[0].to_bits() as i32, v[1].to_bits() as i32, v[2].to_bits() as i32);
            println!("color {} {seed} {} skin {}", q(outfit), b(c), b(s));
        }
    }
    println!("== alternates");
    for h in &hs.male {
        for cat in ["default", "Group01", "group02", "nohair", "NoHairNoBeard", "x"] {
            println!("altm {} {cat} -> {}", h.name, nm(hs.get_alternate_for_hat(h, cat)));
        }
    }
    for h in &hs.female {
        for cat in ["default", "Group01", "Group05", "nohair"] {
            println!("altf {} {cat} -> {}", h.name, nm(hs.get_alternate_for_hat(h, cat)));
        }
    }
    println!(
        "find '' m={} f={} beard ''={}",
        nm(hs.find_male_style(Some(""))),
        nm(hs.find_female_style(Some(""))),
        bs.find_style(Some("")).map_or("null".into(), |b| b.name.clone())
    );
}
