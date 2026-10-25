//! `IsoObject.factoryFromFileInput`, `IsoObject.load`/`save` and the
//! `load`/`save` overrides of the object classes chunk files hold.

use super::animal::IsoAnimal;
use super::io::{err, has, BitHeaderWrite, Reader, SaveResult, Writer};
use super::items::ItemBlob;
use super::lua_table::{present, LuaKey, LuaTable, LuaValue};
use super::visual::{load_container, save_container, HumanVisual};
use super::SaveCtx;
use crate::objects::*;
use crate::sprite::SpriteIdx;

/// `IsoSprite.CreateSprite` gives a sprite this id.
pub const UNREGISTERED_SPRITE_ID: i32 = 20_000_000;

/// The object `IsoObject.factoryFromFileInput` creates for `name`, in its
/// state before `load`. `Err` for classes chunk files cannot hold here
/// (characters, vehicles, animals) or that are not ported.
fn instantiate(name: Option<&str>) -> SaveResult<ObjectKind> {
    Ok(match name {
        None | Some("IsoObject") => ObjectKind::Object,
        Some("Jukebox") => ObjectKind::Jukebox,
        Some("Curtain") => ObjectKind::Curtain(IsoCurtain::default()),
        Some("Radio") => ObjectKind::Radio(DeviceData::default()),
        Some("Television") => ObjectKind::Television(DeviceData::default()),
        Some("Barbecue") => ObjectKind::Barbecue(IsoBarbecue::default()),
        Some("ClothingDryer") => ObjectKind::ClothingDryer(DryerLogic::default()),
        Some("ClothingWasher") => ObjectKind::ClothingWasher(WasherLogic::default()),
        Some("Fireplace") => ObjectKind::Fireplace(IsoFireplace::default()),
        Some("Stove") => ObjectKind::Stove(IsoStove::default()),
        Some("Door") => ObjectKind::Door(IsoDoor::default()),
        Some("Thumpable") => ObjectKind::Thumpable(Box::default()),
        Some("IsoBrokenGlass") => ObjectKind::BrokenGlass,
        Some("Mannequin") => ObjectKind::Mannequin(None),
        Some("Window") => ObjectKind::Window(IsoWindow::default()),
        Some("Tree") => ObjectKind::Tree(IsoTree {
            size: 1,
            log_yield: 1,
            damage: 500,
        }),
        Some("LightSwitch") => ObjectKind::LightSwitch(IsoLightSwitch::default()),
        Some("CombinationWasherDryer") => {
            ObjectKind::CombinationWasherDryer(WasherDryer::default())
        }
        Some("StackedWasherDryer") => ObjectKind::StackedWasherDryer(WasherDryer::default()),
        Some("ButcherHook") => ObjectKind::ButcherHook,
        Some("IsoWindowFrame") => ObjectKind::WindowFrame { north: false },
        Some("IsoAnimalTrack") => ObjectKind::AnimalTrack(Box::default()),
        // The remaining classes read every saved field in load; the kind is
        // built there.
        Some(
            n @ ("WorldInventoryItem"
            | "IsoCompost"
            | "FeedingTrough"
            | "Barricade"
            | "IsoHutch"
            | "IsoGenerator"
            | "IsoTrap"
            | "IsoCarBatteryCharger"
            | "Fire"
            | "Pushable"
            | "WheelieBin"),
        ) => return err(format!("deferred:{n}")),
        Some(n) => return err(format!("object class {n} is not supported in chunk files")),
    })
}

/// Reads one `IsoObject.factoryFromFileInput` + `load`. `Ok(None)` when the
/// serialise flag is clear (nothing follows).
pub fn load_object(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    id: ObjectId,
) -> SaveResult<Option<IsoObject>> {
    if !r.bool()? {
        return Ok(None);
    }
    let class_id = r.i8()?;
    // An unknown id makes a plain IsoObject (outside debug builds).
    let name = ctx.tables.class_name(class_id);
    let mut obj = IsoObject::new(id, ObjectKind::Object, None);
    match instantiate(name) {
        Ok(kind) => {
            obj.kind = kind;
            obj.components = ctx.hooks.factory_components(name.unwrap_or("IsoObject"));
            obj.has_components = !obj.components.is_empty();
            load_base(r, ctx, wv, &mut obj)?;
            load_tail(r, ctx, wv, &mut obj)?;
        }
        Err(e) if e.0.starts_with("deferred:") => {
            obj.components = ctx.hooks.factory_components(name.unwrap_or("IsoObject"));
            if name == Some("FeedingTrough") {
                feeding_trough_constructor(&mut obj);
            }
            obj.has_components = !obj.components.is_empty();
            load_special(r, ctx, wv, &mut obj, &e.0["deferred:".len()..])?;
        }
        Err(e) => return Err(e),
    }
    Ok(Some(obj))
}

/// `new IsoFeedingTrough(cell)`: `createFluidContainer()` while `maxWater`
/// is still 0 (the definition is read after `load`).
fn feeding_trough_constructor(obj: &mut IsoObject) {
    let fc = pz_entity::factory::feeding_trough_fluid_container(0.0);
    let blob = pz_inventory::component::encode_component(pz_entity::Component::FluidContainer(fc));
    // GameEntityFactory.AddComponent(this, true, fc) replaces one of the type.
    match obj
        .components
        .iter_mut()
        .find(|c| c.type_id == blob.type_id)
    {
        Some(c) => *c = blob,
        None => obj.components.push(blob),
    }
}

fn sprite_or_null(ctx: &SaveCtx<'_>, id: i32) -> Option<SpriteIdx> {
    ctx.sprites.idx_by_id(id)
}

/// `GameEntity.loadEntity`: through the item environment's component
/// codec, so components the constructor made load in place.
fn load_entity(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    obj: &mut IsoObject,
) -> SaveResult<()> {
    let loaded = ctx
        .items
        .components()
        .load_entity(r, wv, &mut obj.components)?;
    if loaded.contains(&SPRITE_CONFIG_ID) {
        sprite_config_added(ctx, obj);
    }
    obj.has_components = !obj.components.is_empty();
    Ok(())
}

/// `ComponentType.SpriteConfig`.
const SPRITE_CONFIG_ID: i16 = 3;

/// `SpriteConfig.load`, then `onAddedToOwner` (`initObjectInfo`) on the
/// object: what `save` writes after the script (`isMultiSquareMaster()`)
/// is the tile info of the object's sprite in the script's
/// `SpriteConfigManager` entry, not the saved flag. A script reference the
/// dictionary does not resolve leaves no script (saved as a single 0).
/// Without a script dictionary the bytes stay as loaded.
fn sprite_config_added(ctx: &SaveCtx<'_>, obj: &mut IsoObject) {
    let Some(c) = obj
        .components
        .iter_mut()
        .find(|c| c.type_id == SPRITE_CONFIG_ID)
    else {
        return;
    };
    // [has script] [has id, id] [script version] [master]
    let d = &c.data;
    if d.first().is_none_or(|&b| b == 0) {
        return;
    }
    let (id, master_at) = match d.get(1) {
        Some(0) => (None, 10),
        Some(_) if d.len() >= 4 => (Some(i16::from_be_bytes([d[2], d[3]])), 12),
        _ => return,
    };
    if d.len() != master_at + 1 {
        return;
    }
    let script = match id {
        // `loadScript` reads no id: no script.
        None => None,
        Some(id) => match ctx.items.script_dictionary_name("SpriteConfigs", id) {
            None => return,
            Some(name) => name.and_then(|n| ctx.sprites.sprite_config_scripts.get(&n)),
        },
    };
    let Some(config) = script else {
        // `configScript` stays null: `save` writes only the flag.
        c.data = vec![0];
        return;
    };
    let sprite = obj
        .sprite
        .and_then(|s| ctx.sprites.sprites[s].name.as_deref());
    c.data[master_at] = ctx.sprites.is_multi_square_master(config, sprite) as u8;
}

