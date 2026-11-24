//! `zombie.iso.objects.ObjectRenderEffects` (`VegetationRustle`,
//! `HitTreeShudder`, `HitDoor`) and `zombie.audio.TreeSoundManager` (10-slot
//! `"Bushes"` sound pool) engine integration.

use std::cell::RefCell;
use std::rc::Rc;

use pz_pathfind::mover::ObjectRef;
use pz_sound::emitter::ParentObject;
use pz_sound::world_emitters::EmitterRef;
use pz_world::java::GameRand;
use pz_world::IsoCell;
use pz_world_states::{NewRenderEffect, RenderEffectType};

thread_local! {
    static SHARED_RENDER_EFFECTS: RefCell<Option<Rc<RefCell<WorldRenderEffects>>>> =
        const { RefCell::new(None) };
}

/// Returns the active world's shared [`WorldRenderEffects`], if installed.
pub fn shared_render_effects() -> Option<Rc<RefCell<WorldRenderEffects>>> {
    SHARED_RENDER_EFFECTS.with(|s| s.borrow().clone())
}

/// Installs or clears the active world's shared [`WorldRenderEffects`].
pub fn set_shared_render_effects(store: Option<Rc<RefCell<WorldRenderEffects>>>) {
    SHARED_RENDER_EFFECTS.with(|s| *s.borrow_mut() = store);
}

/// `TreeSoundManager` slot count (`new Slot[10]`).
pub const TREE_SOUND_SLOTS: usize = 10;
/// `TreeSoundManager.TREE_SOUND_RADIUS^2` (`20 * 20 = 400.0`).
const TREE_SOUND_RADIUS_SQ: f32 = 400.0;
/// `TreeSoundManager` sustain timeout (`1000` ms since last touch).
const TREE_SOUND_TIMEOUT_MS: i64 = 1000;
/// `SoundKey.BUSHES.getSoundName()`.
const BUSHES_SOUND_NAME: &str = "Bushes";

const MAX_PLAYERS: usize = 4;

type Corners = [[f32; 2]; 4];

/// Closest living local player's squared distance to `(x, y, z)`
/// (`FMODParameterUtils.getClosestListenerDistanceSquared`).
fn closest_listener_distance_sq(x: f32, y: f32, z: i32) -> f32 {
    let mut best = f32::MAX;
    for slot in 0..MAX_PLAYERS {
        let Some(p) = pz_lua_api::bind_players::player(slot) else {
            continue;
        };
        let Ok(c) = p.try_borrow() else {
            continue;
        };
        if c.is_dead() {
            continue;
        }
        let dx = c.vars.x - x;
        let dy = c.vars.y - y;
        let dz = (c.vars.z - z as f32) * 3.0;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 < best {
            best = d2;
        }
    }
    best
}

/// Active dynamic `ObjectRenderEffects` instance attached to a world object.
#[derive(Clone, Debug)]
struct DynamicRenderEffect {
    pos: ObjectRef,
    kind: RenderEffectType,
    is_tree: bool,
    is_big: bool,
    elapsed: f32,
    duration: f32,
    phase: f32,
    phase_length: f32,
    finishing: bool,
    last: Corners,
    target: Corners,
    offsets: Corners,
}

impl DynamicRenderEffect {
    fn new(pos: ObjectRef, effect: NewRenderEffect) -> Self {
        Self {
            pos,
            kind: effect.kind,
            is_tree: effect.is_tree,
            is_big: effect.is_big,
            elapsed: 0.0,
            duration: effect.total_max_time,
            phase: 0.0,
            phase_length: 0.0,
            finishing: false,
            last: [[0.0; 2]; 4],
            target: [[0.0; 2]; 4],
            offsets: [[0.0; 2]; 4],
        }
    }

