//! Linked hutch tiles and the doors driven by the installed Lua definition.
use super::*;
use crate::bind_map_objects::{apply_tile, PlacedTile};
use crate::bind_world::{square_coords, try_with_world, try_with_world_read};
use crate::expose::{expose_class, global, int_arg, num_arg, str_arg};
use pz_kahlua::{NativeCall, TableRef};
use pz_world::objects::{HutchMaster, HutchRuntime, IsoHutch, NestBox, SpriteName};
use pz_world::square_mutation::SquarePos;
use pz_world::{ObjectSlot, SquareRef};

const CLASS: &str = "zombie.iso.objects.IsoHutch";

fn square(pos: SquarePos) -> LuaResult<Option<SquareRef>> {
    try_with_world_read(|w| {
        Ok(w.cell
            .as_ref()
            .and_then(|c| c.get_grid_square(pos.0, pos.1, pos.2)))
    })
    .map_err(host_error)
}

fn parts_at(pos: SquarePos) -> LuaResult<Vec<IsoObjectRef>> {
    let Some(sq) = square(pos)? else {
        return Ok(Vec::new());
    };
    let sq = sq.try_borrow().map_err(|_| borrowed("hutch square"))?;
    let mut parts = Vec::new();
    for id in &sq.special_objects {
        if let Some(i) = sq.object_index(*id) {
            let part = &sq.objects[i];
            if matches!(
                part.try_borrow().map_err(|_| borrowed("hutch tile"))?.kind,
                ObjectKind::Hutch(_)
            ) {
                parts.push(part.ref_handle());
            }
        }
    }
    Ok(parts)
}

fn coordinate(obj: &IsoObjectRef) -> LuaResult<SquarePos> {
    let body = obj.try_borrow().map_err(|_| borrowed("IsoHutch"))?;
    let sq = body
        .square_slot()
        .ok_or_else(|| LuaError::runtime("hutch has no square"))?;
    let sq = sq.try_borrow().map_err(|_| borrowed("hutch square"))?;
    Ok((sq.x, sq.y, sq.z))
}

fn in_hutch<R>(
    obj: &IsoObjectRef,
    f: impl FnOnce(&mut IsoHutch, &mut HutchRuntime) -> R,
) -> LuaResult<R> {
    let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoHutch"))?;
    let IsoObject {
        kind: ObjectKind::Hutch(h),
        hutch_runtime,
        ..
    } = &mut *body
    else {
        return Err(LuaError::runtime("expected IsoHutch"));
    };
    Ok(f(h, hutch_runtime.get_or_insert_with(Default::default)))
}

fn definition(thread: &Thread, obj: &IsoObjectRef) -> LuaResult<TableRef> {
    if let Some(def) = in_hutch(obj, |_, r| r.definition.as_ref().map(|d| d.0.clone()))? {
        return Ok(def);
    }
    let manager = crate::bind_sprites::manager()?;
    let sprites = manager
        .try_borrow()
        .map_err(|_| borrowed("IsoSpriteManager"))?;
    let body = obj.try_borrow().map_err(|_| borrowed("IsoHutch"))?;
    let sprite = body.sprite.try_read(&sprites).map_err(host_error)?;
    let def = sprite
        .as_deref()
        .and_then(|s| s.name.as_deref())
        .and_then(|name| crate::farm_definitions::hutch(thread.globals(), name))
        .ok_or_else(|| LuaError::runtime("hutch definition not found"))?;
    drop(sprite);
    drop(body);
    in_hutch(obj, |_, r| {
        r.definition = Some(pz_world::object_mod_data::LiveModData(def.clone()))
    })?;
    Ok(def)
}

