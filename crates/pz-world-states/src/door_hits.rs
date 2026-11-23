//! ObjectRenderEffects.Hit_Door on a surviving barricade or a locked door.

use super::{StatesRuntime, StatesWorldAdapter};
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_pathfind::mover::ObjectRef;
use pz_world::{java::GameRand, IsoCell, IsoObject};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

type Corners = [[f32; 2]; 4];

#[derive(Clone, Debug)]
pub(crate) struct DoorHit {
    object: Weak<RefCell<IsoObject>>,
    pos: ObjectRef,
    elapsed: f32,
    duration: f32,
    phase: f32,
    phase_length: f32,
    finishing: bool,
    last: Corners,
    target: Corners,
    offsets: Corners,
}

impl DoorHit {
    fn advance(&mut self, multiplier: f32, rand: &mut dyn GameRand) -> bool {
        self.phase += multiplier;
        self.elapsed += multiplier;
        if self.phase > self.phase_length {
            if self.finishing {
                return false;
            }
            self.phase = 0.0;
            self.last = self.target;
            if self.elapsed > self.duration {
                self.phase_length = 3.0;
                self.target[0][0] = 0.0;
                self.target[1][0] = 0.0;
                self.finishing = true;
            } else {
                self.phase_length = 1.0 + 2.0 * (self.elapsed / self.duration).clamp(0.0, 1.0);
                self.target = [[
                    rand.next_float_range(-0.005, 0.005),
                    rand.next_float_range(-0.0075, 0.0075),
                ]; 4];
            }
        }
        if self.phase_length > 0.0 {
            for i in 0..4 {
                for axis in 0..2 {
                    self.offsets[i][axis] = pz_core::weather::clerp(
                        self.phase / self.phase_length,
                        self.last[i][axis],
                        self.target[i][axis],
                    );
                }
            }
        }
        true
    }
}

impl StatesWorldAdapter<'_> {
    pub(crate) fn hit_door_effect(&mut self, pos: ObjectRef) {
        let Some(object) = self.object_ref(pos) else {
            return;
        };
        let weak = Rc::downgrade(&object);
        let effects = &mut self.runtime.door_hits;
        let existing = effects.iter().position(|hit| hit.object.ptr_eq(&weak));
        if !self.engine.door_sprite_effects_enabled() {
            if let Some(index) = existing {
                effects.remove(index);
            }
            return;
        }
        let duration = self.engine.rand().next_float_range(15.0, 30.0);
        if let Some(index) = existing {
            // getNew(..., true) keeps the current phase and target on a repeated hit.
            let hit = &mut effects[index];
            hit.elapsed = 0.0;
            hit.duration = duration;
            hit.finishing = false;
        } else {
            effects.push(DoorHit {
                object: weak,
                pos,
                elapsed: 0.0,
                duration,
                phase: 0.0,
                phase_length: 0.0,
                finishing: false,
                last: [[0.0; 2]; 4],
                target: [[0.0; 2]; 4],
                offsets: [[0.0; 2]; 4],
            });
        }
    }
}

impl StatesRuntime {
    pub fn update_door_hits(&mut self, multiplier: f32, rand: &mut dyn GameRand) {
        for index in (0..self.door_hits.len()).rev() {
            if !self.door_hits[index].advance(multiplier, rand) {
                self.door_hits.remove(index);
            }
        }
    }

    /// The renderer must not transfer an old effect to a replacement that
    /// reused the same object ID in a reloaded chunk.
    pub fn visible_door_hits(&self, cell: &IsoCell) -> HostResult<Vec<(ObjectRef, Corners)>> {
        let mut current = Vec::with_capacity(self.door_hits.len());
        for hit in &self.door_hits {
            let p = hit.pos;
            let Some(owner) = hit.object.upgrade() else {
                continue;
            };
            let Some(square) = cell.get_grid_square(p.x, p.y, p.z) else {
                continue;
            };
            let square = square
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("door render effect square"))?;
            if square
                .objects
                .iter()
                .any(|s| Rc::ptr_eq(&s.ref_handle(), &owner))
            {
                current.push((p, hit.offsets));
            }
        }
        Ok(current)
    }
}
