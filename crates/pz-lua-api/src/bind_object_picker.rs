//! `zombie.iso.IsoObjectPicker` as Lua sees it: the `IsoObjectPicker.Instance`
//! whose `PickDoor`, `PickWindow`, `PickWindowFrame`, `PickThumpable`,
//! `PickHoppable`, `PickTree` and `PickCorpse` the world context menu
//! (`ISObjectClickHandler.doRClick`) asks for the objects under the click.
//!
//! The picks themselves need the world and the chunk renderer's records, so
//! the engine installs an [`ObjectPicker`] ([`set_object_picker`]); without
//! one every pick answers `nil`, as with nothing under the mouse.
//!
//! `PickVehicle`, which `ISVehicleMenu.OnFillWorldObjectContextMenu` asks
//! for, needs no renderer records: it turns the screen point into a world
//! point and tests the cell's vehicles.

use crate::expose::{bool_arg, expose_class, int_arg, this};
use pz_entity::runtime_load::RuntimeHostError;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const CLASS: &str = "zombie.iso.IsoObjectPicker";

/// One of `IsoObjectPicker`'s `Pick*` methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// `PickDoor(x, y, bTransparent)`.
    Door { transparent: bool },
    Window,
    WindowFrame,
    Thumpable,
    Hoppable,
    Tree,
    Corpse,
}

/// The world's end of [`Pick`]: the `IsoObject`'s Lua object under the
/// screen point (`x`, `y`), if any.
pub trait ObjectPicker {
    fn pick(&self, what: Pick, x: i32, y: i32) -> LuaResult<Option<Value>>;
}

thread_local! {
    static PICKER: RefCell<Option<Rc<dyn ObjectPicker>>> = const { RefCell::new(None) };
}

/// Installs (or removes) the world's picker.
pub fn set_object_picker(picker: Option<Rc<dyn ObjectPicker>>) {
    PICKER.with(|p| *p.borrow_mut() = picker);
}

/// The Lua object of the object at `index` in the objects of the square
/// `square` (x, y, z); `None` when the square or the index is gone.
pub fn world_object_value(square: [i32; 3], index: i32) -> LuaResult<Option<Value>> {
    let [x, y, z] = square;
    let object = crate::bind_world::try_with_world_read(|w| {
        let Some(cell) = w.cell.as_ref() else {
            return Ok(None);
        };
        let Some(square) = cell.get_grid_square(x, y, z) else {
            return Ok(None);
        };
        let square = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("picked square"))?;
        Ok(usize::try_from(index)
            .ok()
            .and_then(|i| square.objects.get(i))
            .map(|slot| slot.ref_handle()))
    })
    .map_err(crate::bind_world_object::host_error)?;
    let Some(object) = object else {
        return Ok(None);
    };
    let value = crate::bind_world_object::object_value(&object)?;
    crate::bind_world_object::remember_square(&value, square);
    Ok(Some(value))
}

/// `IsoObject.getObjectIndex()` for the Lua object `value` that was
/// picked on `square`: its index in that square's objects, -1 when the
/// square is gone or no longer holds it (or `value` is no world object).
///
/// (Objects loaded with their chunk do not point back at their square, so
/// the picked square stands in for `IsoObject.square`.)
pub fn world_object_index(value: &Value, square: [i32; 3]) -> i32 {
    let Some(object) = crate::bind_world_object::object_from_value(value) else {
        return -1;
    };
    let object = &object.object;
    let [x, y, z] = square;
    crate::bind_world::try_with_world_read(|w| {
        let Some(square) = w.cell.as_ref().and_then(|c| c.get_grid_square(x, y, z)) else {
            return Ok(-1);
        };
        let square = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("picked square"))?;
        Ok(square
            .objects
            .iter()
            .position(|slot| Rc::ptr_eq(slot.owner_slot(), object))
            .map_or(-1, |i| i as i32))
    })
    .unwrap_or(-1)
}

struct LuaObjectPicker;

