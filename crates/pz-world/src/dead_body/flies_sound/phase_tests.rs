//! Synthetic current-world services witness the real manager's completed
//! reference-list prefixes. These are not native corpse/engine oracles.

use super::*;
use crate::square_mutation::RenderSquareHooks;
use crate::{IsoGridSquare, SquareSlot};
use std::collections::HashSet;

const A: LevelKey = (-1, -1, 0);
const B: LevelKey = (0, -1, 0);

struct Render;
impl RenderSquareHooks for Render {}

struct RetirementBoundary;
impl Drop for RetirementBoundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}

struct Sound;
impl FliesSoundHooks for Sound {
    fn sickness_from_corpses_rate(&self, _: i32) -> f32 {
        0.0
    }
    fn get_free_emitter(&mut self, _: f32, _: f32, _: f32) -> EmitterId {
        panic!("zero corpse count must not allocate an emitter")
    }
    fn play_sound_looped_impl(&mut self, _: EmitterId, _: &str) {}
    fn set_volume_all(&mut self, _: EmitterId, _: f32) {}
    fn set_pos(&mut self, _: EmitterId, _: f32, _: f32, _: f32) {}
    fn stop_all(&mut self, _: EmitterId) {}
}

struct World {
    chunks: HashSet<(i32, i32)>,
    fail_count: Option<LevelKey>,
    fail_deref: bool,
    sound: Sound,
}

impl World {
    fn new() -> Self {
        Self {
            chunks: HashSet::from([(A.0, A.1), (B.0, B.1)]),
            fail_count: None,
            fail_deref: false,
            sound: Sound,
        }
    }
}

impl FliesWorld for World {
    fn building(&self, _: &SquareRef) -> HostResult<Option<BuildingIdx>> {
        Ok(None)
    }
    fn has_chunk(&self, wx: i32, wy: i32) -> HostResult<bool> {
        Ok(self.chunks.contains(&(wx, wy)))
    }
    fn corpse_count(
        &self,
        wx: i32,
        wy: i32,
        z: i32,
        _: Option<BuildingIdx>,
        _: bool,
    ) -> HostResult<i32> {
        if self.fail_count == Some((wx, wy, z)) {
            return Err(RuntimeHostError::Borrowed("authored later chunk count"));
        }
        Ok(0)
    }
    fn corpse_squares(
        &self,
        _: i32,
        _: i32,
        _: i32,
        _: Option<BuildingIdx>,
    ) -> HostResult<Vec<SquareRef>> {
        Ok(Vec::new())
    }
    fn set_has_flies(&mut self, square: &SquareRef, flies: bool) -> HostResult<()> {
        if self.fail_deref {
            return Err(RuntimeHostError::Borrowed("authored old-level deref"));
        }
        set_has_flies(square, &mut Render, flies)
    }
    fn hooks(&mut self) -> &mut dyn FliesSoundHooks {
        &mut self.sound
    }
}

fn player(x: i32) -> FliesPlayer {
    FliesPlayer {
        index: 0,
        xi: x,
        yi: 0,
        zi: 0,
        square: Some(SquareSlot::new(IsoGridSquare::new(x, 0, 0)).ref_handle()),
        deaf: false,
    }
}

fn env() -> FliesEnv {
    FliesEnv {
        decaying_corpse_health_impact: 3,
        zombie_health_impact: false,
        food_sickness_decrease: 0.5,
        thirty_fps_multiplier: 0.0,
        num_players: 1,
    }
}

