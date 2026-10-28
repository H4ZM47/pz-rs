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