fn save_entity(w: &mut Writer, obj: &IsoObject) -> SaveResult<()> {
    if obj.has_components && obj.components.is_empty() {
        // Components made by GameEntityFactory at map load have no saved
        // form here.
        return err("object has entity components without saved data");
    }
    w.i8(obj.components.len() as i8);
    for c in &obj.components {
        w.i32(c.data.len() as i32 + 2);
        w.i16(c.type_id);
        w.bytes(&c.data);
    }
    Ok(())
}

/// `IsoObject.load` (after the factory bytes).
fn load_base(r: &mut Reader<'_>, ctx: &SaveCtx<'_>, wv: i32, o: &mut IsoObject) -> SaveResult<()> {
    let m = ctx.sprites;
    let sid = r.i32()?;
    o.sprite = if sid == -1 {
        m.named_map.get("").copied()
    } else {
        sprite_or_null(ctx, sid)
    };
    let header = r.i8()? as i64 as u64;
    if header != 0 {
        if has(header, 1) {
            let child = if has(header, 2) {
                1
            } else {
                (r.i8()? as u8) as i32
            };
            for _ in 0..child {
                let list = o.attached_anim_sprite.get_or_insert_with(Vec::new);
                let wtf = r.i32()?;
                let spr = sprite_or_null(ctx, wtf);
                let mut a = spr.map(IsoSpriteInstance::new);
                let flags = r.i8()?;
                let read_data = flags & 2 != 0;
                let read_alpha = flags & 0x10 != 0;
                if let Some(a) = a.as_mut() {
                    if flags & 4 != 0 {
                        a.flip = true;
                    }
                    if flags & 8 != 0 {
                        a.copy_target_alpha = true;
                    }
                    if read_alpha {
                        a.multiply_object_alpha = true;
                    }
                }
                if read_data {
                    let (x, y, z) = (r.f32()?, r.f32()?, r.f32()?);
                    let (cr, cg, cb) = (r.unit_byte()?, r.unit_byte()?, r.unit_byte()?);
                    if let Some(a) = a.as_mut() {
                        a.off_x = x;
                        a.off_y = y;
                        a.off_z = z;
                        a.tint_r = cr;
                        a.tint_g = cg;
                        a.tint_b = cb;
                    }
                } else if let Some(a) = a.as_mut() {
                    a.off_x = 0.0;
                    a.off_y = 0.0;
                    a.off_z = 0.0;
                    a.tint_r = 1.0;
                    a.tint_g = 1.0;
                    a.tint_b = 1.0;
                    a.alpha = 1.0;
                    a.target_alpha = 1.0;
                }
                if read_alpha {
                    let alpha = r.f32()?;
                    if let Some(a) = a.as_mut() {
                        a.alpha = alpha;
                    }
                }
                let (Some(s), Some(a)) = (spr, a) else {
                    continue;
                };
                if m.sprites[s]
                    .name
                    .as_deref()
                    .is_some_and(|n| n.starts_with("overlay_blood_"))
                {
                    o.wall_blood_splats
                        .get_or_insert_with(Vec::new)
                        .push(WallBloodSplat {
                            world_age: ctx.settings.world_age_hours as f32,
                            sprite: Some(s),
                        });
                    continue;
                }
                list.push(a);
            }
        }
        if has(header, 4) {
            let flags = r.i8()?;
            if flags & 2 != 0 {
                o.name = Some(ObjectName::Grass);
            } else if flags & 4 != 0 {
                o.name = Some(ObjectName::Dict(r.i8()?));
            } else if flags & 8 != 0 {
                o.name = Some(ObjectName::Str(r.string()?));
            }
            if flags & 0x10 != 0 {
                o.sprite_name = Some(SpriteName::Dict(r.i32()?));
            } else if flags & 0x20 != 0 {
                o.sprite_name = Some(SpriteName::Str(r.string()?));
            }
        }
        if has(header, 8) {
            o.custom_color = Some([r.unit_byte()?, r.unit_byte()?, r.unit_byte()?]);
        }
        o.do_not_sync = has(header, 16);
        o.outline_on_mouseover = has(header, 32);
        if has(header, 64) {
            let bits = super::io::read_header(r, 2)?;
            if has(bits, 1) {
                let count = r.i8()?;
                if count > 0 {
                    let list = o.wall_blood_splats.get_or_insert_with(Vec::new);
                    let lifespan = ctx.settings.blood_splat_lifespan_days;
                    let wah = ctx.settings.world_age_hours as f32;
                    for _ in 0..count {
                        let age = r.f32()?;
                        let spr = sprite_or_null(ctx, r.i32()?);
                        let age = if age < wah { age } else { wah };
                        if lifespan > 0 && wah - age >= (lifespan * 24) as f32 {
                            continue;
                        }
                        list.push(WallBloodSplat {
                            world_age: age,
                            sprite: spr,
                        });
                    }
                }
            }
            if has(bits, 2) {
                let n = r.i8()?;
                for i in 0..n.max(0) {
                    let c = load_container(r, wv, ctx.items)?;
                    if i == 0 {
                        o.container = Some(c);
                    } else {
                        o.secondary_containers.push(c);
                    }
                }
            }
            if has(bits, 4) {
                o.mod_data = Some(LuaTable::load(r, wv)?);
            }
            o.special_tooltip = has(bits, 8);
            if has(bits, 16) {
                o.key_id = r.i32()?;
            }
            o.uses_external_water_source = has(bits, 32);
            if has(bits, 64) {
                o.sheet_rope = true;
                o.sheet_rope_health = r.f32()?;
            } else {
                o.sheet_rope = false;
            }
            if has(bits, 128) {
                o.render_y_offset = r.f32()?;
            }
            if has(bits, 256) {
                let name = if has(bits, 512) {
                    SpriteName::Str(r.string()?)
                } else {
                    SpriteName::Dict(r.i32()?)
                };
                if !matches!(&name, SpriteName::Str(s) if s.is_empty()) {
                    o.overlay_sprite = Some(OverlaySprite { name, color: None });
                }
            }
            if has(bits, 1024) {
                let c = [
                    r.unit_byte()?,
                    r.unit_byte()?,
                    r.unit_byte()?,
                    r.unit_byte()?,
                ];
                if let Some(ov) = o.overlay_sprite.as_mut() {
                    ov.color = Some(c);
                }
            }
            o.moved_thumpable = has(bits, 2048);
            if has(bits, 4096) {
                load_entity(r, ctx, wv, o)?;
            }
            if has(bits, 8192) {
                o.sprite_model_name = Some(r.string()?);
            }
        }
    }
    if o.sprite.is_none() {
        o.sprite_id_override = Some(UNREGISTERED_SPRITE_ID);
    }
    Ok(())
}

/// The id `IsoObject.save` writes for the object's sprite.
pub fn sprite_save_id(ctx: &SaveCtx<'_>, o: &IsoObject) -> i32 {
    match o.sprite {
        Some(s) => ctx.sprite_id(s),
        None => o.sprite_id_override.unwrap_or(-1),
    }
}

/// `getObjectName()`.
pub fn object_name(ctx: &SaveCtx<'_>, o: &IsoObject) -> String {
    if let Some(n) = o.kind.object_name() {
        return n.to_string();
    }
    match &o.name {
        Some(ObjectName::Grass) => return "Grass".into(),
        Some(ObjectName::Str(s)) => return s.clone(),
        // A dictionary name never names a factory class.
        Some(ObjectName::Dict(_)) => return "IsoObject".into(),
        None => {}
    }
    if let Some(p) = o
        .sprite
        .and_then(|s| ctx.sprites.sprites[s].parent_object_name.as_deref())
    {
        return p.to_string();
    }
    "IsoObject".into()
}

