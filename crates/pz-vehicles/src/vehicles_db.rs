//! Runtime vehicles into `vehicles.db`: `BaseVehicle.save`,
//! `BaseVehicle.shouldUpdateInMeta` and `VehiclesDB2.VehicleBuffer.set`
//! on the runtime [`BaseVehicle`], and [`RuntimeVehicleSaver`], the
//! [`ChunkVehicleSaver`] pz-world's `unload_chunk`
//! (`VehiclesDB2.unloadChunk`) writes a chunk's list with.
//!
//! `IsoChunk.removeFromWorld` asks the same saver which vehicles stand on
//! the chunk's squares, whether they keep updating in the meta, and for
//! their rows off the square (`pz_world::save::vehicles_db::remove_vehicles_to_meta`).
//!
//! `VehicleBuffer.set` runs, in order: the `sqlId` asserts (off in the
//! shipped game, so an id below 1 is written as it is), `vehicle.chunk.wx`
//! (a vehicle without a chunk throws `NullPointerException`), the
//! position, `shouldUpdateInMeta()` and `save`. `shouldUpdateInMeta` starts
//! with `hasAlarm()`, which reads `getScript().getSounds()`: a vehicle
//! without a script (one `setScript` never attached a script to, such as a
//! vehicle loaded but never added to the world, or one with a blank script
//! name) throws `NullPointerException` there. `unloadChunk` logs the
//! exception and goes on with the next vehicle of the list, so only that
//! vehicle is not written.

use crate::save::part_to_save;
use crate::vehicle::BaseVehicle;
use pz_inventory::SavedItem;
use pz_world::chunk::IsoChunk;
use pz_world::chunk_vehicles::VehicleHandle;
use pz_world::save::io::{SaveResult, Writer};
use pz_world::save::items::ItemRegistry;
use pz_world::save::vehicle::VehicleRow;
use pz_world::save::vehicles_db::{ChunkVehicleSaver, LoadedVehicleSaver, NoVehicleScripts};
use pz_world::save::visual::MovingBase;
use pz_world::save::SaveCtx;

const NPE: &str = "java.lang.NullPointerException";

