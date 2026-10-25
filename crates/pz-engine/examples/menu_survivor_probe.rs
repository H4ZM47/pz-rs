//! Local parity probe: character creation's survivor through the Lua
//! bindings (`SurvivorFactory.CreateSurvivor`, `UI3DModel.setSurvivorDesc`,
//! `SurvivorDesc:dressInNamedOutfit`, `IsoSurvivor.new`) with seeded `Rand`
//! and `OutfitRNG`, printed in the format of the local reference oracle.
//! Usage: `menu_survivor_probe <game dir> <scratch cache dir>`.

use pz_core::random::{self as rand, outfit_rng};
use pz_inventory::visual::{HumanVisual, Rgb};
use pz_kahlua::Value;
use pz_lua_api::bind_character_sim::CharacterObject;
use pz_lua_api::bind_characters::{LuaSurvivorDesc, SurvivorDescObject};
use pz_lua_api::bind_items::LuaInventoryItem;
use pz_lua_api::LuaManager;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

fn f(v: f32) -> String {
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    format!("{:.6}", v as f64)
}

fn col(c: Option<Rgb>) -> String {
    c.map_or("null".into(), |[r, g, b]| format!("{},{},{},{}", f(r), f(g), f(b), f(1.0)))
}

fn s(o: Option<&str>) -> String {
    o.unwrap_or("null").to_string()
}

fn visual(tag: &str, hv: &HumanVisual) {
    println!(
        "{tag} skin={} skinName={} rot={} hair={} beard={} bodyHair={} hc={} bc={} nhc={} nbc={} sc={} outfit={}",
        hv.skin_texture,
        s(hv.skin_texture_name.as_deref()),
        hv.zombie_rot_stage,
        s(hv.hair_model.as_deref()),
        s(hv.beard_model.as_deref()),
        hv.body_hair,
        col(hv.hair_color),
        col(hv.beard_color),
        col(hv.natural_hair_color),
        col(hv.natural_beard_color),
        col(hv.skin_color),
        s(hv.outfit.as_deref())
    );
}

fn desc(tag: &str, d: &LuaSurvivorDesc) {
    println!(
        "{tag} id={} type={} female={} fore={} sur={} torso={} stats={},{},{},{},{},{},{},{} voice={},{},{}",
        d.id,
        d.survivor_type.name(),
        d.is_female(),
        s(d.forename.as_deref()),
        s(d.surname.as_deref()),
        s(d.torso.as_deref()),
        f(d.bravery),
        f(d.aggressiveness),
        f(d.compassion),
        f(d.loner),
        f(d.temper),
        f(d.friendliness),
        f(d.favourindoors),
        f(d.loyalty),
        s(d.voice_prefix.as_deref()),
        f(d.voice_pitch),
        d.voice_type
    );
    visual(&format!("{tag}  visual"), &d.human_visual);
}