/// `IsoObject.save`, with the `IsoMannequin`/`IsoFire`/`IsoGenerator`
/// tweaks applied by the caller through the arguments.
fn save_base(
    w: &mut Writer,
    ctx: &SaveCtx<'_>,
    o: &IsoObject,
    with_containers: bool,
    with_attached: bool,
    mod_data: Option<&LuaTable>,
) -> SaveResult<()> {
    w.i8(1);
    w.i8(ctx.class_id(&object_name(ctx, o))?);
    w.i32(sprite_save_id(ctx, o));
    let mut header = BitHeaderWrite::alloc(w, 1);
    if let Some(list) = o.attached_anim_sprite.as_ref().filter(|_| with_attached) {
        header.add(1);
        if list.len() == 1 {
            header.add(2);
        }
        let s = list.len().min(255);
        if s != 1 {
            w.i8(s as u8 as i8);
        }
        for a in &list[..s] {
            w.i32(ctx.sprite_id(a.sprite));
            let mut flags = 0i8;
            let write_data = a.off_x != 0.0
                || a.off_y != 0.0
                || a.off_z != 0.0
                || a.tint_r != 1.0
                || a.tint_g != 1.0
                || a.tint_b != 1.0;
            if write_data {
                flags |= 2;
            }
            if a.flip {
                flags |= 4;
            }
            if a.copy_target_alpha {
                flags |= 8;
            }
            if a.multiply_object_alpha {
                flags |= 0x10;
            }
            w.i8(flags);
            if write_data {
                w.f32(a.off_x);
                w.f32(a.off_y);
                w.f32(a.off_z);
                w.unit_byte(a.tint_r);
                w.unit_byte(a.tint_g);
                w.unit_byte(a.tint_b);
            }
            if a.multiply_object_alpha {
                w.f32(a.alpha);
            }
        }
    }
    if o.name.is_some() || o.sprite_name.is_some() {
        header.add(4);
        let mut flags = 0i8;
        match &o.name {
            Some(ObjectName::Grass) => flags |= 2,
            Some(ObjectName::Str(s)) if s == "Grass" => flags |= 2,
            Some(ObjectName::Dict(_)) => flags |= 4,
            Some(ObjectName::Str(_)) => flags |= 8,
            None => {}
        }
        match &o.sprite_name {
            Some(SpriteName::Dict(_)) => flags |= 0x10,
            Some(SpriteName::Str(_)) => flags |= 0x20,
            None => {}
        }
        w.i8(flags);
        match &o.name {
            Some(ObjectName::Dict(id)) => w.i8(*id),
            Some(ObjectName::Str(s)) if s != "Grass" => w.string(s),
            _ => {}
        }
        match &o.sprite_name {
            Some(SpriteName::Dict(id)) => w.i32(*id),
            Some(SpriteName::Str(s)) => w.string(s),
            None => {}
        }
    }
    if let Some(c) = &o.custom_color {
        header.add(8);
        for &v in c {
            w.unit_byte(v);
        }
    }
    if o.do_not_sync {
        header.add(16);
    }
    if o.outline_on_mouseover {
        header.add(32);
    }
    let mut bits = BitHeaderWrite::alloc(w, 2);
    if let Some(list) = &o.wall_blood_splats {
        bits.add(1);
        let count = list.len().min(32);
        w.i8(count as i8);
        for s in &list[list.len() - count..] {
            w.f32(s.world_age);
            let Some(spr) = s.sprite else {
                return err("NullPointerException: wall blood splat without sprite");
            };
            w.i32(ctx.sprite_id(spr));
        }
    }
    if with_containers && o.container_count() > 0 {
        bits.add(2);
        w.i8(o.container_count() as i8);
        for c in o.containers() {
            save_container(w, c, ctx.items)?;
        }
    }
    if let Some(t) = mod_data.filter(|t| !t.is_empty()) {
        bits.add(4);
        t.save(w);
    }
    if o.special_tooltip {
        bits.add(8);
    }
    if o.key_id != -1 {
        bits.add(16);
        w.i32(o.key_id);
    }
    if o.uses_external_water_source {
        bits.add(32);
    }
    if o.sheet_rope {
        bits.add(64);
        w.f32(o.sheet_rope_health);
    }
    if o.render_y_offset != 0.0 {
        bits.add(128);
        w.f32(o.render_y_offset);
    }
    if let Some(ov) = &o.overlay_sprite {
        bits.add(256);
        match &ov.name {
            SpriteName::Str(s) => {
                bits.add(512);
                w.string(s);
            }
            SpriteName::Dict(id) => w.i32(*id),
        }
        if let Some(c) = &ov.color {
            bits.add(1024);
            for &v in c {
                w.unit_byte(v);
            }
        }
    }
    if o.moved_thumpable {
        bits.add(2048);
    }
    if o.has_components || !o.components.is_empty() {
        bits.add(4096);
        save_entity(w, o)?;
    }
    if let Some(s) = &o.sprite_model_name {
        bits.add(8192);
        w.string(s);
    }
    if bits.flags != 0 {
        header.add(64);
        bits.write(w);
    } else if w.pos() == bits.start() + 2 {
        bits.discard(w);
    } else {
        unreachable!("empty bit header followed by data");
    }
    header.write(w);
    Ok(())
}

fn opt_sprite(r: &mut Reader<'_>, ctx: &SaveCtx<'_>) -> SaveResult<Option<SpriteIdx>> {
    Ok(if r.bool()? {
        sprite_or_null(ctx, r.i32()?)
    } else {
        None
    })
}

fn save_opt_sprite(w: &mut Writer, ctx: &SaveCtx<'_>, s: Option<SpriteIdx>) {
    match s {
        Some(s) => {
            w.i8(1);
            w.i32(ctx.sprite_id(s));
        }
        None => w.i8(0),
    }
}

fn sprite_id_npe(ctx: &SaveCtx<'_>, s: Option<SpriteIdx>, what: &str) -> SaveResult<i32> {
    match s {
        Some(s) => Ok(ctx.sprite_id(s)),
        None => err(format!("NullPointerException: {what} is null")),
    }
}

fn load_device(r: &mut Reader<'_>, d: &mut DeviceData) -> SaveResult<()> {
    d.load(r)
}

fn save_device(w: &mut Writer, d: &DeviceData) {
    d.save(w)
}

fn load_washer(r: &mut Reader<'_>) -> SaveResult<WasherLogic> {
    Ok(WasherLogic {
        activated: r.bool()?,
        last_update: r.f32()?,
    })
}

fn load_dryer(r: &mut Reader<'_>) -> SaveResult<DryerLogic> {
    Ok(DryerLogic {
        activated: r.bool()?,
    })
}

