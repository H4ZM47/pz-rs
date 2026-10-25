//! Chunk files built from hand-made chunks: save, load, save again.

use super::chunk_file::{
    chunk_path, do_load_gridsquare, load_chunk, save_chunk, ChunkLoadHook, HookCtx,
};
use super::io::{Reader, Writer};
use super::items::{AllItems, ItemBlob, ItemRegistry};
use super::lua_table::{LuaKey, LuaTable, LuaValue};
use super::object_io::{load_object, save_object};
use super::visual::{CorpseVisual, HumanVisual, IsoDeadBody, MovingBase};
use super::*;
use crate::chunk::{FloorBloodSplat, IsoChunk, IsoGridSquare};
use crate::erosion_data::ErosionSquareData;
use crate::objects::*;
use crate::sprite::test_tables;

fn sprites() -> IsoSpriteManager {
    let mut m = IsoSpriteManager::new(test_tables());
    for (i, n) in [
        "floor_0",
        "door_0",
        "door_1",
        "window_0",
        "tree_0",
        "overlay_blood_0",
        "wall_0",
    ]
    .iter()
    .enumerate()
    {
        let s = m.add_sprite(n, 100 + i as i32);
        m.sprites[s].name = Some(n.to_string());
    }
    m
}

fn blob(reg: i16, id: i32, extra: &[u8]) -> ItemBlob {
    let mut d = reg.to_be_bytes().to_vec();
    d.push(0xff);
    d.extend_from_slice(&id.to_be_bytes());
    d.extend_from_slice(extra);
    ItemBlob::Raw(d)
}

fn ctx<'a>(
    m: &'a IsoSpriteManager,
    t: &'a SaveTables,
    s: &'a SaveSettings,
    items: &'a dyn ItemRegistry,
) -> SaveCtx<'a> {
    SaveCtx {
        sprites: m,
        tables: t,
        settings: s,
        items,
        hooks: &NoSaveHooks,
        meta: None,
    }
}

fn body(parts: usize) -> IsoDeadBody {
    let mut c = ItemContainer::new(Some("inventorymale"));
    c.id = 42;
    c.items.push(blob(9, 77, &[1, 2, 3]));
    c.items.push(blob(9, 78, &[1, 2, 3]));
    IsoDeadBody {
        base: MovingBase {
            offset_x: 64.0,
            offset_y: 192.0,
            x: 10.5,
            y: 20.25,
            z: 0.0,
            dir: 3,
            mod_data: None,
        },
        female: false,
        was_zombie: true,
        animal: None,
        object_id: (5, 4),
        persistent_outfit_id: 123,
        desc: None,
        visual: CorpseVisual::Human(Box::new(HumanVisual::new(parts))),
        container: Some(c),
        worn_items: vec![("base:shirt".into(), 1)],
        attached_items: vec![],
        death_time: 1.5,
        reanimate_time: 0.0,
        fall_on_front: true,
        killed_by_fall: false,
        was_skeleton: false,
        angle: 0.5,
        zombie_rot_stage_at_death: 1,
        animal_rot_stage_at_death: 0,
        rotten_texture: String::new(),
        skel_inv_icon: String::new(),
        crawling: false,
        fake_dead: false,
        ragdoll: None,
        burn_timer: 0.0,
    }
}