/// `PZMath.clamp(float, float, float)`.
fn clamp_f(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `isSirenActive()`: `hasSiren() && lightbarSirenMode.isEnable() &&
/// getBatteryCharge() > 0` (the script is attached).
fn is_siren_active(v: &BaseVehicle) -> bool {
    v.has_lightbar() && v.st().lightbar_siren_mode != 0 && v.battery_charge() > 0.0
}

/// `BaseVehicle.shouldUpdateInMeta()`: `isAlarmActive() ||
/// isSirenActive() || isEngineRunning()`. Without a script `hasAlarm()`
/// dereferences the null script (`NullPointerException`).
pub fn should_update_in_meta(v: &BaseVehicle) -> Result<bool, String> {
    if v.script().is_none() {
        return Err(NPE.into());
    }
    Ok(v.is_alarm_active() || is_siren_active(v) || v.is_engine_running())
}

/// `BaseVehicle.save(output)` (with the serialise and class bytes) for the
/// runtime vehicle.
///
/// `class_id` is the vehicle's object class id, `tile_scale`
/// `Core.tileScale` (the offsets of a vehicle that was never loaded).
/// Like the original, saving leaves `savedRot` set to the rotation of the
/// world transform and lets `VehicleAlarm.getChosenSound()` choose an alarm
/// sound (a `Rand` draw when the chosen one is not the script's).
pub fn save(
    v: &BaseVehicle,
    w: &mut Writer,
    class_id: i8,
    tile_scale: i32,
    items: &dyn ItemRegistry,
) -> SaveResult<()> {
    // `IsoMovingObject.save`, at the position clamped into `square`.
    {
        let st = v.st();
        let mut base = st.iso_object.clone().unwrap_or(MovingBase {
            offset_x: (32 * tile_scale) as f32,
            offset_y: (96 * tile_scale) as f32,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dir: 0,
            mod_data: None,
        });
        base.z = st.z;
        let (mut x, mut y) = (st.x, st.y);
        if let Some((sx, sy, _)) = st.square {
            let d = 5.0e-4f32;
            x = clamp_f(x, sx as f32 + d, sx as f32 + 1.0 - d);
            y = clamp_f(y, sy as f32 + d, sy as f32 + 1.0 - d);
        }
        base.save_at(w, class_id, x, y);
    }
    // `getWorldTransform(xfrm)`: the origin's height, then
    // `xfrm.getRotation(savedRot)`.
    let (origin_y, q) = {
        let st = v.st();
        (st.jni_transform.origin.y, st.jni_transform.rotation())
    };
    v.st_mut().saved_rot = q;
    w.f32(origin_y);
    w.f32(q.x);
    w.f32(q.y);
    w.f32(q.z);
    w.f32(q.w);
    {
        let st = v.st();
        w.string(&st.script_name);
        w.i32(st.skin_index);
    }
    w.bool(v.is_engine_running());
    {
        let st = v.st();
        w.i32(st.front_end_durability);
        w.i32(st.rear_end_durability);
        w.i32(st.current_front_end_durability);
        w.i32(st.current_rear_end_durability);
    }
    w.i32(v.engine_loudness());
    w.i32(v.engine_quality());
    {
        let st = v.st();
        w.i32(st.key_id);
        w.i8(st.key_spawned);
        w.bool(st.headlights_on);
        w.bool(st.created);
        w.bool(st.sound_horn_on);
        w.bool(st.sound_back_move_on);
        w.i8(st.lightbar_lights_mode.mode as i8);
        w.i8(st.lightbar_siren_mode as i8);
    }
    let parts = v.parts();
    w.i16(parts.len() as i16);
    for p in &parts {
        part_to_save(p).save(w, items)?;
    }
    {
        let st = v.st();
        w.bool(st.key_is_on_door);
        w.bool(st.hotwired);
        w.bool(st.hotwired_broken);
        w.bool(st.keys_in_ignition);
        w.f32(st.rust);
        w.f32(st.color_hue);
        w.f32(st.color_saturation);
        w.f32(st.color_value);
    }
    w.i32(v.engine_power());
    {
        let st = v.st();
        w.i16(st.vehicle_id);
        w.i32(st.mechanical_id);
        w.bool(st.alarmed);
        w.f64(st.alarm.start_time);
    }
    let chosen = v.chosen_alarm_sound();
    w.opt_string(chosen.as_deref());
    w.f64(v.st().siren_start_time);
    match v.current_key() {
        Some(k) => {
            w.i8(1);
            SavedItem::Item(Box::new(k.borrow().clone())).save(w, items)?;
        }
        None => w.i8(0),
    }
    let st = v.st();
    // `bloodIntensity` is a `HashMap<String, Byte>`: its iteration order.
    let mut blood = st.blood_intensity.clone();
    pz_inventory::java::sort_string_hash_map(&mut blood, |e| e.0.as_str());
    w.i8(blood.len() as i8);
    for (k, b) in &blood {
        w.string(k);
        w.i8(*b);
    }
    if st.vehicle_towing_id != -1 {
        w.i8(1);
        w.i32(st.vehicle_towing_id);
        w.opt_string(st.tow_attachment_self.as_deref());
        w.opt_string(st.tow_attachment_other.as_deref());
        w.f32(st.row_constraint_z_offset);
    } else {
        w.i8(0);
    }
    w.f32(st.regulator_speed);
    w.bool(st.previously_entered);
    w.bool(st.previously_moved);
    // The animals block after its length (kept as saved; an empty list
    // saves as the single `0` byte).
    let animals: &[u8] = if st.animals.is_empty() {
        &[0]
    } else {
        &st.animals
    };
    w.i32(animals.len() as i32);
    w.bytes(animals);
    Ok(())
}

/// `VehiclesDB2.VehicleBuffer.set(vehicle)`: the row `updateVehicle`
/// writes, or the exception `set` throws (see the module docs).
///
/// `x`/`y` are `getX()`/`getY()` (not clamped), `wx`/`wy` those of
/// `vehicle.chunk`, `worldversion` `IsoWorld.getWorldVersion()`.
pub fn vehicle_buffer(v: &BaseVehicle, ctx: &SaveCtx<'_>) -> Result<VehicleRow, String> {
    let (id, chunk, x, y) = {
        let st = v.st();
        (st.sql_id, st.chunk, st.x, st.y)
    };
    let Some((wx, wy)) = chunk else {
        return Err(NPE.into());
    };
    let in_meta = should_update_in_meta(v)?;
    let mut w = Writer::new();
    let class = ctx.tables.class_id("Vehicle");
    save(v, &mut w, class, ctx.settings.tile_scale, ctx.items).map_err(|e| e.0)?;
    Ok(VehicleRow {
        id,
        wx,
        wy,
        x,
        y,
        world_version: pz_world::save::WORLD_VERSION,
        in_meta,
        data: w.buf,
    })
}

/// [`ChunkVehicleSaver`] for a chunk list holding runtime vehicles: each
/// runtime [`BaseVehicle`] is written with [`vehicle_buffer`]; any other
/// entry (a vehicle chunk loading put there that the runtime has not taken
/// over, such as pz-world's `LoadedVehicle`) goes to `others`.
///
/// For a chunk that is unloading, the streamer passes this as
/// `StreamCtx::vehicle_saver`, with `others` set to pz-world's
/// `LoadedVehicleSaver { scripts: &NoVehicleScripts }` (a vehicle never
/// added to the world has no script, so the original throws for it too).
pub struct RuntimeVehicleSaver<'a> {
    pub others: &'a dyn ChunkVehicleSaver,
}