fn place_part(
    thread: &mut Thread,
    sq: &SquareRef,
    name: &str,
    def: Option<&TableRef>,
    linked: Option<SquarePos>,
) -> LuaResult<IsoObjectRef> {
    let pos = {
        let sq = sq.try_borrow().map_err(|_| borrowed("hutch square"))?;
        (sq.x, sq.y, sq.z)
    };
    let manager = crate::bind_sprites::manager()?;
    let idx = {
        let mut sprites = manager
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        sprites
            .named_map
            .get(name)
            .copied()
            .unwrap_or_else(|| sprites.add_sprite_named(name))
    };
    let (linked_x, linked_y, linked_z) = linked.unwrap_or_default();
    let master = (!(linked_x > 0 && linked_y > 0)).then(|| HutchMaster {
        sprite_name: name.to_owned(),
        open: false,
        open_egg_hatch: false,
        saved_x: 0,
        saved_y: 0,
        saved_z: 0,
        animals: Vec::new(),
        hutch_dirt: 0.,
        nest_box_dirt: 0.,
        nest_boxes: Vec::new(),
    });
    let mut body = IsoObject::new(
        0,
        ObjectKind::Hutch(Box::new(IsoHutch {
            linked_x,
            linked_y,
            linked_z,
            master,
        })),
        Some(idx),
    );
    body.tile = Some(name.into());
    body.sprite_name = Some(SpriteName::Str(name.into()));
    body.hutch_runtime = Some(Box::new(HutchRuntime {
        definition: def.cloned().map(pz_world::object_mod_data::LiveModData),
        ..Default::default()
    }));
    body.replace_square(Some(sq), &mut pz_entity::RetiredComponents::new());
    let obj = ObjectSlot::new(body).ref_handle();
    apply_tile(
        thread,
        pos,
        PlacedTile::Existing {
            object: obj.clone(),
            index: -1,
            special: true,
        },
    )?;
    Ok(obj)
}

fn construct(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    if c.nargs() == 1 && c.arg(0).raw_equals(&crate::bind_world::current_cell()) {
        let h = IsoHutch {
            linked_x: 0,
            linked_y: 0,
            linked_z: 0,
            master: Some(HutchMaster::default()),
        };
        return Ok(c.push(object_value(
            ObjectSlot::new(IsoObject::new(0, ObjectKind::Hutch(Box::new(h)), None)).owner_slot(),
        )?));
    }
    let pos = square_coords(&c.arg(0)).ok_or_else(|| LuaError::runtime("expected hutch square"))?;
    let sq = square(pos)?.ok_or_else(|| LuaError::runtime("hutch square is unloaded"))?;
    let name = str_arg(c, 2).ok_or_else(|| LuaError::runtime("expected hutch sprite name"))?;
    let def = c.arg(3);
    let def = if def.is_nil() {
        None
    } else {
        Some(
            def.as_table()
                .ok_or_else(|| LuaError::runtime("expected hutch definition"))?,
        )
    };
    let linked = c.arg(4);
    let linked = if linked.is_nil() {
        None
    } else {
        Some(square_coords(&linked).ok_or_else(|| LuaError::runtime("expected linked square"))?)
    };
    let obj = place_part(c.thread, &sq, &name, def, linked)?;
    if linked.is_none() {
        if let Some((def, extras)) = def.and_then(|def| {
            def.get_str("extraSprites")
                .as_table()
                .cloned()
                .map(|extras| (def, extras))
        }) {
            in_hutch(&obj, |h, _| {
                if let Some(m) = &mut h.master {
                    (m.saved_x, m.saved_y, m.saved_z) = pos;
                }
            })?;
            register_master(&obj, pos)?;
            crate::bind_designation::register_farm_object(pos, &obj, true)?;
            for (_, v) in extras.entries() {
                let entry = v
                    .as_table()
                    .ok_or_else(|| LuaError::runtime("expected hutch sprite entry"))?;
                let p = offset(pos, entry, false);
                if let Some(part) = square(p)? {
                    let sprite = entry.get_str("sprite");
                    place_part(
                        c.thread,
                        &part,
                        sprite
                            .as_str()
                            .ok_or_else(|| LuaError::runtime("missing hutch sprite"))?,
                        Some(def),
                        Some(pos),
                    )?;
                }
            }
            let max = field_int(def, "maxNestBox");
            let count = usize::try_from(i64::from(max) + 1).unwrap_or(0);
            let mut boxes = Vec::new();
            boxes
                .try_reserve_exact(count)
                .map_err(|_| LuaError::runtime("hutch nest boxes exceed memory"))?;
            boxes.resize_with(count, || NestBox {
                eggs: Vec::new(),
                animal: None,
            });
            in_hutch(&obj, |h, r| {
                r.max_nest_box = max;
                if let Some(m) = &mut h.master {
                    m.nest_boxes = boxes;
                }
            })?;
        }
    }
    Ok(c.push(object_value(&obj)?))
}