/// A chunk with one square of each kind of content.
fn sample_chunk(m: &IsoSpriteManager, t: &SaveTables) -> IsoChunk {
    let idx = |n: &str| m.named_map[n];
    let mut ch = IsoChunk::new(10, 20);
    ch.saved.floor_blood_splats.push(FloorBloodSplat {
        x: 2.0,
        y: 3.0,
        z: 0.0,
        kind: 2,
        world_age: 1.0,
        index: 4,
    });
    ch.saved.generators = Some(vec![(80, 160, 0)]);
    ch.saved.spawned_rooms = vec![7, 9];
    ch.saved.attachments_partial = Some(vec![(1, 2, 3)]);
    for x in 0..8 {
        for y in 0..8 {
            ch.set_square(x, y, 0, Some(IsoGridSquare::new(80 + x, 160 + y, 0)));
        }
    }
    let sq = ch.get_grid_square_mut(0, 0, 0).unwrap();
    sq.erosion = Some(ErosionSquareData {
        init: true,
        soil: 2,
        ..Default::default()
    });
    sq.seen = [true, false, true, false];
    sq.have_roof = true;
    let mut floor = IsoObject::new(sq.new_object_id(), ObjectKind::Object, Some(idx("floor_0")));
    floor.attach_anim_sprite(idx("wall_0"));
    floor.name = Some(ObjectName::Str("Grass".into()));
    floor.custom_color = Some([0.5, 1.0, 0.0]);
    let mut md = LuaTable::default();
    md.rawset(LuaKey::Str("k".into()), Some(LuaValue::Num(1.0)));
    floor.mod_data = Some(md);
    floor.wall_blood_splats = Some(vec![WallBloodSplat {
        world_age: 0.5,
        sprite: Some(idx("overlay_blood_0")),
    }]);
    let mut c = ItemContainer::new(Some("crate"));
    c.items.push(blob(3, 1, &[9]));
    floor.container = Some(c);
    floor.components.push(ComponentBlob {
        type_id: 17,
        data: vec![1, 2],
    });
    floor.has_components = true;
    sq.objects.push(floor);
    let door = IsoDoor {
        north: true,
        health: 500,
        max_health: 500,
        open_sprite: Some(idx("door_1")),
        closed_sprite: Some(idx("door_0")),
        has_curtain: true,
        curtain_open: true,
        ..Default::default()
    };
    let mut d = IsoObject::new(
        sq.new_object_id(),
        ObjectKind::Door(door),
        Some(idx("door_0")),
    );
    d.outline_on_mouseover = true;
    d.key_id = 77;
    let did = d.id;
    sq.objects.push(d);
    sq.special_objects.push(did);
    let sq = ch.get_grid_square_mut(1, 0, 0).unwrap();
    let wi = IsoWorldInventoryObject {
        xoff: 0.25,
        yoff: 0.5,
        zoff: 0.0,
        offset_x: 0.0,
        offset_y: 0.0,
        item: blob(4, 11, &[]),
        drop_time: 2.0,
        ignore_remove_sandbox: true,
        extended_placement: false,
    };
    let mut o = IsoObject::new(
        sq.new_object_id(),
        ObjectKind::WorldInventory(Box::new(wi)),
        None,
    );
    o.sprite_id_override = Some(object_io::UNREGISTERED_SPRITE_ID);
    sq.world_objects.push(o.id);
    sq.objects.push(o);
    let tree = IsoTree {
        size: 1,
        log_yield: 3,
        damage: 120,
    };
    let id = sq.new_object_id();
    sq.objects.push(IsoObject::new(
        id,
        ObjectKind::Tree(tree),
        Some(idx("tree_0")),
    ));
    let th = IsoThumpable {
        is_door: true,
        light_source_fuel: Some(12),
        locked_by_code: 1234,
        ..Default::default()
    };
    let mut o = IsoObject::new(
        sq.new_object_id(),
        ObjectKind::Thumpable(Box::new(th)),
        Some(idx("wall_0")),
    );
    o.key_id = 5;
    sq.objects.push(o);
    let gen = IsoGenerator {
        connected: true,
        activated: false,
        fuel: 4.0,
        condition: 90,
        last_hour: 7,
        total_power_using: 0.0,
    };
    let id = sq.new_object_id();
    sq.objects.push(IsoObject::new(
        id,
        ObjectKind::Generator(gen),
        Some(idx("wall_0")),
    ));
    let id = sq.new_object_id();
    sq.objects.push(IsoObject::new(
        id,
        ObjectKind::Barricade(IsoBarricade {
            dir: 2,
            plank_health: [100, 0, 0, 0],
            metal_health: 0,
            metal_bar_health: 0,
        }),
        None,
    ));
    sq.static_moving.push(body(t.blood_body_parts));
    sq.burnt_out = true;
    ch
}

