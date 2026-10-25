//! `zombie.vehicles.VirtualVehicle` and `VirtualVehicleManager` (single
//! player): vehicles that left the world with their chunk (or were loaded
//! with `inMeta` set) but keep running in the "meta" while their alarm,
//! siren or engine is on.
//!
//! A virtual vehicle keeps the [`BaseVehicle`] it came from and shares its
//! parts, engine, alarm and lightbar modes (the original hands the same
//! objects to the virtual vehicle); it copies the id, `sqlId`, position,
//! script name, headlights and siren start time. While in the meta the
//! alarm's light flashes switch the virtual vehicle's headlights, not the
//! vehicle's (`onVehicleAlarmEvent`). When it stops (`stopUpdatingInMeta`)
//! the vehicle takes the state back (`removeFromMeta`) and its row is
//! written (`save`: `VehiclesDB2.updateVehicle` with a chunk at the
//! virtual position). Adding the vehicle to the world again
//! (`addToWorld`) takes it out of the meta by `sqlId`.
//!
//! The manager is a thread-local singleton, as
//! `VirtualVehicleManager.getInstance()`; [`VirtualVehicleMeta`] is the
//! [`VehicleMetaHook`] the streamer calls (`IsoChunk.removeFromWorld`).
//! Sounds (`VehicleSounds`, the meta world-sound repeats that need a
//! listener in range) and the world-map rendering are not modelled.

use crate::jmath::{coorddivision, fastfloor};
use crate::vehicle::{BaseVehicle, VehicleRef};
use crate::{env, vehicles_db};
use pz_world::chunk_vehicles::{LoadedVehicle, VehicleHandle, VehicleOrigin};
use pz_world::save::vehicle::VehicleRow;
use pz_world::save::vehicles_db::{VehicleIds, VehicleMetaHook};
use pz_world::save::SaveCtx;
use std::cell::RefCell;

/// `VirtualVehicle`.
pub struct VirtualVehicle {
    pub vehicle: VehicleRef,
    pub vehicle_id: i16,
    pub sql_id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub script_name: String,
    pub headlights_on: bool,
    pub siren_start_time: f64,
}

impl VirtualVehicle {
    /// `set(vehicle)`: the copies, then the vehicle leaves its square
    /// (`setSquare(null)`, `setCurrent(null)`).
    pub fn set(vehicle: VehicleRef) -> VirtualVehicle {
        let vv = {
            let st = vehicle.st();
            VirtualVehicle {
                vehicle_id: st.vehicle_id,
                sql_id: st.sql_id,
                x: st.x,
                y: st.y,
                z: st.z,
                script_name: st.script_name.clone(),
                headlights_on: st.headlights_on,
                siren_start_time: st.siren_start_time,
                vehicle: vehicle.clone(),
            }
        };
        {
            let mut st = vehicle.st_mut();
            st.square = None;
            st.current = None;
        }
        vv
    }

    /// `update()`: parts, engine and alarm (sounds are not modelled). The
    /// parts, engine and alarm see this record as their owner: its
    /// headlights are the ones the headlight parts read and the alarm's
    /// light flashes switch.
    pub fn update(&mut self) {
        let v = &self.vehicle;
        v.st_mut().meta_headlights = Some(self.headlights_on);
        v.update_parts_in_meta();
        if v.engine_part().is_some() {
            v.update_engine_in_meta();
        }
        v.update_alarm_in_meta();
        if let Some(h) = v.st_mut().meta_headlights.take() {
            self.headlights_on = h;
        }
    }

    /// `shouldUpdateInMeta()`: an active alarm or siren (with battery) or
    /// a running engine.
    pub fn should_update_in_meta(&self) -> bool {
        let v = &self.vehicle;
        let battery = v.battery_charge() > 0.0;
        let alarm = battery && v.has_alarm() && v.st().alarm.start_time > 0.0;
        let siren = battery && v.has_lightbar() && v.st().lightbar_siren_mode != 0;
        alarm || siren || v.is_engine_running()
    }

    /// `removeFromMeta(vehicle)`: the vehicle takes back the state (the
    /// parts, sounds and alarm are the same objects here), its headlights
    /// from this record.
    pub fn remove_from_meta(&self, vehicle: Option<&BaseVehicle>) {
        let Some(v) = vehicle else { return };
        v.set_headlights_on(self.headlights_on);
    }

