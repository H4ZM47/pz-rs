//! Engine implementation of [`pz_animals::AnimalWorld`] backing
//! `StateWorld::animal_host(chr)` for live Build 42.22 animals.
//!
//! Captures live `SquareRef` (`Rc<RefCell<IsoGridSquare>>`) handles around the
//! active animal and borrows `&IsoMetaGrid` immutably, so it coexists cleanly
//! alongside `pz_world_states::StatesWorldAdapter<'a>`'s `&mut IsoCell` borrow
//! without re-entering the global `WORLD` `RefCell`.

use std::collections::HashMap;

use pz_characters::pz_animals::population::zone;
use pz_characters::pz_animals::{
    character_ref, square_ref_xyz, AnimalWorld, AnimalZoneJunction, NewAnimal, ObjectRef,
};
use pz_world::objects::SpriteName;
use pz_world::{IsoCell, IsoMetaGrid, IsoSpriteManager, SquareRef};

/// Copyable sprite flag indices extracted from `IsoSpriteManager::tables`.
#[derive(Clone, Copy, Debug)]
pub struct AnimalSpriteFlags {
    pub exterior: pz_world::sprite::IsoFlagType,
    pub solidfloor: pz_world::sprite::IsoFlagType,
    pub solid: pz_world::sprite::IsoFlagType,
    pub solidtrans: pz_world::sprite::IsoFlagType,
    pub water: pz_world::sprite::IsoFlagType,
    pub tree: pz_world::sprite::IsoObjectType,
}

impl AnimalSpriteFlags {
    pub fn from_sprites(sprites: &IsoSpriteManager) -> Self {
        let f = &sprites.tables.flag;
        Self {
            exterior: f.exterior,
            solidfloor: f.solidfloor,
            solid: f.solid,
            solidtrans: f.solidtrans,
            water: f.water,
            tree: sprites.tables.ty.tree,
        }
    }
}

/// Engine-side [`AnimalWorld`] view over the live world cell neighborhood,
/// meta grid, weather, and puddles singletons.
pub struct EngineAnimalWorld<'a> {
    pub meta: &'a IsoMetaGrid,
    pub flags: AnimalSpriteFlags,
    /// Live `SquareRef` handles around the active animal.
    pub squares: HashMap<(i32, i32, i32), SquareRef>,
    /// Current `(x, y, z)` square of the animal being ticked (if any).
    pub animal_square: Option<(i32, i32, i32)>,
    /// Speed in km/h of the vehicle carrying this animal (if inside one).
    pub vehicle_speed_kmh: Option<f32>,
    /// Newly constructed babies / spawned animals awaiting materialization.
    pub pending_new_animals: Vec<NewAnimal>,
    /// Replacement grown animal from `data_grow`, if triggered this tick.
    pub pending_grown: Option<NewAnimal>,
    pub added_to_world_count: u32,
    pub removed_from_world_count: u32,
    pub recently_removed_count: u32,
    pub instance_manager_updates: u32,
}

impl<'a> EngineAnimalWorld<'a> {
    /// Captures live `SquareRef` handles in a `(2*radius + 1)^2` neighborhood
    /// around `center` on `cell`.
    pub fn from_cell_radius(
        cell: &IsoCell,
        meta: &'a IsoMetaGrid,
        sprites: &IsoSpriteManager,
        center: (i32, i32, i32),
        radius: i32,
    ) -> Self {
        let mut squares = HashMap::new();
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let key = (center.0 + dx, center.1 + dy, center.2);
                if let Some(sq) = cell.get_grid_square(key.0, key.1, key.2) {
                    squares.insert(key, sq);
                }
            }
        }
        Self {
            meta,
            flags: AnimalSpriteFlags::from_sprites(sprites),
            squares,
            animal_square: Some(center),
            vehicle_speed_kmh: None,
            pending_new_animals: Vec::new(),
            pending_grown: None,
            added_to_world_count: 0,
            removed_from_world_count: 0,
            recently_removed_count: 0,
            instance_manager_updates: 0,
        }
    }

    fn resolve_zone(
        &self,
        msb: i64,
        lsb: i64,
    ) -> Option<(pz_world::zones::ZoneId, &pz_world::zones::Zone)> {
        let id = zone::zone_by_uuid(self.meta, msb, lsb)?;
        let z = self.meta.zone(id);
        Some((id, z))
    }

    fn square_at(&self, square: ObjectRef) -> Option<&SquareRef> {
        let key = square_ref_xyz(square)?;
        self.squares.get(&key)
    }
}

