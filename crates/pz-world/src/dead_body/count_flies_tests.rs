//! Counts/flies on actual retained square fields. Serialized body fixtures
//! supply only presence/count inputs; no native corpse identity is inferred.

use super::corpse_count;
use super::flies_sound::{
    self, EmitterId, FliesEnv, FliesPlayer, FliesSound, FliesSoundHooks, FliesWorld,
};
use crate::defs::BuildingIdx;
use crate::save::visual::{CorpseVisual, HumanVisual, IsoDeadBody, MovingBase};
use crate::square_mutation::RenderSquareHooks;
use crate::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, SquareRef, SquareSlot};
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use std::rc::Rc;

struct RetirementBoundary;
impl Drop for RetirementBoundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}

// This fixture starts with the installed class's initial threshold. The
// production API requires a current field service and never installs it.
fn get_corpse_count(
    cell: &IsoCell,
    meta: &IsoMetaGrid,
    wx: i32,
    wy: i32,
    z: i32,
    building: Option<BuildingIdx>,
    zombies: Option<&dyn Fn(i32, i32, i32) -> i32>,
) -> HostResult<i32> {
    corpse_count::get_corpse_count(
        cell,
        meta,
        wx,
        wy,
        z,
        building,
        &|| Ok(corpse_count::INITIAL_MAX_CORPSE_COUNT),
        zombies,
    )
}

#[derive(Default)]
struct Render {
    calls: Vec<String>,
    watched: Option<SquareRef>,
    observed_before_putfield: Vec<bool>,
}

impl RenderSquareHooks for Render {
    fn invalidate_render_chunk_level(&mut self, wx: i32, wy: i32, z: i32, flags: u64) {
        if let Some(square) = self.watched.as_ref() {
            self.observed_before_putfield
                .push(square.borrow_mut().has_flies);
        }
        self.calls.push(format!("render {wx},{wy},{z} {flags}"));
    }

    fn flies_changed(&mut self, square: SquareRef, flies: bool) {
        let sq = square.borrow_mut();
        assert_eq!(sq.has_flies, flies);
        self.calls
            .push(format!("flies {},{},{} {flies}", sq.x, sq.y, sq.z));
    }
}

#[derive(Default)]
struct Sound(Vec<String>, u64);

impl FliesSoundHooks for Sound {
    fn sickness_from_corpses_rate(&self, count: i32) -> f32 {
        if count > 2 {
            1.0
        } else {
            0.0
        }
    }
    fn get_free_emitter(&mut self, x: f32, y: f32, z: f32) -> EmitterId {
        self.1 += 1;
        self.0.push(format!("emitter{} {x},{y},{z}", self.1));
        self.1
    }
    fn play_sound_looped_impl(&mut self, emitter: EmitterId, name: &str) {
        self.0.push(format!("play{emitter} {name}"));
    }
    fn set_volume_all(&mut self, emitter: EmitterId, value: f32) {
        self.0.push(format!("vol{emitter} {value}"));
    }
    fn set_pos(&mut self, emitter: EmitterId, x: f32, y: f32, z: f32) {
        self.0.push(format!("pos{emitter} {x},{y},{z}"));
    }
    fn stop_all(&mut self, emitter: EmitterId) {
        self.0.push(format!("stop{emitter}"));
    }
}

struct World {
    cell: IsoCell,
    meta: IsoMetaGrid,
    render: Render,
    sound: Sound,
}

impl World {
    fn new() -> Self {
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(0, 0);
        for y in 0..8 {
            for x in 0..8 {
                chunk.set_square(x, y, 0, Some(IsoGridSquare::new(x, y, 0).into()));
            }
        }
        corpse_count::chunk_loaded(&mut chunk);
        cell.chunks.insert((0, 0), chunk);
        let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
        meta.create_cell(0, 0);
        Self {
            cell,
            meta,
            render: Render::default(),
            sound: Sound::default(),
        }
    }