    /// `save()`: `VehiclesDB2.updateVehicle(vehicle)` with the chunk at the
    /// virtual position (a vehicle without an id gets one first); the row,
    /// or the exception `VehicleBuffer.set` throws.
    pub fn save(
        &self,
        ids: Option<&mut VehicleIds>,
        ctx: &SaveCtx<'_>,
    ) -> Result<VehicleRow, String> {
        let v = &self.vehicle;
        let chunk = (
            coorddivision(fastfloor(self.x), 8),
            coorddivision(fastfloor(self.y), 8),
        );
        v.st_mut().chunk = Some(chunk);
        if v.st().sql_id == -1 {
            if let Some(ids) = ids {
                v.st_mut().sql_id = ids.allocate_id();
            }
        }
        let row = vehicles_db::vehicle_buffer(v, ctx);
        // `this.vehicle.chunk = null`.
        v.st_mut().chunk = None;
        row
    }
}

/// `VirtualVehicleManager`.
#[derive(Default)]
pub struct VirtualVehicleManager {
    pub vehicles: Vec<VirtualVehicle>,
    /// Rows `VehiclesDB2.updateVehicle` queued for the streamer thread
    /// (`ChunkStreamer::queue_vehicle_update`), or the exceptions.
    pub queued: Vec<Result<VehicleRow, String>>,
}

impl VirtualVehicleManager {
    /// `addToMeta(vehicle)`.
    pub fn add_to_meta(&mut self, vehicle: VehicleRef) {
        self.vehicles.push(VirtualVehicle::set(vehicle));
    }

    /// `removeFromMeta(vehicle)`: the first record with the vehicle's
    /// `sqlId` gives its state back and is dropped.
    pub fn remove_from_meta(&mut self, vehicle: &BaseVehicle) {
        let id = vehicle.st().sql_id;
        if let Some(i) = self.vehicles.iter().position(|v| v.sql_id == id) {
            self.vehicles[i].remove_from_meta(Some(vehicle));
            self.vehicles.remove(i);
        }
    }

    /// `update()`: each record updates; those that stopped give the
    /// vehicle its state back, are saved (`stopUpdatingInMeta`) and leave.
    pub fn update(&mut self, mut ids: Option<&mut VehicleIds>, ctx: &SaveCtx<'_>) {
        let mut i = 0;
        while i < self.vehicles.len() {
            self.vehicles[i].update();
            if self.vehicles[i].should_update_in_meta() {
                i += 1;
                continue;
            }
            let vv = self.vehicles.remove(i);
            vv.remove_from_meta(Some(&vv.vehicle));
            self.queued.push(vv.save(ids.as_deref_mut(), ctx));
        }
    }

    /// `save()` (game save): every record's row.
    pub fn save(&mut self, mut ids: Option<&mut VehicleIds>, ctx: &SaveCtx<'_>) {
        for vv in &self.vehicles {
            self.queued.push(vv.save(ids.as_deref_mut(), ctx));
        }
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        self.vehicles.clear();
    }

    /// The queued rows (for the streamer) and errors, in order.
    pub fn take_queued(&mut self) -> Vec<Result<VehicleRow, String>> {
        std::mem::take(&mut self.queued)
    }
}

thread_local! {
    static MANAGER: RefCell<VirtualVehicleManager> = RefCell::new(VirtualVehicleManager::default());
}