#[test]
fn chunk_round_trip() {
    let m = sprites();
    let t = SaveTables::for_tests();
    // Saved ages are clamped to the world age on load.
    let s = SaveSettings {
        world_age_hours: 50.0,
        ..Default::default()
    };
    let c = ctx(&m, &t, &s, &AllItems);
    let ch = sample_chunk(&m, &t);
    let bytes = save_chunk(&ch, &c).unwrap();
    assert!(chunk_file::validate_header(&bytes));
    let loaded = load_chunk(&bytes, 10, 20, &c).unwrap();
    assert!(loaded.crc_ok);
    assert_eq!(loaded.world_version, WORLD_VERSION);
    let back = loaded.chunk;
    assert_eq!(back.saved.floor_blood_splats.len(), 1);
    assert_eq!(back.saved.next_splat_index, 4);
    assert_eq!(back.saved.generators, Some(vec![(80, 160, 0)]));
    let sq = back.get_grid_square(0, 0, 0).unwrap();
    assert_eq!(sq.objects.len(), 2);
    assert_eq!(sq.special_objects, vec![sq.objects[1].id]);
    assert_eq!(sq.seen, [true, false, true, false]);
    // "Grass" is saved as the shortcut.
    assert_eq!(sq.objects[0].name, Some(ObjectName::Grass));
    assert_eq!(sq.objects[0].components.len(), 1);
    let ObjectKind::Door(d) = &sq.objects[1].kind else {
        panic!()
    };
    assert!(d.has_curtain && d.curtain_open && !d.curtain_inside);
    assert_eq!(sq.objects[1].key_id, 77);
    let sq1 = back.get_grid_square(1, 0, 0).unwrap();
    assert_eq!(sq1.world_objects.len(), 1);
    assert_eq!(sq1.static_moving.len(), 1);
    assert_eq!(
        sq1.static_moving[0]
            .container
            .as_ref()
            .unwrap()
            .item_count(),
        2
    );
    let ObjectKind::Tree(tr) = &sq1.objects[1].kind else {
        panic!()
    };
    assert_eq!((tr.log_yield, tr.damage), (3, 120));
    // The generator's modData gains an unsaveable entry: the table is
    // written (empty) on the next save too.
    assert!(sq1.objects[3].mod_data.is_some());
    // Saving the loaded chunk reproduces the bytes.
    assert_eq!(save_chunk(&back, &c).unwrap(), bytes);
    // Empty squares above ground are not saved; the levels shrink.
    let mut ch2 = sample_chunk(&m, &t);
    ch2.set_square(0, 0, 3, Some(IsoGridSquare::new(80, 160, 3)));
    assert_eq!(ch2.max_level, 3);
    let b2 = save_chunk(&ch2, &c).unwrap();
    assert_eq!(b2, bytes);
    assert_eq!(
        chunk_path(std::path::Path::new("s"), 10, 20),
        std::path::Path::new("s/map/10/20.bin")
    );
}

#[test]
fn load_transforms() {
    let m = sprites();
    let t = SaveTables::for_tests();
    let s = SaveSettings {
        world_age_hours: 100.0,
        world_item_removal_list: vec!["Base.Hat".into()],
        ..Default::default()
    };
    struct Reg;
    impl ItemRegistry for Reg {
        fn exists(&self, r: i16) -> bool {
            r != 3
        }
        fn full_type(&self, r: i16) -> Option<String> {
            Some(if r == 4 { "Base.Hat" } else { "Base.Other" }.into())
        }
    }
    let c0 = ctx(&m, &t, &s, &AllItems);
    let mut ch = sample_chunk(&m, &t);
    // An attached overlay_blood sprite becomes a wall blood splat.
    let floor = &mut ch.get_grid_square_mut(0, 0, 0).unwrap().objects[0];
    floor.attach_anim_sprite(m.named_map["overlay_blood_0"]);
    // An empty attached-sprite list is saved with a zero count and loads
    // back as no list.
    let d = &mut ch.get_grid_square_mut(0, 0, 0).unwrap().objects[1];
    d.attached_anim_sprite = Some(Vec::new());
    let bytes = save_chunk(&ch, &c0).unwrap();
    let c = ctx(&m, &t, &s, &Reg);
    let back = load_chunk(&bytes, 10, 20, &c).unwrap().chunk;
    let sq = back.get_grid_square(0, 0, 0).unwrap();
    let f = &sq.objects[0];
    assert_eq!(f.attached_anim_sprite.as_ref().unwrap().len(), 1);
    let splats = f.wall_blood_splats.as_ref().unwrap();
    assert_eq!(splats.len(), 2);
    // Attached sprites load before the saved splats.
    assert_eq!(splats[0].world_age, 100.0);
    assert_eq!(splats[1].world_age, 0.5);
    // The container lost the item the registry cannot create.
    assert_eq!(f.container.as_ref().unwrap().item_count(), 0);
    assert_eq!(sq.objects[1].attached_anim_sprite, None);
    // The hat on the floor (dropped at hour 2) is past the 24h removal, but
    // its ignoreRemoveSandbox flag keeps it.
    assert_eq!(
        back.get_grid_square(1, 0, 0).unwrap().world_objects.len(),
        1
    );
    let mut ch = sample_chunk(&m, &t);
    if let ObjectKind::WorldInventory(w) =
        &mut ch.get_grid_square_mut(1, 0, 0).unwrap().objects[0].kind
    {
        w.ignore_remove_sandbox = false;
    }
    let bytes = save_chunk(&ch, &c0).unwrap();
    let back = load_chunk(&bytes, 10, 20, &c).unwrap().chunk;
    let sq1 = back.get_grid_square(1, 0, 0).unwrap();
    assert!(sq1.world_objects.is_empty());
    assert!(!sq1
        .objects
        .iter()
        .any(|o| matches!(o.kind, ObjectKind::WorldInventory(_))));
}

