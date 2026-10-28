//! `VirtualAnimal` (a herd moving along the animal zones while no chunk
//! around it is loaded) and `VirtualAnimalState`.

use pz_world::save::io::{Reader, SaveResult, Writer};
use pz_world::zones::ZoneId;
use pz_world::IsoMetaGrid;

use super::zone::{self, Junction};
use super::{AnimalPopulation, PopCx, PopulationResult};
use crate::java::JResult;

/// `VirtualAnimalState`'s subclasses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualAnimalState {
    Sleep,
    MoveFromEat,
    MoveToEat,
    MoveFromSleep,
    MoveToSleep,
    Follow,
    Eat,
}

impl VirtualAnimalState {
    /// `getClass().getSimpleName()`.
    pub fn simple_name(self) -> &'static str {
        match self {
            VirtualAnimalState::Sleep => "StateSleep",
            VirtualAnimalState::MoveFromEat => "StateMoveFromEat",
            VirtualAnimalState::MoveToEat => "StateMoveToEat",
            VirtualAnimalState::MoveFromSleep => "StateMoveFromSleep",
            VirtualAnimalState::MoveToSleep => "StateMoveToSleep",
            VirtualAnimalState::Follow => "StateFollow",
            VirtualAnimalState::Eat => "StateEat",
        }
    }
}

/// `VirtualAnimal`. `animals` are the herd's `IsoAnimal`s, which stay
/// characters while virtual (`A` is the host's animal).
#[derive(Clone, Debug)]
pub struct VirtualAnimal<A> {
    /// The object's identity (the original compares virtual animals by
    /// reference).
    pub uid: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub forward_direction: (f32, f32),
    pub animals: Vec<A>,
    pub state: Option<VirtualAnimalState>,
    pub move_forward_on_zone: bool,
    pub current_zone_action: Option<String>,
    pub next_rest_time: f64,
    pub next_eat_time: f64,
    pub id: f64,
    pub migration_group: Option<String>,
    pub speed: f32,
    pub time_to_eat: i32,
    pub time_to_sleep: i32,
    pub track_chance: i32,
    pub poop_chance: i32,
    pub broken_twigs_chance: i32,
    pub herb_graze_chance: i32,
    pub fur_chance: i32,
    pub flat_herb_chance: i32,
    pub wake_time: f64,
    pub eat_start_time: f64,
    pub sleep_period_start: Vec<i32>,
    pub sleep_period_end: Vec<i32>,
    pub eat_period_start: Vec<i32>,
    pub eat_period_end: Vec<i32>,
    pub debug_force_sleep: bool,
    pub debug_force_eat: bool,
    pub zone: Option<ZoneId>,
    pub removed: bool,
}

impl<A> VirtualAnimal<A> {
    /// `new VirtualAnimal()`: `moveForwardOnZone = Rand.NextBool(2)`,
    /// `id = Rand.Next(9999999) + 100000`.
    pub fn new(uid: u64, rand: &mut dyn pz_world::GameRand) -> Self {
        let move_forward_on_zone = next_bool(rand, 2);
        let id = rand.next(9_999_999) as f64 + 100_000.0;
        VirtualAnimal {
            uid,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            forward_direction: (0.0, 0.0),
            animals: Vec::new(),
            state: None,
            move_forward_on_zone,
            current_zone_action: Some("Follow".into()),
            next_rest_time: -1.0,
            next_eat_time: -1.0,
            id,
            migration_group: None,
            speed: 1.0,
            time_to_eat: 0,
            time_to_sleep: 0,
            track_chance: 200,
            poop_chance: 200,
            broken_twigs_chance: 200,
            herb_graze_chance: 200,
            fur_chance: 200,
            flat_herb_chance: 200,
            wake_time: 0.0,
            eat_start_time: 0.0,
            sleep_period_start: Vec::new(),
            sleep_period_end: Vec::new(),
            eat_period_start: Vec::new(),
            eat_period_end: Vec::new(),
            debug_force_sleep: false,
            debug_force_eat: false,
            zone: None,
            removed: false,
        }
    }

    /// `setZ(z)`: clamped to the world's levels.
    pub fn set_z(&mut self, z: f32) {
        self.z = z.max(-32.0).min(31.0);
    }

