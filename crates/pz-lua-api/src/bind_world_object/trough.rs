//! A feeding trough can span several squares. Only its master creates the
//! fluid component and owns the inventory used for feed.
use super::*;
use crate::bind_world::{square_coords, try_with_world_read};
use crate::expose::{bool_arg, expose_class, int_arg, str_arg};
use pz_entity::{ComponentType, RetiredComponents};
use pz_kahlua::NativeCall;
use pz_world::objects::{IsoFeedingTrough, ItemContainer, SpriteName};

const CLASS: &str = "zombie.iso.objects.IsoFeedingTrough";
const FLUID: ComponentType = ComponentType::FluidContainer;

fn inspect<R>(obj: &IsoObjectRef, f: impl FnOnce(&mut IsoFeedingTrough) -> R) -> LuaResult<R> {
    let mut b = obj
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoFeedingTrough"))?;
    let ObjectKind::FeedingTrough(t) = &mut b.kind else {
        return Err(LuaError::runtime("expected feeding trough"));
    };
    Ok(f(t))
}

fn init_definition(t: &Thread, obj: &IsoObjectRef) -> LuaResult<()> {
    let manager = crate::bind_sprites::manager()?;
    let sprites = manager
        .try_borrow()
        .map_err(|_| borrowed("IsoSpriteManager"))?;
    let mut obj = obj
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoFeedingTrough"))?;
    let IsoObject {
        sprite,
        kind: ObjectKind::FeedingTrough(trough),
        trough_definition,
        ..
    } = &mut *obj
    else {
        return Err(LuaError::runtime("expected feeding trough"));
    };
    if trough.linked_x > 0 && trough.linked_y > 0 {
        return Ok(());
    }
    let sprite = sprite.try_read(&sprites).map_err(host_error)?;
    if let Some(def) = sprite
        .as_deref()
        .and_then(|s| s.name.as_deref())
        .and_then(|n| crate::farm_definitions::trough(t.globals(), n))
    {
        trough.max_water = def.get_str("maxWater").as_num().unwrap_or(0.) as f32;
        trough.max_feed = def.get_str("maxFeed").as_num().unwrap_or(0.) as i32;
        *trough_definition = Some(pz_world::object_mod_data::LiveModData(def));
    }
    Ok(())
}

fn create_fluid(obj: &IsoObjectRef) -> LuaResult<()> {
    let max = inspect(obj, |t| t.max_water)?;
    let c = pz_entity::runtime_pool::obtain_component(FLUID)
        .map_err(|e| host_error(pz_inventory::component_host::pool_error(e)))?;
    let mut retired = RetiredComponents::new();
    retired.push_component(c.clone());
    let result = (|| {
        let mut component = c.try_borrow_mut().map_err(|_| borrowed("FluidContainer"))?;
        let fluid = component
            .fluid_mut()
            .ok_or_else(|| LuaError::runtime("expected FluidContainer"))?;
        let defaults = pz_entity::factory::feeding_trough_fluid_container(max);
        fluid.set_capacity(max);
        fluid.rain_catcher = defaults.rain_catcher;
        fluid.whitelist = defaults.whitelist;
        let env = crate::bind_items::item_env();
        let context = env
            .entity()
            .ok_or_else(|| unsupported("installed fluid registry"))?;
        component
            .initialize_fluid_data(context)
            .map_err(runtime_error)?;
        drop(component);
        pz_world::floor_object::runtime::add_before_world(obj, &c, &mut retired)
            .map_err(host_error)?;
        Ok(())
    })();
    pz_inventory::component_host::retire_components(retired);
    result
}