    fn add_presence_fixture(&mut self, x: i32, y: i32) {
        self.cell
            .get_grid_square(x, y, 0)
            .unwrap()
            .borrow_mut()
            .static_moving
            .push(IsoDeadBody {
                base: MovingBase {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    x: x as f32,
                    y: y as f32,
                    z: 0.0,
                    dir: 0,
                    mod_data: None,
                },
                female: false,
                was_zombie: true,
                animal: None,
                object_id: (-1, 4),
                persistent_outfit_id: 0,
                desc: None,
                visual: CorpseVisual::Human(Box::new(HumanVisual::new(4))),
                container: None,
                worn_items: Vec::new(),
                attached_items: Vec::new(),
                death_time: 0.0,
                reanimate_time: -1.0,
                fall_on_front: false,
                killed_by_fall: false,
                was_skeleton: false,
                angle: 0.0,
                zombie_rot_stage_at_death: 1,
                animal_rot_stage_at_death: 0,
                rotten_texture: String::new(),
                skel_inv_icon: String::new(),
                crawling: false,
                fake_dead: false,
                ragdoll: None,
                burn_timer: 0.0,
            });
        corpse_count::corpse_added(&mut self.cell, &self.meta, x, y, 0).unwrap();
    }

    fn player(&mut self, x: i32, y: i32) -> FliesPlayer {
        if self.cell.get_grid_square(x, y, 0).is_none() {
            let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
            let chunk = self
                .cell
                .chunks
                .entry((wx, wy))
                .or_insert_with(|| IsoChunk::new(wx, wy));
            chunk.set_square(
                x.rem_euclid(8),
                y.rem_euclid(8),
                0,
                Some(IsoGridSquare::new(x, y, 0).into()),
            );
        }
        FliesPlayer {
            index: 0,
            xi: x,
            yi: y,
            zi: 0,
            deaf: false,
            square: Some(self.cell.get_grid_square(x, y, 0).unwrap()),
        }
    }
}

impl FliesWorld for World {
    fn building(&self, square: &SquareRef) -> HostResult<Option<BuildingIdx>> {
        Ok(square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("fixture actual building square"))?
            .room_def(&self.meta)
            .and_then(|room| room.building))
    }
    fn has_chunk(&self, wx: i32, wy: i32) -> HostResult<bool> {
        Ok(self.cell.chunks.contains_key(&(wx, wy)))
    }
    fn corpse_count(
        &self,
        wx: i32,
        wy: i32,
        z: i32,
        building: Option<BuildingIdx>,
        _zombies: bool,
    ) -> HostResult<i32> {
        get_corpse_count(&self.cell, &self.meta, wx, wy, z, building, None)
    }
    fn corpse_squares(
        &self,
        wx: i32,
        wy: i32,
        z: i32,
        building: Option<BuildingIdx>,
    ) -> HostResult<Vec<SquareRef>> {
        let Some(chunk) = self.cell.chunks.get(&(wx, wy)) else {
            return Ok(Vec::new());
        };
        let mut squares = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let Some(square) = chunk.get_grid_square(x, y, z) else {
                    continue;
                };
                let selected = {
                    let sq = square
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("fixture actual corpse square"))?;
                    !sq.static_moving.is_empty()
                        && sq.room_def(&self.meta).and_then(|room| room.building) == building
                };
                if selected {
                    squares.push(square);
                }
            }
        }
        Ok(squares)
    }
    fn set_has_flies(&mut self, square: &SquareRef, flies: bool) -> HostResult<()> {
        flies_sound::set_has_flies(square, &mut self.render, flies)
    }
    fn hooks(&mut self) -> &mut dyn FliesSoundHooks {
        &mut self.sound
    }
}

fn env() -> FliesEnv {
    FliesEnv {
        decaying_corpse_health_impact: 3,
        zombie_health_impact: false,
        food_sickness_decrease: 0.5,
        thirty_fps_multiplier: 50.0,
        num_players: 1,
    }
}

#[test]
fn counts_reset_on_activation_and_drop_removed_chunk_levels() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    world.add_presence_fixture(1, 1);
    world.add_presence_fixture(5, 2);
    world.add_presence_fixture(6, 6);
    assert_eq!(
        get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, None).unwrap(),
        3
    );
    corpse_count::corpse_removed(&mut world.cell, &world.meta, 1, 1, 0).unwrap();
    assert_eq!(
        get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, None).unwrap(),
        2
    );
    let chunk = world.cell.chunks.get_mut(&(0, 0)).unwrap();
    chunk.set_min_max_level(1, 1);
    assert!(chunk.corpse_count.as_ref().unwrap().level(0).is_none());
    chunk.set_min_max_level(0, 0);
    corpse_count::chunk_loaded(chunk);
    assert_eq!(
        get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, None).unwrap(),
        0
    );
}