pub fn register_master(obj: &IsoObjectRef, pos: SquarePos) -> LuaResult<()> {
    try_with_world(|w| {
        for old in &w.hutches {
            let mut old = old
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("hutch manager"))?;
            if let ObjectKind::Hutch(h) = &old.kind {
                if h.master
                    .as_ref()
                    .is_some_and(|m| (m.saved_x, m.saved_y, m.saved_z) == pos)
                {
                    let sq = w
                        .cell
                        .as_ref()
                        .and_then(|c| c.get_grid_square(pos.0, pos.1, pos.2));
                    let mut retired = pz_entity::RetiredComponents::new();
                    old.replace_square(sq.as_ref(), &mut retired);
                    pz_inventory::component_host::retire_components(retired);
                    return Ok(());
                }
            }
        }
        w.hutches.push(obj.clone());
        Ok(())
    })
    .map_err(host_error)
}

pub fn unregister_master(obj: &IsoObjectRef) -> LuaResult<()> {
    try_with_world(|w| {
        w.hutches.retain(|h| !Rc::ptr_eq(h, obj));
        Ok(())
    })
    .map_err(host_error)
}

fn field_int(t: &TableRef, key: &str) -> i32 {
    t.get_str(key).as_num().unwrap_or(0.) as i32
}
fn offset(pos: SquarePos, t: &TableRef, egg: bool) -> SquarePos {
    (
        pos.0.wrapping_add(field_int(t, "xoffset")),
        pos.1.wrapping_add(field_int(t, "yoffset")),
        pos.2
            .wrapping_add(if egg { field_int(t, "zoffset") } else { 0 }),
    )
}

fn toggle(c: &mut NativeCall<'_>, egg: bool) -> LuaResult<usize> {
    let obj = receiver_object(c)?;
    if !egg {
        try_with_world(|w| {
            if !w.hutches.iter().any(|h| Rc::ptr_eq(h, &obj)) {
                w.hutches.push(obj.clone());
            }
            Ok(())
        })
        .map_err(host_error)?;
    }
    let open = in_hutch(&obj, |h, r| {
        let open = match (&mut h.master, egg) {
            (Some(m), true) => &mut m.open_egg_hatch,
            (Some(m), false) => &mut m.open,
            (None, true) => &mut r.slave_egg_open,
            (None, false) => &mut r.slave_open,
        };
        *open = !*open;
        *open
    })?;
    let pos = coordinate(&obj)?;
    let def = definition(c.thread, &obj)?;
    let entries = def.get_str(if egg { "eggHatchDoors" } else { "extraSprites" });
    let Some(entries) = entries.as_table() else {
        return if egg {
            Err(LuaError::runtime("missing eggHatchDoors"))
        } else {
            Ok(0)
        };
    };
    let manager = crate::bind_sprites::manager()?;
    for (_, entry) in entries.entries() {
        let e = entry
            .as_table()
            .ok_or_else(|| LuaError::runtime("expected hutch door entry"))?;
        let (closed, opened) = if egg {
            ("closedSprite", "sprite")
        } else {
            ("sprite", "spriteOpen")
        };
        let (before, after) = if open {
            (closed, opened)
        } else {
            (opened, closed)
        };
        let (before, after) = (e.get_str(before), e.get_str(after));
        let (Some(before), Some(after)) = (before.as_str(), after.as_str()) else {
            continue;
        };
        let pair = {
            let sprites = manager
                .try_borrow()
                .map_err(|_| borrowed("IsoSpriteManager"))?;
            sprites
                .named_map
                .get(before)
                .copied()
                .zip(sprites.named_map.get(after).copied())
        };
        let Some((from, to)) = pair else { continue };
        let target = offset(pos, e, egg);
        for part in parts_at(target)? {
            if part
                .try_borrow()
                .map_err(|_| borrowed("hutch door"))?
                .sprite
                .tile_projection()
                != Some(from)
            {
                continue;
            }
            let value = object_value(&part)?;
            let set = c.thread.table_get(&value, &Value::str("setSprite"))?;
            c.thread.call1(
                &set,
                &[value, crate::bind_sprites::sprite_value(Some(to), false)],
            )?;
            if egg {
                let mut body = part.try_borrow_mut().map_err(|_| borrowed("hutch door"))?;
                body.sprite_name = Some(SpriteName::Str(after.into()));
                if let ObjectKind::Hutch(h) = &mut body.kind {
                    if let Some(m) = &mut h.master {
                        m.sprite_name = after.into();
                    }
                }
            }
        }
    }
    Ok(0)
}