    fn in_period(starts: &[i32], ends: &[i32], hour: i32) -> bool {
        for (&start, &end) in starts.iter().zip(ends) {
            if start >= end {
                continue;
            }
            if hour >= start && hour < end {
                return true;
            }
            if hour < end || hour >= start {
                continue;
            }
            return true;
        }
        false
    }

    fn has_periods(starts: &[i32], ends: &[i32]) -> bool {
        !starts.is_empty() && !ends.is_empty() && starts.len() == ends.len()
    }

    /// `isTimeToSleep()`.
    pub fn is_time_to_sleep(&self, hour: i32, world_age_hours: f64) -> bool {
        if self.debug_force_sleep {
            return true;
        }
        if Self::has_periods(&self.sleep_period_start, &self.sleep_period_end) {
            Self::in_period(&self.sleep_period_start, &self.sleep_period_end, hour)
        } else {
            world_age_hours > self.next_rest_time
        }
    }

    /// `isTimeToEat()`.
    pub fn is_time_to_eat(&self, hour: i32, world_age_hours: f64) -> bool {
        if self.debug_force_eat {
            return true;
        }
        if Self::has_periods(&self.eat_period_start, &self.eat_period_end) {
            Self::in_period(&self.eat_period_start, &self.eat_period_end, hour)
        } else {
            world_age_hours > self.next_eat_time
        }
    }

    /// `isEating()` / `isSleeping()`.
    pub fn is_eating(&self) -> bool {
        self.state == Some(VirtualAnimalState::Eat)
    }
    pub fn is_sleeping(&self) -> bool {
        self.state == Some(VirtualAnimalState::Sleep)
    }

    /// `forceRest`, `forceEat`, `forceWakeUp`, `forceStopEat`.
    pub fn force_rest(&mut self) {
        self.debug_force_sleep = true;
    }
    pub fn force_eat(&mut self) {
        self.debug_force_eat = true;
    }
    pub fn force_wake_up(&mut self) {
        self.wake_time = 0.0;
    }
    pub fn force_stop_eat(&mut self) {
        self.eat_start_time = -100_000.0;
    }

    /// `MigrationGroupDefinitions.initValueFromDef(this)`.
    pub fn init_value_from_def(&mut self, defs: &crate::AnimalDefinitionTables) -> JResult<()> {
        let Some(def) = self.migration_group.as_deref().and_then(|g| defs.migration.get(g)) else {
            return Ok(());
        };
        let v = def.virtual_animal_values()?;
        self.speed = v.speed;
        self.time_to_eat = v.time_to_eat;
        self.time_to_sleep = v.time_to_sleep;
        self.track_chance = v.track_chance;
        self.poop_chance = v.poop_chance;
        self.broken_twigs_chance = v.broken_twigs_chance;
        self.herb_graze_chance = v.herb_graze_chance;
        self.fur_chance = v.fur_chance;
        self.flat_herb_chance = v.flat_herb_chance;
        self.sleep_period_start.extend(v.sleep_period_start);
        self.sleep_period_end.extend(v.sleep_period_end);
        self.eat_period_start.extend(v.eat_period_start);
        self.eat_period_end.extend(v.eat_period_end);
        Ok(())
    }

    /// `save(output)`.
    pub fn save(
        &self,
        meta: &IsoMetaGrid,
        w: &mut Writer,
        save_animal: &mut dyn FnMut(&A, &mut Writer) -> SaveResult<()>,
    ) -> SaveResult<()> {
        w.f64(self.id);
        // A zone without an id yet (ids are drawn when `map_animals.bin` is
        // first saved) saves as no zone.
        let uuid = self.zone.and_then(|z| meta.zone(z).id);
        w.i64(uuid.map_or(0, |u| u.msb));
        w.i64(uuid.map_or(0, |u| u.lsb));
        w.f32(self.x);
        w.f32(self.y);
        w.f32(self.z);
        w.f32(self.forward_direction.0);
        w.f32(self.forward_direction.1);
        w.opt_string(self.migration_group.as_deref());
        w.f64(self.next_eat_time);
        w.f64(self.next_rest_time);
        w.i16(self.animals.len() as i16);
        for a in &self.animals {
            save_animal(a, w)?;
        }
        Ok(())
    }