/// `VirtualVehicleManager.getInstance()`.
pub fn with_manager<R>(f: impl FnOnce(&mut VirtualVehicleManager) -> R) -> R {
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

/// The [`VehicleMetaHook`] for the streamer (`StreamCtx::vehicle_meta`):
/// runtime vehicles go to the thread's [`VirtualVehicleManager`]; other
/// entries (vehicles chunk loading put there that the runtime never took
/// over) are not runtime vehicles and are dropped, as nothing could run
/// them.
#[derive(Default)]
pub struct VirtualVehicleMeta {
    /// Entries that were not runtime vehicles.
    pub skipped: usize,
}

impl VehicleMetaHook for VirtualVehicleMeta {
    fn add_to_meta(&mut self, vehicle: VehicleHandle) {
        match vehicle.downcast::<BaseVehicle>() {
            Ok(v) => with_manager(|m| m.add_to_meta(v)),
            Err(_) => self.skipped += 1,
        }
    }
}

/// `QueueLoadVehiclesInMeta.processMain` for the vehicles pz-world's
/// `vehicles_db::load_vehicles_in_meta` loaded: `make` builds the runtime
/// vehicle of each (for example [`crate::save::from_save`]), which takes
/// the row id as its `sqlId` and the dummy square, then goes through
/// [`add_loaded_to_meta`]. Returns the vehicles made, in order (those
/// already loaded included, as the original skips them only there).
pub fn add_loaded_vehicles_to_meta(
    loaded: &[LoadedVehicle],
    mut make: impl FnMut(&LoadedVehicle) -> Option<VehicleRef>,
) -> Vec<VehicleRef> {
    let mut out = Vec::new();
    for lv in loaded {
        let Some(v) = make(lv) else { continue };
        {
            let mut st = v.st_mut();
            st.square = lv.square;
            st.current = lv.square;
            if let VehicleOrigin::Database { sql_id } = lv.origin {
                st.sql_id = sql_id;
            }
        }
        add_loaded_to_meta(v.clone());
        out.push(v);
    }
    out
}

/// `VehiclesDB2.loadVehiclesInMeta` main-thread part for one loaded row
/// vehicle (`vehicle.square = null; addToWorld(); removeFromWorld();
/// addToMeta(vehicle)`), skipped when its id is already loaded.
pub fn add_loaded_to_meta(vehicle: VehicleRef) {
    let loaded =
        env::vehicle_ids().is_some_and(|ids| ids.borrow().is_vehicle_loaded(vehicle.st().sql_id));
    if loaded {
        return;
    }
    vehicle.st_mut().square = None;
    vehicle.add_to_world();
    vehicle.remove_from_world();
    with_manager(|m| m.add_to_meta(vehicle));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::save::items::AllItems;
    use pz_world::save::{NoSaveHooks, SaveSettings, SaveTables};
    use pz_world::IsoSpriteManager;
    use std::rc::Rc;

    fn install_ids(used: &[i32]) -> env::VehicleIdsRef {
        let ids = Rc::new(RefCell::new(VehicleIds {
            used: used.iter().copied().collect(),
            ..Default::default()
        }));
        env::set_vehicle_ids(Some(ids.clone()));
        ids
    }

    fn placed(sql_id: i32, x: f32, y: f32) -> VehicleRef {
        let v = BaseVehicle::new();
        v.set_position(x, y, 0.0);
        let mut st = v.st_mut();
        st.sql_id = sql_id;
        st.square = Some((x as i32, y as i32, 0));
        st.current = st.square;
        drop(st);
        v
    }

    #[test]
    fn add_to_world_allocates_and_loads_ids() {
        let ids = install_ids(&[1, 2, 4]);
        let fresh = placed(-1, 10.5, 20.5);
        fresh.add_to_world();
        assert_eq!(fresh.st().sql_id, 3);
        let saved = placed(2, 11.5, 20.5);
        saved.add_to_world();
        assert_eq!(saved.st().sql_id, 2);
        {
            let ids = ids.borrow();
            assert_eq!(ids.used.iter().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);
            assert!(ids.is_vehicle_loaded(3) && ids.is_vehicle_loaded(2));
        }
        saved.remove_from_world();
        assert!(!ids.borrow().is_vehicle_loaded(2));
        assert!(ids.borrow().is_vehicle_loaded(3));
        // A second fresh vehicle takes the next free id.
        let next = placed(-1, 12.5, 20.5);
        next.add_to_world();
        assert_eq!(next.st().sql_id, 5);
    }

    #[test]
    fn the_hook_moves_runtime_vehicles_into_the_meta() {
        let v = placed(7, 100.25, 200.75);
        v.st_mut().headlights_on = true;
        let mut hook = VirtualVehicleMeta::default();
        hook.add_to_meta(crate::cell_link::handle(&v));
        hook.add_to_meta(Rc::new(5_i32));
        assert_eq!(hook.skipped, 1);
        with_manager(|m| {
            assert_eq!(m.vehicles.len(), 1);
            let vv = &m.vehicles[0];
            assert_eq!((vv.sql_id, vv.x, vv.y), (7, 100.25, 200.75));
            assert!(vv.headlights_on);
            assert!(Rc::ptr_eq(&vv.vehicle, &v));
        });
        // `set` takes the vehicle off its square.
        assert!(v.st().square.is_none() && v.st().current.is_none());
    }

    #[test]
    fn adding_to_the_world_takes_the_vehicle_out_of_the_meta() {
        install_ids(&[7]);
        let v = placed(7, 100.25, 200.75);
        let other = placed(8, 300.5, 400.5);
        with_manager(|m| {
            m.add_to_meta(other.clone());
            m.add_to_meta(v.clone());
            // The alarm switched the record's lights on while in the meta.
            m.vehicles[1].headlights_on = true;
        });
        assert!(!v.headlights_on());
        v.add_to_world();
        assert!(v.headlights_on());
        with_manager(|m| {
            assert_eq!(m.vehicles.len(), 1);
            assert_eq!(m.vehicles[0].sql_id, 8);
        });
    }

    #[test]
    fn records_that_stop_are_saved_at_their_position() {
        let ids = install_ids(&[1]);
        let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let tables = SaveTables {
            factory: vec![(0, "IsoObject".into()), (33, "Vehicle".into())],
            blood_body_parts: 0,
            character_stats: Vec::new(),
        };
        let settings = SaveSettings::default();
        let ctx = SaveCtx {
            sprites: &sprites,
            tables: &tables,
            settings: &settings,
            items: &AllItems,
            hooks: &NoSaveHooks,
            meta: None,
        };
        // Without a battery, alarm, siren or running engine a record stops
        // at its first update: the vehicle takes its state back and its row
        // is written (a vehicle without a script throws in
        // `VehicleBuffer.set`, after the id and the chunk were set).
        let v = placed(-1, -0.5, 17.25);
        let mut m = VirtualVehicleManager::default();
        m.add_to_meta(v.clone());
        m.vehicles[0].headlights_on = true;
        assert!(!m.vehicles[0].should_update_in_meta());
        m.update(Some(&mut ids.borrow_mut()), &ctx);
        assert!(m.vehicles.is_empty());
        assert!(v.headlights_on());
        assert_eq!(v.st().sql_id, 2);
        assert!(v.st().chunk.is_none());
        let q = m.take_queued();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].as_ref().unwrap_err(), "java.lang.NullPointerException");
        // `save` writes every record, the chunk from `coorddivision`.
        let w = placed(-1, -0.5, 17.25);
        m.add_to_meta(w.clone());
        let vv = &m.vehicles[0];
        assert_eq!(
            (
                coorddivision(fastfloor(vv.x), 8),
                coorddivision(fastfloor(vv.y), 8)
            ),
            (-1, 2)
        );
        m.save(Some(&mut ids.borrow_mut()), &ctx);
        assert_eq!(m.vehicles.len(), 1);
        assert_eq!(w.st().sql_id, 3);
        assert_eq!(m.take_queued().len(), 1);
        m.reset();
        assert!(m.vehicles.is_empty());
    }

    #[test]
    fn loaded_meta_rows_go_through_the_world_into_the_meta() {
        let ids = install_ids(&[5, 6]);
        let saved = pz_world::save::vehicle::VehicleRow {
            id: 5,
            wx: 2,
            wy: 3,
            x: 20.5,
            y: 30.5,
            world_version: pz_world::save::WORLD_VERSION,
            in_meta: true,
            data: {
                let mut w = pz_world::save::io::Writer::new();
                crate::vehicles_db::save(&BaseVehicle::new(), &mut w, 33, 2, &AllItems).unwrap();
                w.buf
            },
        }
        .load(33, (64.0, 192.0), &AllItems)
        .unwrap()
        .unwrap();
        let lv = |id| LoadedVehicle {
            origin: VehicleOrigin::Database { sql_id: id },
            square: Some((4, 6, 0)),
            vehicle: saved.clone(),
        };
        // Vehicle 6 is in the world already: it is not added again.
        ids.borrow_mut().loaded.insert(6);
        let made = add_loaded_vehicles_to_meta(&[lv(5), lv(6)], |l| {
            Some(crate::save::from_save(&l.vehicle))
        });
        assert_eq!(made.len(), 2);
        with_manager(|m| {
            assert_eq!(m.vehicles.len(), 1);
            assert_eq!(m.vehicles[0].sql_id, 5);
        });
        assert!(made[0].st().square.is_none());
        assert_eq!(made[1].st().square, Some((4, 6, 0)));
        // `addToWorld` then `removeFromWorld`: 5 is not loaded.
        assert!(!ids.borrow().is_vehicle_loaded(5));
        assert!(ids.borrow().is_vehicle_loaded(6));
    }
}