#[test]
fn set_has_flies_invalidates_before_putfield_without_a_square_guard() {
    let _retirement = RetirementBoundary;
    let square = Rc::new(std::cell::RefCell::new(IsoGridSquare::new(5, 2, 0)));
    let mut render = Render {
        watched: Some(square.clone()),
        ..Default::default()
    };
    flies_sound::set_has_flies(&square, &mut render, true).unwrap();
    flies_sound::set_has_flies(&square, &mut render, true).unwrap();
    flies_sound::set_has_flies(&square, &mut render, false).unwrap();
    assert_eq!(render.observed_before_putfield, [false, true]);
    assert_eq!(
        render.calls,
        [
            "render 0,0,0 64",
            "flies 5,2,0 true",
            "render 0,0,0 128",
            "flies 5,2,0 false"
        ]
    );
    assert!(!square.borrow().has_flies);
}

#[test]
fn flies_follow_the_corpse_count_and_the_player() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    for (x, y) in [(1, 1), (5, 2), (6, 6)] {
        world.add_presence_fixture(x, y);
    }
    let mut flies = FliesSound::new();
    flies.chunk_loaded(0, 0);
    let player = world.player(4, 4);
    flies
        .update(&mut world, &[Some(player.clone())], &env())
        .unwrap();
    assert!(
        world
            .cell
            .get_grid_square(5, 2, 0)
            .unwrap()
            .borrow()
            .has_flies
    );
    assert_eq!(
        world.render.calls,
        vec!["render 0,0,0 64", "flies 5,2,0 true"]
    );
    assert_eq!(
        world.sound.0,
        vec!["emitter1 5,2,0", "play1 CorpseFlies", "vol1 0", "vol1 0.5"]
    );
    world.sound.0.clear();
    flies.update(&mut world, &[Some(player)], &env()).unwrap();
    assert!(world.sound.0.is_empty());
    world.render.calls.clear();
    let away = world.player(100, 100);
    flies
        .update(&mut world, &[Some(away.clone())], &env())
        .unwrap();
    assert!(
        !world
            .cell
            .get_grid_square(5, 2, 0)
            .unwrap()
            .borrow()
            .has_flies
    );
    assert_eq!(
        world.render.calls,
        vec!["render 0,0,0 128", "flies 5,2,0 false"]
    );
    assert_eq!(world.sound.0, vec!["vol1 0.5"]);
    flies.update(&mut world, &[Some(away)], &env()).unwrap();
    assert_eq!(world.sound.0, vec!["vol1 0.5", "stop1"]);
    assert!(flies.playing().unwrap().is_empty());
}

#[test]
fn fading_emitter_mutates_its_retained_square_after_current_slot_replacement() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    for (x, y) in [(1, 1), (5, 2), (6, 6)] {
        world.add_presence_fixture(x, y);
    }
    let mut flies = FliesSound::new();
    flies.chunk_loaded(0, 0);
    let player = world.player(4, 4);
    flies.update(&mut world, &[Some(player)], &env()).unwrap();
    let old = world.cell.get_grid_square(5, 2, 0).unwrap();
    let replacement = Rc::new(std::cell::RefCell::new(IsoGridSquare::new(5, 2, 0)));
    replacement.borrow_mut().has_flies = true;
    world.cell.chunks.get_mut(&(0, 0)).unwrap().set_square(
        5,
        2,
        0,
        Some(SquareSlot::retain(&replacement)),
    );
    let mut traced = Vec::new();
    flies.visit_square_refs(|square| traced.push(square.clone()));
    assert_eq!(traced.len(), 1);
    assert!(Rc::ptr_eq(&traced[0], &old));
    let away = world.player(100, 100);
    flies.update(&mut world, &[Some(away)], &env()).unwrap();
    assert!(
        !old.borrow().has_flies,
        "native field retained the same actual square"
    );
    assert!(
        replacement.borrow().has_flies,
        "replacement at equal coordinates is a separate owner"
    );
}