    /// `load(input, worldVersion)` into a new virtual animal.
    pub fn load(
        &mut self,
        meta: &IsoMetaGrid,
        r: &mut Reader<'_>,
        world_version: i32,
        load_animal: &mut dyn FnMut(&mut Reader<'_>, i32) -> SaveResult<A>,
    ) -> SaveResult<()> {
        self.id = r.f64()?;
        let (msb, lsb) = (r.i64()?, r.i64()?);
        self.zone = if msb == 0 && lsb == 0 { None } else { zone::zone_by_uuid(meta, msb, lsb) };
        self.x = r.f32()?;
        self.y = r.f32()?;
        self.z = r.f32()?;
        self.forward_direction = (r.f32()?, r.f32()?);
        let group = r.string()?;
        self.migration_group = Some(group);
        self.next_eat_time = r.f64()?;
        self.next_rest_time = r.f64()?;
        let count = r.i16()?;
        for _ in 0..count {
            let a = load_animal(r, world_version)?;
            self.animals.push(a);
        }
        Ok(())
    }
}

impl<A: super::HerdAnimal> AnimalPopulation<A> {
    /// `VirtualAnimal.update()` for the virtual animal `uid` (taken out of
    /// the population while it runs).
    pub(super) fn update_virtual_animal(
        &mut self,
        va: &mut VirtualAnimal<A>,
        cx: &mut PopCx<'_, A>,
    ) -> PopulationResult<()> {
        let Some(group) = va.migration_group.as_deref() else {
            return Ok(());
        };
        if !cx.defs.migration.contains_key(group) {
            return Ok(());
        }
        if va.state.is_none() {
            va.state = Some(VirtualAnimalState::Follow);
            if let Some(z) = self.closest_zone(&*cx.meta, va.x, va.y, None) {
                va.current_zone_action = zone::action(cx.meta.zone(z)).map(str::to_owned);
            }
        }
        let state = va.state.unwrap();
        self.update_state(state, va, cx)
    }

    fn random_track_type(
        &self,
        va: &VirtualAnimal<A>,
        action: &str,
        cx: &mut PopCx<'_, A>,
    ) -> JResult<Option<crate::AnimalTracksType>> {
        let chance = cx.host.animal_track_chance();
        if chance == 1 {
            return Ok(None);
        }
        let Some(def) = va.migration_group.as_deref().and_then(|g| cx.defs.tracks.get(g)) else {
            return Ok(None);
        };
        Ok(def.random_track(action, chance, cx.host.rand())?.cloned())
    }

    fn update_state(
        &mut self,
        state: VirtualAnimalState,
        va: &mut VirtualAnimal<A>,
        cx: &mut PopCx<'_, A>,
    ) -> PopulationResult<()> {
        use VirtualAnimalState::*;
        match state {
            Sleep => {
                let track = self.random_track_type(va, "sleep", cx)?;
                self.add_animal_tracks(va, track.as_ref(), cx)?;
                if cx.host.world_age_hours() > va.wake_time {
                    va.debug_force_sleep = false;
                    va.state = Some(MoveFromSleep);
                    va.next_rest_time = -1.0;
                }
            }
            MoveFromEat | MoveToEat | MoveFromSleep | MoveToSleep => {
                let distance = cx.host.thirty_fps_multiplier() * 0.2;
                let action = va.current_zone_action.clone();
                self.move_along_path(state, va, action.as_deref(), distance, cx)?;
            }
            Follow => {
                let distance = cx.host.multiplier() / 2.0 * 0.2;
                let action = va.current_zone_action.clone();
                self.move_along_path(state, va, action.as_deref(), distance, cx)?;
                if va.removed {
                    return Ok(());
                }
                if va.next_eat_time < 0.0 {
                    va.next_eat_time = self.next_eat_time(va, cx);
                }
                let (hour, age) = (cx.host.hour(), cx.host.world_age_hours());
                if va.current_zone_action.as_deref() == Some("Eat") && va.is_time_to_eat(hour, age) {
                    va.state = Some(MoveToEat);
                    return Ok(());
                }
                if va.next_rest_time < 0.0 {
                    va.next_rest_time = self.next_sleep_time(va, cx);
                }
                if va.current_zone_action.as_deref() == Some("Sleep") && va.is_time_to_sleep(hour, age) {
                    va.state = Some(MoveToSleep);
                    return Ok(());
                }
                let track = self.random_track_type(va, "walk", cx)?;
                self.add_animal_tracks(va, track.as_ref(), cx)?;
            }
            Eat => {
                let world_age_hours = cx.host.world_age_hours();
                let track = self.random_track_type(va, "eat", cx)?;
                self.add_animal_tracks(va, track.as_ref(), cx)?;
                if world_age_hours - va.eat_start_time > (va.time_to_eat as f32 / 60.0) as f64 {
                    va.state = Some(MoveFromEat);
                    va.next_eat_time = -1.0;
                    va.debug_force_eat = false;
                }
            }
        }
        Ok(())
    }