impl AnimalWorld for EngineAnimalWorld<'_> {
    fn is_raining(&self) -> bool {
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let raining = gt.borrow().climate.get_rain_intensity() > 0.0;
        raining
    }

    fn puddles_size(&self) -> f32 {
        pz_lua_api::bind_puddles::shared_puddles()
            .borrow()
            .state
            .puddles_size
            .get()
    }

    fn animal_zone_exists(&self, msb: i64, lsb: i64) -> bool {
        zone::zone_by_uuid(self.meta, msb, lsb).is_some()
    }

    fn animal_zone_action(&self, z: (i64, i64)) -> Option<String> {
        let (_, z) = self.resolve_zone(z.0, z.1)?;
        zone::action(z).map(str::to_string)
    }

    fn animal_zone_closest_point_on_polyline(&self, z: (i64, i64), x: f32, y: f32) -> f32 {
        let Some((_, z)) = self.resolve_zone(z.0, z.1) else {
            return 0.0;
        };
        let mut out = (0.0, 0.0);
        zone::closest_point_on_polyline(z, x, y, &mut out)
    }

    fn animal_zone_polyline_length(&self, z: (i64, i64)) -> f32 {
        let Some((_, z)) = self.resolve_zone(z.0, z.1) else {
            return 1.0;
        };
        zone::polyline_length(z)
    }

    fn animal_zone_junctions_between(
        &self,
        z: (i64, i64),
        t1: f32,
        t2: f32,
    ) -> Vec<AnimalZoneJunction> {
        let Some((id, _)) = self.resolve_zone(z.0, z.1) else {
            return Vec::new();
        };
        let Ok(juncs) = zone::junctions_between(self.meta, id, t1, t2) else {
            return Vec::new();
        };
        juncs
            .into_iter()
            .filter_map(|j| {
                let other_zone = self.meta.zone(j.zone_other);
                let uuid = other_zone.id?;
                let z1 = self.meta.zone(id);
                let n1 = (z1.points.len() / 2) as i32;
                let n2 = (other_zone.points.len() / 2) as i32;
                Some(AnimalZoneJunction {
                    zone_other: (uuid.msb, uuid.lsb),
                    point_index_other: j.point_index_other,
                    first_point_on_zone1: j.point_index_self == 0,
                    last_point_on_zone1: n1 > 0 && j.point_index_self == n1 - 1,
                    first_point_on_zone2: j.point_index_other == 0,
                    last_point_on_zone2: n2 > 0 && j.point_index_other == n2 - 1,
                })
            })
            .collect()
    }

    fn animal_zone_distance_of_point_from_start(&self, z: (i64, i64), point_index: i32) -> f32 {
        let Some((_, z)) = self.resolve_zone(z.0, z.1) else {
            return 0.0;
        };
        zone::distance_of_point_from_start(z, point_index)
    }

    fn animal_zone_point_on_polyline(&self, z: (i64, i64), t: f32) -> (f32, f32) {
        let Some((_, z)) = self.resolve_zone(z.0, z.1) else {
            return (0.0, 0.0);
        };
        zone::point_on_polyline(z, t).1
    }

    fn animal_zone_direction_on_polyline(&self, z: (i64, i64), t: f32) -> (f32, f32) {
        let Some((_, z)) = self.resolve_zone(z.0, z.1) else {
            return (1.0, 0.0);
        };
        zone::direction_on_polyline(z, t).unwrap_or((1.0, 0.0))
    }

    fn square_xyz(&self, square: ObjectRef) -> (i32, i32, i32) {
        square_ref_xyz(square).unwrap_or((0, 0, 0))
    }

    fn square_is_free(&self, square: ObjectRef) -> bool {
        let Some(sq) = self.square_at(square) else {
            return false;
        };
        sq.try_borrow().is_ok_and(|s| {
            let blocked = s.has(self.flags.solid)
                || s.has(self.flags.solidtrans)
                || s.has_type(self.flags.tree);
            !blocked && s.has(self.flags.solidfloor)
        })
    }

    fn square_has_water_object(&self, square: ObjectRef) -> bool {
        let Some(sq) = self.square_at(square) else {
            return false;
        };
        sq.try_borrow().is_ok_and(|s| s.has(self.flags.water))
    }

    fn square_has_puddle_floor(&self, square: ObjectRef) -> bool {
        if self.puddles_size() <= 0.0 {
            return false;
        }
        let Some(sq) = self.square_at(square) else {
            return false;
        };
        sq.try_borrow()
            .is_ok_and(|s| s.has(self.flags.exterior) && s.has(self.flags.solidfloor))
    }

    fn square_has_grass_floor(&self, square: ObjectRef) -> bool {
        let Some(sq) = self.square_at(square) else {
            return false;
        };
        let Ok(s) = sq.try_borrow() else { return false };
        s.objects.iter().any(|slot| {
            slot.owner_slot().try_borrow().is_ok_and(|obj| {
                matches!(
                    &obj.sprite_name,
                    Some(SpriteName::Str(n)) if n.starts_with("blends_natural_01")
                )
            })
        })
    }

    fn facing_position(&self, obj: ObjectRef, _from_x: f32, _from_y: f32) -> Option<(f32, f32)> {
        let (x, y, _) = square_ref_xyz(obj)?;
        Some((x as f32 + 0.5, y as f32 + 0.5))
    }

    fn floor_facing_position(
        &self,
        square: ObjectRef,
        _from_x: f32,
        _from_y: f32,
    ) -> Option<(f32, f32)> {
        let (x, y, _) = square_ref_xyz(square)?;
        Some((x as f32 + 0.5, y as f32 + 0.5))
    }

    fn add_new_animal(&mut self, animal: NewAnimal, _into_vehicle: Option<ObjectRef>) -> ObjectRef {
        let id = animal.animal.animal_id;
        self.pending_new_animals.push(animal);
        character_ref(id)
    }

    fn replace_grown_animal(&mut self, grown: NewAnimal) -> Option<ObjectRef> {
        let id = grown.animal.animal_id;
        self.pending_grown = Some(grown);
        Some(character_ref(id))
    }

    fn vehicle_speed_kmh(&self) -> Option<f32> {
        self.vehicle_speed_kmh
    }

    fn under_open_sky(&self) -> bool {
        let Some(key) = self.animal_square else {
            return true;
        };
        let Some(sq) = self.squares.get(&key) else {
            return true;
        };
        sq.try_borrow().is_ok_and(|s| s.has(self.flags.exterior))
    }

    fn added_to_world(&mut self) {
        self.added_to_world_count = self.added_to_world_count.saturating_add(1);
    }

    fn removed_from_world(&mut self) {
        self.removed_from_world_count = self.removed_from_world_count.saturating_add(1);
    }

    fn add_to_recently_removed(&mut self) {
        self.recently_removed_count = self.recently_removed_count.saturating_add(1);
    }

    fn instance_manager_update(&mut self) {
        self.instance_manager_updates = self.instance_manager_updates.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::iso_world::StandardRand;
    use pz_characters::character::state::StateWorld;
    use pz_characters::pz_animals::{square_ref, AnimalDefinitionTables};
    use pz_kahlua::java_io::JavaUuid;
    use pz_lua_api::bind_characters::LuaItem;
    use pz_world::objects::{IsoObject, ObjectKind, SpriteName};
    use pz_world::zones::{AnimalZoneData, Zone, ZoneGeometryType, ZoneKind};
    use pz_world::{
        IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, ObjectSlot, SquareSlot,
    };
    use pz_world_states::{StatesRuntime, StatesWorldAdapter, ThumpSettings};

    #[test]
    fn engine_animal_world_queries_weather_zones_squares_and_state_world() {
        let mut sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 2);
        let mut sq = IsoGridSquare::new(10, 20, 0);
        sq.properties.set_flag(sprites.tables.flag.exterior);
        sq.properties.set_flag(sprites.tables.flag.solidfloor);
        let mut grass = IsoObject::new(1, ObjectKind::Object, None);
        grass.sprite_name = Some(SpriteName::Str("blends_natural_01_64".to_string()));
        sq.objects.push(ObjectSlot::new(grass));
        chunk.set_square(2, 4, 0, Some(SquareSlot::new(sq)));
        cell.chunks.insert((1, 2), chunk);

        let mut meta = IsoMetaGrid::with_bounds(0, 0, 0, 0);
        let uuid = JavaUuid {
            msb: 0x1122_3344_5566_7788,
            lsb: 0x99aa_bbcc_ddee_ff00_u64 as i64,
        };
        let mut z = Zone::default();
        z.id = Some(uuid);
        z.kind = ZoneKind::Animal(AnimalZoneData {
            action: Some("Follow".to_string()),
            ..Default::default()
        });
        z.geometry_type = ZoneGeometryType::Polyline;
        z.points = vec![0, 0, 10, 0];
        let zid = meta.register_zone_def(z);
        if !meta.animal_zones.contains(&zid) {
            meta.animal_zones.push(zid);
        }

        // Set live puddles size > 0 to verify puddles_size() and square_has_puddle_floor().
        pz_lua_api::bind_puddles::shared_puddles()
            .borrow_mut()
            .state
            .puddles_size
            .set(0.5);

        let defs = Rc::new(AnimalDefinitionTables::default());
        let aw = EngineAnimalWorld::from_cell_radius(&cell, &meta, &sprites, (10, 20, 0), 2);

        assert!(aw.animal_zone_exists(uuid.msb, uuid.lsb));
        assert_eq!(
            aw.animal_zone_action((uuid.msb, uuid.lsb)).as_deref(),
            Some("Follow")
        );
        assert!((aw.animal_zone_polyline_length((uuid.msb, uuid.lsb)) - 10.0).abs() < 0.01);
        let sq_ref = square_ref(10, 20, 0);
        assert!(aw.square_is_free(sq_ref));
        assert!(aw.square_has_grass_floor(sq_ref));
        assert!(aw.square_has_puddle_floor(sq_ref));
        assert!(aw.under_open_sky());
        assert_eq!(aw.facing_position(sq_ref, 0.0, 0.0), Some((10.5, 20.5)));

        // Verify StateWorld::animal_host & virtualize_animal_if_unloaded through StatesWorldAdapter.
        let mut engine = crate::world_update::EngineStates {
            geometry: None,
            sim: None,
            closing_windows: Vec::new(),
            rand: StandardRand,
            movers: Vec::new(),
            models: RefCell::new(None),
            animation_time_delta: 0.0,
            floor_world: None,
            bullet: None,
            player_firearms: Vec::new(),
            animal_defs: Some(defs),
            animal_world: Some(aw),
            virtualized_animals: Vec::new(),
        };
        let mut runtime = StatesRuntime::default();
        {
            let mut ws = StatesWorldAdapter::new(
                &mut cell,
                &meta,
                &mut sprites,
                ThumpSettings::default(),
                &mut runtime,
                &mut engine,
            );
            {
                let host = StateWorld::<LuaItem>::animal_host(&mut ws, 42)
                    .expect("animal_host installed");
                assert!(host.world.under_open_sky());
                assert!(host.world.square_has_grass_floor(sq_ref));
            }
            // Loaded square (10, 20) does NOT virtualize; unloaded square (500, 500) DOES.
            assert!(!StateWorld::<LuaItem>::virtualize_animal_if_unloaded(
                &mut ws, 42, 10, 20
            ));
            assert!(StateWorld::<LuaItem>::virtualize_animal_if_unloaded(
                &mut ws, 42, 500, 500
            ));
        }
        assert_eq!(engine.virtualized_animals, vec![42]);
    }
}