/// The `load` overrides of classes that call `IsoObject.load` first.
fn load_tail(r: &mut Reader<'_>, ctx: &SaveCtx<'_>, wv: i32, o: &mut IsoObject) -> SaveResult<()> {
    let sprite = o.sprite;
    let m = ctx.sprites;
    let prop = |name: &str| sprite.and_then(|s| m.prop(&m.sprites[s], name).map(str::to_string));
    match &mut o.kind {
        ObjectKind::Object
        | ObjectKind::Jukebox
        | ObjectKind::BrokenGlass
        | ObjectKind::ButcherHook
        | ObjectKind::Pushable(_) => {}
        ObjectKind::AnimalTrack(t) => **t = super::movers::AnimalTracks::load(r)?,
        ObjectKind::Door(d) => {
            d.open = r.bool()?;
            d.locked = r.bool()?;
            d.north = r.bool()?;
            d.health = r.i32()?;
            d.max_health = r.i32()?;
            d.closed_sprite = sprite_or_null(ctx, r.i32()?);
            d.open_sprite = sprite_or_null(ctx, r.i32()?);
            o.outline_on_mouseover = true;
            d.pushed_strength = 2500;
            d.pushed_max_strength = 2500;
            o.key_id = r.i32()?;
            d.locked_by_key = r.bool()?;
            let b = r.i8()?;
            if b & 1 != 0 {
                d.has_curtain = true;
                d.curtain_open = b & 2 != 0;
                d.curtain_inside = b & 4 != 0;
            }
        }
        ObjectKind::Window(win) => {
            win.open = r.bool()?;
            win.north = r.bool()?;
            win.health = r.i32()?;
            win.locked = r.bool()?;
            win.perma_locked = r.bool()?;
            win.destroyed = r.bool()?;
            win.glass_removed = r.bool()?;
            win.open_sprite = opt_sprite(r, ctx)?;
            win.closed_sprite = opt_sprite(r, ctx)?;
            win.smashed_sprite = opt_sprite(r, ctx)?;
            win.glass_removed_sprite = opt_sprite(r, ctx)?;
            win.max_health = r.i32()?;
        }
        ObjectKind::Curtain(c) => {
            c.open = r.bool()?;
            c.north = r.bool()?;
            c.health = r.i32()?;
            c.barricade_strength = r.i32()?;
            let other = sprite_or_null(ctx, r.i32()?);
            if c.open {
                c.closed_sprite = other;
                c.open_sprite = sprite;
            } else {
                c.open_sprite = other;
                c.closed_sprite = sprite;
            }
        }
        ObjectKind::LightSwitch(s) => {
            s.light_room = r.bool()?;
            if wv >= 206 {
                s.room_id = r.i64()?;
            } else {
                return err("IsoLightSwitch from world version < 206");
            }
            s.activated = r.bool()?;
            s.can_be_modified = r.bool()?;
            if s.can_be_modified {
                s.use_battery = r.bool()?;
                s.has_battery = r.bool()?;
                s.bulb_item = if r.bool()? { Some(r.string()?) } else { None };
                s.power = r.f32()?;
                s.delta = r.f32()?;
                s.primary_r = r.f32()?;
                s.primary_g = r.f32()?;
                s.primary_b = r.f32()?;
            }
            s.last_minute_stamp = r.i64()?;
            s.bulb_burn_minutes = r.i32()?;
            s.street_light = sprite.is_some_and(|sp| m.has_prop(&m.sprites[sp], "streetlight"));
        }
        ObjectKind::Tree(t) => {
            t.log_yield = r.i8()? as i32;
            t.damage = r.i8()? as i32 * 10;
            if let Some(v) = prop("tree") {
                let size = crate::java::parse_int(crate::java::java_trim(&v))
                    .ok_or_else(|| super::io::SaveError(format!("NumberFormatException: {v}")))?;
                t.size = size.clamp(1, 8);
            }
        }
        ObjectKind::WindowFrame { north } => *north = r.bool()?,
        ObjectKind::Barbecue(b) => {
            b.has_propane_tank = r.bool()?;
            b.fuel_amount_minutes = r.i32()?;
            b.lit = r.bool()?;
            b.last_update_time = r.f32()?;
            b.minutes_since_extinguished = r.i32()?;
            b.normal_sprite = opt_sprite(r, ctx)?;
            b.no_tank_sprite = opt_sprite(r, ctx)?;
        }
        ObjectKind::Fireplace(f) => {
            f.fuel_amount_minutes = r.i32()?;
            f.lit = r.bool()?;
            f.last_update_time = r.f32()?;
            f.minutes_since_extinguished = r.i32()?;
        }
        ObjectKind::Stove(s) => {
            s.activated = r.bool()?;
            s.seconds_timer = r.i32()?;
            s.max_temperature = r.f32()?;
            s.first_turn_on = r.bool()?;
            s.broken = r.bool()?;
        }
        ObjectKind::ClothingDryer(d) => *d = load_dryer(r)?,
        ObjectKind::ClothingWasher(wl) => *wl = load_washer(r)?,
        ObjectKind::CombinationWasherDryer(wd) => {
            wd.dryer_mode = r.i8()? != 0;
            wd.washer = load_washer(r)?;
            wd.dryer = load_dryer(r)?;
        }
        ObjectKind::StackedWasherDryer(wd) => {
            wd.washer = load_washer(r)?;
            wd.dryer = load_dryer(r)?;
        }
        ObjectKind::Radio(d) | ObjectKind::Television(d) => {
            if r.bool()? {
                load_device(r, d)?;
            }
        }
        ObjectKind::Thumpable(t) => load_thumpable(r, ctx, wv, t, &mut o.key_id)?,
        ObjectKind::Mannequin(slot) => {
            let dir = r.i8()? & 7;
            let init = r.bool()?;
            let female = r.bool()?;
            let zombie = r.bool()?;
            let skeleton = r.bool()?;
            let mannequin_script_name = r.string()?;
            let pose = r.string()?;
            let human_visual = HumanVisual::load(r, ctx.tables.blood_body_parts)?;
            let mut container = o
                .container
                .take()
                .unwrap_or_else(|| ItemContainer::new(Some("mannequin")));
            if o.container.is_none() {
                container.explored = true;
            }
            container.items.clear();
            let mut worn_items = Vec::new();
            if r.bool()? {
                let id = r.i32()?;
                let mut c = load_container(r, wv, ctx.items)?;
                c.id = id;
                container = c;
                let n = r.i8()?;
                for _ in 0..n.max(0) {
                    worn_items.push((r.string()?, r.i16()?));
                }
            }
            *slot = Some(Box::new(IsoMannequin {
                dir,
                init,
                female,
                zombie,
                skeleton,
                mannequin_script_name,
                pose,
                human_visual,
                container: Some(container),
                worn_items,
            }));
        }
        _ => unreachable!("load_tail for {}", o.kind.class_name()),
    }
    if let ObjectKind::Television(_) = o.kind {
        o.overlay_sprite = None;
    }
    Ok(())
}

