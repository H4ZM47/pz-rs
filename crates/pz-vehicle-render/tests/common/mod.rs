//! Shared setup for the media-gated tests: the game's scripts, a vehicle
//! of a script with its parts' items installed, and the model loader.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_scripts::ScriptManager;
use pz_vehicles::part::PartItem;
use pz_vehicles::{BaseVehicle, VehicleRef};

pub fn game_dir() -> Option<PathBuf> {
    let game = std::env::var_os("PZ_GAME_DIR")
        .map(PathBuf::from)
        .filter(|g| g.join("media").is_dir());
    if game.is_none() {
        eprintln!("PZ_GAME_DIR not set or has no media/; skipping");
    }
    game
}

thread_local! {
    static SCRIPTS: RefCell<Option<Rc<ScriptManager>>> = const { RefCell::new(None) };
}

/// The game's scripts, loaded once per thread and installed for
/// pz-vehicles.
pub fn scripts(game: &std::path::Path) -> Rc<ScriptManager> {
    SCRIPTS.with(|s| {
        if let Some(sm) = s.borrow().as_ref() {
            return sm.clone();
        }
        let sm = Rc::new(ScriptManager::new());
        sm.load_builtin_registries(&game.join("projectzomboid.jar"))
            .expect("built-in registries");
        sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
            .expect("scripts");
        pz_vehicles::vehicle::set_script_manager(sm.clone());
        *s.borrow_mut() = Some(sm.clone());
        sm
    })
}

/// A part item of condition `condition`.
pub fn item(full_type: &str, condition: i32) -> PartItem {
    let (module, type_) = full_type.split_once('.').unwrap_or(("Base", full_type));
    let mut t = ItemTemplate::new(ItemClass::Normal, module, type_);
    t.condition_max = 100;
    let mut i = InventoryItem::create(Rc::new(t), -1);
    i.condition = condition;
    i.condition_max = 100;
    PartItem::Item(Rc::new(RefCell::new(i)))
}

/// A vehicle of `script` at `(x, y, 0)` facing `angle` (radians about the
/// vertical, as `savedRot.setAngleAxis(angle, 0, 1, 0)`), every part with
/// an item type given an item of condition 100.
pub fn vehicle(script: &str, x: f32, y: f32, angle: f32) -> VehicleRef {
    let v = BaseVehicle::new();
    v.set_script_name(script);
    v.set_script(script);
    v.set_position(x, y, 0.0);
    {
        let q = pz_vehicles::jmath::Quaternionf::angle_axis(angle, 0.0, 1.0, 0.0);
        let mut st = v.st_mut();
        st.saved_rot = q;
        st.jni_transform.set_rotation(&q);
        st.jni_transform.origin = pz_vehicles::jmath::Vector3f::new(x, 0.0, y);
        st.skin_index = 0;
        st.square = Some((x.floor() as i32, y.floor() as i32, 0));
        st.poly_dirty = true;
    }
    for p in v.parts() {
        let has_type = p.item_type().is_some_and(|t| !t.is_empty());
        if has_type {
            p.set_inventory_item(Some(item("Base.Part", 100)), 0);
            p.st_mut().condition = 100;
        }
    }
    v
}