    /// `MigrationGroupDefinitions.getNextEatTime(group)` (0 without a
    /// definition).
    fn next_eat_time(&self, va: &VirtualAnimal<A>, cx: &mut PopCx<'_, A>) -> f64 {
        let age = cx.host.world_age_hours();
        match va.migration_group.as_deref().and_then(|g| cx.defs.migration.get(g)) {
            Some(def) => def.next_eat_time(age, cx.host.rand()),
            None => 0.0,
        }
    }

    fn next_sleep_time(&self, va: &VirtualAnimal<A>, cx: &mut PopCx<'_, A>) -> f64 {
        let age = cx.host.world_age_hours();
        match va.migration_group.as_deref().and_then(|g| cx.defs.migration.get(g)) {
            Some(def) => def.next_sleep_time(age, cx.host.rand()),
            None => 0.0,
        }
    }

    /// `new StateSleep(animal)` / `new StateEat(animal)` set their times.
    fn enter(&self, va: &mut VirtualAnimal<A>, state: VirtualAnimalState, cx: &mut PopCx<'_, A>) {
        match state {
            VirtualAnimalState::Sleep => {
                va.wake_time = cx.host.world_age_hours() + (va.time_to_sleep as f32 / 60.0) as f64;
            }
            VirtualAnimalState::Eat => va.eat_start_time = cx.host.world_age_hours(),
            _ => {}
        }
        va.state = Some(state);
    }

    /// `VirtualAnimalState.addAnimalTracks(trackType)`.
    fn add_animal_tracks(
        &mut self,
        va: &VirtualAnimal<A>,
        track: Option<&crate::AnimalTracksType>,
        cx: &mut PopCx<'_, A>,
    ) -> JResult<()> {
        let Some(track) = track else {
            return Ok(());
        };
        let Some(key) = self.get_or_create_animal_chunk(va.x, va.y, cx) else {
            return Ok(());
        };
        let group = va.migration_group.as_deref().unwrap_or("");
        let tt = track.type_.as_deref().unwrap_or("");
        let chunk = self.chunk(key);
        if chunk.animal_tracks.iter().any(|t| {
            crate::java::eq_ignore_case(&t.animal_type, group) && crate::java::eq_ignore_case(&t.track_type, tt)
        }) {
            return Ok(());
        }
        self.chunk_add_tracks(key, va, track, cx);
        Ok(())
    }

    /// `visitJunction(zone, bEndPoint, junctions)` of `state`.
    fn visit_junction(
        &self,
        state: VirtualAnimalState,
        va: &mut VirtualAnimal<A>,
        zone: ZoneId,
        end_point: bool,
        junctions: &[Junction],
        cx: &mut PopCx<'_, A>,
    ) -> Option<Junction> {
        use VirtualAnimalState::*;
        let action_of = |j: &Junction, meta: &IsoMetaGrid| zone::action(meta.zone(j.zone_other)).map(str::to_owned);
        match state {
            MoveFromEat | MoveFromSleep => {
                let possible: Vec<Junction> = junctions
                    .iter()
                    .filter(|j| action_of(j, cx.meta).as_deref() == Some("Follow"))
                    .copied()
                    .collect();
                let j = pick_random(&possible, cx.host.rand())?;
                va.state = Some(Follow);
                Some(j)
            }
            Follow => {
                let (hour, age) = (cx.host.hour(), cx.host.world_age_hours());
                let mut possible = Vec::new();
                for j in junctions {
                    let action = action_of(j, cx.meta);
                    match action.as_deref() {
                        Some("Eat") if va.is_time_to_eat(hour, age) => return Some(*j),
                        Some("Sleep") if va.is_time_to_sleep(hour, age) => return Some(*j),
                        Some("Follow") => possible.push(*j),
                        _ => {}
                    }
                }
                if zone::action(cx.meta.zone(zone)) == Some("Follow")
                    && !end_point
                    && next_bool(cx.host.rand(), possible.len() as i32 + 1)
                {
                    return None;
                }
                pick_random(&possible, cx.host.rand())
            }
            _ => None,
        }
    }