fn worn(tag: &str, d: &LuaSurvivorDesc) {
    let items = d.worn_items.items();
    println!("{tag} worn={}", items.len());
    for (i, w) in items.iter().enumerate() {
        let Value::Object(o) = &w.item.0 else { continue };
        let item = o.as_any().downcast_ref::<LuaInventoryItem>().unwrap().item.borrow();
        let vs = match &item.visual {
            None => "novisual".to_string(),
            Some(v) => format!(
                "ci={} tint={} hue={} base={} choice={} decal={}",
                v.clothing_item_name,
                col(v.tint),
                f(v.hue),
                v.base_texture,
                v.texture_choice,
                s(v.decal.as_deref())
            ),
        };
        let loc = w.location.as_ref().map_or("null".into(), |k| k.location.id());
        println!("{tag}  {i} loc={loc} type={} {vs}", item.get_full_type());
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game dir");
    let cache = args.next().expect("scratch cache dir");
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&game)));
    fs.borrow_mut().set_cache_dir(Path::new(&cache));
    pz_lua_api::bind_core::shared_core().borrow_mut().set_cache_dir(Path::new(&cache));
    pz_engine::boot::load_scripts(Path::new(&game), &mut fs.borrow_mut()).expect("boot");
    let mut mgr = LuaManager::init(fs).expect("LuaManager.init");
    {
        let core = pz_lua_api::bind_core::shared_core();
        let fs = mgr.fs.borrow();
        pz_lua_api::bind_text::boot(&fs, &core.borrow()).expect("text");
    }
    mgr.thread.log = Box::new(|m| eprintln!("LUAERR: {m}"));
    pz_ui::UIManager::init();
    for f in [
        "shared/NPCs/BodyLocations.lua",
        "shared/Definitions/HairOutfitDefinitions.lua",
        "shared/Definitions/DefaultClothing.lua",
        "shared/NPCs/MainCreationMethods.lua",
    ] {
        let path = format!("{game}/media/lua/{f}");
        pz_lua_api::lua_manager::run_lua(&mut mgr.thread, &mgr.loader, &path);
    }
    let run = |mgr: &mut LuaManager, src: &str| -> Vec<Value> {
        let f = pz_kahlua::stdlib::load_string(src, Some("probe"), mgr.thread.globals().clone()).expect("compile");
        match mgr.thread.pcall(&f, &[]) {
            Ok(v) => v,
            Err((e, t)) => {
                eprintln!("probe error: {e:?}\n{t}");
                Vec::new()
            }
        }
    };
    run(
        &mut mgr,
        r#"
        -- Without the OnCreateSurvivor handler (its AddItem needs
        -- ItemContainer, not bound yet): the constructor's own draws.
        Events.OnCreateSurvivor.Remove(BaseGameCharacterDetails.CreateCharacterInstance)
        BaseGameCharacterDetails.DoHairColor()
        BaseGameCharacterDetails.DoTrouserColor()
        for _, n in ipairs({"Ann", "Beth"}) do SurvivorFactory.addFemaleForename(n) end
        for _, n in ipairs({"Carl", "Dan"}) do SurvivorFactory.addMaleForename(n) end
        for _, n in ipairs({"Eve", "Fox", "Gray"}) do SurvivorFactory.addSurname(n) end
        "#,
    );
    let fmt = |r: f64, g: f64, b: f64, a: f64| format!("{},{},{},{}", f(r as f32), f(g as f32), f(b as f32), f(a as f32));
    let lists = run(
        &mut mgr,
        r#"
        local function dump(l)
            local t = {}
            for i = 0, l:size() - 1 do
                local c = l:get(i)
                t[#t + 1] = {c:getRedFloat(), c:getGreenFloat(), c:getBlueFloat(), c:getAlphaFloat()}
            end
            return t
        end
        local d = SurvivorFactory.CreateSurvivor()
        assert(d:getCommonHairColor() == SurvivorDesc.HairCommonColors)
        SurvivorDesc.setIDCount(0)
        return dump(SurvivorDesc.HairCommonColors), dump(SurvivorDesc.TrouserCommonColors)
        "#,
    );
    for (name, v) in ["hairColors", "trouserColors"].iter().zip(&lists) {
        let Value::Table(t) = v else { continue };
        let mut line = name.to_string();
        for i in 1..=t.len() {
            let Value::Table(c) = t.rawget(&Value::Num(i as f64)) else { continue };
            let n = |k: usize| match c.rawget(&Value::Num(k as f64)) {
                Value::Num(x) => x,
                _ => f64::NAN,
            };
            line += &format!(" {}", fmt(n(1), n(2), n(3), n(4)));
        }
        println!("{line}");
    }
    let global = |mgr: &LuaManager, n: &str| mgr.thread.globals().rawget(&Value::str(n));
    for seed in [1i64, 42, 12345, 777] {
        println!("== seed {seed}");
        rand::set_seed(seed);
        outfit_rng::set_seed(seed * 31 + 7);
        run(&mut mgr, "_d = SurvivorFactory.CreateSurvivor()");
        let Value::Object(o) = global(&mgr, "_d") else { panic!("no desc") };
        let d = o.as_any().downcast_ref::<SurvivorDescObject>().unwrap().desc.clone();
        desc("create", &d.borrow());
        run(&mut mgr, "_m = UI3DModel.new(); _m:setSurvivorDesc(_d)");
        desc("setSurvivorDesc", &d.borrow());
        let tex = run(&mut mgr, "return _d:getHumanVisual():getSkinTexture()");
        println!("skinTexture {}", tex.first().and_then(|v| v.as_str()).unwrap_or("null"));
        let outfit = if seed == 777 { "Police" } else { "Generic01" };
        run(&mut mgr, &format!("_d:dressInNamedOutfit(\"{outfit}\")"));
        visual("dressed", &d.borrow().human_visual);
        worn("dressed", &d.borrow());
        println!("rand {} outfitRng {}", rand::next(1000000), outfit_rng::next(1000000));
        rand::set_seed(seed + 1);
        let r = run(&mut mgr, "_s = IsoSurvivor.new(_d, nil, 0, 0, 0); return _d:getInstance() == _s");
        let Value::Object(o) = global(&mgr, "_s") else { panic!("no survivor") };
        let chr = o.as_any().downcast_ref::<CharacterObject>().unwrap().0.clone();
        let c = chr.borrow();
        let [r1, g1, b1, a1] = c.speak_colour;
        println!(
            "survivor patience={} speak={} neighbour={} instance={} female={} descWorn={}",
            c.patience,
            fmt(r1 as f64, g1 as f64, b1 as f64, a1 as f64),
            c.neightbour_update,
            matches!(r.first(), Some(Value::Bool(true))),
            c.descriptor.as_ref().is_some_and(|d| d.borrow().is_female()),
            d.borrow().worn_items.items().len()
        );
        println!("rand {}", rand::next(1000000));
    }
}