#[test]
fn errors_and_versions() {
    let m = sprites();
    let t = SaveTables::for_tests();
    let s = SaveSettings::default();
    let c = ctx(&m, &t, &s, &AllItems);
    let mut bytes = save_chunk(&sample_chunk(&m, &t), &c).unwrap();
    bytes[1..5].copy_from_slice(&250i32.to_be_bytes());
    assert!(load_chunk(&bytes, 10, 20, &c).is_err());
    bytes[1..5].copy_from_slice(&WORLD_VERSION.to_be_bytes());
    bytes[40] ^= 1;
    // A CRC mismatch is only reported.
    if let Ok(l) = load_chunk(&bytes, 10, 20, &c) {
        assert!(!l.crc_ok);
    }
    assert!(load_chunk(&bytes[..30], 10, 20, &c).is_err());
    // Unsupported classes.
    let mut w = Writer::new();
    w.i8(1);
    w.i8(t.class_id("Zombie"));
    assert!(load_object(&mut Reader::new(&w.buf), &c, WORLD_VERSION, 1).is_err());
    let mut w = Writer::new();
    w.i8(0);
    assert_eq!(
        load_object(&mut Reader::new(&w.buf), &c, WORLD_VERSION, 1).unwrap(),
        None
    );
    // A map mannequin has no saved state yet.
    let o = IsoObject::new(1, ObjectKind::Mannequin(None), None);
    assert!(save_object(&mut Writer::new(), &c, &o).is_err());
    assert!(SaveTables::from_text("1:Player\nbloodBodyParts=3").is_err());
    let tt = SaveTables::from_text("0:IsoObject\n5:Door\nbloodBodyParts=3").unwrap();
    assert_eq!(tt.class_id("Door"), 5);
    assert_eq!(tt.class_id("Nope"), 0);
}

#[test]
fn erosion_hook_order() {
    struct Log(Vec<(i32, i32, i32)>, bool);
    impl ChunkLoadHook for Log {
        fn load_grid_square(
            &mut self,
            _c: &mut IsoChunk,
            x: i32,
            y: i32,
            z: i32,
            _ctx: &mut HookCtx<'_>,
        ) {
            self.0.push((x, y, z));
        }
        fn chunk_loaded(&mut self, _c: &mut IsoChunk, _ctx: &mut HookCtx<'_>) {
            self.1 = true;
        }
    }
    let mut m = sprites();
    let t = SaveTables::for_tests();
    let mut ch = sample_chunk(&m, &t);
    let cell = crate::chunk::IsoCell::default();
    let mut ctx = HookCtx {
        sprites: &mut m,
        neighbours: &cell,
    };
    let mut log = Log(Vec::new(), false);
    do_load_gridsquare(&mut ch, Some(&mut log), &mut ctx);
    assert_eq!(log.0, vec![(0, 0, 0), (1, 0, 0)]);
    assert!(log.1);
    do_load_gridsquare(&mut ch, None, &mut ctx);
}

#[test]
fn finish_chunk_load_recalculates_before_hook() {
    struct Seen(Vec<bool>, bool);
    impl ChunkLoadHook for Seen {
        fn load_grid_square(
            &mut self,
            c: &mut IsoChunk,
            x: i32,
            y: i32,
            z: i32,
            ctx: &mut HookCtx<'_>,
        ) {
            // The chunk is out of the cell while hooks run.
            self.1 |= ctx
                .neighbours
                .get_grid_square(c.wx * 8 + x, c.wy * 8 + y, z)
                .is_some();
            let sq = c.get_grid_square(x, y, z).unwrap();
            let mut fresh = sq.clone();
            fresh.recalc_properties(ctx.sprites);
            self.0
                .push(format!("{:?}", fresh.properties) == format!("{:?}", sq.properties));
        }
        fn chunk_loaded(&mut self, _c: &mut IsoChunk, _ctx: &mut HookCtx<'_>) {}
    }
    let mut m = sprites();
    let t = SaveTables::for_tests();
    let ch = sample_chunk(&m, &t);
    let (wx, wy) = (ch.wx, ch.wy);
    let mut cell = crate::chunk::IsoCell::default();
    cell.chunks.insert((wx, wy), ch);
    let meta = crate::meta_grid::IsoMetaGrid::empty();
    let mut seen = Seen(Vec::new(), false);
    crate::save::chunk_file::finish_chunk_load(&mut cell, wx, wy, &meta, &mut m, Some(&mut seen));
    assert!(!seen.0.is_empty() && seen.0.iter().all(|&b| b));
    assert!(!seen.1);
    assert!(cell.chunks.contains_key(&(wx, wy)));
}