fn load_thumpable(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    t: &mut IsoThumpable,
    key_id: &mut i32,
) -> SaveResult<()> {
    let bits = super::io::read_header(r, 8)?;
    if bits != 0 {
        t.open = has(bits, 1);
        t.locked = has(bits, 2);
        t.north = has(bits, 4);
        if has(bits, 8) {
            t.max_health = r.i32()?;
        }
        t.health = if has(bits, 16) {
            r.i32()?
        } else {
            t.max_health
        };
        if has(bits, 32) {
            t.closed_sprite = sprite_or_null(ctx, r.i32()?);
        }
        if has(bits, 64) {
            t.open_sprite = sprite_or_null(ctx, r.i32()?);
        }
        if has(bits, 128) {
            t.thump_dmg = r.i32()?;
        }
        t.is_door = has(bits, 512);
        t.is_door_frame = has(bits, 1024);
        t.is_corner = has(bits, 2048);
        t.is_stairs = has(bits, 4096);
        t.is_container = has(bits, 8192);
        t.is_floor = has(bits, 16384);
        t.can_barricade = has(bits, 32768);
        t.can_pass_through = has(bits, 65536);
        t.dismantable = has(bits, 131072);
        t.can_be_plastered = has(bits, 262144);
        t.paintable = has(bits, 524288);
        if has(bits, 0x100000) {
            t.cross_speed = r.f32()?;
        }
        if has(bits, 0x200000) {
            t.table = Some(LuaTable::load(r, wv)?);
        }
        if has(bits, 0x400000) {
            t.mod_data = Some(LuaTable::load(r, wv)?);
        }
        t.block_all_the_square = has(bits, 0x800000);
        t.is_thumpable = has(bits, 0x1000000);
        t.is_hoppable = has(bits, 0x2000000);
        if has(bits, 0x4000000) {
            t.light_source_life = r.i32()?;
        }
        if has(bits, 0x8000000) {
            t.light_source_radius = r.i32()?;
        }
        if has(bits, 0x10000000) {
            t.light_source_x_offset = r.i32()?;
        }
        if has(bits, 0x20000000) {
            t.light_source_y_offset = r.i32()?;
        }
        if has(bits, 0x40000000) {
            let id = r.i16()?;
            // WorldDictionary.getItemTypeFromID: null for an unknown id.
            t.light_source_fuel = ctx.items.exists(id).then_some(id);
        }
        if has(bits, 0x80000000) {
            t.life_delta = r.f32()?;
        }
        if has(bits, 0x100000000) {
            t.life_left = r.f32()?;
        }
        if has(bits, 0x200000000) {
            // The IsoObject keyId field.
            *key_id = r.i32()?;
        }
        t.locked_by_key = has(bits, 0x400000000);
        t.locked_by_padlock = has(bits, 0x800000000);
        t.can_be_lock_by_padlock = has(bits, 0x1000000000);
        if has(bits, 0x2000000000) {
            t.locked_by_code = r.i32()?;
        }
        if has(bits, 0x4000000000) {
            let s = r.string()?;
            t.thump_sound = match s.as_str() {
                "thumpa2" => "ZombieThumpGeneric".into(),
                "metalthump" => "ZombieThumpMetal".into(),
                _ => s,
            };
        }
        if has(bits, 0x8000000000) {
            t.last_update_hours = r.f32()?;
        }
        if has(bits, 0x10000000000) {
            t.have_fuel = true;
        }
        if has(bits, 0x20000000000) {
            t.light_source_on = true;
        }
        if has(bits, 0x40000000000) {
            t.build_materials = Some(LuaTable::load(r, wv)?);
        }
    }
    if wv < 248 {
        if let Some(md) = &mut t.mod_data {
            let mut migrated = Vec::new();
            for (k, v) in &md.entries {
                if let LuaKey::Str(s) = k {
                    if let Some(rest) = s.strip_prefix("need:") {
                        migrated.push((s.clone(), rest.to_string(), v.clone()));
                    }
                }
            }
            for (key, rest, v) in migrated {
                t.build_materials
                    .get_or_insert_with(LuaTable::default)
                    .rawset(LuaKey::Str(rest), Some(v));
                md.rawset(LuaKey::Str(key), None);
            }
        }
    }
    Ok(())
}

/// `IsoThumpable.save`. `key_id` is the `IsoObject.keyId` field.
fn save_thumpable(w: &mut Writer, ctx: &SaveCtx<'_>, t: &IsoThumpable, key_id: i32) {
    let mut bits = BitHeaderWrite::alloc(w, 8);
    let mut flag = |b: bool, f: u64| {
        if b {
            bits.add(f);
        }
    };
    flag(t.open, 1);
    flag(t.locked, 2);
    flag(t.north, 4);
    if t.max_health != 500 {
        bits.add(8);
        w.i32(t.max_health);
    }
    if t.health != t.max_health {
        bits.add(16);
        w.i32(t.health);
    }
    if let Some(s) = t.closed_sprite {
        bits.add(32);
        w.i32(ctx.sprite_id(s));
    }
    if let Some(s) = t.open_sprite {
        bits.add(64);
        w.i32(ctx.sprite_id(s));
    }
    if t.thump_dmg != 8 {
        bits.add(128);
        w.i32(t.thump_dmg);
    }
    for (b, f) in [
        (t.is_door, 512),
        (t.is_door_frame, 1024),
        (t.is_corner, 2048),
        (t.is_stairs, 4096),
        (t.is_container, 8192),
        (t.is_floor, 16384),
        (t.can_barricade, 32768),
        (t.can_pass_through, 65536),
        (t.dismantable, 131072),
        (t.can_be_plastered, 262144),
        (t.paintable, 524288),
    ] {
        if b {
            bits.add(f);
        }
    }
    if t.cross_speed != 1.0 {
        bits.add(0x100000);
        w.f32(t.cross_speed);
    }
    if let Some(tb) = present(&t.table) {
        bits.add(0x200000);
        tb.save(w);
    }
    if let Some(tb) = present(&t.mod_data) {
        bits.add(0x400000);
        tb.save(w);
    }
    for (b, f) in [
        (t.block_all_the_square, 0x800000),
        (t.is_thumpable, 0x1000000),
        (t.is_hoppable, 0x2000000),
    ] {
        if b {
            bits.add(f);
        }
    }
    if t.light_source_life != -1 {
        bits.add(0x4000000);
        w.i32(t.light_source_life);
    }
    if t.light_source_radius != -1 {
        bits.add(0x8000000);
        w.i32(t.light_source_radius);
    }
    if t.light_source_x_offset != 0 {
        bits.add(0x10000000);
        w.i32(t.light_source_x_offset);
    }
    if t.light_source_y_offset != 0 {
        bits.add(0x20000000);
        w.i32(t.light_source_y_offset);
    }
    if let Some(f) = t.light_source_fuel {
        bits.add(0x40000000);
        w.i16(f);
    }
    if t.life_delta != 0.0 {
        bits.add(0x80000000);
        w.f32(t.life_delta);
    }
    if t.life_left != -1.0 {
        bits.add(0x100000000);
        w.f32(t.life_left);
    }
    if key_id != -1 {
        bits.add(0x200000000);
        w.i32(key_id);
    }
    for (b, f) in [
        (t.locked_by_key, 0x400000000),
        (t.locked_by_padlock, 0x800000000),
        (t.can_be_lock_by_padlock, 0x1000000000),
    ] {
        if b {
            bits.add(f);
        }
    }
    if t.locked_by_code != 0 {
        bits.add(0x2000000000);
        w.i32(t.locked_by_code);
    }
    // Compared with "ZombieThumbGeneric", which no sound is called: every
    // thumpable writes its sound.
    if t.thump_sound != "ZombieThumbGeneric" {
        bits.add(0x4000000000);
        w.string(&t.thump_sound);
    }
    if t.last_update_hours != -1.0 {
        bits.add(0x8000000000);
        w.f32(t.last_update_hours);
    }
    if t.have_fuel {
        bits.add(0x10000000000);
    }
    if t.light_source_on {
        bits.add(0x20000000000);
    }
    if let Some(tb) = present(&t.build_materials) {
        bits.add(0x40000000000);
        tb.save(w);
    }
    bits.write(w);
}

fn opt_item(r: &mut Reader<'_>, ctx: &SaveCtx<'_>, wv: i32) -> SaveResult<Option<ItemBlob>> {
    ItemBlob::load(r, wv, ctx.items)
}

/// `IsoHutch.addAnimalInside(animal, false)` while loading: a taken place
/// (or one a nest box animal prefers) draws a new one with `Rand`.
fn add_animal_inside(
    animals: &mut Vec<(i32, IsoAnimal)>,
    nest_boxes: &[NestBox],
    mut a: IsoAnimal,
    ctx: &SaveCtx<'_>,
    sprite_name: &str,
) -> SaveResult<()> {
    let taken = |p: i32, animals: &[(i32, IsoAnimal)]| {
        animals.iter().any(|(k, _)| *k == p)
            || nest_boxes
                .iter()
                .filter_map(|n| n.animal.as_ref())
                .any(|n| n.preferred_hutch_position == p)
    };
    let draw = |a: &mut IsoAnimal| -> SaveResult<()> {
        let Some(max) = ctx.hooks.hutch_max_animals(sprite_name) else {
            return err("hutch maxAnimals unknown");
        };
        a.preferred_hutch_position = ctx.hooks.rand_next(0, max);
        Ok(())
    };
    if a.preferred_hutch_position == -1 {
        draw(&mut a)?;
    }
    let mut tries = 0;
    while taken(a.preferred_hutch_position, animals) {
        tries += 1;
        if tries > 100 {
            break;
        }
        draw(&mut a)?;
    }
    let p = a.preferred_hutch_position;
    if animals.iter().any(|(k, _)| *k == p) {
        // No place: the animal is dropped.
        return Ok(());
    }
    a.hutch_position = p;
    a.item_id = 0;
    animals.push((p, a));
    Ok(())
}