fn construct(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut trough = IsoFeedingTrough {
        feeding_types: Vec::new(),
        water: 0.,
        linked_x: 0,
        linked_y: 0,
        north: false,
        max_water: 0.,
        max_feed: 0,
    };
    let mut slot = None;
    let mut tile = None;
    let mut container = None;
    let mut grid_square = None;
    let mut register_zone = None;
    if c.nargs() == 1 {
        if !c.arg(0).raw_equals(&crate::bind_world::current_cell()) {
            return Err(LuaError::runtime("expected IsoCell"));
        }
    } else {
        let pos = square_coords(&c.arg(0))
            .ok_or_else(|| LuaError::runtime("expected feeding trough square"))?;
        let name =
            str_arg(c, 1).ok_or_else(|| LuaError::runtime("expected feeding trough sprite"))?;
        grid_square = try_with_world_read(|w| {
            Ok(w.cell
                .as_ref()
                .and_then(|c| c.get_grid_square(pos.0, pos.1, pos.2)))
        })
        .map_err(host_error)?;
        if grid_square.is_none() {
            return Err(LuaError::runtime("feeding trough square is unloaded"));
        }
        let manager = crate::bind_sprites::manager()?;
        let mut sprites = manager
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        let idx = sprites
            .named_map
            .get(name.as_ref())
            .copied()
            .unwrap_or_else(|| sprites.add_sprite_named(&name));
        let sp = &sprites.sprites[idx];
        if let Some(cap) = sprites.prop(sp, "ContainerCapacity") {
            let kind = sprites
                .prop(sp, "container")
                .ok_or_else(|| LuaError::runtime("missing trough container type"))?;
            let mut con = ItemContainer::new(Some(kind));
            con.capacity = cap
                .parse()
                .map_err(|_| LuaError::runtime("invalid trough capacity"))?;
            con.explored = true;
            container = Some(con);
        }
        trough.north = matches!(sprites.prop(sp, "Facing"), Some("N" | "S"));
        let linked = c.arg(2);
        if !linked.is_nil() {
            let (x, y, _) = square_coords(&linked)
                .ok_or_else(|| LuaError::runtime("expected linked trough square"))?;
            (trough.linked_x, trough.linked_y) = (x, y);
        } else if let Some(grid) = sp.sprite_grid.and_then(|i| sprites.grids.get(i)) {
            let (dx, dy, dz) = grid.sprite_pos(idx).unwrap_or((-1, -1, -1));
            if grid.sprite_index(dx, dy, dz) == grid.width * grid.height - 1 {
                register_zone = Some(pos)
            } else {
                trough.linked_x = pos.0.wrapping_add(grid.width - dx - 1);
                trough.linked_y = pos.1.wrapping_add(grid.height - dy - 1);
            }
        } else {
            register_zone = Some(pos)
        }
        tile = Some(name.to_string());
        slot = Some(idx);
    }
    let slave = trough.linked_x > 0 && trough.linked_y > 0;
    let mut body = IsoObject::new(0, ObjectKind::FeedingTrough(trough), slot);
    body.special_tooltip = true;
    body.container = if slave { None } else { container };
    body.sprite_name = tile.clone().map(SpriteName::Str);
    body.tile = tile;
    body.replace_square(grid_square.as_ref(), &mut RetiredComponents::new());
    let obj = pz_world::ObjectSlot::new(body).ref_handle();
    if let Some(pos) = register_zone {
        crate::bind_designation::register_farm_object(pos, &obj, false)?;
    }
    if c.nargs() != 1 {
        init_definition(c.thread, &obj)?;
    }
    if !slave {
        create_fluid(&obj)?;
    }
    Ok(c.push(object_value(&obj)?))
}

fn fluid(obj: &IsoObjectRef) -> LuaResult<Option<ComponentRef>> {
    obj.try_borrow()
        .map_err(|_| borrowed("IsoFeedingTrough"))?
        .components
        .get_ref(FLUID)
        .map_err(runtime_error)
}

fn master(obj: &IsoObjectRef) -> LuaResult<Option<IsoObjectRef>> {
    let (x, y) = inspect(obj, |t| (t.linked_x, t.linked_y))?;
    if x <= 0 || y <= 0 {
        return Ok(Some(obj.clone()));
    }
    let z = {
        let obj = obj.try_borrow().map_err(|_| borrowed("IsoFeedingTrough"))?;
        let Some(sq) = obj.square_slot() else {
            return Ok(None);
        };
        let z = sq.borrow().z;
        z
    };
    try_with_world_read(|w| {
        let Some(sq) = w.cell.as_ref().and_then(|c| c.get_grid_square(x, y, z)) else {
            return Ok(None);
        };
        let sq = sq
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("master trough square"))?;
        for slot in &sq.objects {
            if matches!(
                slot.try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("master trough"))?
                    .kind,
                ObjectKind::FeedingTrough(_)
            ) {
                return Ok(Some(slot.ref_handle()));
            }
        }
        Ok(None)
    })
    .map_err(host_error)
}