    /// `ObjectRenderEffects.update()`: advances `curTime`/`totalTime` by
    /// `multiplier` and updates the 4 corner offsets. Returns `false` when the
    /// effect has finished and should be removed.
    fn advance(&mut self, multiplier: f32, wind_tick_final: f64, rand: &mut dyn GameRand) -> bool {
        self.phase += multiplier;
        self.elapsed += multiplier;
        if self.phase > self.phase_length {
            if self.finishing {
                return false;
            }
            self.phase = 0.0;
            self.last = self.target;
            let progress = if self.duration > 0.0 {
                (self.elapsed / self.duration).clamp(0.0, 1.0)
            } else {
                1.0
            };
            match self.kind {
                RenderEffectType::HitTreeShudder => {
                    if self.elapsed > self.duration {
                        self.phase_length = 10.0;
                        self.target[0][0] = 0.0;
                        self.target[1][0] = 0.0;
                        self.finishing = true;
                    } else {
                        self.phase_length = 3.0 + 15.0 * progress;
                        let rem = 1.0 - progress;
                        let tx = if self.is_big {
                            rand.next_float_range(-0.01 - 0.08 * rem, 0.01 + 0.08 * rem)
                        } else {
                            rand.next_float_range(-0.02 - 0.16 * rem, 0.02 + 0.16 * rem)
                        };
                        self.target[0][0] = tx;
                        self.target[1][0] = tx;
                    }
                }
                RenderEffectType::VegetationRustle => {
                    if self.elapsed > self.duration {
                        self.phase_length = 3.0;
                        self.target[0][0] = 0.0;
                        self.target[1][0] = 0.0;
                        self.finishing = true;
                    } else {
                        self.phase_length = 2.0 + 6.0 * progress;
                        let mut x = if self.is_big {
                            rand.next_float_range(-0.00625, 0.00625)
                        } else {
                            rand.next_float_range(-0.015, 0.015)
                        };
                        let mut y = if self.is_big {
                            rand.next_float_range(-0.00625, 0.00625)
                        } else {
                            rand.next_float_range(-0.015, 0.015)
                        };
                        if wind_tick_final < 0.15 {
                            x *= 0.6;
                            y *= 0.6;
                        }
                        self.target[0] = [x, y];
                        self.target[1] = [x, y];
                    }
                }
                RenderEffectType::HitDoor => {
                    if self.elapsed > self.duration {
                        self.phase_length = 3.0;
                        self.target[0][0] = 0.0;
                        self.target[1][0] = 0.0;
                        self.finishing = true;
                    } else {
                        self.phase_length = 1.0 + 2.0 * progress;
                        self.target = [[
                            rand.next_float_range(-0.005, 0.005),
                            rand.next_float_range(-0.0075, 0.0075),
                        ]; 4];
                    }
                }
            }
        }
        if self.phase_length > 0.0 {
            let t = (self.phase / self.phase_length).clamp(0.0, 1.0);
            for i in 0..4 {
                for axis in 0..2 {
                    self.offsets[i][axis] =
                        pz_core::weather::clerp(t, self.last[i][axis], self.target[i][axis]);
                }
            }
        }
        true
    }
}

/// One `TreeSoundManager.Slot`.
struct TreeSoundSlot {
    sound_time: i64,
    square: Option<(i32, i32, i32)>,
    playing: bool,
    emitter: Option<EmitterRef>,
    instance: i64,
}

impl Default for TreeSoundSlot {
    fn default() -> Self {
        Self {
            sound_time: 0,
            square: None,
            playing: false,
            emitter: None,
            instance: 0,
        }
    }
}

impl TreeSoundSlot {
    fn play_sound(&mut self, sq: (i32, i32, i32)) {
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|we, ctx| {
            let emitter = self.emitter.get_or_insert_with(|| {
                let e = we.get_free();
                we.take_ownership(&e);
                e
            });
            let mut em = emitter.borrow_mut();
            em.set_pos(sq.0 as f32 + 0.5, sq.1 as f32 + 0.5, sq.2 as f32);
            if !em.is_playing_name(BUSHES_SOUND_NAME) {
                self.instance = em.play_sound_impl(BUSHES_SOUND_NAME, ParentObject::None, ctx);
                if self.instance != 0 {
                    em.set_parameter_value_by_name(self.instance, "Occlusion", 0.0, ctx);
                }
            }
            self.square = Some(sq);
            self.playing = true;
            em.tick(ctx);
        });
    }

    fn stop_playing(&mut self) {
        let Some(emitter) = self.emitter.as_ref() else {
            self.instance = 0;
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|_we, ctx| {
            let mut em = emitter.borrow_mut();
            if self.instance == 0 {
                if !em.is_empty() {
                    em.tick(ctx);
                }
                return;
            }
            if em.has_sustain_points(self.instance, ctx.sounds) {
                em.trigger_cue(self.instance, ctx);
            } else {
                em.stop_all(ctx);
            }
            self.instance = 0;
        });
    }
}

/// `zombie.audio.TreeSoundManager`: plays `"Bushes"` on up to 10 nearest
/// rustled tree/bush squares within 20 tiles of a living local player.
pub struct TreeSoundManager {
    squares: Vec<(i32, i32, i32)>,
    slots: [TreeSoundSlot; TREE_SOUND_SLOTS],
}

impl Default for TreeSoundManager {
    fn default() -> Self {
        Self {
            squares: Vec::new(),
            slots: std::array::from_fn(|_| TreeSoundSlot::default()),
        }
    }
}

impl TreeSoundManager {
    /// `TreeSoundManager.addSquare(square)`.
    pub fn add_square(&mut self, sq: (i32, i32, i32)) {
        if !self.squares.contains(&sq) {
            self.squares.push(sq);
        }
    }

