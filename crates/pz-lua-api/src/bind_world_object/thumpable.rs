//! Player-built objects retain their constructor table and their own material
//! and mod-data tables independently of IsoObject's base table.
use super::*;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};
use pz_inventory::moddata::ItemModData;
use pz_kahlua::{NativeCall, TableRef};
use pz_world::objects::IsoThumpable;

const CLASS: &str = "zombie.iso.objects.IsoThumpable";

fn argument_table(value: Value) -> LuaResult<Option<ItemModData>> {
    match value {
        Value::Nil => Ok(None),
        Value::Table(t) => Ok(Some(ItemModData::Live(t))),
        other => Err(crate::bind_java_util::class_cast(
            &other,
            "se.krka.kahlua.vm.KahluaTable",
        )),
    }
}

fn with_thumpable<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut IsoThumpable) -> LuaResult<R>,
) -> LuaResult<R> {
    let object = receiver_object(c)?;
    let mut body = object
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoThumpable"))?;
    match &mut body.kind {
        ObjectKind::Thumpable(t) => f(t),
        _ => Err(crate::bind_java_util::class_cast(&c.arg(0), CLASS)),
    }
}

fn construct(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let cell = c.arg(0);
    if !cell.is_nil() && !cell.raw_equals(&crate::bind_world::current_cell()) {
        return Err(crate::bind_java_util::class_cast(
            &cell,
            "zombie.iso.IsoCell",
        ));
    }
    let mut thumpable = IsoThumpable::default();
    let mut square = None;
    if c.nargs() != 1 {
        if !(4..=6).contains(&c.nargs()) {
            return Err(LuaError::runtime("invalid IsoThumpable constructor"));
        }
        if !c.arg(1).is_nil() {
            let (x, y, z) = crate::bind_world::square_coords(&c.arg(1)).ok_or_else(|| {
                crate::bind_java_util::class_cast(&c.arg(1), "zombie.iso.IsoGridSquare")
            })?;
            square = crate::bind_world::try_with_world_read(|w| {
                Ok(w.cell
                    .as_ref()
                    .and_then(|cell| cell.get_grid_square(x, y, z)))
            })
            .map_err(host_error)?;
        }
        let manager = crate::bind_sprites::manager()?;
        let mut manager = manager
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        let mut sprite = |i| {
            let name =
                str_arg(c, i).ok_or_else(|| LuaError::runtime("expected thumpable sprite name"))?;
            Ok::<_, LuaError>(
                manager
                    .named_map
                    .get(name.as_ref())
                    .copied()
                    .unwrap_or_else(|| manager.add_sprite_named(&name)),
            )
        };
        let north_arg = if c.nargs() == 6 {
            thumpable.open_sprite = Some(sprite(3)?);
            4
        } else {
            3
        };
        thumpable.closed_sprite = Some(sprite(2)?);
        thumpable.north = bool_arg(c, north_arg);
        thumpable.table = argument_table(c.arg(north_arg + 1))?;
    }
    let sprite = thumpable.closed_sprite;
    let mut object = IsoObject::new(0, ObjectKind::Thumpable(Box::new(thumpable)), sprite);
    if c.nargs() != 1 {
        object.outline_on_mouseover = true;
    }
    object.replace_square(square.as_ref(), &mut pz_entity::RetiredComponents::new());
    let owner = pz_world::ObjectSlot::new(object);
    Ok(c.push(object_value(owner.owner_slot())?))
}