fn feed(obj: &IsoObjectRef) -> LuaResult<f32> {
    let value = object_value(obj)?;
    let (main, _) = object_containers(&value, obj)?;
    let Some(main) = main else { return Ok(0.) };
    let inv = main
        .try_borrow()
        .map_err(|_| borrowed("trough feed container"))?;
    let sm = crate::bind_registry::shared_script_manager();
    let mut amount = 0.;
    for item in &inv.items {
        let item = item
            .try_borrow()
            .map_err(|_| borrowed("trough feed item"))?;
        if matches!(item.kind, pz_inventory::item::ItemKind::Food(_)) {
            amount += item.get_hunger_change().abs();
        }
        if crate::bind_items::class_of(&item) == "zombie.inventory.types.DrainableComboItem"
            && pz_characters::character::items::script_item(&sm, &item).is_some_and(|s| {
                s.borrow()
                    .animal_feed_type
                    .as_ref()
                    .is_some_and(|s| !s.is_empty())
            })
        {
            amount += item.get_current_uses() as f32 * 0.1;
        }
    }
    Ok((amount * 100.).round() / 100.)
}

pub(super) fn register(t: &mut Thread) {
    let class = expose_class(t, CLASS, Some(ISO_OBJECT));
    class.static_fn("new", construct);
    class.method("removeFluidContainer", |c| {
        remove_fluid(&receiver_object(c)?)?;
        Ok(0)
    });
    class.method("checkWaterFromRain", |c| {
        check_water_from_rain(&receiver_object(c)?)?;
        Ok(0)
    });
    class.method("update", |c| {
        let obj = receiver_object(c)?;
        update(c.thread, &obj)?;
        Ok(0)
    });
    class.method("addWater", |c| {
        let object = receiver_object(c)?;
        let value = c.arg(1);
        let Value::Object(arg) = &value else {
            return Err(LuaError::runtime("expected FluidType"));
        };
        let ty = arg
            .as_any()
            .downcast_ref::<crate::expose::JavaEnum>()
            .filter(|e| e.class == "zombie.entity.components.fluids.FluidType")
            .and_then(|e| pz_scripts::objects::definitions::FluidType::from_name_lower(e.name))
            .ok_or_else(|| LuaError::runtime("invalid fluid type"))?;
        let amount = crate::expose::num_arg(c, 2) as f32;
        let fluid =
            fluid(&object)?.ok_or_else(|| LuaError::runtime("trough has no fluid container"))?;
        let env = crate::bind_items::item_env();
        let ctx = env
            .entity()
            .ok_or_else(|| unsupported("installed fluid registry"))?;
        // addFluid reads isFull/getAmount before changing the fluid instance.
        crate::bind_fluid_container::amount(&fluid)?;
        {
            let mut component = fluid
                .try_borrow_mut()
                .map_err(|_| borrowed("FluidContainer"))?;
            let f = component
                .fluid_mut()
                .ok_or_else(|| LuaError::runtime("expected FluidContainer"))?;
            f.add_fluid(pz_entity::FluidRef::Type(ty.id()), None, amount, ctx);
        }
        recalculate_water(c.thread, &object, &fluid)?;
        check_container(c.thread, &object)?;
        overlay(&object, false)?;
        notify_global(c.thread, &object)?;
        Ok(0)
    });
    class.method("checkContainer", |c| {
        let o = receiver_object(c)?;
        check_container(c.thread, &o)?;
        Ok(0)
    });
    class.method("checkOverlayFull", |c| {
        let o = receiver_object(c)?;
        overlay(&o, true)?;
        Ok(0)
    });
    class.method("checkOverlayAfterAnimalEat", |c| {
        let o = receiver_object(c)?;
        overlay(&o, false)?;
        Ok(0)
    });
    class.method("updateLuaObject", |c| {
        let o = receiver_object(c)?;
        notify_global(c.thread, &o)?;
        Ok(0)
    });
    class.method("setDef", |c| {
        let arg = c.arg(1);
        let def = if arg.is_nil() {
            None
        } else {
            Some(pz_world::object_mod_data::LiveModData(
                arg.as_table()
                    .ok_or_else(|| LuaError::runtime("expected trough definition"))?
                    .clone(),
            ))
        };
        receiver_object(c)?
            .try_borrow_mut()
            .map_err(|_| borrowed("feeding trough"))?
            .trough_definition = def;
        Ok(0)
    });
    class.method("initWithDef", |c| {
        init_definition(c.thread, &receiver_object(c)?)?;
        Ok(0)
    });
    class.method("getMasterTrough", |c| {
        let m = master(&receiver_object(c)?)?;
        Ok(c.push(
            m.as_ref()
                .map(object_value)
                .transpose()?
                .unwrap_or(Value::Nil),
        ))
    });
    class.method("getCurrentFeedAmount", |c| {
        let n = feed(&receiver_object(c)?)?;
        Ok(c.push(n as f64))
    });
    class.method("isEmptyFeed", |c| {
        let v = inspect(&receiver_object(c)?, |t| t.feeding_types.is_empty())?;
        Ok(c.push(v))
    });
    class.method("getAllFeedingTypes", |c| {
        let keys=inspect(&receiver_object(c)?,|t|{
            let mut keys: Vec<&str>=t.feeding_types.iter().map(|(key,_)| key.as_str()).collect();
            pz_inventory::java::sort_string_hash_map(&mut keys,|s|*s);
            keys.into_iter().map(Value::str).collect()
        })?;
        Ok(c.push(crate::bind_java_util::list_value(keys)))
    });
    class.method("getFeedAmount", |c| {
        let key = str_arg(c, 1);
        let n = inspect(&receiver_object(c)?, |t| {
            t.feeding_types
                .iter()
                .find(|(k, _)| Some(k.as_str()) == key.as_deref())
                .map_or(0., |(_, v)| *v)
        })?;
        Ok(c.push(n as f64))
    });
    for (get, set) in [
        ("getLinkedX", "setLinkedX"),
        ("getLinkedY", "setLinkedY"),
        ("getMaxFeed", "setMaxFeed"),
    ] {
        class.method(get, move |c| {
            let n = inspect(&receiver_object(c)?, |t| match get {
                "getLinkedX" => t.linked_x,
                "getLinkedY" => t.linked_y,
                _ => t.max_feed,
            })?;
            Ok(c.push(n as f64))
        });
        class.method(set, move |c| {
            let n = int_arg(c, 1);
            inspect(&receiver_object(c)?, |t| match set {
                "setLinkedX" => t.linked_x = n,
                "setLinkedY" => t.linked_y = n,
                _ => t.max_feed = n,
            })?;
            Ok(0)
        });
    }
    class.method("setNorth", |c| {
        let north = bool_arg(c, 1);
        inspect(&receiver_object(c)?, |t| t.north = north)?;
        Ok(0)
    });
    for method in ["getWater", "getMaxWater"] {
        class.method(method, move |c| {
            let fluid = fluid(&receiver_object(c)?)?;
            let value = match fluid {
                None => 0.,
                Some(f) if method == "getWater" => crate::bind_fluid_container::amount(&f)?,
                Some(f) => crate::bind_fluid_container::capacity(&f)?,
            };
            Ok(c.push(value as f64))
        });
    }
}

