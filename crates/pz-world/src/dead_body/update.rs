//! `IsoDeadBody.updateBodies()`: corpse decomposition, skeleton transition,
//! burn removal, `HoursForCorpseRemoval` cleanup, and timed reanimation.

use crate::save::visual::DeadBodyRef;
use crate::IsoCell;

/// Inputs to [`update_bodies`] from the live world clock and sandbox.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeadBodyUpdateConfig {
    /// `GameTime.getInstance().getWorldAgeHours()`.
    pub world_age_hours: f32,
    /// Sandbox `HoursForCorpseRemoval` (hours until a corpse is removed;
    /// `<= 0.0` disables time-based decay removal).
    pub hours_for_corpse_removal: f32,
}

impl Default for DeadBodyUpdateConfig {
    fn default() -> Self {
        Self {
            world_age_hours: 0.0,
            hours_for_corpse_removal: 216.0,
        }
    }
}

/// Burn duration (seconds) after which a burning corpse is destroyed
/// (`IsoDeadBody.burnTimer` threshold).
pub const BURN_REMOVAL_SECONDS: f32 = 40.0;

/// What one [`update_bodies`] tick changed across the loaded cell.
#[derive(Debug, Default)]
pub struct DeadBodyUpdateReport {
    /// Squares from which a decomposed or burnt corpse was removed.
    pub removed: Vec<(i32, i32, i32)>,
    /// Corpses whose `reanimate_time` elapsed and were removed from their
    /// square so the engine can spawn a live reanimated zombie.
    pub reanimated: Vec<DeadBodyRef>,
    /// Unique chunk coordinates whose corpse visuals or list changed.
    pub dirty_chunks: Vec<(i32, i32)>,
}