#[test]
fn later_chunk_failure_keeps_completed_refs_and_allows_native_force_update() {
    let _retirement = RetirementBoundary;
    let mut flies = FliesSound::new();
    flies.chunk_loaded(A.0, A.1);
    flies.chunk_loaded(B.0, B.1);
    flies.player_data[0].refs_prev = vec![A];
    let mut world = World::new();
    world.fail_count = Some(B);
    let player = player(0);
    let error = flies
        .update(&mut world, &[Some(player.clone())], &env())
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeHostError::Borrowed("authored later chunk count")
    ));
    assert_eq!(flies.player_data[0].refs, vec![A]);
    assert_eq!(flies.player_data[0].refs_prev, vec![A]);
    assert!(!flies.player_data[0].force_update);
    flies.corpse_added(A.0 * 8, A.1 * 8, A.2);
    assert!(
        flies.player_data[0].force_update,
        "completed refs.add prefix drives the real corpse-change path"
    );
    world.fail_count = None;
    flies.update(&mut world, &[Some(player)], &env()).unwrap();
    assert_eq!(flies.player_data[0].refs, vec![A, B]);
    assert_eq!(flies.player_data[0].refs_prev, vec![A, B]);
}

#[test]
fn failed_deref_preserves_refs_prev_until_the_native_end_phase() {
    let _retirement = RetirementBoundary;
    let mut flies = FliesSound::new();
    flies.chunk_loaded(A.0, A.1);
    flies.chunk_loaded(B.0, B.1);
    let old_square = SquareSlot::new(IsoGridSquare::new(-4, -4, 0)).ref_handle();
    old_square.borrow_mut().has_flies = true;
    let mut emitter = FadeEmitter::new();
    emitter.emitter = Some(1);
    emitter.sq = Some(old_square.clone());
    flies.emitters.push(emitter);
    *flies.slot(A, 0) = Some(0);
    let data = &mut flies.player_data[0];
    data.wx = 0;
    data.wy = 0;
    data.z = 0;
    data.force_update = false;
    data.refs = vec![A];
    data.refs_prev = vec![A];
    let mut world = World::new();
    world.fail_deref = true;
    let player = player(8);
    let error = flies
        .update(&mut world, &[Some(player.clone())], &env())
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeHostError::Borrowed("authored old-level deref")
    ));
    assert_eq!(flies.player_data[0].refs, vec![B]);
    assert_eq!(flies.player_data[0].refs_prev, vec![A]);
    assert_eq!(flies.emitters[0].target_volume, 0.0);
    assert!(flies.fade_emitters.contains(&0));
    assert!(
        old_square.borrow().has_flies,
        "failure retains the later flag-write boundary"
    );
    world.fail_deref = false;
    flies.corpse_added(B.0 * 8, B.1 * 8, B.2);
    assert!(flies.player_data[0].force_update);
    flies.update(&mut world, &[Some(player)], &env()).unwrap();
    assert!(!old_square.borrow().has_flies);
    assert_eq!(flies.player_data[0].refs_prev, vec![B]);
}


/// These services expose each selected building query as a distinct phase;
/// they do not claim genuine native building identity or a live engine adapter.
struct PrefixWorld {
    base: World,
    building_calls: std::cell::Cell<usize>,
    fail_building_at: Option<usize>,
    building_value: Option<BuildingIdx>,
}

impl PrefixWorld {
    fn new() -> Self {
        Self {
            base: World::new(),
            building_calls: std::cell::Cell::new(0),
            fail_building_at: None,
            building_value: Some(BuildingIdx(7)),
        }
    }
}

impl FliesWorld for PrefixWorld {
    fn building(&self, _: &SquareRef) -> HostResult<Option<BuildingIdx>> {
        let call = self.building_calls.get() + 1;
        self.building_calls.set(call);
        if self.fail_building_at == Some(call) {
            return Err(RuntimeHostError::Borrowed("authored player building phase"));
        }
        Ok(self.building_value)
    }
    fn has_chunk(&self, wx: i32, wy: i32) -> HostResult<bool> {
        self.base.has_chunk(wx, wy)
    }
    fn corpse_count(&self, wx: i32, wy: i32, z: i32, building: Option<BuildingIdx>, zombie: bool) -> HostResult<i32> {
        self.base.corpse_count(wx, wy, z, building, zombie)
    }
    fn corpse_squares(&self, wx: i32, wy: i32, z: i32, building: Option<BuildingIdx>) -> HostResult<Vec<SquareRef>> {
        self.base.corpse_squares(wx, wy, z, building)
    }
    fn set_has_flies(&mut self, square: &SquareRef, flies: bool) -> HostResult<()> {
        self.base.set_has_flies(square, flies)
    }
    fn hooks(&mut self) -> &mut dyn FliesSoundHooks {
        self.base.hooks()
    }
}