fn notify_global(thread: &mut Thread, obj: &IsoObjectRef) -> LuaResult<()> {
    use crate::global_objects::{system_by_name, Side};
    let Some(system) = system_by_name(Side::Server, "feedingTrough") else {
        return Ok(());
    };
    let pos = {
        let obj = obj.try_borrow().map_err(|_| borrowed("IsoFeedingTrough"))?;
        let sq = obj
            .square_slot()
            .ok_or_else(|| LuaError::runtime("trough has no square"))?
            .borrow();
        (sq.x, sq.y, sq.z)
    };
    if system.get_object_at(pos.0, pos.1, pos.2)?.is_none() {
        return Ok(());
    }
    let callback = system.mod_data.get_str("OnIsoObjectChangedItself");
    if callback.is_nil() {
        return Err(LuaError::runtime(
            "OnIsoObjectChangedItself method undefined for feedingTrough",
        ));
    }
    if let Err((error, trace)) = thread.pcall(
        &callback,
        &[Value::Table(system.mod_data.clone()), object_value(obj)?],
    ) {
        (thread.log)(&format!(
            "{}\n{trace}",
            pz_kahlua::exec::java_value_of(&error)
        ));
    }
    Ok(())
}

fn recalculate_water(thread: &mut Thread, obj: &IsoObjectRef, f: &ComponentRef) -> LuaResult<()> {
    let env = crate::bind_items::item_env();
    let ctx = env
        .entity()
        .ok_or_else(|| unsupported("installed fluid registry"))?;
    let result = f
        .try_borrow_mut()
        .map_err(|_| borrowed("FluidContainer"))?
        .fluid_recalculate_data(ctx, false)
        .map_err(runtime_error)?;
    if result.recalculated
        && result
            .owner
            .as_ref()
            .is_some_and(|owner| owner.ptr_eq(&EntityOwner::from_ref(obj)))
    {
        check_container(thread, obj)?;
        overlay(obj, false)?;
    } else {
        pz_inventory::component_host::notify_fluid_update(&result).map_err(host_error)?;
    }
    Ok(())
}