/// Runs one `IsoDeadBody.updateBodies()` pass over all loaded squares in `cell`.
pub fn update_bodies(cell: &mut IsoCell, cfg: DeadBodyUpdateConfig) -> DeadBodyUpdateReport {
    let mut report = DeadBodyUpdateReport::default();
    let removal_hours = cfg.hours_for_corpse_removal;

    for (&(wx, wy), chunk) in &cell.chunks {
        let mut chunk_dirty = false;
        for sq_ref in chunk.squares() {
            let has_bodies = sq_ref
                .try_borrow()
                .is_ok_and(|sq| !sq.static_moving.is_empty());
            if !has_bodies {
                continue;
            }
            let Ok(mut sq) = sq_ref.try_borrow_mut() else {
                continue;
            };
            let (sx, sy, sz) = (sq.x, sq.y, sq.z);
            let mut i = 0;
            while i < sq.static_moving.len() {
                let body_ref = sq.static_moving[i].clone();
                enum Action {
                    Keep,
                    Remove,
                    Reanimate,
                }
                let action = match body_ref.try_borrow_mut() {
                    Ok(mut body) => {
                        if body.burn_timer >= BURN_REMOVAL_SECONDS {
                            Action::Remove
                        } else if body.reanimate_time > 0.0
                            && cfg.world_age_hours >= body.reanimate_time
                            && body.animal.is_none()
                        {
                            Action::Reanimate
                        } else if removal_hours > 0.0 {
                            let age = (cfg.world_age_hours - body.death_time).max(0.0);
                            if age >= removal_hours {
                                Action::Remove
                            } else if age >= removal_hours * 0.66 {
                                if !body.was_skeleton || body.zombie_rot_stage_at_death < 3 {
                                    body.was_skeleton = true;
                                    body.zombie_rot_stage_at_death =
                                        body.zombie_rot_stage_at_death.max(3);
                                    chunk_dirty = true;
                                }
                                Action::Keep
                            } else if age >= removal_hours * 0.33 {
                                if body.zombie_rot_stage_at_death < 2 {
                                    body.zombie_rot_stage_at_death = 2;
                                    chunk_dirty = true;
                                }
                                Action::Keep
                            } else {
                                Action::Keep
                            }
                        } else {
                            Action::Keep
                        }
                    }
                    Err(_) => Action::Keep,
                };
                match action {
                    Action::Keep => i += 1,
                    Action::Remove => {
                        sq.static_moving.remove(i);
                        report.removed.push((sx, sy, sz));
                        chunk_dirty = true;
                    }
                    Action::Reanimate => {
                        sq.static_moving.remove(i);
                        report.reanimated.push(body_ref);
                        chunk_dirty = true;
                    }
                }
            }
        }
        if chunk_dirty {
            report.dirty_chunks.push((wx, wy));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::visual::IsoDeadBody;
    use crate::{IsoChunk, IsoGridSquare};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn cell_with_body(body: IsoDeadBody) -> (IsoCell, DeadBodyRef) {
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(0, 0);
        let mut sq = IsoGridSquare::new(3, 4, 0);
        let body_ref = Rc::new(RefCell::new(body));
        sq.static_moving.push(body_ref.clone());
        chunk.set_square(3, 4, 0, Some(sq.into()));
        cell.chunks.insert((0, 0), chunk);
        (cell, body_ref)
    }

    #[test]
    fn corpse_progresses_rot_then_skeleton_then_removes_at_hours_for_corpse_removal() {
        let mut body = IsoDeadBody::for_tests(3.5, 4.5, 0.0);
        body.death_time = 10.0;
        body.reanimate_time = -1.0;
        let (mut cell, body_ref) = cell_with_body(body);

        // Fresh (< 33% of 100h): stays stage 0, not skeleton.
        let r0 = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 30.0,
                hours_for_corpse_removal: 100.0,
            },
        );
        assert!(r0.removed.is_empty() && r0.dirty_chunks.is_empty());
        assert_eq!(body_ref.borrow().zombie_rot_stage_at_death, 0);
        assert!(!body_ref.borrow().was_skeleton);

        // Rotten (>= 33% of 100h): advances to rot stage 2.
        let r1 = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 50.0,
                hours_for_corpse_removal: 100.0,
            },
        );
        assert!(r1.removed.is_empty());
        assert_eq!(r1.dirty_chunks, vec![(0, 0)]);
        assert_eq!(body_ref.borrow().zombie_rot_stage_at_death, 2);
        assert!(!body_ref.borrow().was_skeleton);

        // Skeleton (>= 66% of 100h): transitions to skeleton stage 3.
        let r2 = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 80.0,
                hours_for_corpse_removal: 100.0,
            },
        );
        assert!(r2.removed.is_empty());
        assert_eq!(r2.dirty_chunks, vec![(0, 0)]);
        assert!(body_ref.borrow().was_skeleton);
        assert_eq!(body_ref.borrow().zombie_rot_stage_at_death, 3);

        // Decomposed (>= 100% of 100h): removed from square.
        let r3 = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 110.0,
                hours_for_corpse_removal: 100.0,
            },
        );
        assert_eq!(r3.removed, vec![(3, 4, 0)]);
        assert_eq!(r3.dirty_chunks, vec![(0, 0)]);
        assert!(cell
            .get_grid_square(3, 4, 0)
            .unwrap()
            .borrow()
            .static_moving
            .is_empty());
    }

    #[test]
    fn reanimate_time_removes_corpse_and_reports_reanimation() {
        let mut body = IsoDeadBody::for_tests(3.5, 4.5, 0.0);
        body.death_time = 5.0;
        body.reanimate_time = 6.5;
        body.was_zombie = false;
        body.crawling = true;
        let (mut cell, body_ref) = cell_with_body(body);

        let before = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 6.0,
                hours_for_corpse_removal: 216.0,
            },
        );
        assert!(before.reanimated.is_empty());

        let due = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 6.5,
                hours_for_corpse_removal: 216.0,
            },
        );
        assert_eq!(due.reanimated.len(), 1);
        assert!(Rc::ptr_eq(&due.reanimated[0], &body_ref));
        assert!(cell
            .get_grid_square(3, 4, 0)
            .unwrap()
            .borrow()
            .static_moving
            .is_empty());
    }

    #[test]
    fn burnt_corpse_is_removed_even_when_decay_removal_disabled() {
        let mut body = IsoDeadBody::for_tests(3.5, 4.5, 0.0);
        body.burn_timer = BURN_REMOVAL_SECONDS;
        body.reanimate_time = -1.0;
        let (mut cell, _) = cell_with_body(body);

        let r = update_bodies(
            &mut cell,
            DeadBodyUpdateConfig {
                world_age_hours: 1.0,
                hours_for_corpse_removal: -1.0,
            },
        );
        assert_eq!(r.removed, vec![(3, 4, 0)]);
        assert!(cell
            .get_grid_square(3, 4, 0)
            .unwrap()
            .borrow()
            .static_moving
            .is_empty());
    }
}