/// Classes whose state is built entirely in their `load`.
fn load_special(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    o: &mut IsoObject,
    name: &str,
) -> SaveResult<()> {
    match name {
        "Pushable" | "WheelieBin" => {
            let wheelie = name == "WheelieBin";
            let (p, sprite, container) = super::movers::IsoPushableObject::load(
                r,
                wv,
                wheelie,
                ctx.settings.tile_scale,
                ctx.items,
            )?;
            // IsoSpriteManager.getSprite(int): null for an unknown id. A
            // wheelie bin keeps the unregistered sprite CreateSprite made.
            match sprite {
                Some(id) => o.sprite = sprite_or_null(ctx, id),
                None => o.sprite_id_override = Some(UNREGISTERED_SPRITE_ID),
            }
            o.container = container;
            o.kind = ObjectKind::Pushable(Box::new(p));
            return Ok(());
        }
        "WorldInventoryItem" => {
            let xoff = r.f32()?;
            let yoff = r.f32()?;
            let zoff = r.f32()?;
            let _offset_x = r.f32()?;
            let _offset_y = r.f32()?;
            let rec = super::items::read_record(r)?;
            let exists = ctx.items.exists(i16::from_be_bytes([rec[0], rec[1]]));
            let item = if exists {
                ItemBlob::decode(rec, wv, ctx.items)
            } else {
                ItemBlob::Raw(rec.to_vec())
            };
            let drop_time = r.f64()?;
            let bits = super::io::read_header(r, 1)?;
            o.kind = ObjectKind::WorldInventory(Box::new(IsoWorldInventoryObject {
                xoff,
                yoff,
                zoff,
                offset_x: 0.0,
                offset_y: 0.0,
                item,
                drop_time,
                ignore_remove_sandbox: exists && has(bits, 1),
                extended_placement: exists && has(bits, 4),
            }));
            if !exists {
                // The item is null: the square drops the object. The entity
                // block (if any) is not read, as upstream.
                o.sprite_id_override = None;
                return Ok(());
            }
            if has(bits, 2) {
                load_entity(r, ctx, wv, o)?;
            }
            o.sprite_id_override = Some(UNREGISTERED_SPRITE_ID);
            return Ok(());
        }
        "Barricade" => {
            let dir = r.i8()? & 7;
            let n = r.i8()?;
            let mut plank_health = [0i32; 4];
            for i in 0..n.max(0) as usize {
                let h = r.i16()?;
                if let Some(p) = plank_health.get_mut(i) {
                    *p = h as i32;
                }
            }
            let b = IsoBarricade {
                dir,
                plank_health,
                metal_health: r.i16()? as i32,
                metal_bar_health: r.i16()? as i32,
            };
            // `chooseSprite()`. `getSprite(name)` adds a sprite for a name it
            // does not know; the tiles it picks are all defined. A diagonal
            // direction leaves the sprite null and the original's
            // `LoadFramesNoDirPageSimple` throws.
            let Some(n) = b.sprite_name() else {
                return err("NullPointerException: IsoBarricade.chooseSprite with no sprite");
            };
            o.sprite = ctx.sprites.named_map.get(&n).copied();
            o.kind = ObjectKind::Barricade(b);
            return Ok(());
        }
        _ => {}
    }
    load_base(r, ctx, wv, o)?;
    o.kind = match name {
        "IsoCompost" => {
            if let Some(c) = &mut o.container {
                c.kind = Some("composter".into());
            }
            let compost = r.f32()?;
            let last_updated = r.f32()?;
            let (health, max_health) = if wv >= 213 {
                (r.i32()?, r.i32()?)
            } else {
                return err("IsoCompost from world version < 213");
            };
            ObjectKind::Compost(IsoCompost {
                compost,
                last_updated,
                health,
                max_health,
            })
        }
        "FeedingTrough" => {
            let n = r.i32()?;
            let mut feeding_types: Vec<(String, f32)> = Vec::new();
            for _ in 0..n.max(0) {
                let k = r.string()?;
                let v = r.f32()?;
                match feeding_types.iter_mut().find(|(e, _)| *e == k) {
                    Some(e) => e.1 = v,
                    None => feeding_types.push((k, v)),
                }
            }
            let water = r.f32()?;
            let (mut linked_x, mut linked_y) = (0, 0);
            if r.bool()? {
                linked_x = r.i32()?;
                linked_y = r.i32()?;
                o.container = None;
            }
            let north = r.bool()?;
            // initWithDef: masters only.
            let (max_water, max_feed) = if linked_x > 0 && linked_y > 0 {
                (0.0, 0)
            } else {
                o.sprite
                    .and_then(|s| ctx.sprites.sprites[s].name.as_deref())
                    .and_then(|n| ctx.hooks.feeding_trough_def(n))
                    .unwrap_or((0.0, 0))
            };
            ObjectKind::FeedingTrough(IsoFeedingTrough {
                feeding_types,
                water,
                linked_x,
                linked_y,
                north,
                max_water,
                max_feed,
            })
        }
        "IsoHutch" => {
            let linked_x = r.i32()?;
            let linked_y = r.i32()?;
            let linked_z = r.i32()?;
            let mut master = None;
            if !(linked_x > 0 && linked_y > 0) {
                if wv < 212 {
                    return err("IsoHutch from world version < 212");
                }
                let sprite_name = r.string()?;
                // this.sprite = IsoSpriteManager.instance.getSprite(spriteName)
                if let Some(&s) = ctx.sprites.named_map.get(&sprite_name) {
                    o.sprite = Some(s);
                    o.sprite_id_override = None;
                }
                let open = r.bool()?;
                let open_egg_hatch = r.bool()?;
                let saved = (r.i32()?, r.i32()?, r.i32()?);
                let size = r.i32()?;
                if size < 0 || (size as usize) > r.remaining() {
                    return err("hutch animal block length");
                }
                let mut loaded = Vec::new();
                let nb = r.i8()?;
                for _ in 0..nb.max(0) {
                    let _serialize = r.bool()?;
                    let _class_id = r.i8()?;
                    loaded.push(IsoAnimal::load(r, wv, ctx)?);
                }
                let hutch_dirt = r.f32()?;
                let nest_box_dirt = r.f32()?;
                let n = r.i8()?;
                let max = ctx.hooks.hutch_max_nest_box(&sprite_name);
                let mut nest_boxes = Vec::new();
                for i in 0..n.max(0) as i32 {
                    let ne = r.i8()?;
                    let mut eggs = Vec::new();
                    for _ in 0..ne.max(0) {
                        // Only Food items are kept.
                        if let Some(e) = opt_item(r, ctx, wv)? {
                            eggs.push(e);
                        }
                    }
                    let animal = if r.bool()? {
                        Some(IsoAnimal::load(r, wv, ctx)?)
                    } else {
                        None
                    };
                    if max.is_none_or(|m| i <= m) {
                        nest_boxes.push(NestBox { eggs, animal });
                    }
                }
                let mut animals: Vec<(i32, IsoAnimal)> = Vec::new();
                for a in loaded {
                    add_animal_inside(&mut animals, &nest_boxes, a, ctx, &sprite_name)?;
                }
                master = Some(HutchMaster {
                    sprite_name,
                    open,
                    open_egg_hatch,
                    saved_x: saved.0,
                    saved_y: saved.1,
                    saved_z: saved.2,
                    animals,
                    hutch_dirt,
                    nest_box_dirt,
                    nest_boxes,
                });
            }
            ObjectKind::Hutch(Box::new(IsoHutch {
                linked_x,
                linked_y,
                linked_z,
                master,
            }))
        }
        "IsoGenerator" => {
            let md = o.mod_data.get_or_insert_with(LuaTable::default);
            let total_power_using = match md.rawget(&LuaKey::Str("totalPowerDraw".into())) {
                Some(LuaValue::Num(d)) => *d as f32,
                _ => 0.0,
            };
            ObjectKind::Generator(IsoGenerator {
                connected: r.bool()?,
                activated: r.bool()?,
                fuel: r.f32()?.min(10.0),
                condition: r.i32()?,
                last_hour: r.i32()?,
                total_power_using,
            })
        }
        "IsoTrap" => {
            let mut t = IsoTrap {
                sensor_range: r.i32()?,
                fire_starting_chance: r.i32()?,
                fire_starting_energy: r.i32()?,
                fire_range: r.i32()?,
                explosion_power: r.i32()?,
                explosion_range: r.i32()?,
                explosion_duration: 0,
                explosion_start_time: 0.0,
                smoke_range: 0,
                noise_range: 0,
                noise_duration: 0,
                noise_start_time: 0.0,
                extra_damage: 0.0,
                remote_control_id: 0,
                timer_before_explosion: 0,
                count_down_sound: String::new(),
                explosion_sound: String::new(),
                weapon: None,
            };
            if wv >= 219 {
                t.explosion_duration = r.i32()?;
                t.explosion_start_time = r.f32()?;
            }
            t.smoke_range = r.i32()?;
            t.noise_range = r.i32()?;
            t.noise_duration = r.i32()?;
            t.noise_start_time = r.f32()?;
            t.extra_damage = r.f32()?;
            t.remote_control_id = r.i32()?;
            t.timer_before_explosion = r.i32()?;
            t.count_down_sound = r.string()?;
            let s = r.string()?;
            t.explosion_sound = match s.as_str() {
                "bigExplosion" => "BigExplosion".into(),
                "smallExplosion" => "SmallExplosion".into(),
                "feedback" => "NoiseTrapExplosion".into(),
                _ => s,
            };
            if r.bool()? {
                t.weapon = opt_item(r, ctx, wv)?;
            }
            ObjectKind::Trap(Box::new(t))
        }
        "IsoCarBatteryCharger" => {
            let item = if r.bool()? {
                opt_item(r, ctx, wv)?
            } else {
                None
            };
            let battery = if r.bool()? {
                opt_item(r, ctx, wv)?
            } else {
                None
            };
            ObjectKind::CarBatteryCharger(Box::new(IsoCarBatteryCharger {
                item,
                battery,
                activated: r.bool()?,
                last_update: r.f32()?,
                charge_rate: r.f32()?,
            }))
        }
        "Fire" => {
            o.sprite = None;
            o.sprite_id_override = None;
            let mut life = || r.i32();
            let mut s = crate::fire::FireSave {
                life: life()?,
                spread_delay: life()?,
                life_stage: life()?,
                life_stage_timer: life()?,
                life_stage_duration: life()?,
                energy: life()?,
                num_flame_particles: life()?,
                spread_timer: life()?,
                age: life()?,
                ..Default::default()
            };
            s.perm = r.bool()?;
            s.light_radius = (r.i8()? as u8) as i32;
            s.smoke = r.bool()?;
            if wv >= 242 {
                s.saved_update_time = Some(r.i64()?);
            }
            // The animations IsoFire.load attaches draw from Rand; chunk
            // loading leaves them to the fire's owner
            // (IsoFire::attach_loaded_anims).
            let f = IsoFire::from_save(&s);
            ObjectKind::Fire(f)
        }
        _ => unreachable!(),
    };
    Ok(())
}