fn remove_fluid(obj: &IsoObjectRef) -> LuaResult<()> {
    use pz_world::floor_object::runtime::{connect_world, release_detached, remove_world_component};
    let mut retired = RetiredComponents::new();
    let result = (|| {
        if let Some(component) = fluid(obj)? {
            if let Some(removed) = remove_world_component(obj, &component, &mut retired).map_err(host_error)? {
                release_detached(&removed, &mut retired).map_err(host_error)?;
            }
        }
        connect_world(obj).map_err(host_error)
    })();
    pz_inventory::component_host::retire_components(retired);
    result
}

fn check_water_from_rain(obj: &IsoObjectRef) -> LuaResult<()> {
    let has_items = {
        let body = obj.try_borrow().map_err(|_| borrowed("feeding trough inventory"))?;
        if let Some(runtime) = &body.runtime_containers {
            match &runtime.main {
                Some(main) => !main.try_borrow().map_err(|_| borrowed("trough contents"))?.is_empty(),
                None => false,
            }
        } else {
            body.container.as_ref().is_some_and(|c| !c.items.is_empty())
        }
    };
    if has_items && fluid(obj)?.is_some() {
        remove_fluid(obj)?;
    }
    Ok(())
}

/// IsoFeedingTrough.update. The installed checkIsoRegion has no further work.
pub fn update(thread: &mut Thread, obj: &IsoObjectRef) -> LuaResult<()> {
    check_water_from_rain(obj)?;
    check_container(thread, obj)
}

fn check_container(thread: &mut Thread, obj: &IsoObjectRef) -> LuaResult<()> {
    if inspect(obj, |t| t.linked_x > 0 && t.linked_y > 0)? {
        return Ok(());
    }
    let wet = fluid(obj)?
        .map(|f| crate::bind_fluid_container::amount(&f))
        .transpose()?
        .unwrap_or(0.)
        != 0.;
    if obj.try_borrow().map_err(|_| borrowed("IsoFeedingTrough"))?.has_main_container() != wet {
        return Ok(());
    }
    let value = object_value(obj)?;
    object_containers(&value, obj)?;
    let new = if wet {
        None
    } else {
        let main = Rc::new(RefCell::new(pz_inventory::shared::SharedContainer::new(
            0, "trough",
        )));
        pz_inventory::shared::observe_container(&main);
        crate::bind_item_container::set_parent(&main, value);
        Some(main)
    };
    let old = {
        let mut b = obj
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoFeedingTrough"))?;
        std::mem::replace(
            &mut b
                .runtime_containers
                .as_mut()
                .expect("container adoption")
                .main,
            new,
        )
    };
    drop(old);
    let trigger = thread.globals().get_str("triggerEvent");
    thread.call1(&trigger, &[Value::str("OnContainerUpdate")])?;
    Ok(())
}

