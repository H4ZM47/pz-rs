//! Local parity probe: zombie dressing (`PersistentOutfits.init`,
//! `pickOutfit` / `getOutfit`, `IsoZombie.dressInPersistentOutfitID` and
//! `ModelManager.dressInRandomOutfit` with `ZombiesZoneDefinition` over a
//! small cell) with seeded `Rand` and `OutfitRNG`, printed in the format of
//! the local reference oracle.
//! Usage: `zombie_dress_probe <game dir> <scratch cache dir> <scenarios>`.

use pz_characters::character::zombie::dress::{DressSettings, ZombieDressCx};
use pz_characters::population::zombie_defs::{ZoneView, ZoneWorld};
use pz_characters::{CharacterKind, GameCharacter};
use pz_core::random::{self as rand, outfit_rng};
use pz_inventory::visual::{ItemVisual, Rgb};
use pz_inventory::ItemKind;
use pz_lua_api::bind_zombie_outfits::{init_persistent_outfits, shared_zombie_outfits, LuaDressWorld};
use pz_lua_api::LuaManager;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

fn b(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

fn col(c: Option<Rgb>) -> String {
    c.map_or("null".into(), |[r, g, bl]| format!("{},{},{}", b(r), b(g), b(bl)))
}

fn s(o: Option<&str>) -> String {
    o.unwrap_or("null").to_string()
}

fn bytes(a: Option<&[i8]>) -> String {
    a.map_or("null".into(), |a| a.iter().map(|v| format!("{:02x}", *v as u8)).collect())
}

fn item_visual(tag: &str, v: &ItemVisual) {
    println!(
        "{tag} type={} ci={} tint={} hue={} base={} choice={} decal={} blood={} dirt={} holes={}",
        v.full_type,
        v.clothing_item_name,
        col(v.tint),
        b(v.hue),
        v.base_texture,
        v.texture_choice,
        s(v.decal.as_deref()),
        bytes(v.arrays[0].as_deref()),
        bytes(v.arrays[1].as_deref()),
        bytes(v.arrays[2].as_deref())
    );
}

fn dump(z: &GameCharacter<i32>) {
    let d = z.descriptor.as_ref().unwrap().borrow();
    println!(
        " Z id={} init={} rnd={} female={} fore={} voice={} sprite={} hurt={}",
        z.zombie.persistent_outfit_id,
        z.zombie.persistent_outfit_init,
        z.zombie.dress_in_random_outfit,
        d.is_female(),
        s(d.forename.as_deref()),
        s(d.voice_prefix.as_deref()),
        z.zombie.sprite_name,
        if z.zombie.hurt_sound.is_empty() { "null" } else { &z.zombie.hurt_sound }
    );
    let hv = z.human_visual.as_ref().unwrap();
    println!(
        " HV skin={} name={} rot={} bodyHair={} hair={} beard={} hc={} bc={} nhc={} nbc={} sc={} outfit={}",
        hv.skin_texture,
        s(hv.skin_texture_name.as_deref()),
        hv.zombie_rot_stage,
        hv.body_hair,
        s(hv.hair_model.as_deref()),
        s(hv.beard_model.as_deref()),
        col(hv.hair_color),
        col(hv.beard_color),
        col(hv.natural_hair_color),
        col(hv.natural_beard_color),
        col(hv.skin_color),
        s(hv.outfit.as_deref())
    );
    println!(" HB blood={} dirt={} holes={}", bytes(Some(&hv.blood)), bytes(Some(&hv.dirt)), bytes(Some(&hv.holes)));
    println!(" BV {}", hv.body_visuals.len());
    for v in &hv.body_visuals {
        item_visual("  B", v);
    }
    println!(" IV {}", z.zombie.item_visuals.len());
    for v in &z.zombie.item_visuals {
        item_visual("  I", v);
    }
    println!(" AT {}", z.zombie.attached_items.len());
    for (loc, it) in &z.zombie.attached_items {
        let w = match &it.kind {
            ItemKind::Weapon(wd) => format!(
                " ammo={} clip={} chambered={} blood={}",
                it.get_current_ammo_count(),
                it.is_contains_clip(),
                it.is_round_chambered(),
                b(wd.blood_level)
            ),
            _ => String::new(),
        };
        println!("  A loc={loc} type={} cond={}/{}{w}", it.get_full_type(), it.condition, it.condition_max);
    }
    let mut sd = String::from(" SD");
    for it in &z.zombie.items_to_spawn_at_death {
        sd += &format!(" {}", it.get_full_type());
    }
    println!("{sd}");
}

/// The oracle's one-chunk cell: squares with rooms and the zones of
/// meta chunk (0, 0), in its order.
struct Cell {
    squares: Vec<((i32, i32), Option<&'static str>)>,
    zones: Vec<(&'static str, &'static str, i32, i32, i32, i32)>,
}

impl ZoneWorld for Cell {
    fn square_room(&self, x: i32, y: i32, z: i32) -> Option<Option<String>> {
        if z != 0 {
            return None;
        }
        self.squares.iter().find(|(p, _)| *p == (x, y)).map(|(_, r)| r.map(str::to_string))
    }
    fn zones_at(&self, x: i32, y: i32, z: i32) -> Vec<ZoneView> {
        self.zones
            .iter()
            .enumerate()
            .filter(|(_, &(_, _, zx, zy, w, h))| z == 0 && x >= zx && x < zx + w && y >= zy && y < zy + h)
            .map(|(i, &(name, ty, ..))| ZoneView { key: i as u64, name: Some(name.into()), zone_type: Some(ty.into()), id: Some(i as u128) })
            .collect()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game dir");
    let cache = args.next().expect("scratch cache dir");
    let count: usize = args.next().map_or(60, |c| c.parse().expect("count"));
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
    for f in [
        "shared/NPCs/BodyLocations.lua",
        "shared/NPCs/AttachedLocations.lua",
        "shared/Definitions/HairOutfitDefinitions.lua",
        "shared/Definitions/DefaultClothing.lua",
        "shared/NPCs/MainCreationMethods.lua",
        "shared/NPCs/ZombiesZoneDefinition.lua",
        "shared/Definitions/UnderwearDefinition.lua",
        "shared/Definitions/AttachedWeaponDefinitions.lua",
    ] {
        let path = format!("{game}/media/lua/{f}");
        pz_lua_api::lua_manager::run_lua(&mut mgr.thread, &mgr.loader, &path);
    }
    let src = r#"
        BaseGameCharacterDetails.DoHairColor()
        for _, n in ipairs({"Ann", "Beth", "Cleo"}) do SurvivorFactory.addFemaleForename(n) end
        for _, n in ipairs({"Carl", "Dan"}) do SurvivorFactory.addMaleForename(n) end
        for _, n in ipairs({"Eve", "Fox", "Gray"}) do SurvivorFactory.addSurname(n) end
    "#;
    let f = pz_kahlua::stdlib::load_string(src, Some("probe"), mgr.thread.globals().clone()).expect("compile");
    if let Err((e, t)) = mgr.thread.pcall(&f, &[]) {
        eprintln!("probe error: {e:?}\n{t}");
    }

    rand::set_seed(42);
    let names = init_persistent_outfits(&mgr.thread, None);
    println!("NAMES {}", names.len());
    for (i, n) in names.iter().enumerate() {
        println!("N {i} {n}");
    }
    let outfits = shared_zombie_outfits();
    let seeds: Vec<String> = outfits.borrow().persistent.seeds.iter().map(|v| format!("{v:x}")).collect();
    println!("SEEDS {}", seeds.join(" "));
    println!("R {}", rand::next(1000000));

    let cell = Cell {
        squares: vec![
            ((1, 1), None),
            ((2, 2), Some("church")),
            ((3, 3), Some("medicalstorage")),
            ((4, 4), Some("kitchen")),
            ((5, 5), None),
            ((6, 6), Some("bedroom")),
            ((7, 7), None),
        ],
        zones: vec![
            ("", "TownZone", 0, 0, 8, 8),
            ("", "Army", 0, 0, 2, 2),
            ("Prison", "ZombiesType", 2, 2, 2, 2),
            ("Wedding", "ZombiesType", 2, 2, 1, 1),
            ("x", "Police", 4, 4, 1, 1),
            ("", "Police", 5, 5, 1, 1),
            ("", "Rich", 5, 5, 2, 2),
        ],
    };
    let squares = [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (7, 7)];
    let names_list = [
        "Generic01", "Police", "Bandit", "Survivalist02", "ZZD.Army.ArmyCamoGreen", "Nurse", "Survivalist", "ZZD.Prison.Priest", "Generic_Skirt",
        "Santa", "ZZD.Wedding.WeddingDress", "Nope",
    ];
    const DAYS: [i32; 6] = [0, 7, 20, 45, 100, 230];
    const CHANCES: [i32; 3] = [6, 100, 30];
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let chars = pz_lua_api::bind_characters::shared_characters();
    let gt = pz_lua_api::bind_climate::shared_game_time();
    for k in 0..count {
        let (days, deg, chance, op, v) = (DAYS[k % 6], 1 + (k as i32 / 6) % 4, CHANCES[(k / 24) % 3], k % 6, k / 6);
        outfits.borrow_mut().weapons.dirty = true;
        {
            let mut g = gt.borrow_mut();
            g.nights_survived = days;
            g.time_of_day = 7.0;
        }
        rand::set_seed(1000 + k as i64);
        let d = chars.borrow_mut().create_survivor::<i32>(&sm).expect("CreateSurvivor");
        let female = d.is_female();
        outfit_rng::set_seed(31 * k as i64 + 5);
        let mut z: GameCharacter<i32> = GameCharacter::new(&sm, CharacterKind::Zombie);
        z.descriptor = Some(Rc::new(RefCell::new(d)));
        z.human_visual = Some(pz_inventory::visual::HumanVisual::new(18));
        z.zombie.sprite_name = "BobZ".into();
        println!("S {k} op={op} days={days} deg={deg} chance={chance} female={female}");
        let globals = mgr.thread.globals().clone();
        let r = pz_lua_api::bind_appearance::with_dress_context(&mgr.thread, |dress| {
            let chars = chars.borrow();
            let mut o = outfits.borrow_mut();
            let mut create = |t: &str| pz_lua_api::bind_items::create_item(t, 1.0).map(|i| i.borrow().clone());
            let mut world = LuaDressWorld { globals, zones: &cell, create_item: &mut create, loot_modifier: &|_| 1.0, chance_of_attached_weapon: chance };
            let world_age = gt.borrow().world_age_days_since_begin(1);
            let mut cx = ZombieDressCx {
                chars: &chars,
                dress,
                outfits: &mut o,
                world: &mut world,
                settings: DressSettings {
                    clothing_degradation: deg,
                    remove_zombie_loot: false,
                    tutorial: false,
                    world_age_days: world_age as f32,
                    world_age_days_since_begin: world_age,
                },
            };
            match op {
                0 | 2 => {
                    let p = &cx.outfits.persistent;
                    let mut id = if female { p.pick_random_female() } else { p.pick_random_male() };
                    if op == 2 {
                        id |= 0x8000;
                    }
                    println!(" pick {id}");
                    z.dress_in_persistent_outfit_id(id, &mut cx)
                }
                1 => {
                    let name = names_list[v % names_list.len()];
                    let adv = pz_characters::population::zombie_defs::advanced_outfit_name(name, world_age);
                    let id = cx.outfits.persistent.pick_outfit(name, adv.as_deref(), female);
                    println!(" pick {id}");
                    z.dress_in_persistent_outfit_id(id, &mut cx)
                }
                3 => {
                    let (x, y) = squares[v % squares.len()];
                    z.path.current_square = Some((x, y, 0));
                    z.zombie.dress_in_random_outfit = true;
                    z.model_manager_dress_in_random_outfit(&mut cx)
                }
                4 => {
                    let id = cx.outfits.persistent.pick_random_female();
                    println!(" pick {id}");
                    z.zombie.persistent_outfit_id = id;
                    z.model_manager_dress_in_random_outfit(&mut cx)
                }
                _ => {
                    let n = cx.outfits.persistent.outfit_names().len();
                    let id = (((v * 7) % n) << 16) as i32 | if female { i32::MIN } else { 0 };
                    println!(" pick {id}");
                    z.dress_in_persistent_outfit_id(id, &mut cx)
                }
            }
        });
        match r {
            None => println!(" EXC no dress context"),
            Some(Err(e)) => println!(" EXC {e}"),
            Some(Ok(())) => {}
        }
        dump(&z);
        println!(" R {} {}", rand::next(1000000), outfit_rng::next(1000000));
    }
}