impl LuaObject for LuaObjectPicker {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `PickVehicle(x, y)`: the screen point on the floor of
/// `PZMath.fastfloor(IsoPlayer.players[0].getZ())`, through
/// `IsoUtils.XToIso/YToIso` (the camera of `IsoPlayer.getPlayerIndex()`),
/// then the first of `IsoCell.getVehicles()` that `isInBounds` there.
fn pick_vehicle(x: i32, y: i32) -> LuaResult<Value> {
    let player = crate::bind_players::player(0).ok_or_else(|| LuaError::Runtime {
        message: "Cannot invoke \"zombie.characters.IsoPlayer.getZ()\" because \"zombie.characters.IsoPlayer.players[0]\" is null"
            .into(),
        class: "java.lang.NullPointerException",
    })?;
    let floor = pz_world::java::fastfloor(player.borrow().vars.z) as f32;
    let index = usize::try_from(crate::bind_players::player_index()).ok().filter(|&i| i < 4).ok_or_else(|| {
        LuaError::Runtime {
            message: format!("Index {} out of bounds for length 4", crate::bind_players::player_index()),
            class: "java.lang.ArrayIndexOutOfBoundsException",
        }
    })?;
    let (ox, oy) = crate::hud::host::with_host(|h| h.camera.camera_offset(index));
    let (px, py) = (x as f32 + ox, y as f32 + oy);
    let ts = crate::bind_core::shared_core().borrow().tile_scale as f32;
    let wx = (px + 2.0 * py) / (64.0 * ts) + 3.0 * floor;
    let wy = (px - 2.0 * py) / (-64.0 * ts) + 3.0 * floor;
    Ok(pz_vehicles::manager::cell_vehicles()
        .iter()
        .find(|v| v.is_in_bounds(wx, wy))
        .map_or(Value::Nil, pz_vehicles::lua::vehicle_value))
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.field("Instance", Value::Object(Rc::new(LuaObjectPicker)));
    b.method("PickVehicle", |c| {
        this::<LuaObjectPicker>(c)?;
        let v = pick_vehicle(int_arg(c, 1), int_arg(c, 2))?;
        Ok(c.push(v))
    });
    let picks: [(&str, fn(&pz_kahlua::NativeCall<'_>) -> Pick); 7] = [
        ("PickDoor", |c| Pick::Door {
            transparent: bool_arg(c, 3),
        }),
        ("PickWindow", |_| Pick::Window),
        ("PickWindowFrame", |_| Pick::WindowFrame),
        ("PickThumpable", |_| Pick::Thumpable),
        ("PickHoppable", |_| Pick::Hoppable),
        ("PickTree", |_| Pick::Tree),
        ("PickCorpse", |_| Pick::Corpse),
    ];
    for (name, which) in picks {
        b.method(name, move |c| {
            this::<LuaObjectPicker>(c)?;
            let what = which(c);
            let picker = PICKER.with(|p| p.borrow().clone());
            let found = match picker {
                Some(p) => p.pick(what, int_arg(c, 1), int_arg(c, 2))?,
                None => None,
            };
            Ok(c.push(found.unwrap_or(Value::Nil)))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{TableRef, Thread};

    struct Recorder(RefCell<Vec<(Pick, i32, i32)>>);

    impl ObjectPicker for Recorder {
        fn pick(&self, what: Pick, x: i32, y: i32) -> LuaResult<Option<Value>> {
            self.0.borrow_mut().push((what, x, y));
            Ok((what == Pick::Tree).then(|| Value::str("a tree")))
        }
    }

    fn run(t: &mut Thread, src: &str) -> Value {
        let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?} {tr}")).unwrap().into_iter().next().unwrap_or(Value::Nil)
    }

    #[test]
    fn instance_methods_reach_the_picker_with_their_arguments() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        // Nothing installed: nothing is under the mouse.
        assert!(run(&mut t, "return IsoObjectPicker.Instance:PickWindow(1, 2)").is_nil());
        let rec = Rc::new(Recorder(RefCell::default()));
        set_object_picker(Some(rec.clone()));
        let tree = run(
            &mut t,
            "local p = IsoObjectPicker.Instance
             p:PickDoor(10, 20, true)
             p:PickDoor(11, 21, false)
             p:PickWindow(12, 22)
             p:PickWindowFrame(13, 23)
             p:PickThumpable(14, 24)
             p:PickHoppable(15, 25)
             p:PickCorpse(16, 26)
             return p:PickTree(17, 27)",
        );
        assert_eq!(tree.as_str(), Some("a tree"));
        assert_eq!(
            *rec.0.borrow(),
            [
                (Pick::Door { transparent: true }, 10, 20),
                (Pick::Door { transparent: false }, 11, 21),
                (Pick::Window, 12, 22),
                (Pick::WindowFrame, 13, 23),
                (Pick::Thumpable, 14, 24),
                (Pick::Hoppable, 15, 25),
                (Pick::Corpse, 16, 26),
                (Pick::Tree, 17, 27),
            ]
        );
        set_object_picker(None);
    }

    #[test]
    fn pick_vehicle_finds_the_cell_vehicle_under_the_screen_point() {
        struct Restore(Option<crate::bind_character_sim::LuaGameCharacterRef>);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::bind_players::set_player(0, self.0.take());
                pz_vehicles::manager::reset();
            }
        }
        let _restore = Restore(crate::bind_players::player(0));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        // No players[0]: getZ() on null.
        crate::bind_players::set_player(0, None);
        let err = run(&mut t, "local ok, e = pcall(IsoObjectPicker.Instance.PickVehicle, IsoObjectPicker.Instance, 0, 0) return tostring(e)");
        assert!(pz_kahlua::exec::java_value_of(&err).contains("players[0]\" is null"), "{err:?}");
        let player = Rc::new(RefCell::new(crate::bind_character_sim::LuaGameCharacter::new(
            &pz_scripts::ScriptManager::new(),
            pz_characters::CharacterKind::Player,
        )));
        player.borrow_mut().vars.z = 0.5;
        crate::bind_players::set_player(0, Some(player));
        // A vehicle whose polygon covers the world point (0, 0): the screen
        // point (0, 0) with no camera offset on floor 0.
        let v = pz_vehicles::BaseVehicle::new();
        {
            let mut st = v.st_mut();
            let p = &mut st.poly;
            (p.x1, p.y1, p.x2, p.y2, p.x3, p.y3, p.x4, p.y4) = (-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
            for (i, corner) in p.corners().into_iter().enumerate() {
                (p.borders[i].x, p.borders[i].y) = corner;
            }
            st.poly_dirty = false;
        }
        pz_vehicles::manager::cell_add(&v);
        pz_vehicles::manager::process_add_vehicles();
        let hit = run(&mut t, "return IsoObjectPicker.Instance:PickVehicle(0, 0) ~= nil");
        let miss = run(&mut t, "return IsoObjectPicker.Instance:PickVehicle(100000, 0) == nil");
        assert!(hit.truthy() && miss.truthy(), "{hit:?} {miss:?}");
    }
}

/// The Lua object with `id` among the objects of the square `square`
/// (`ObjectRef.id` is the object's `IsoObject.id`); `None` when the square
/// or the object is gone.
pub fn world_object_value_by_id(square: [i32; 3], id: u32) -> LuaResult<Option<Value>> {
    let [x, y, z] = square;
    let object = crate::bind_world::try_with_world_read(|w| {
        let Some(cell) = w.cell.as_ref() else {
            return Ok(None);
        };
        let Some(square) = cell.get_grid_square(x, y, z) else {
            return Ok(None);
        };
        let square = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("contextual action square"))?;
        Ok(square
            .objects
            .iter()
            .map(|slot| slot.ref_handle())
            .find(|object| object.try_borrow().is_ok_and(|object| object.id == id)))
    })
    .map_err(crate::bind_world_object::host_error)?;
    let Some(object) = object else {
        return Ok(None);
    };
    let value = crate::bind_world_object::object_value(&object)?;
    crate::bind_world_object::remember_square(&value, square);
    Ok(Some(value))
}