fn overlay(obj: &IsoObjectRef, full: bool) -> LuaResult<()> {
    let (def, has_container) = {
        let body = obj.try_borrow().map_err(|_| borrowed("IsoFeedingTrough"))?;
        (
            body.trough_definition.as_ref().map(|d| d.0.clone()),
            body.has_main_container(),
        )
    };
    let Some(def) = def else { return Ok(()) };
    let mut parts = vec![obj.clone()];
    for value in crate::world_menu_logic::sprite_grid::objects(&object_value(obj)?, false)? {
        if let Some(p) = object_from_value(&value) {
            if matches!(
                p.object
                    .try_borrow()
                    .map_err(|_| borrowed("feeding trough grid"))?
                    .kind,
                ObjectKind::FeedingTrough(_)
            ) {
                parts.push(p.object.clone())
            }
        }
    }
    for water in [false, true] {
        if !full && water == has_container {
            continue;
        }
        let (amount, max) = if water {
            match fluid(obj)? {
                Some(f) => (
                    crate::bind_fluid_container::amount(&f)?,
                    crate::bind_fluid_container::capacity(&f)?,
                ),
                None => (0., 0.),
            }
        } else {
            (feed(obj)?, inspect(obj, |t| t.max_feed as f32)?)
        };
        let low = if water { 10. } else { 1. };
        let field = if amount >= max / 2. {
            Some(if water {
                "spriteWaterOverlay2"
            } else {
                "spriteFoodOverlay2"
            })
        } else if amount > low {
            Some(if water {
                "spriteWaterOverlay1"
            } else {
                "spriteFoodOverlay1"
            })
        } else {
            None
        };
        let overrides = field.and_then(|k| def.get_str(k).as_table().cloned());
        for part in &parts {
            let (name, pos) = {
                let manager = crate::bind_sprites::manager()?;
                let sprites = manager
                    .try_borrow()
                    .map_err(|_| borrowed("IsoSpriteManager"))?;
                let b = part.try_borrow().map_err(|_| borrowed("trough overlay"))?;
                let sprite = b.sprite.try_read(&sprites).map_err(host_error)?;
                let name = if let Some(overrides) = &overrides {
                    sprite
                        .as_deref()
                        .and_then(|s| s.name.as_deref())
                        .and_then(|name| overrides.get_str(name).as_str().map(str::to_owned))
                } else if full {
                    b.overlay_sprite
                        .as_ref()
                        .map(|s| resolve_sprite_name(&s.name, &sprites))
                        .transpose()?
                        .flatten()
                } else {
                    None
                };
                let pos = b.square_slot().map(|sq| {
                    let s = sq.borrow();
                    (s.x, s.y, s.z)
                });
                (name, pos)
            };
            crate::bind_tile_overlays::set_overlay_sprite(part, name.as_deref())?;
            if let Some(pos) = pos {
                let manager = crate::bind_sprites::manager()?;
                let sprites = manager
                    .try_borrow()
                    .map_err(|_| borrowed("IsoSpriteManager"))?;
                try_with_world_read(|w| {
                    if let (Some(c), Some(m)) = (&w.cell, &w.meta_grid) {
                        w.geometry.notify(c, m, &sprites, pos);
                    }
                    Ok(())
                })
                .map_err(host_error)?;
            }
        }
    }
    Ok(())
}

pub fn added_to_world(thread: &mut Thread, obj: &IsoObjectRef) -> LuaResult<()> {
    if obj
        .try_borrow()
        .map_err(|_| borrowed("IsoFeedingTrough"))?
        .trough_definition
        .is_none()
    {
        init_definition(thread, obj)?;
    }
    check_container(thread, obj)?;
    if let Some(master) = master(obj)? {
        overlay(&master, false)?;
    }
    Ok(())
}