fn lookup(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let parts = parts_at((int_arg(c, 0), int_arg(c, 1), int_arg(c, 2)))?;
    Ok(c.push(
        parts
            .first()
            .map(object_value)
            .transpose()?
            .unwrap_or(Value::Nil),
    ))
}

pub(super) fn register(t: &mut Thread) {
    let class = expose_class(t, CLASS, Some(ISO_OBJECT));
    class.static_fn("new", construct);
    class.static_fn("getHutch", lookup);
    global(t, "getHutch", lookup);
    class.method("getHutch", |c| {
        let obj = receiver_object(c)?;
        let link = in_hutch(&obj, |h, _| {
            (h.linked_x > 0 && h.linked_y > 0).then_some((h.linked_x, h.linked_y, h.linked_z))
        })?;
        let value = if let Some(pos) = link {
            parts_at(pos)?
                .first()
                .map(object_value)
                .transpose()?
                .unwrap_or(Value::Nil)
        } else {
            c.arg(0)
        };
        Ok(c.push(value))
    });
    class.method("isSlave", |c| {
        let v = in_hutch(&receiver_object(c)?, |h, _| {
            h.linked_x > 0 && h.linked_y > 0
        })?;
        Ok(c.push(v))
    });
    class.method("toggleDoor", |c| toggle(c, false));
    class.method("toggleEggHatchDoor", |c| toggle(c, true));
    for method in ["isDoorClosed", "isOpen", "isEggHatchDoorOpen"] {
        class.method(method, move |c| {
            let (open, egg) = in_hutch(&receiver_object(c)?, |h, r| {
                h.master
                    .as_ref()
                    .map_or((r.slave_open, r.slave_egg_open), |m| {
                        (m.open, m.open_egg_hatch)
                    })
            })?;
            Ok(c.push(match method {
                "isDoorClosed" => !open,
                "isOpen" => open,
                _ => egg,
            }))
        });
    }
    for (method, field) in [
        ("getMaxAnimals", "maxAnimals"),
        ("getMaxNestBox", "maxNestBox"),
        ("getEnterSpotX", "enterSpotX"),
        ("getEnterSpotY", "enterSpotY"),
    ] {
        class.method(method, move |c| {
            let obj = receiver_object(c)?;
            let def = definition(c.thread, &obj)?;
            let n = in_hutch(&obj, |_, r| {
                let n = match field {
                    "maxAnimals" => &mut r.max_animals,
                    "maxNestBox" => &mut r.max_nest_box,
                    "enterSpotX" => &mut r.enter_spot_x,
                    _ => &mut r.enter_spot_y,
                };
                if *n == 0 {
                    *n = field_int(&def, field)
                }
                *n
            })?;
            Ok(c.push(n as f64))
        });
    }
    for (method, nest, set) in [
        ("getHutchDirt", false, false),
        ("setHutchDirt", false, true),
        ("getNestBoxDirt", true, false),
        ("setNestBoxDirt", true, true),
    ] {
        class.method(method, move |c| {
            let value = num_arg(c, 1) as f32;
            let dirt = in_hutch(&receiver_object(c)?, |h, r| {
                let dirt = match (&mut h.master, nest) {
                    (Some(m), false) => &mut m.hutch_dirt,
                    (Some(m), true) => &mut m.nest_box_dirt,
                    (None, false) => &mut r.slave_dirt,
                    (None, true) => &mut r.slave_nest_dirt,
                };
                if set {
                    *dirt = value
                }
                *dirt
            })?;
            if set {
                Ok(0)
            } else {
                Ok(c.push(dirt as f64))
            }
        });
    }
}