    fn should_play(sq: (i32, i32, i32)) -> bool {
        closest_listener_distance_sq(sq.0 as f32 + 0.5, sq.1 as f32 + 0.5, sq.2)
            < TREE_SOUND_RADIUS_SQ
    }

    fn existing_slot(&self, sq: (i32, i32, i32)) -> Option<usize> {
        self.slots.iter().position(|s| s.square == Some(sq))
    }

    fn free_slot(&self, now_ms: i64) -> Option<usize> {
        if let Some(idx) = self.slots.iter().position(|s| !s.playing && s.square.is_none()) {
            return Some(idx);
        }
        let mut best_idx = None;
        let mut oldest = i64::MAX;
        for (i, slot) in self.slots.iter().enumerate() {
            if !slot.playing && now_ms.saturating_sub(slot.sound_time) >= TREE_SOUND_TIMEOUT_MS
                && slot.sound_time < oldest
            {
                oldest = slot.sound_time;
                best_idx = Some(i);
            }
        }
        best_idx
    }

    fn stop_not_playing(&mut self, now_ms: i64) {
        for slot in &mut self.slots {
            if !slot.playing {
                if slot.square.is_some()
                    && now_ms.saturating_sub(slot.sound_time) >= TREE_SOUND_TIMEOUT_MS
                {
                    slot.stop_playing();
                    slot.square = None;
                } else {
                    slot.stop_playing();
                }
            }
        }
    }

    /// `TreeSoundManager.update()`.
    pub fn update(&mut self, now_ms: i64) {
        for slot in &mut self.slots {
            slot.playing = false;
        }
        if self.squares.is_empty() {
            self.stop_not_playing(now_ms);
            return;
        }
        self.squares.sort_by(|a, b| {
            let da = closest_listener_distance_sq(a.0 as f32 + 0.5, a.1 as f32 + 0.5, a.2);
            let db = closest_listener_distance_sq(b.0 as f32 + 0.5, b.1 as f32 + 0.5, b.2);
            da.total_cmp(&db)
        });
        let count = self.squares.len().min(TREE_SOUND_SLOTS);
        // First pass: keep squares that already own a slot.
        for i in 0..count {
            let sq = self.squares[i];
            if !Self::should_play(sq) {
                continue;
            }
            if let Some(slot_idx) = self.existing_slot(sq) {
                self.slots[slot_idx].play_sound(sq);
                self.slots[slot_idx].sound_time = now_ms;
            }
        }
        // Second pass: assign free slots to remaining close squares.
        for i in 0..count {
            let sq = self.squares[i];
            if !Self::should_play(sq) || self.existing_slot(sq).is_some() {
                continue;
            }
            if let Some(slot_idx) = self.free_slot(now_ms) {
                self.slots[slot_idx].play_sound(sq);
                self.slots[slot_idx].sound_time = now_ms;
            }
        }
        self.squares.clear();
        self.stop_not_playing(now_ms);
    }

