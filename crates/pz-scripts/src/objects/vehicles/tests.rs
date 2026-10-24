use std::path::PathBuf;

use pz_fs::GameFs;

use super::*;
use crate::base::ScriptObject;
use crate::math::{Vector2, Vector3};

/// Loads `text` as the game's only script file.
fn load(tag: &str, text: &str) -> ScriptManager {
    let dir: PathBuf = std::env::temp_dir().join(format!("pz-scripts-vehicles-{tag}-{}", std::process::id()));
    let scripts = dir.join("media").join("scripts");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("vehicles.txt"), text).unwrap();
    let sm = ScriptManager::new();
    sm.load(&GameFs::new(dir.clone()), &[]).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    sm
}

/// A loaded vehicle, after `Loaded` (run here while the bucket's
/// `onScriptLoad` hook is not wired).
fn vehicle(sm: &ScriptManager, name: &str) -> std::rc::Rc<std::cell::RefCell<VehicleScript>> {
    let v = sm.get::<VehicleScript>(ScriptType::Vehicle, name).expect(name);
    if v.borrow().bullet_params().is_none() {
        v.borrow_mut().loaded(sm).unwrap();
    }
    v
}

/// Loads a single vehicle body without a module.
fn script(body: &str) -> ScriptResult<VehicleScript> {
    let sm = ScriptManager::new();
    let mut v = VehicleScript::new();
    v.init_load_pp(&sm, "Test");
    v.load(&sm, "Test", body).map(|_| v)
}

#[test]
fn loads_values_and_children() {
    let v = script(
        "vehicle Test {
            mass = 1200, offRoadEfficiency = 0.5, gearRatioCount = 5, gearRatio5 = 0.9,
            forcedColor = 0.1 0.2 0.3, zombieType = A; B ;, specialKeyRing = ;K1; ;K2,
            textureRust = , hasSiren = TRUE, engineRPMType = van,
            model Default { file = Car, scale = 2, offset = 0 0.5 0, }
            skin { texture = T1, }
            skin { textureMask = M, }
            wheel FrontLeft { front = true, offset = 0.5 -0.25 1, radius = 0.25, }
            area Engine { xywh = 0 1 2 3, }
            passenger FrontLeft { door = DoorFrontLeft, position inside { offset = 0 0 0.5, } }
            physics box { extents = 1 1 1, }
            physics sphere { radius = 0, }
            sound { horn = Horn, alarm = A1  A2, }
            lightbar { leftCol = 2;0.5;-1, }
        }",
    )
    .unwrap();
    assert_eq!(v.mass(), 1200.0);
    assert_eq!(v.offroad_efficiency(), 0.5);
    assert_eq!(v.gear_ratio_count, 5);
    assert_eq!(v.gear_ratio[5], 0.9);
    assert_eq!((v.forced_hue(), v.forced_sat(), v.forced_val()), (0.1, 0.2, 0.3));
    assert_eq!(v.zombie_type().unwrap(), ["A", "B"]);
    assert_eq!(v.special_key_ring().unwrap(), ["K1", "K2"]);
    assert_eq!(v.textures().texture_rust, None);
    assert!(v.has_siren());
    assert_eq!(v.engine_rpm_type(), "van");
    assert_eq!(v.model().unwrap().scale, 2.0);
    assert_eq!(v.skin_count(), 1);
    assert_eq!(v.wheel(0).radius, 0.25);
    assert_eq!(v.area_by_id(Some("Engine")).unwrap().h, 3.0);
    assert_eq!(v.passenger(0).positions[0].offset.z, 0.5);
    assert_eq!(v.physics_shape_count(), 1);
    assert_eq!(v.sounds().alarm, ["A1", "A2"]);
    assert_eq!(v.sounds().horn.as_deref(), Some("Horn"));
    assert_eq!(v.left_siren_col, Some(ImmutableColor::rgb(1.0, 0.5, 0.0)));
    assert!(v.lightbar().enable);
    assert!(v.part_by_id(Some("lightbar")).is_some());
}

#[test]
fn java_exceptions_become_errors() {
    let err = |body: &str| script(body).unwrap_err().0;
    assert_eq!(
        err("vehicle Test { mass, }"),
        "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1"
    );
    assert_eq!(err("vehicle Test { mass = heavy, }"), "java.lang.NumberFormatException: For input string: \"heavy\"");
    assert_eq!(err("vehicle Test { extents = 1  2 3, }"), "java.lang.NumberFormatException: empty String");
    assert_eq!(err("vehicle Test { seats = 2.5, }"), "java.lang.NumberFormatException: For input string: \"2.5\"");
    assert_eq!(
        err("vehicle Test { part P { lua { create, } } }"),
        "java.lang.RuntimeException: expected \"key = value\", got \"create\" in null.Test"
    );
    assert_eq!(
        err("vehicle Test { attachment { bone = b, } }"),
        "java.lang.IllegalArgumentException: ModelAttachment id is null or empty"
    );
}