    /// `reachedEnd()` of `state`.
    fn reached_end(&self, state: VirtualAnimalState, va: &mut VirtualAnimal<A>, cx: &mut PopCx<'_, A>) {
        match state {
            VirtualAnimalState::MoveToEat => self.enter(va, VirtualAnimalState::Eat, cx),
            VirtualAnimalState::MoveToSleep => self.enter(va, VirtualAnimalState::Sleep, cx),
            _ => {}
        }
    }

    /// `moveAlongPath(action, distance)`.
    fn move_along_path(
        &mut self,
        state: VirtualAnimalState,
        va: &mut VirtualAnimal<A>,
        action: Option<&str>,
        distance: f32,
        cx: &mut PopCx<'_, A>,
    ) -> PopulationResult<()> {
        let Some(closest) = self.closest_zone(&*cx.meta, va.x, va.y, action) else {
            for a in &mut va.animals {
                a.set_animal_zone(None);
            }
            return Ok(());
        };
        // distanceBoost is 1.
        self.move_along_zone(state, va, distance * 1.0 * va.speed, closest, cx)
    }

    fn move_along_zone(
        &mut self,
        state: VirtualAnimalState,
        va: &mut VirtualAnimal<A>,
        distance: f32,
        mut zone: ZoneId,
        cx: &mut PopCx<'_, A>,
    ) -> PopulationResult<()> {
        let mut pos = (0.0, 0.0);
        let t = zone::closest_point_on_polyline(cx.meta.zone(zone), va.x, va.y, &mut pos);
        let zone_length = zone::polyline_length(cx.meta.zone(zone));
        let move_forward_on_zone = va.move_forward_on_zone;
        let mut t2 = if move_forward_on_zone { t + distance / zone_length } else { t - distance / zone_length };
        let junctions = zone::junctions_between(cx.meta, zone, t, t2)?;
        if let Some(first) = junctions.first() {
            let end_point = first.first_point_on_zone1 || first.last_point_on_zone1;
            if let Some(j) = self.visit_junction(state, va, zone, end_point, &junctions, cx) {
                zone = j.zone_other;
                let z = cx.meta.zone(zone);
                let len = zone::polyline_length(z);
                t2 = zone::distance_of_point_from_start(z, j.point_index_other) / len;
                va.move_forward_on_zone =
                    j.first_point_on_zone2 || !j.last_point_on_zone2 && next_bool(cx.host.rand(), 2);
                let len = zone::polyline_length(cx.meta.zone(zone));
                t2 = if va.move_forward_on_zone { t2 + 1.0 / len } else { t2 - 1.0 / len };
                va.current_zone_action = zone::action(cx.meta.zone(zone)).map(str::to_owned);
            }
        }
        if t2 >= 1.0 && move_forward_on_zone {
            t2 = 1.0;
            va.move_forward_on_zone = false;
            self.reached_end(state, va, cx);
        }
        if t2 <= 0.0 && !move_forward_on_zone {
            t2 = 0.0;
            va.move_forward_on_zone = true;
            self.reached_end(state, va, cx);
        }
        let (_, pos) = zone::point_on_polyline(cx.meta.zone(zone), t2);
        if let Some(d) = zone::direction_on_polyline(cx.meta.zone(zone), t2) {
            va.forward_direction = d;
        }
        if !va.move_forward_on_zone {
            va.forward_direction = (va.forward_direction.0 * -1.0, va.forward_direction.1 * -1.0);
        }
        let forward = va.move_forward_on_zone;
        for a in &mut va.animals {
            a.set_animal_zone(Some(zone));
            a.set_move_forward_on_zone(forward);
        }
        self.move_animal(va, pos.0, pos.1, cx);
        Ok(())
    }
}

/// `Rand.NextBool(n)`.
pub(super) fn next_bool(rand: &mut dyn pz_world::GameRand, n: i32) -> bool {
    rand.next(n) == 0
}

/// `PZArrayUtil.pickRandom(list)`.
pub(super) fn pick_random<T: Copy>(list: &[T], rand: &mut dyn pz_world::GameRand) -> Option<T> {
    if list.is_empty() {
        return None;
    }
    Some(list[rand.next(list.len() as i32) as usize])
}