/// `IsoObject.save` of `o` with its class's override.
pub fn save_object(w: &mut Writer, ctx: &SaveCtx<'_>, o: &IsoObject) -> SaveResult<()> {
    match &o.kind {
        ObjectKind::WorldInventory(wi) => {
            w.i8(1);
            w.i8(ctx.class_id("WorldInventoryItem")?);
            w.f32(wi.xoff);
            w.f32(wi.yoff);
            w.f32(wi.zoff);
            w.f32(wi.offset_x);
            w.f32(wi.offset_y);
            wi.item.save(w, ctx.items)?;
            w.f64(wi.drop_time);
            let mut bits = BitHeaderWrite::alloc(w, 1);
            if wi.ignore_remove_sandbox {
                bits.add(1);
            }
            if o.has_components || !o.components.is_empty() {
                bits.add(2);
                save_entity(w, o)?;
            }
            if wi.extended_placement {
                bits.add(4);
            }
            bits.write(w);
            return Ok(());
        }
        ObjectKind::Pushable(p) => {
            let class = ctx.class_id(p.object_name())?;
            let sprite = o.sprite.map(|s| ctx.sprite_id(s));
            return p.save(w, class, sprite, o.container.as_ref(), ctx.items);
        }
        ObjectKind::Barricade(b) => {
            w.i8(1);
            w.i8(ctx.class_id("Barricade")?);
            w.i8(b.dir);
            w.i8(4);
            for &h in &b.plank_health {
                w.i16(h as i16);
            }
            w.i16(b.metal_health as i16);
            w.i16(b.metal_bar_health as i16);
            return Ok(());
        }
        _ => {}
    }
    let generator_md;
    let mut mod_data = o.mod_data.as_ref();
    let mut with_containers = true;
    let mut with_attached = true;
    match &o.kind {
        ObjectKind::Mannequin(_) => with_containers = false,
        ObjectKind::Fire(_) => with_attached = false,
        ObjectKind::Generator(_) => {
            // getModData().rawset("totalPowerDraw", (Object) float): a Float
            // the table cannot save, which still makes it non-empty.
            let mut md = o.mod_data.clone().unwrap_or_default();
            md.rawset(
                LuaKey::Str("totalPowerDraw".into()),
                Some(LuaValue::Unsaveable),
            );
            generator_md = md;
            mod_data = Some(&generator_md);
        }
        _ => {}
    }
    save_base(w, ctx, o, with_containers, with_attached, mod_data)?;
    match &o.kind {
        ObjectKind::Object
        | ObjectKind::Jukebox
        | ObjectKind::BrokenGlass
        | ObjectKind::ButcherHook
        | ObjectKind::Pushable(_) => {}
        ObjectKind::AnimalTrack(t) => t.save(w),
        ObjectKind::Door(d) => {
            w.bool(d.open);
            w.bool(d.locked);
            w.bool(d.north);
            w.i32(d.health);
            w.i32(d.max_health);
            w.i32(sprite_id_npe(ctx, d.closed_sprite, "closedSprite")?);
            w.i32(sprite_id_npe(ctx, d.open_sprite, "openSprite")?);
            w.i32(o.key_id);
            w.bool(d.locked_by_key);
            let mut b = 0i8;
            if d.has_curtain {
                b |= 1;
                if d.curtain_open {
                    b |= 2;
                }
                if d.curtain_inside {
                    b |= 4;
                }
            }
            w.i8(b);
        }
        ObjectKind::Window(win) => {
            w.bool(win.open);
            w.bool(win.north);
            w.i32(win.health);
            w.bool(win.locked);
            w.bool(win.perma_locked);
            w.bool(win.destroyed);
            w.bool(win.glass_removed);
            save_opt_sprite(w, ctx, win.open_sprite);
            save_opt_sprite(w, ctx, win.closed_sprite);
            save_opt_sprite(w, ctx, win.smashed_sprite);
            save_opt_sprite(w, ctx, win.glass_removed_sprite);
            w.i32(win.max_health);
        }
        ObjectKind::Curtain(c) => {
            w.bool(c.open);
            w.bool(c.north);
            w.i32(c.health);
            w.i32(c.barricade_strength);
            if c.open {
                w.i32(sprite_id_npe(ctx, c.closed_sprite, "closedSprite")?);
            } else {
                w.i32(sprite_id_npe(ctx, c.open_sprite, "openSprite")?);
            }
        }
        ObjectKind::LightSwitch(s) => {
            w.bool(s.light_room);
            w.i64(s.room_id);
            w.bool(s.activated);
            w.bool(s.can_be_modified);
            if s.can_be_modified {
                w.bool(s.use_battery);
                w.bool(s.has_battery);
                w.bool(s.bulb_item.is_some());
                if let Some(b) = &s.bulb_item {
                    w.string(b);
                }
                w.f32(s.power);
                w.f32(s.delta);
                w.f32(s.primary_r);
                w.f32(s.primary_g);
                w.f32(s.primary_b);
            }
            w.i64(s.last_minute_stamp);
            w.i32(s.bulb_burn_minutes);
        }
        ObjectKind::Tree(t) => {
            w.i8(t.log_yield as i8);
            w.i8((t.damage / 10) as i8);
        }
        ObjectKind::WindowFrame { north } => w.bool(*north),
        ObjectKind::Barbecue(b) => {
            w.bool(b.has_propane_tank);
            w.i32(b.fuel_amount_minutes);
            w.bool(b.lit);
            w.f32(b.last_update_time);
            w.i32(b.minutes_since_extinguished);
            save_opt_sprite(w, ctx, b.normal_sprite);
            save_opt_sprite(w, ctx, b.no_tank_sprite);
        }
        ObjectKind::Fireplace(f) => {
            w.i32(f.fuel_amount_minutes);
            w.bool(f.lit);
            w.f32(f.last_update_time);
            w.i32(f.minutes_since_extinguished);
        }
        ObjectKind::Stove(s) => {
            w.bool(s.activated);
            w.i32(s.seconds_timer);
            w.f32(s.max_temperature);
            w.bool(s.first_turn_on);
            w.bool(s.broken);
        }
        ObjectKind::ClothingDryer(d) => w.bool(d.activated),
        ObjectKind::ClothingWasher(wl) => {
            w.bool(wl.activated);
            w.f32(wl.last_update);
        }
        ObjectKind::CombinationWasherDryer(wd) => {
            w.bool(wd.dryer_mode);
            w.bool(wd.washer.activated);
            w.f32(wd.washer.last_update);
            w.bool(wd.dryer.activated);
        }
        ObjectKind::StackedWasherDryer(wd) => {
            w.bool(wd.washer.activated);
            w.f32(wd.washer.last_update);
            w.bool(wd.dryer.activated);
        }
        ObjectKind::Radio(d) | ObjectKind::Television(d) => {
            w.i8(1);
            save_device(w, d);
        }
        ObjectKind::Thumpable(t) => save_thumpable(w, ctx, t, o.key_id),
        ObjectKind::Mannequin(m) => {
            let Some(m) = m else {
                return err("IsoMannequin was never initialised (initOutfit)");
            };
            w.i8(m.dir);
            w.bool(m.init);
            w.bool(m.female);
            w.bool(m.zombie);
            w.bool(m.skeleton);
            w.string(&m.mannequin_script_name);
            w.string(&m.pose);
            m.human_visual.save(w);
            match &m.container {
                Some(c) => {
                    w.i8(1);
                    w.i32(c.id);
                    save_container(w, c, ctx.items)?;
                    w.i8(m.worn_items.len() as i8);
                    for (loc, i) in &m.worn_items {
                        w.string(loc);
                        w.i16(*i);
                    }
                }
                None => w.i8(0),
            }
        }
        ObjectKind::Compost(c) => {
            w.f32(c.compost);
            w.f32(c.last_updated);
            w.i32(c.health);
            w.i32(c.max_health);
        }
        ObjectKind::FeedingTrough(t) => {
            w.i32(t.feeding_types.len() as i32);
            let keys: Vec<&str> = t.feeding_types.iter().map(|(k, _)| k.as_str()).collect();
            for k in crate::java::hashmap_order(&keys) {
                let v = t.feeding_types.iter().find(|(e, _)| e == k).unwrap().1;
                w.string(k);
                w.f32(v);
            }
            w.f32(t.water);
            if t.linked_x > 0 && t.linked_y > 0 {
                w.i8(1);
                w.i32(t.linked_x);
                w.i32(t.linked_y);
            } else {
                w.i8(0);
            }
            w.bool(t.north);
        }
        ObjectKind::Hutch(h) => {
            w.i32(h.linked_x);
            w.i32(h.linked_y);
            w.i32(h.linked_z);
            if !(h.linked_x > 0 && h.linked_y > 0) {
                let Some(m) = &h.master else {
                    return err("IsoHutch master without state");
                };
                w.string(&m.sprite_name);
                w.bool(m.open);
                w.bool(m.open_egg_hatch);
                w.i32(m.saved_x);
                w.i32(m.saved_y);
                w.i32(m.saved_z);
                let len_at = w.pos();
                w.i32(0);
                let start = w.pos();
                let mut inside: Vec<&(i32, IsoAnimal)> = m.animals.iter().collect();
                pz_inventory::java::sort_int_hash_map(&mut inside, |e| e.0);
                w.i8(inside.len() as i8);
                for (_, a) in inside {
                    a.save(w, ctx, true)?;
                }
                let end = w.pos();
                w.patch_i32(len_at, (end - start) as i32);
                w.f32(m.hutch_dirt);
                w.f32(m.nest_box_dirt);
                w.i8(m.nest_boxes.len() as i8);
                for nb in &m.nest_boxes {
                    w.i8(nb.eggs.len() as i8);
                    for e in &nb.eggs {
                        e.save(w, ctx.items)?;
                    }
                    match &nb.animal {
                        Some(a) => {
                            w.i8(1);
                            a.save(w, ctx, false)?;
                        }
                        None => w.i8(0),
                    }
                }
            }
        }
        ObjectKind::Generator(g) => {
            w.bool(g.connected);
            w.bool(g.activated);
            w.f32(g.fuel);
            w.i32(g.condition);
            w.i32(g.last_hour);
        }
        ObjectKind::Trap(t) => {
            w.i32(t.sensor_range);
            w.i32(t.fire_starting_chance);
            w.i32(t.fire_starting_energy);
            w.i32(t.fire_range);
            w.i32(t.explosion_power);
            w.i32(t.explosion_range);
            w.i32(t.explosion_duration);
            w.f32(t.explosion_start_time);
            w.i32(t.smoke_range);
            w.i32(t.noise_range);
            w.i32(t.noise_duration);
            w.f32(t.noise_start_time);
            w.f32(t.extra_damage);
            w.i32(t.remote_control_id);
            w.i32(t.timer_before_explosion);
            w.string(&t.count_down_sound);
            w.string(&t.explosion_sound);
            match &t.weapon {
                Some(i) => {
                    w.i8(1);
                    i.save(w, ctx.items)?;
                }
                None => w.i8(0),
            }
        }
        ObjectKind::CarBatteryCharger(c) => {
            for i in [&c.item, &c.battery] {
                match i {
                    Some(i) => {
                        w.i8(1);
                        i.save(w, ctx.items)?;
                    }
                    None => w.i8(0),
                }
            }
            w.bool(c.activated);
            w.f32(c.last_update);
            w.f32(c.charge_rate);
        }
        ObjectKind::Fire(f) => {
            w.i32(f.life);
            w.i32(f.spread_delay);
            w.i32(f.life_stage - 1);
            w.i32(f.life_stage_timer);
            w.i32(f.life_stage_duration);
            w.i32(f.energy);
            w.i32(f.num_flame_particles);
            w.i32(f.spread_timer);
            w.i32(f.age);
            w.bool(f.perm);
            w.i8(f.light_radius as i8);
            w.bool(f.smoke);
            w.i64(ctx.settings.calendar_millis);
        }
        ObjectKind::WorldInventory(_) | ObjectKind::Barricade(_) => unreachable!(),
    }
    Ok(())
}
