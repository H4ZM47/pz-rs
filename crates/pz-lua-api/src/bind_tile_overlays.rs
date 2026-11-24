//! `zombie.iso.TileOverlays` and `zombie.iso.ContainerOverlays` as the
//! shipped `media/lua/server/Items/*Overlays.lua` and `WorldFiller.lua`
//! use them: `getTileOverlays():addOverlays(overlayMap)` /
//! `getContainerOverlays():addOverlays(overlayMap)` fill the shared
//! registries ([`shared_tile_overlays`], [`shared_container_overlays`]) the
//! object code reads when it picks a square's overlay sprite.
//!
//! `hasOverlays(IsoObject)`, `updateTileOverlaySprite`,
//! `updateContainerOverlaySprite` and `fixTableTopOverlays(IsoGridSquare)`
//! take world objects, which have no Lua binding here.

use crate::bind_java_util::{class_cast, JavaList};
use crate::expose::{expose_class, global, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::tile_overlays::{
    ContainerOverlayV0Entry, ContainerOverlays, OverlayDef, OverlayEntryDef, OverlayError,
    TileOverlays, UnderlyingSpriteNames,
};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

const TILE_CLASS: &str = "zombie.iso.TileOverlays";
const CONTAINER_CLASS: &str = "zombie.iso.ContainerOverlays";
const TABLE_CLASS: &str = "se.krka.kahlua.j2se.KahluaTableImpl";

thread_local! {
    static TILES: Rc<RefCell<TileOverlays>> = Rc::default();
    static CONTAINERS: Rc<RefCell<ContainerOverlays>> = Rc::default();
    /// A retained Java list owns its backing cell through its callbacks,
    /// so this weak cache cannot confuse a recycled allocation with a
    /// still-live list. Old lists remain valid after registry Reset.
    static NAME_LISTS: RefCell<HashMap<usize, Weak<JavaList>>> = RefCell::default();
}

/// `TileOverlays.instance.updateTileOverlaySprite(obj)` for `objects[index]`
/// on square `sq`, whose sprite is named `sprite_name`: the picked overlay
/// sprite goes first in the object's `attachedAnimSprite`, tinted with the
/// usage's alpha, `copyTargetAlpha` off and `multiplyObjectAlpha` on.
pub fn update_tile_overlay_sprite(
    objects: &[pz_world::IsoObjectRef],
    index: usize,
    sprite_name: &str,
    sq: pz_world::tile_overlays::OverlaySquare<'_>,
) -> LuaResult<()> {
    let busy = |what: &str| LuaError::runtime(format!("updateTileOverlaySprite: {what} is in use"));
    let sprites = crate::bind_sprites::manager()?;
    let pick = {
        let manager = sprites.try_borrow().map_err(|_| busy("IsoSpriteManager"))?;
        // `isTableSurface()` / `isTableTopObject()`: the sprite's
        // `IsTable` / `IsTableTop` property.
        let has = |o: &pz_world::IsoObjectRef, prop: &str| -> LuaResult<bool> {
            let body = o.try_borrow().map_err(|_| busy("IsoObject"))?;
            let sprite = body
                .sprite
                .try_read(&manager)
                .map_err(|e| LuaError::runtime(format!("updateTileOverlaySprite: sprite: {e:?}")))?;
            Ok(sprite.is_some_and(|s| manager.has_prop(&s, prop)))
        };
        // `hasObjectOnTop(obj)`: a table surface with a table-top object or
        // another table surface above it on the square.
        let mut on_top: LuaResult<bool> = Ok(false);
        let picked = shared_tile_overlays().try_borrow().map_err(|_| busy("TileOverlays"))?.pick_overlay(
            Some(sprite_name),
            sq,
            false,
            || {
                on_top = (|| {
                    if !has(&objects[index], "IsTable")? {
                        return Ok(false);
                    }
                    for o in &objects[index + 1..] {
                        if has(o, "IsTableTop")? || has(o, "IsTable")? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                })();
                *on_top.as_ref().unwrap_or(&true)
            },
            |name| pz_render::get_shared_texture(name).is_some(),
        );
        on_top?;
        picked
    };
    let Some(pick) = pick else { return Ok(()) };
    // `IsoSpriteManager.instance.getSprite(name)`: the named sprite, or a
    // new one under that name; then `sprite.name = name`.
    let sprite = {
        let mut manager = sprites.try_borrow_mut().map_err(|_| busy("IsoSpriteManager"))?;
        let idx = match manager.named_map.get(&pick.sprite) {
            Some(&idx) => idx,
            None => manager.add_sprite_named(&pick.sprite),
        };
        manager.sprites[idx].name = Some(pick.sprite.clone());
        idx
    };
    let mut instance = pz_world::objects::IsoSpriteInstance::new(sprite);
    if let Some([r, g, b, a]) = pick.tint {
        instance.tint_r = r;
        instance.tint_g = g;
        instance.tint_b = b;
        instance.alpha = a;
    }
    instance.copy_target_alpha = false;
    instance.multiply_object_alpha = true;
    objects[index]
        .try_borrow_mut()
        .map_err(|_| busy("IsoObject"))?
        .attached_anim_sprite
        .get_or_insert_with(|| Vec::with_capacity(4))
        .insert(0, instance);
    Ok(())
}

/// `TileOverlays.instance.fixTableTopOverlays(square)`: for each table object
/// (`IsTable`) on `objects` that has a table-top or another table object
/// above it, remove any attached overlay sprite whose name belongs to that
/// table's `tabletop` overlay entries. Returns whether any overlay instance
/// was removed.
pub fn fix_table_top_overlays(objects: &[pz_world::IsoObjectRef]) -> LuaResult<bool> {
    let busy = |what: &str| LuaError::runtime(format!("fixTableTopOverlays: {what} is in use"));
    let sprites = crate::bind_sprites::manager()?;
    let mut removed_any = false;
    for index in 0..objects.len() {
        let (is_table, sprite_name, has_on_top) = {
            let manager = sprites.try_borrow().map_err(|_| busy("IsoSpriteManager"))?;
            let has = |o: &pz_world::IsoObjectRef, prop: &str| -> LuaResult<bool> {
                let body = o.try_borrow().map_err(|_| busy("IsoObject"))?;
                let sprite = body
                    .sprite
                    .try_read(&manager)
                    .map_err(|e| LuaError::runtime(format!("fixTableTopOverlays: sprite: {e:?}")))?;
                Ok(sprite.is_some_and(|s| manager.has_prop(&s, prop)))
            };
            if !has(&objects[index], "IsTable")? {
                (false, None, false)
            } else {
                let mut on_top = false;
                for o in &objects[index + 1..] {
                    if has(o, "IsTableTop")? || has(o, "IsTable")? {
                        on_top = true;
                        break;
                    }
                }
                let name = {
                    let body = objects[index].try_borrow().map_err(|_| busy("IsoObject"))?;
                    let res = body
                        .sprite
                        .try_read(&manager)
                        .map_err(|e| LuaError::runtime(format!("fixTableTopOverlays: sprite: {e:?}")))?
                        .and_then(|s| s.name.clone());
                    res
                };
                (true, name, on_top)
            }
        };
        if !is_table || !has_on_top {
            continue;
        }
        let Some(sprite_name) = sprite_name else {
            continue;
        };
        let to_remove: Vec<String> = shared_tile_overlays()
            .try_borrow()
            .map_err(|_| busy("TileOverlays"))?
            .table_top_overlay_names(&sprite_name)
            .into_iter()
            .map(str::to_string)
            .collect();
        if to_remove.is_empty() {
            continue;
        }
        let manager = sprites.try_borrow().map_err(|_| busy("IsoSpriteManager"))?;
        let mut body = objects[index]
            .try_borrow_mut()
            .map_err(|_| busy("IsoObject"))?;
        if let Some(attached) = body.attached_anim_sprite.as_mut() {
            let before = attached.len();
            attached.retain(|inst| {
                let name = manager
                    .sprites
                    .get(inst.sprite)
                    .and_then(|s| s.name.as_deref());
                !name.is_some_and(|n| to_remove.iter().any(|r| r == n))
            });
            if attached.len() != before {
                removed_any = true;
            }
        }
    }
    Ok(removed_any)
}

fn object_sprite_name(obj: &pz_world::IsoObjectRef) -> LuaResult<Option<String>> {
    let busy = |what: &str| LuaError::runtime(format!("overlay sprite lookup: {what} is in use"));
    let sprites = crate::bind_sprites::manager()?;
    let manager = sprites.try_borrow().map_err(|_| busy("IsoSpriteManager"))?;
    let body = obj.try_borrow().map_err(|_| busy("IsoObject"))?;
    let sprite = body
        .sprite
        .try_read(&manager)
        .map_err(|e| LuaError::runtime(format!("overlay sprite lookup: {e:?}")))?;
    Ok(sprite.and_then(|s| s.name.clone()))
}

fn square_objects_and_room(
    x: i32,
    y: i32,
    z: i32,
) -> LuaResult<Option<(Vec<pz_world::IsoObjectRef>, Option<String>)>> {
    crate::bind_world::try_with_world_read(|w| {
        let Some(cell) = w.cell.as_ref() else {
            return Ok(None);
        };
        let Some(sq_ref) = cell.get_grid_square(x, y, z) else {
            return Ok(None);
        };
        let sq = sq_ref
            .try_borrow()
            .map_err(|_| pz_entity::runtime_load::RuntimeHostError::Borrowed("overlay square"))?;
        let objects: Vec<pz_world::IsoObjectRef> =
            sq.objects.iter().map(|s| s.ref_handle()).collect();
        let room = w
            .meta_grid
            .as_ref()
            .and_then(|meta| sq.room_def(meta))
            .map(|r| r.name.clone());
        Ok(Some((objects, room)))
    })
    .map_err(|e| LuaError::runtime(format!("overlay square lookup: {e:?}")))
}

/// `ContainerOverlays.instance.updateContainerOverlaySprite(obj)`
/// (`ItemPickerJava.updateOverlaySprite`) for `object` on square `sq`: the
/// "many items" or "few items" (fewer than 7) overlay for its sprite and
/// room goes to `setOverlaySprite`; an empty or missing container clears it.
/// Stoves and feeding troughs keep theirs.
pub fn update_container_overlay_sprite(
    object: &pz_world::IsoObjectRef,
    sq: pz_world::tile_overlays::OverlaySquare<'_>,
) -> LuaResult<()> {
    let busy = |what: &str| LuaError::runtime(format!("updateContainerOverlaySprite: {what} is in use"));
    let sprites = crate::bind_sprites::manager()?;
    let (sprite_name, count) = {
        let body = object.try_borrow().map_err(|_| busy("IsoObject"))?;
        if matches!(body.kind, pz_world::ObjectKind::Stove(_) | pz_world::ObjectKind::FeedingTrough(_)) {
            return Ok(());
        }
        let manager = sprites.try_borrow().map_err(|_| busy("IsoSpriteManager"))?;
        let sprite = body
            .sprite
            .try_read(&manager)
            .map_err(|e| LuaError::runtime(format!("updateContainerOverlaySprite: sprite: {e:?}")))?;
        let count = match body.runtime_containers.as_ref().and_then(|r| r.main.as_ref()) {
            Some(c) => Some(c.try_borrow().map_err(|_| busy("ItemContainer"))?.items.len()),
            None => None,
        };
        (sprite.and_then(|s| s.name.clone()), count)
    };
    let name = shared_container_overlays()
        .try_borrow()
        .map_err(|_| busy("ContainerOverlays"))?
        .pick_overlay(sprite_name.as_deref(), count, sq, false, |n| pz_render::get_shared_texture(n).is_some());
    set_overlay_sprite(object, name.as_deref())
}

/// `IsoObject.setOverlaySprite(name)` (colour -1, the overlay colour
/// cleared): a null or blank name removes the overlay; the same name with
/// no colour is left alone; otherwise the named sprite (created under that
/// name when missing) becomes the overlay.
/// TODO(port): `invalidateRenderChunkLevel` for a change after load.
pub fn set_overlay_sprite(object: &pz_world::IsoObjectRef, name: Option<&str>) -> LuaResult<()> {
    let busy = |what: &str| LuaError::runtime(format!("setOverlaySprite: {what} is in use"));
    let name = name.filter(|n| !n.trim().is_empty());
    let mut body = object.try_borrow_mut().map_err(|_| busy("IsoObject"))?;
    let Some(name) = name else {
        body.overlay_sprite = None;
        return Ok(());
    };
    if let Some(ov) = &body.overlay_sprite {
        if ov.color.is_none() && matches!(&ov.name, pz_world::objects::SpriteName::Str(n) if n == name) {
            return Ok(());
        }
    }
    let sprites = crate::bind_sprites::manager()?;
    let mut manager = sprites.try_borrow_mut().map_err(|_| busy("IsoSpriteManager"))?;
    let idx = match manager.named_map.get(name) {
        Some(&idx) => idx,
        None => manager.add_sprite_named(name),
    };
    manager.sprites[idx].name = Some(name.to_string());
    body.overlay_sprite = Some(pz_world::objects::OverlaySprite {
        name: pz_world::objects::SpriteName::Str(name.to_string()),
        color: None,
    });
    Ok(())
}

/// `TileOverlays.instance`.
pub fn shared_tile_overlays() -> Rc<RefCell<TileOverlays>> {
    TILES.with(Rc::clone)
}

/// `ContainerOverlays.instance`.
pub fn shared_container_overlays() -> Rc<RefCell<ContainerOverlays>> {
    CONTAINERS.with(Rc::clone)
}

struct LuaTileOverlays;
struct LuaContainerOverlays;

impl LuaObject for LuaTileOverlays {
    fn class_name(&self) -> &str {
        TILE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaObject for LuaContainerOverlays {
    fn class_name(&self) -> &str {
        CONTAINER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

fn overlay_err(e: OverlayError) -> LuaError {
    let class = match e {
        OverlayError::NullPointer => "java.lang.NullPointerException",
        OverlayError::NumberFormat(_) => "java.lang.NumberFormatException",
        OverlayError::UnknownVersion(_) => "java.lang.RuntimeException",
    };
    let message = match e {
        OverlayError::NullPointer => String::new(),
        e => e.to_string(),
    };
    LuaError::Runtime { message, class }
}

/// `(KahluaTableImpl) v`; `null` stays `None`.
fn opt_table(v: Value) -> LuaResult<Option<TableRef>> {
    match v {
        Value::Nil => Ok(None),
        Value::Table(t) => Ok(Some(t)),
        other => Err(class_cast(&other, TABLE_CLASS)),
    }
}

/// `(KahluaTableImpl) v` that is then dereferenced.
fn table(v: Value) -> LuaResult<TableRef> {
    opt_table(v)?.ok_or_else(npe)
}

/// `rawgetStr(key)`: `(String) rawget(key)`.
fn rawget_str(t: &TableRef, k: &Value) -> LuaResult<Option<String>> {
    match t.rawget(k) {
        Value::Nil => Ok(None),
        Value::Str(s) => Ok(Some(s.to_string())),
        other => Err(class_cast(&other, "java.lang.String")),
    }
}

/// `rawgetInt(key)`: -1 unless the value is a number.
fn rawget_int(t: &TableRef, k: &str) -> i32 {
    t.get_str(k).as_num().map_or(-1, |d| d as i32)
}

/// One key of the `overlayMap` table of `TileOverlays.addOverlays`, read
/// as the Java loop reads it (`toString()` of keys and tiles); `None` for
/// `VERSION`.
pub fn tile_overlay_def(k: &Value, v: Value) -> LuaResult<Option<OverlayDef>> {
    {
        let tile = pz_kahlua::exec::java_value_of(k);
        if tile.eq_ignore_ascii_case("VERSION") {
            return Ok(None);
        }
        let mut d = OverlayDef {
            tile,
            entries: Vec::new(),
        };
        for (_, room) in table(v)?.entries() {
            let room = table(room)?;
            let mut e = OverlayEntryDef {
                room: rawget_str(&room, &Value::str("name"))?,
                chance: rawget_int(&room, "chance"),
                usage: rawget_str(&room, &Value::str("usage"))?,
                tiles: Vec::new(),
            };
            if e.usage.is_none() {
                return Err(npe());
            }
            for (_, t) in table(room.get_str("tiles"))?.entries() {
                e.tiles.push(pz_kahlua::exec::java_value_of(&t));
            }
            d.entries.push(e);
        }
        Ok(Some(d))
    }
}

/// `ContainerOverlays.addOverlays(overlayMap)`: `VERSION` -1 (missing) is
/// the version-0 layout, 1 the current one.
pub fn add_container_overlays(c: &mut ContainerOverlays, map: &TableRef) -> LuaResult<()> {
    match rawget_int(map, "VERSION") {
        -1 => {
            let mut defs = Vec::new();
            for (k, v) in map.entries() {
                let mut entries = Vec::new();
                for (room, tiles) in table(v)?.entries() {
                    let tiles = table(tiles)?;
                    let at = |i: i64| {
                        let v = tiles.get_int(i);
                        (!v.is_nil()).then(|| pz_kahlua::exec::java_value_of(&v))
                    };
                    entries.push(ContainerOverlayV0Entry {
                        room: pz_kahlua::exec::java_value_of(&room),
                        many_items: at(1),
                        few_items: at(2),
                    });
                }
                defs.push((pz_kahlua::exec::java_value_of(&k), entries));
            }
            c.add_overlays_v0(&defs);
        }
        1 => {
            for (k, v) in map.entries() {
                let key = pz_kahlua::exec::java_value_of(&k);
                if key.eq_ignore_ascii_case("VERSION") {
                    continue;
                }
                let mut entries = Vec::new();
                for (_, room) in table(v)?.entries() {
                    let room = table(room)?;
                    let name = rawget_str(&room, &Value::str("name"))?;
                    let tiles = table(room.get_str("tiles"))?;
                    let many = rawget_str(&tiles, &Value::Num(1.0))?;
                    let few = rawget_str(&tiles, &Value::Num(2.0))?;
                    entries.push((name, many, few));
                }
                // Each overlay is stored before the next one is read.
                c.add_overlays_v1(&[(key, entries)]);
            }
        }
        v => return Err(overlay_err(OverlayError::UnknownVersion(v))),
    }
    Ok(())
}

fn names_value(names: Option<UnderlyingSpriteNames>) -> Value {
    let Some(names) = names else {
        return Value::Nil;
    };
    let key = Rc::as_ptr(&names) as usize;
    NAME_LISTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|_, list| list.strong_count() != 0);
        if let Some(list) = cache.get(&key).and_then(Weak::upgrade) {
            return Value::Object(list);
        }
        let read = names.clone();
        let list = Rc::new(JavaList::live(
            move || read.borrow().clone(),
            Some(Box::new(move |values| {
                *names.borrow_mut() = values;
                Ok(())
            })),
        ));
        cache.insert(key, Rc::downgrade(&list));
        Value::Object(list)
    })
}

pub fn register(thread: &mut Thread) {
    let t = expose_class(thread, TILE_CLASS, Some("java.lang.Object"));
    t.field("instance", Value::Object(Rc::new(LuaTileOverlays)));
    t.method("addOverlays", |c: &mut NativeCall<'_>| {
        crate::expose::this::<LuaTileOverlays>(c)?;
        let map = table(c.arg(1))?;
        // Each overlay is stored before the next one is read, so an error
        // keeps the overlays before it.
        for (k, v) in map.entries() {
            if let Some(d) = tile_overlay_def(&k, v)? {
                TILES
                    .with(|t| t.borrow_mut().add_overlays(&[d]))
                    .map_err(overlay_err)?;
            }
        }
        Ok(0)
    });
    t.method("getUnderlyingSpriteNames", |c| {
        crate::expose::this::<LuaTileOverlays>(c)?;
        let name = str_arg(c, 1);
        let v = TILES.with(|t| {
            let t = t.borrow();
            names_value(name.as_deref().and_then(|n| t.underlying_sprite_names(n)))
        });
        Ok(c.push(v))
    });
    t.method("hasOverlays", |c| {
        crate::expose::this::<LuaTileOverlays>(c)?;
        let arg = c.arg(1);
        let Some(world) = crate::bind_world_object::object_from_value(&arg) else {
            return Ok(c.push(false));
        };
        let name = object_sprite_name(&world.object)?;
        let has = TILES.with(|t| t.borrow().has_overlays(name.as_deref()));
        Ok(c.push(has))
    });
    t.method("updateTileOverlaySprite", |c| {
        crate::expose::this::<LuaTileOverlays>(c)?;
        let arg = c.arg(1);
        let Some(world) = crate::bind_world_object::object_from_value(&arg) else {
            return Ok(0);
        };
        let sq_val = crate::bind_world_object::object_square(world)?;
        let Some((x, y, z)) = crate::bind_world::square_coords(&sq_val) else {
            return Ok(0);
        };
        let Some(sprite_name) = object_sprite_name(&world.object)? else {
            return Ok(0);
        };
        let Some((objects, room)) = square_objects_and_room(x, y, z)? else {
            return Ok(0);
        };
        let Some(index) = objects.iter().position(|o| Rc::ptr_eq(o, &world.object)) else {
            return Ok(0);
        };
        let sq = pz_world::tile_overlays::OverlaySquare {
            x,
            y,
            z,
            room: room.as_deref(),
        };
        update_tile_overlay_sprite(&objects, index, &sprite_name, sq)?;
        Ok(0)
    });
    t.method("fixTableTopOverlays", |c| {
        crate::expose::this::<LuaTileOverlays>(c)?;
        let Some((x, y, z)) = crate::bind_world::square_coords(&c.arg(1)) else {
            return Ok(0);
        };
        if let Some((objects, _)) = square_objects_and_room(x, y, z)? {
            fix_table_top_overlays(&objects)?;
        }
        Ok(0)
    });
    t.method("Reset", |c| {
        crate::expose::this::<LuaTileOverlays>(c)?;
        TILES.with(|t| t.borrow_mut().reset());
        Ok(0)
    });
    let k = expose_class(thread, CONTAINER_CLASS, Some("java.lang.Object"));
    k.field("instance", Value::Object(Rc::new(LuaContainerOverlays)));
    k.method("addOverlays", |c: &mut NativeCall<'_>| {
        crate::expose::this::<LuaContainerOverlays>(c)?;
        let map = table(c.arg(1))?;
        CONTAINERS.with(|k| add_container_overlays(&mut k.borrow_mut(), &map))?;
        Ok(0)
    });
    k.method("getUnderlyingSpriteNames", |c| {
        crate::expose::this::<LuaContainerOverlays>(c)?;
        let name = str_arg(c, 1);
        let v = CONTAINERS.with(|k| {
            let k = k.borrow();
            names_value(name.as_deref().and_then(|n| k.underlying_sprite_names(n)))
        });
        Ok(c.push(v))
    });
    k.method("hasOverlays", |c| {
        crate::expose::this::<LuaContainerOverlays>(c)?;
        let arg = c.arg(1);
        let Some(world) = crate::bind_world_object::object_from_value(&arg) else {
            return Ok(c.push(false));
        };
        let name = object_sprite_name(&world.object)?;
        let has = CONTAINERS.with(|k| k.borrow().has_overlays(name.as_deref()));
        Ok(c.push(has))
    });
    k.method("updateContainerOverlaySprite", |c| {
        crate::expose::this::<LuaContainerOverlays>(c)?;
        let arg = c.arg(1);
        let Some(world) = crate::bind_world_object::object_from_value(&arg) else {
            return Ok(0);
        };
        let sq_val = crate::bind_world_object::object_square(world)?;
        let Some((x, y, z)) = crate::bind_world::square_coords(&sq_val) else {
            return Ok(0);
        };
        let room = square_objects_and_room(x, y, z)?.and_then(|(_, r)| r);
        let sq = pz_world::tile_overlays::OverlaySquare {
            x,
            y,
            z,
            room: room.as_deref(),
        };
        update_container_overlay_sprite(&world.object, sq)?;
        Ok(0)
    });
    k.method("Reset", |c| {
        crate::expose::this::<LuaContainerOverlays>(c)?;
        CONTAINERS.with(|k| k.borrow_mut().reset());
        Ok(0)
    });
    global(thread, "getTileOverlays", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaTileOverlays))))
    });
    global(thread, "getContainerOverlays", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaContainerOverlays))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread() -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);
        t
    }

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        // assert belongs to the installed stdlib.lua, absent from this
        // invented fixture; keep every assertion active with a local check.
        let src = format!(
            "local function assert(c) if not c then error('assertion failed') end end\n{src}"
        );
        let f = pz_kahlua::stdlib::load_string(&src, Some("test"), t.globals().clone()).unwrap();
        // Runtime exceptions may deliberately have an empty message;
        // retain their class when checking the fixture's error contract.
        t.call(&f, &[]).map(|_| ()).map_err(|e| format!("{e:?}"))
    }

    #[test]
    fn tile_overlays_from_lua() {
        let mut t = thread();
        run(
            &mut t,
            r#"
            local overlayMap = {}
            overlayMap.VERSION = 1
            overlayMap["base_0"] = {{ name = "other", chance = 3, usage = "z=0", tiles = {"ov_0", "none"} }}
            overlayMap["base_1"] = {{ name = "kitchen", chance = 2, usage = "", tiles = {"ov_0"} },
                                    { name = "other", chance = 2, usage = "", tiles = {"ov_1"} }}
            getTileOverlays():addOverlays(overlayMap)
            local names = TileOverlays.instance:getUnderlyingSpriteNames("ov_0")
            assert(names:size() == 2 and names:get(0) == "base_0" and names:get(1) == "base_1")
            assert(TileOverlays.instance:getUnderlyingSpriteNames("nope") == nil)
        "#,
        )
        .unwrap();
        let o = shared_tile_overlays();
        let o = o.borrow();
        assert_eq!(o.overlay_map["base_0"].entries[0].tiles, vec!["ov_0", ""]);
        assert_eq!(o.overlay_map["base_0"].entries[0].usage.z_only, 0);
        assert_eq!(o.overlay_map["base_1"].entries[1].room, "other");
        drop(o);
        let e = run(
            &mut t,
            r#"getTileOverlays():addOverlays({ x = {{ name = "other", tiles = {} }} })"#,
        )
        .unwrap_err();
        assert!(e.contains("NullPointerException"), "{e}");
    }

    #[test]
    fn container_overlays_from_lua() {
        let mut t = thread();
        run(
            &mut t,
            r#"
            local overlayMap = {}
            overlayMap.VERSION = 1
            overlayMap["fridge_0"] = {{ name = "other", tiles = {"food_0", "food_1"} }, { name = "other", tiles = {"none", ""} }}
            getContainerOverlays():addOverlays(overlayMap)
        "#,
        )
        .unwrap();
        let c = shared_container_overlays();
        let c2 = c.borrow();
        let o = &c2.overlay_map["fridge_0"];
        assert_eq!(o.entries[0].few_items.as_deref(), Some("food_1"));
        assert_eq!(o.entries[1].many_items, None);
        drop(c2);
        let e = run(
            &mut t,
            r#"getContainerOverlays():addOverlays({ VERSION = 2 })"#,
        )
        .unwrap_err();
        assert!(e.contains("unknown overlayMap.VERSION 2"), "{e}");
        // Version 0: rooms are keys.
        run(
            &mut t,
            r#"getContainerOverlays():addOverlays({ crate_0 = { other = {"a", "b"} } })"#,
        )
        .unwrap();
        assert_eq!(
            c.borrow().overlay_map["crate_0"].entries[0]
                .few_items
                .as_deref(),
            Some("b")
        );
    }

    #[test]
    fn underlying_lists_keep_identity_mutations_and_retired_allocations() {
        for container in [false, true] {
            let mut t = thread();
            let target = if container {
                Value::Object(Rc::new(LuaContainerOverlays))
            } else {
                Value::Object(Rc::new(LuaTileOverlays))
            };
            t.globals().set_str("target", target);
            run(
                &mut t,
                r#"
                target:Reset()
                target:addOverlays({ VERSION = 1, base_a = {
                    { name = "other", chance = 1, usage = "", tiles = { "overlay_a" } }
                } })
                retained = target:getUnderlyingSpriteNames("overlay_a")
                assert(retained == target:getUnderlyingSpriteNames("overlay_a"))
                retained:add("from_mod")
                assert(target:getUnderlyingSpriteNames("overlay_a"):get(1) == "from_mod")
            "#,
            )
            .unwrap();
            let rust_names = if container {
                shared_container_overlays()
                    .borrow()
                    .get_underlying_sprite_names("overlay_a")
            } else {
                shared_tile_overlays()
                    .borrow()
                    .get_underlying_sprite_names("overlay_a")
            };
            assert_eq!(rust_names.unwrap(), ["base_a", "from_mod"]);

            // A registration made by the Rust owner must reach a Lua
            // reference obtained before that registration.
            if container {
                shared_container_overlays().borrow_mut().add_overlays_v1(&[(
                    "base_b".into(),
                    vec![(Some("other".into()), Some("overlay_a".into()), None)],
                )]);
            } else {
                shared_tile_overlays()
                    .borrow_mut()
                    .add_overlays(&[OverlayDef {
                        tile: "base_b".into(),
                        entries: vec![OverlayEntryDef {
                            room: Some("other".into()),
                            chance: 1,
                            usage: Some(String::new()),
                            tiles: vec!["overlay_a".into()],
                        }],
                    }])
                    .unwrap();
            }
            run(
                &mut t,
                r#"
                assert(retained:size() == 3 and retained:get(2) == "base_b")
                retained:clear()
                retained:add(42)
                retained:add(nil)
                assert(retained:size() == 2)
                assert(retained:get(0) == 42 and retained:get(1) == nil)
            "#,
            )
            .unwrap();
            let backing = if container {
                shared_container_overlays()
                    .borrow()
                    .underlying_sprite_names("overlay_a")
            } else {
                shared_tile_overlays()
                    .borrow()
                    .underlying_sprite_names("overlay_a")
            }
            .unwrap();
            assert_eq!(backing.borrow().len(), 2);
            assert_eq!(backing.borrow()[0].as_num(), Some(42.0));
            assert!(backing.borrow()[1].is_nil());

            // Reset clears the map, not an ArrayList still retained by
            // Lua. Re-registering the same name creates a new list.
            run(
                &mut t,
                r#"
                target:Reset()
                assert(target:getUnderlyingSpriteNames("overlay_a") == nil)
                assert(retained:size() == 2 and retained:get(0) == 42)
                target:addOverlays({ VERSION = 1, base_new = {
                    { name = "other", chance = 1, usage = "", tiles = { "overlay_a" } }
                } })
                replacement = target:getUnderlyingSpriteNames("overlay_a")
                assert(replacement ~= retained)
                assert(replacement == target:getUnderlyingSpriteNames("overlay_a"))
                retained:add("retired_only")
                assert(retained:size() == 3)
                assert(replacement:size() == 1 and replacement:get(0) == "base_new")
            "#,
            )
            .unwrap();
            let current = if container {
                shared_container_overlays()
                    .borrow()
                    .underlying_sprite_names("overlay_a")
            } else {
                shared_tile_overlays()
                    .borrow()
                    .underlying_sprite_names("overlay_a")
            }
            .unwrap();
            assert!(!Rc::ptr_eq(&backing, &current));
            assert_eq!(backing.borrow().len(), 3);
            assert_eq!(current.borrow().len(), 1);
        }
    }

    #[test]
    fn fix_table_top_overlays_strips_tabletop_overlays_when_object_placed_above() {
        let mut sm = pz_world::IsoSpriteManager::new(pz_world::sprite::test_tables());
        let mut values = pz_world::sprite::PropertyValueMap::default();
        values.put("IsTable", vec![String::new()]);
        values.put("IsTableTop", vec![String::new()]);
        sm.alias = pz_world::sprite::TilePropertyAliasMap::generate(&values, &[]);
        let table_idx = sm.add_sprite_named("table_0");
        sm.sprites[table_idx].name = Some("table_0".into());
        sm.sprites[table_idx]
            .properties
            .set(&sm.tables, &sm.alias, "IsTable", "", false);
        let top_idx = sm.add_sprite_named("lamp_0");
        sm.sprites[top_idx].name = Some("lamp_0".into());
        sm.sprites[top_idx]
            .properties
            .set(&sm.tables, &sm.alias, "IsTableTop", "", false);
        let ov_tt_idx = sm.add_sprite_named("ov_tabletop");
        sm.sprites[ov_tt_idx].name = Some("ov_tabletop".into());
        let ov_keep_idx = sm.add_sprite_named("ov_keep");
        sm.sprites[ov_keep_idx].name = Some("ov_keep".into());
        crate::bind_sprites::install_sprite_manager(Some(Rc::new(RefCell::new(sm))));

        shared_tile_overlays().borrow_mut().reset();
        shared_tile_overlays()
            .borrow_mut()
            .add_overlays(&[OverlayDef {
                tile: "table_0".into(),
                entries: vec![
                    OverlayEntryDef {
                        room: Some("other".into()),
                        chance: 1,
                        usage: Some("tabletop".into()),
                        tiles: vec!["ov_tabletop".into()],
                    },
                    OverlayEntryDef {
                        room: Some("other".into()),
                        chance: 1,
                        usage: Some(String::new()),
                        tiles: vec!["ov_keep".into()],
                    },
                ],
            }])
            .unwrap();

        let mut table_obj = pz_world::IsoObject::new(
            1,
            pz_world::objects::ObjectKind::Object,
            Some(table_idx),
        );
        table_obj.attached_anim_sprite = Some(vec![
            pz_world::objects::IsoSpriteInstance::new(ov_tt_idx),
            pz_world::objects::IsoSpriteInstance::new(ov_keep_idx),
        ]);
        let table_ref = pz_world::ObjectSlot::new(table_obj).ref_handle();
        let top_ref = pz_world::ObjectSlot::new(pz_world::IsoObject::new(
            2,
            pz_world::objects::ObjectKind::Object,
            Some(top_idx),
        ))
        .ref_handle();

        // Without an object on top, tabletop overlays stay.
        assert!(!fix_table_top_overlays(&[table_ref.clone()]).unwrap());
        assert_eq!(table_ref.borrow().attached_anim_sprite.as_ref().unwrap().len(), 2);

        // With an IsTableTop object above it, only the tabletop overlay is removed.
        assert!(fix_table_top_overlays(&[table_ref.clone(), top_ref]).unwrap());
        let remaining = table_ref.borrow().attached_anim_sprite.clone().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].sprite, ov_keep_idx);

        shared_tile_overlays().borrow_mut().reset();
        crate::bind_sprites::install_sprite_manager(None);
    }
}
