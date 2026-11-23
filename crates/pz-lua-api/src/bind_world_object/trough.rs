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

/// The chunk codec preserves opaque entries. Promote just the trough fluid
/// at the archive boundary and bind it to its actual live receiver.
pub fn restore_fluid(obj: &IsoObjectRef) -> LuaResult<()> {
    let mut b = obj
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoFeedingTrough"))?;
    if !b.components.entry_slots().any(|entry|matches!(entry,pz_entity::RuntimeEntryView::Encoded(raw) if raw.type_id()==FLUID.id())){return Ok(())}
    let env = crate::bind_items::item_env();
    let ctx = env
        .entity()
        .ok_or_else(|| unsupported("installed fluid registry"))?;
    let mut p = b
        .components
        .runtime_mut()
        .promote_encoded(FLUID, ctx)
        .map_err(runtime_error)?;
    if p.promoted {
        let owner = EntityOwner::from_ref(obj);
        p.component.borrow_mut().replace_owner(Some(&owner));
        drop(b);
        pz_world::object_slot::observe_object(obj, CLASS);
        p.retired.push_owner(owner);
    }
    pz_inventory::component_host::retire_components(p.retired);
    Ok(())
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