#[test]
fn held_square_read_is_a_host_failure_before_count_or_render_mutation() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    let square = world.cell.get_grid_square(1, 1, 0).unwrap();
    let held = square.borrow_mut();
    let error = corpse_count::corpse_added(&mut world.cell, &world.meta, 1, 1, 0).unwrap_err();
    assert!(matches!(
        error,
        RuntimeHostError::Borrowed("CorpseCount building square")
    ));
    let error = flies_sound::set_has_flies(&square, &mut world.render, true).unwrap_err();
    assert!(matches!(
        error,
        RuntimeHostError::Borrowed("setHasFlies square read")
    ));
    assert!(world.render.calls.is_empty());
    assert_eq!(
        get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, None).unwrap(),
        0
    );
    drop(held);
    corpse_count::corpse_added(&mut world.cell, &world.meta, 1, 1, 0).unwrap();
    flies_sound::set_has_flies(&square, &mut world.render, true).unwrap();
    assert_eq!(
        get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, None).unwrap(),
        1
    );
    assert!(square.borrow().has_flies);
}

#[test]
fn held_player_square_stops_before_sound_hooks_and_can_run_after_release() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    for (x, y) in [(1, 1), (5, 2), (6, 6)] {
        world.add_presence_fixture(x, y);
    }
    let mut flies = FliesSound::new();
    flies.chunk_loaded(0, 0);
    let player = world.player(4, 4);
    let square = player.square.as_ref().unwrap().clone();
    let held = square.borrow_mut();
    let error = flies
        .update(&mut world, &[Some(player.clone())], &env())
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeHostError::Borrowed("FliesSound player square")
    ));
    assert!(world.sound.0.is_empty());
    assert!(world.render.calls.is_empty());
    drop(held);
    flies.update(&mut world, &[Some(player)], &env()).unwrap();
    assert_eq!(
        world.sound.0,
        vec!["emitter1 5,2,0", "play1 CorpseFlies", "vol1 0", "vol1 0.5"]
    );
}

#[test]
fn count_uses_the_current_mutable_threshold_at_each_check() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    for wx in -1..=1 {
        let mut chunk = IsoChunk::new(wx, -1);
        corpse_count::chunk_loaded(&mut chunk);
        world.cell.chunks.insert((wx, -1), chunk);
        for _ in 0..20 {
            corpse_count::corpse_added(&mut world.cell, &world.meta, wx * 8, -8, 0).unwrap();
        }
    }
    assert_eq!(
        corpse_count::get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, &|| Ok(10), None)
            .unwrap(),
        20
    );
    assert_eq!(
        corpse_count::get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, &|| Ok(100), None)
            .unwrap(),
        60
    );
    let reads = std::cell::Cell::new(0);
    let current = || {
        let read = reads.get();
        reads.set(read + 1);
        Ok(if read == 0 { 100 } else { 10 })
    };
    assert_eq!(
        corpse_count::get_corpse_count(&world.cell, &world.meta, 0, 0, 0, None, &current, None)
            .unwrap(),
        40
    );
    assert_eq!(reads.get(), 2);
}

#[test]
fn scalar_counts_preserve_java_integer_wrap() {
    let mut count = corpse_count::CorpseLevelCount {
        corpse_count: i32::MAX,
        ..Default::default()
    };
    count.corpse_added(None);
    assert_eq!(count.corpse_count, i32::MIN);
    count.corpse_removed(None);
    assert_eq!(count.corpse_count, i32::MAX);
}

#[test]
fn unactivated_count_skips_the_held_square_building_read() {
    let _retirement = RetirementBoundary;
    let mut world = World::new();
    world.cell.chunks.get_mut(&(0, 0)).unwrap().corpse_count = None;
    let square = world.cell.get_grid_square(1, 1, 0).unwrap();
    let held = square.borrow_mut();
    corpse_count::corpse_added(&mut world.cell, &world.meta, 1, 1, 0).unwrap();
    corpse_count::corpse_removed(&mut world.cell, &world.meta, 1, 1, 0).unwrap();
    assert!(world.cell.chunks[&(0, 0)].corpse_count.is_none());
    drop(held);
    corpse_count::chunk_loaded(world.cell.chunks.get_mut(&(0, 0)).unwrap());
    let held = square.borrow_mut();
    let error = corpse_count::corpse_added(&mut world.cell, &world.meta, 1, 1, 0).unwrap_err();
    assert!(matches!(
        error,
        RuntimeHostError::Borrowed("CorpseCount building square")
    ));
    drop(held);
}