    /// Stops and returns all acquired tree sound emitters on world exit.
    pub fn reset(&mut self) {
        self.squares.clear();
        let drained: Vec<EmitterRef> = self
            .slots
            .iter_mut()
            .filter_map(|s| {
                s.square = None;
                s.playing = false;
                s.instance = 0;
                s.sound_time = 0;
                s.emitter.take()
            })
            .collect();
        if drained.is_empty() {
            return;
        }
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|we, ctx| {
            for e in drained {
                e.borrow_mut().stop_all(ctx);
                we.return_ownership(e);
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn active_squares(&self) -> Vec<(i32, i32, i32)> {
        self.slots.iter().filter_map(|s| s.square).collect()
    }
}

/// Engine-owned dynamic `ObjectRenderEffects` + [`TreeSoundManager`].
#[derive(Default)]
pub struct WorldRenderEffects {
    effects: Vec<DynamicRenderEffect>,
    pub(crate) tree_sounds: TreeSoundManager,
}

impl WorldRenderEffects {
    /// `IsoObject.getObjectRenderEffects() != null`.
    pub fn has_effect(&self, obj: ObjectRef) -> bool {
        self.effects.iter().any(|e| e.pos == obj)
    }

    /// `IsoObject.setObjectRenderEffects(ObjectRenderEffects.getNew(...))`.
    pub fn set_effect(&mut self, obj: ObjectRef, effect: NewRenderEffect) {
        if let Some(existing) = self.effects.iter_mut().find(|e| e.pos == obj) {
            existing.kind = effect.kind;
            existing.is_tree = effect.is_tree;
            existing.is_big = effect.is_big;
            existing.elapsed = 0.0;
            existing.duration = effect.total_max_time;
            existing.finishing = false;
        } else {
            self.effects.push(DynamicRenderEffect::new(obj, effect));
        }
    }

    /// `IsoTree.TreeSoundManager.addSquare(sq)`.
    pub fn add_tree_noise_square(&mut self, sq: (i32, i32, i32)) {
        self.tree_sounds.add_square(sq);
    }

    /// Advances all dynamic render effects and updates `TreeSoundManager`.
    pub fn update(
        &mut self,
        multiplier: f32,
        wind_tick_final: f64,
        now_ms: i64,
        rand: &mut dyn GameRand,
    ) {
        for i in (0..self.effects.len()).rev() {
            if !self.effects[i].advance(multiplier, wind_tick_final, rand) {
                self.effects.remove(i);
            }
        }
        self.tree_sounds.update(now_ms);
    }

    /// Live render effect corner offsets for objects currently present in `cell`.
    pub fn visible_effects(&self, cell: &IsoCell) -> Vec<(ObjectRef, Corners)> {
        let mut out = Vec::with_capacity(self.effects.len());
        for eff in &self.effects {
            let p = eff.pos;
            let exists = cell
                .get_grid_square(p.x, p.y, p.z)
                .is_some_and(|sq| sq.borrow().objects.iter().any(|o| o.borrow().id == p.id));
            if exists {
                out.push((p, eff.offsets));
            }
        }
        out
    }

    /// Clears all active effects and releases tree sound emitters.
    pub fn reset(&mut self) {
        self.effects.clear();
        self.tree_sounds.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::{IsoChunk, IsoGridSquare, IsoObject, ObjectKind};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct FixedRand;
    impl GameRand for FixedRand {
        fn next(&mut self, _max: i32) -> i32 {
            0
        }
        fn next_float(&mut self) -> f32 {
            0.75
        }
    }

    #[test]
    fn vegetation_rustle_and_tree_shudder_advance_and_expire() {
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 2);
        let mut sq = IsoGridSquare::new(10, 20, 0);
        sq.objects
            .push(IsoObject::new(7, ObjectKind::Object, None).into());
        chunk.set_square(2, 4, 0, Some(sq.into()));
        cell.chunks.insert((1, 2), chunk);

        let obj = ObjectRef {
            x: 10,
            y: 20,
            z: 0,
            id: 7,
        };
        let mut store = WorldRenderEffects::default();
        assert!(!store.has_effect(obj));

        store.set_effect(
            obj,
            NewRenderEffect {
                kind: RenderEffectType::VegetationRustle,
                total_max_time: 10.0,
                is_tree: false,
                is_big: false,
            },
        );
        assert!(store.has_effect(obj));

        let mut rng = FixedRand;
        // Frame 1 initializes the first phase target (`curTime = 0`); frame 2 lerps toward it.
        store.update(1.0, 0.05, 1000, &mut rng);
        store.update(1.0, 0.05, 1016, &mut rng);
        let vis = store.visible_effects(&cell);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].0, obj);
        // Non-zero rustle offset on top two corners.
        assert!(vis[0].1[0][0].abs() > 0.0 || vis[0].1[1][0].abs() > 0.0);

        // Advance past total_max_time + finishing phase (3.0) -> expires cleanly.
        for t in 0..30 {
            store.update(1.0, 0.05, 1000 + t * 16, &mut rng);
        }
        assert!(!store.has_effect(obj));
        assert!(store.visible_effects(&cell).is_empty());

        // HitTreeShudder lifecycle.
        store.set_effect(
            obj,
            NewRenderEffect {
                kind: RenderEffectType::HitTreeShudder,
                total_max_time: 8.0,
                is_tree: true,
                is_big: true,
            },
        );
        store.update(1.0, 0.5, 2000, &mut rng);
        assert_eq!(store.visible_effects(&cell).len(), 1);
        store.reset();
        assert!(!store.has_effect(obj));
    }

    #[test]
    fn tree_sound_manager_assigns_nearest_slots_and_times_out() {
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let player = Rc::new(RefCell::new(
            pz_lua_api::bind_character_sim::LuaGameCharacter::new(
                &sm,
                pz_characters::CharacterKind::Player,
            ),
        ));
        {
            let mut p = player.borrow_mut();
            p.vars.x = 10.5;
            p.vars.y = 20.5;
            p.vars.z = 0.0;
        }
        pz_lua_api::bind_players::set_player(0, Some(player));

        let mut mgr = TreeSoundManager::default();
        // Close square (< 20 tiles) and far square (> 20 tiles).
        mgr.add_square((12, 20, 0));
        mgr.add_square((100, 100, 0));
        mgr.update(5_000);
        assert_eq!(mgr.active_squares(), vec![(12, 20, 0)]);

        // Still held before 1000ms timeout.
        mgr.update(5_500);
        assert_eq!(mgr.active_squares(), vec![(12, 20, 0)]);

        // Released after 1000ms without new rustle touches.
        mgr.update(6_100);
        assert!(mgr.active_squares().is_empty());

        mgr.reset();
        pz_lua_api::bind_players::set_player(0, None);
    }
}