#[test]
fn parts_merge_and_glob() {
    let v = script(
        "vehicle Test {
            part DoorLeft {
                category = door, itemType = Base.Door;Base.Door2,
                container { capacity = 5, soundMap = Open  OpenSound, }
            }
            part DoorRight { area = , table install { items { 1 { type = A, count = 2, } } time = , } }
            part Door* { durability = 3, container { test = T, } }
            part DoorRight { lua { create = Vehicles.Create, } window { openable = true, } }
        }",
    )
    .unwrap();
    let left = v.part_by_id(Some("DoorLeft")).unwrap();
    assert_eq!(left.durability(), 3.0);
    assert_eq!(left.item_type.as_deref().unwrap(), ["Base.Door", "Base.Door2"]);
    let c = left.container.as_ref().unwrap();
    assert_eq!((c.capacity, c.lua_test.as_deref()), (5, Some("T")));
    assert_eq!(c.sound_by_id("Open"), Some("OpenSound"));
    let right = v.part_by_id(Some("DoorRight")).unwrap();
    assert_eq!(right.durability(), 3.0);
    assert_eq!(right.area, None);
    assert!(right.window.as_ref().unwrap().openable);
    assert_eq!(right.lua_functions.as_deref().unwrap(), [("create".to_string(), "Vehicles.Create".to_string())]);
    let install = &right.tables.as_ref().unwrap()[0].1;
    let Some(LuaValue::Table(items)) = install.rawget_str("items") else { panic!("items") };
    let Some(LuaValue::Table(one)) = items.rawget(&LuaKey::Num(1.0)) else { panic!("1") };
    assert_eq!(one.rawget_str("count"), Some(&LuaValue::Str("2".to_string())));
    assert!(install.rawget_str("time").is_none());
}

#[test]
fn glob_is_a_regex_with_star() {
    let hit = |pattern: &str, id: &str| {
        let v = script(&format!("vehicle Test {{ part {id} {{ }} part {pattern} {{ category = hit, }} }}")).unwrap();
        v.part(0).category.as_deref() == Some("hit")
    };
    assert!(hit("Door*", "DoorFrontLeft"));
    assert!(hit("*Left", "DoorFrontLeft"));
    assert!(!hit("Door*", "WindowFrontLeft"));
    assert!(hit("Seat.*", "SeatX"));
}

#[test]
fn templates_copy_into_vehicles_and_loaded_scales() {
    let sm = load(
        "templates",
        "module Base {
            template vehicle Parts {
                part Seat* { category = seat, }
                part SeatFrontLeft { itemType = Base.Seat, container { seat = FrontLeft, } }
                passenger FrontLeft {
                    position inside { offset = 0 1 0, area = SeatFrontLeft, }
                    switchSeat FrontRight { anim = S, }
                }
                passenger FrontRight { }
                area SeatFrontLeft { xywh = 1 1 1 1, }
                physics box { extents = 1 1 1, }
                sound { engine = Engine, horn = Horn, }
            }
            vehicle Car {
                mechanicType = 2,
                model Default { scale = 0.5, offset = 0 0 1, }
                extents = 2 2 4,
                wheel FrontLeft { front = true, offset = 1 0 1, radius = 0.5, }
                template = Parts,
                template = Parts/area/Missing,
            }
            vehicle Bike {
                template = Parts/passenger/FrontRight,
                template! = Parts,
            }
        }",
    );
    let car = vehicle(&sm, "Car");
    let car = car.borrow();
    assert_eq!(car.extents(), Vector3::new(1.0, 1.0, 2.0));
    assert_eq!(car.shadow_extents(), Vector2::new(1.0, 2.0));
    assert_eq!(car.area(0).x, 0.5);
    assert_eq!(car.physics_shape(0).unwrap().extents.x, 0.5);
    assert_eq!(car.wheel(0).radius, 0.25);
    let seat = car.part_by_id(Some("SeatFrontLeft")).unwrap();
    assert_eq!(seat.item_type.as_deref().unwrap(), ["Base.Seat2"]);
    assert_eq!(seat.container.as_ref().unwrap().seat, 0);
    // Position.makeCopy drops the area.
    assert_eq!(car.passenger(0).positions[0].area, None);
    assert_eq!(car.passenger(0).positions[0].offset.y, 0.5);
    assert_eq!(car.passenger(0).switch_seats[0].seat, 1);
    assert_eq!(car.sounds().engine.as_deref(), Some("Engine"));
    // The horn is not copied from templates, so it does not become an alarm.
    assert!(!car.sounds().alarm_enable);
    assert!(!car.crawl_offsets().is_empty());
    assert!(car.bullet_params().is_some());

    let bike = vehicle(&sm, "Bike");
    let bike = bike.borrow();
    assert_eq!(bike.passenger(0).id.as_deref(), Some("FrontRight"));
    // template! loads the template body in place, including its sound.
    assert_eq!(bike.sounds().alarm, ["Horn"]);
    assert_eq!(bike.passenger(1).positions[0].area.as_deref(), Some("SeatFrontLeft"));

    let t = sm.get::<VehicleTemplate>(ScriptType::VehicleTemplate, "Parts").unwrap();
    let ts = t.borrow().script.clone().unwrap();
    assert_eq!(ts.borrow().part_count(), 1);
    assert_eq!(ts.borrow().area(0).x, 1.0);
}