fn prime_prefix(flies: &mut FliesSound) {
    let d = &mut flies.player_data[0];
    d.wx = 0;
    d.wy = 0;
    d.z = 0;
    d.building = Some(BuildingIdx(7));
    d.deaf = false;
    d.force_update = false;
    d.refs = vec![A];
    d.refs_prev = vec![A];
}

#[test]
fn changed_deaf_publishes_prefix_and_resets_force_before_coordinate_failure() {
    let _retirement = RetirementBoundary;
    let mut flies = FliesSound::new();
    prime_prefix(&mut flies);
    let mut player = player(8);
    player.deaf = true;
    let square = player.square.as_ref().unwrap().clone();
    let held = square.borrow_mut();
    let mut world = PrefixWorld::new();
    let error = flies.update(&mut world, &[Some(player)], &env()).unwrap_err();
    assert!(matches!(error, RuntimeHostError::Borrowed("FliesSound player square")));
    let d = &flies.player_data[0];
    assert!(d.deaf, "changed trait is published before any square query");
    assert!(!d.force_update, "native force reset precedes coordinate access");
    assert_eq!((d.wx, d.wy, d.z), (0, 0, 0));
    assert_eq!(d.building, Some(BuildingIdx(7)));
    assert_eq!(d.refs, vec![A]);
    assert_eq!(d.refs_prev, vec![A]);
    assert_eq!(world.building_calls.get(), 0, "changed deaf skips isSameLocation, then held coordinates stop the later building read");
    drop(held);
}

#[test]
fn same_location_returns_without_reading_the_coordinate_body() {
    let _retirement = RetirementBoundary;
    let mut flies = FliesSound::new();
    prime_prefix(&mut flies);
    let player = player(0);
    let square = player.square.as_ref().unwrap().clone();
    let held = square.borrow_mut();
    let mut world = PrefixWorld::new();
    flies.update(&mut world, &[Some(player)], &env()).unwrap();
    let d = &flies.player_data[0];
    assert!(!d.force_update);
    assert_eq!((d.wx, d.wy, d.z), (0, 0, 0));
    assert_eq!(d.refs, vec![A]);
    assert_eq!(d.refs_prev, vec![A]);
    assert_eq!(world.building_calls.get(), 1, "isSameLocation queries building before player coordinates");
    drop(held);
}

#[test]
fn building_failure_keeps_coordinate_prefix_and_distinct_repeat_query() {
    let _retirement = RetirementBoundary;
    for forced in [false, true] {
        let mut flies = FliesSound::new();
        prime_prefix(&mut flies);
        flies.player_data[0].force_update = forced;
        let mut world = PrefixWorld::new();
        let expected_queries = if forced { 1 } else { 2 };
        world.fail_building_at = Some(expected_queries);
        let error = flies.update(&mut world, &[Some(player(8))], &env()).unwrap_err();
        assert!(matches!(error, RuntimeHostError::Borrowed("authored player building phase")));
        let d = &flies.player_data[0];
        assert!(!d.force_update);
        assert_eq!((d.wx, d.wy, d.z), (1, 0, 0), "coordinates publish before the actual building read");
        assert_eq!(d.building, Some(BuildingIdx(7)), "a failed later read does not publish a replacement building");
        assert_eq!(d.refs, vec![A], "refs.clear follows successful building publication");
        assert_eq!(d.refs_prev, vec![A]);
        assert_eq!(world.building_calls.get(), expected_queries);
    }
}