pub(super) fn register(thread: &mut Thread) {
    let class = expose_class(thread, CLASS, Some(ISO_OBJECT));
    class.static_fn("new", construct);
    class.method("getTable", |c| {
        let value = with_thumpable(c, |t| {
            Ok(t.table
                .as_mut()
                .map(ItemModData::live_table)
                .transpose()
                .map_err(|e| e.to_lua_error())?
                .map_or(Value::Nil, Value::Table))
        })?;
        Ok(c.push(value))
    });
    class.method("setTable", |c| {
        let value = argument_table(c.arg(1))?;
        drop(with_thumpable(c, |t| {
            Ok(std::mem::replace(&mut t.table, value))
        })?);
        Ok(0)
    });
    class.method("getBuildMaterials", |c| {
        let table = with_thumpable(c, |t| {
            t.build_materials
                .get_or_insert_with(|| ItemModData::Live(TableRef::new()))
                .live_table()
                .map_err(|e| e.to_lua_error())
        })?;
        Ok(c.push(Value::Table(table)))
    });
    class.method("hasBuildMaterials", |c| {
        let has = with_thumpable(c, |t| {
            Ok(t.build_materials
                .as_ref()
                .map(ItemModData::try_is_empty)
                .transpose()
                .map_err(|e| e.to_lua_error())?
                == Some(false))
        })?;
        Ok(c.push(has))
    });
    class.method("setModData", |c| {
        let data = argument_table(c.arg(1))?;
        if let Some(ItemModData::Live(table)) = &data {
            let needs: Vec<_> = table
                .entries()
                .into_iter()
                .filter_map(|(k, v)| Some((k.as_str()?.strip_prefix("need:")?.to_owned(), k, v)))
                .collect();
            if !needs.is_empty() {
                let materials = with_thumpable(c, |t| {
                    t.build_materials
                        .get_or_insert_with(|| ItemModData::Live(TableRef::new()))
                        .live_table()
                        .map_err(|e| e.to_lua_error())
                })?;
                for (name, key, value) in needs {
                    materials.set_str(&name, value);
                    table.rawset(key, Value::Nil);
                }
            }
        }
        drop(with_thumpable(c, |t| {
            Ok(std::mem::replace(&mut t.mod_data, data))
        })?);
        Ok(0)
    });
    macro_rules! field {
        ($get:literal, $set:literal, $field:ident, $read:expr) => {
            class.method($get, |c| {
                let value = with_thumpable(c, |t| Ok(t.$field))?;
                Ok(c.push(value))
            });
            class.method($set, |c| {
                let value = ($read)(c, 1);
                with_thumpable(c, |t| {
                    t.$field = value;
                    Ok(())
                })?;
                Ok(0)
            });
        };
    }
    field!(
        "isCanPassThrough",
        "setCanPassThrough",
        can_pass_through,
        bool_arg
    );
    field!(
        "getCanBarricade",
        "setCanBarricade",
        can_barricade,
        bool_arg
    );
    field!("getThumpDmg", "setThumpDmg", thump_dmg, int_arg);
    field!("isDoor", "setIsDoor", is_door, bool_arg);
    field!("isDoorFrame", "setIsDoorFrame", is_door_frame, bool_arg);
    field!(
        "isBlockAllTheSquare",
        "setBlockAllTheSquare",
        block_all_the_square,
        bool_arg
    );
    field!("isDismantable", "setIsDismantable", dismantable, bool_arg);
    field!(
        "canBePlastered",
        "setCanBePlastered",
        can_be_plastered,
        bool_arg
    );
    class.method("setIsThumpable", |c| {
        let value = bool_arg(c, 1);
        with_thumpable(c, |t| {
            t.is_thumpable = value;
            Ok(())
        })?;
        Ok(0)
    });
    field!("getMaxHealth", "setMaxHealth", max_health, int_arg);
    field!("getHealth", "setHealth", health, int_arg);
    class.method("setIsContainer", |c| {
        let enable = bool_arg(c, 1);
        with_thumpable(c, |t| {
            t.is_container = enable;
            Ok(())
        })?;
        if enable {
            let this = c.arg(0);
            let owner = receiver_object(c)?;
            let square = object_square(object_from_value(&this).unwrap())?;
            let container = pz_inventory::shared::SharedContainer::from_container(
                pz_world::objects::ItemContainer::new(Some("crate")),
            );
            crate::bind_item_container::set_parent(&container, this.clone());
            container.borrow_mut().source_grid = (!square.is_nil())
                .then(|| Rc::new(crate::bind_item_container::LuaRef(square)) as Rc<dyn Any>);
            let old = {
                let mut body = owner
                    .try_borrow_mut()
                    .map_err(|_| borrowed("IsoThumpable"))?;
                body.install_runtime_containers(|c| {
                    crate::bind_item_container::set_parent(c, this.clone())
                });
                body.runtime_containers
                    .as_mut()
                    .unwrap()
                    .main
                    .replace(container.clone())
            };
            drop(old);
            let manager = crate::bind_sprites::manager()?;
            let manager = manager
                .try_borrow()
                .map_err(|_| borrowed("IsoSpriteManager"))?;
            let body = owner.try_borrow().map_err(|_| borrowed("IsoThumpable"))?;
            let sprite = body
                .sprite
                .try_read(&manager)
                .map_err(host_error)?
                .ok_or_else(crate::world_menu_logic::null_pointer)?;
            let mut body = container
                .try_borrow_mut()
                .map_err(|_| borrowed("ItemContainer"))?;
            if let Some(capacity) = sprite.properties.get(&manager.alias, "ContainerCapacity") {
                body.capacity = capacity
                    .parse()
                    .map_err(|_| LuaError::runtime("invalid ContainerCapacity"))?;
            }
            body.explored = true;
        }
        Ok(0)
    });
    class.method("setIsHoppable", |c| {
        let value = bool_arg(c, 1);
        with_thumpable(c, |t| {
            t.is_hoppable = value;
            Ok(())
        })?;
        Ok(0)
    });
    class.method("setCrossSpeed", |c| {
        let value = num_arg(c, 1) as f32;
        with_thumpable(c, |t| {
            t.cross_speed = value;
            Ok(())
        })?;
        Ok(0)
    });
    class.method("getCrossSpeed", |c| {
        let value = with_thumpable(c, |t| Ok(t.cross_speed))?;
        Ok(c.push(f64::from(value)))
    });
}