impl Default for RuntimeVehicleSaver<'static> {
    /// `others`: `LoadedVehicleSaver { scripts: &NoVehicleScripts }`.
    fn default() -> Self {
        RuntimeVehicleSaver {
            others: &LoadedVehicleSaver {
                scripts: &NoVehicleScripts,
            },
        }
    }
}

impl ChunkVehicleSaver for RuntimeVehicleSaver<'_> {
    fn vehicle_buffer(
        &self,
        vehicle: &VehicleHandle,
        chunk: &IsoChunk,
        ctx: &SaveCtx<'_>,
    ) -> Result<VehicleRow, String> {
        match vehicle.clone().downcast::<BaseVehicle>() {
            Ok(v) => vehicle_buffer(&v, ctx),
            Err(_) => self.others.vehicle_buffer(vehicle, chunk, ctx),
        }
    }

    /// A runtime vehicle added to the world stands in its `current`
    /// square's moving objects.
    fn moving_square(&self, vehicle: &VehicleHandle) -> Option<(i32, i32, i32)> {
        match vehicle.clone().downcast::<BaseVehicle>() {
            Ok(v) => {
                let st = v.st();
                st.current.filter(|_| st.added_to_world)
            }
            Err(_) => self.others.moving_square(vehicle),
        }
    }

    fn sql_id(&self, vehicle: &VehicleHandle) -> i32 {
        match vehicle.clone().downcast::<BaseVehicle>() {
            Ok(v) => v.st().sql_id,
            Err(_) => self.others.sql_id(vehicle),
        }
    }

    fn set_sql_id(&self, vehicle: &VehicleHandle, id: i32) {
        match vehicle.clone().downcast::<BaseVehicle>() {
            Ok(v) => v.st_mut().sql_id = id,
            Err(_) => self.others.set_sql_id(vehicle, id),
        }
    }

    fn should_update_in_meta(&self, vehicle: &VehicleHandle) -> Result<bool, String> {
        match vehicle.clone().downcast::<BaseVehicle>() {
            Ok(v) => should_update_in_meta(&v),
            Err(_) => self.others.should_update_in_meta(vehicle),
        }
    }

    /// `IsoChunk.removeFromWorld` took the vehicle off its square
    /// (`removeFromSquare`, `setSquare(null)`, which the runtime vehicle
    /// keeps), so `save` no longer clamps the position.
    fn vehicle_buffer_off_square(
        &self,
        vehicle: &VehicleHandle,
        chunk: &IsoChunk,
        ctx: &SaveCtx<'_>,
    ) -> Result<VehicleRow, String> {
        match vehicle.clone().downcast::<BaseVehicle>() {
            Ok(v) => {
                v.st_mut().square = None;
                vehicle_buffer(&v, ctx)
            }
            Err(_) => self.others.vehicle_buffer_off_square(vehicle, chunk, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::chunk_vehicles::{LoadedVehicle, VehicleOrigin};
    use pz_world::save::items::AllItems;
    use pz_world::save::{NoSaveHooks, SaveSettings, SaveTables};
    use pz_world::IsoSpriteManager;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    /// A fresh directory for one test call.
    fn temp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "pz-vehicles-vdb-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn tables() -> SaveTables {
        SaveTables {
            factory: vec![(0, "IsoObject".into()), (33, "Vehicle".into())],
            blood_body_parts: 0,
            character_stats: Vec::new(),
        }
    }

    /// Writes the row it is asked for (stands in for the saver of the
    /// entries chunk loading made).
    struct Fixed;
    impl ChunkVehicleSaver for Fixed {
        fn vehicle_buffer(
            &self,
            _vehicle: &VehicleHandle,
            chunk: &IsoChunk,
            _ctx: &SaveCtx<'_>,
        ) -> Result<VehicleRow, String> {
            Ok(VehicleRow {
                id: 9,
                wx: chunk.wx,
                wy: chunk.wy,
                x: 1.5,
                y: 2.5,
                world_version: 249,
                in_meta: false,
                data: vec![1, 33],
            })
        }
    }

    #[test]
    fn a_vehicle_without_script_or_chunk_throws_and_the_rest_still_save() {
        let dir = temp_dir();
        let mut conn = pz_sqlite::Connection::open(dir.join("vehicles.db")).unwrap();
        conn.execute("CREATE TABLE vehicles (id   INTEGER PRIMARY KEY NOT NULL,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,inMeta BOOLEAN NULL DEFAULT FALSE,worldversion    INTEGER,data BLOB);").unwrap();
        conn.set_autocommit(false).unwrap();
        let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let t = tables();
        let settings = SaveSettings::default();
        let ctx = SaveCtx {
            sprites: &sprites,
            tables: &t,
            settings: &settings,
            items: &AllItems,
            hooks: &NoSaveHooks,
            meta: None,
        };
        // No script attached (never added to the world).
        let scriptless = BaseVehicle::new();
        scriptless.st_mut().sql_id = 4;
        scriptless.st_mut().chunk = Some((10, 20));
        assert_eq!(
            vehicle_buffer(&scriptless, &ctx).unwrap_err(),
            "java.lang.NullPointerException"
        );
        // No chunk: `vehicle.chunk.wx` fails first.
        let chunkless = BaseVehicle::new();
        chunkless.st_mut().sql_id = 5;
        let loaded = LoadedVehicle {
            origin: VehicleOrigin::Database { sql_id: 9 },
            square: None,
            vehicle: {
                let mut w = Writer::new();
                save(&chunkless, &mut w, 33, 2, &AllItems).unwrap();
                let row = VehicleRow {
                    id: 9,
                    wx: 10,
                    wy: 20,
                    x: 0.0,
                    y: 0.0,
                    world_version: pz_world::save::WORLD_VERSION,
                    in_meta: false,
                    data: w.buf,
                };
                row.load(33, (64.0, 192.0), &AllItems).unwrap().unwrap()
            },
        };
        let mut chunk = IsoChunk::new(10, 20);
        chunk.vehicles.add(crate::cell_link::handle(&scriptless));
        chunk.vehicles.add(crate::cell_link::handle(&chunkless));
        chunk.vehicles.add(Rc::new(loaded));
        let saver = RuntimeVehicleSaver { others: &Fixed };
        let r = pz_world::save::vehicles_db::unload_chunk(Some(&mut conn), &chunk, &ctx, &saver);
        assert_eq!(r.saved, vec![9]);
        assert_eq!(
            r.errors,
            vec![
                (0, "java.lang.NullPointerException".to_string()),
                (1, "java.lang.NullPointerException".to_string())
            ]
        );
        assert!(!pz_world::save::vehicles_db::is_in_db(&mut conn, 4).unwrap());
        assert!(pz_world::save::vehicles_db::is_in_db(&mut conn, 9).unwrap());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_bytes_load_back_into_the_same_vehicle() {
        let v = BaseVehicle::new();
        {
            let mut st = v.st_mut();
            st.x = 100.25;
            st.y = 200.75;
            st.script_name = "Base.CarNormal".into();
            st.skin_index = 2;
            st.key_id = 1234;
            st.rust = 0.5;
            st.color_hue = 0.25;
            st.mechanical_id = 7;
            st.blood_intensity = vec![("Front".into(), 3), ("Rear".into(), 1)];
            st.vehicle_towing_id = 12;
            st.tow_attachment_self = Some("trailer".into());
            st.regulator_speed = 30.0;
        }
        let save_bytes = |v: &BaseVehicle| {
            let mut w = Writer::new();
            save(v, &mut w, 33, 2, &AllItems).unwrap();
            w.buf
        };
        let a = save_bytes(&v);
        let row = VehicleRow {
            id: 1,
            wx: 12,
            wy: 25,
            x: 100.25,
            y: 200.75,
            world_version: pz_world::save::WORLD_VERSION,
            in_meta: false,
            data: a.clone(),
        };
        let back = row.load(33, (64.0, 192.0), &AllItems).unwrap().unwrap();
        assert_eq!(back.script_name, "Base.CarNormal");
        assert_eq!(back.base.offset_x, 64.0);
        assert_eq!(back.towing.as_ref().unwrap().attachment_other, "");
        assert_eq!(back.animals, vec![0]);
        let v2 = crate::save::from_save(&back);
        assert_eq!(save_bytes(&v2), a);
        // Standing on a square clamps the saved position into it.
        v2.st_mut().square = Some((101, 200, 0));
        let b = save_bytes(&v2);
        let back = VehicleRow { data: b, ..row }
            .load(33, (64.0, 192.0), &AllItems)
            .unwrap()
            .unwrap();
        assert_eq!(back.base.x, 101.0005);
        assert_eq!(back.base.y, 200.75);
    }
}
