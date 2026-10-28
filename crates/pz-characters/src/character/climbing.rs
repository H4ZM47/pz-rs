//! The climbing parts of `IsoGameCharacter` and `IsoPlayer`: the climb
//! odds (`getClimbingFailChanceFloat`, `nearbyZombieClimbPenalty`,
//! `getClimbRopeSpeed`), the arm strain of a climb
//! (`addBothArmMuscleStrain`), the gloves, and the calls that start a
//! climb over a wall (`IsoPlayer.climbOverWall`) or up and down a sheet
//! rope (`climbSheetRope`, `climbDownSheetRope`, `fallFromRope`).
//!
//! The squares are the world's ([`StateWorld::sheet_rope_square`],
//! [`StateWorld::can_climb_over_wall`]).
//!
//! [`StateWorld::sheet_rope_square`]: crate::character::state::StateWorld::sheet_rope_square
//! [`StateWorld::can_climb_over_wall`]: crate::character::state::StateWorld::can_climb_over_wall

use pz_scripts::objects::items::ItemBodyLocation;
use pz_scripts::ScriptManager;
use pz_world::square::IsoDirections;

use super::state::StateCx;
use super::states::{climb_wall, ids};
use super::{GameCharacter, SimContext};
use crate::Characters;

impl<I: 'static> GameCharacter<I> {
    /// `getPerkLevel(PerkFactory.Perks.<id>)`.
    fn climb_perk(&self, chars: &Characters, id: &str) -> i32 {
        self.perk_level(chars.perks.constant(id).as_ref())
    }

    /// `getMoodles().getMoodleLevel(MoodleType.<field>)`.
    fn climb_moodle(&self, sm: &ScriptManager, field: &str) -> i32 {
        self.moodles.moodle_level_of(sm, field).unwrap_or(0)
    }

    /// `characterTraits.get(CharacterTrait.<field>)`.
    fn climb_trait(&self, sm: &ScriptManager, field: &str) -> bool {
        self.has_trait(sm, field).unwrap_or(false)
    }

    /// `getWornItem(ItemBodyLocation.HANDS)`.
    fn hands_item(&self, sm: &ScriptManager) -> Option<i32> {
        let hands = ItemBodyLocation::constant(sm, "HANDS");
        self.worn_item(hands.as_ref()).ok().flatten()
    }

    /// `isWearingGloves()`.
    pub fn is_wearing_gloves(&self, sm: &ScriptManager) -> bool {
        self.hands_item(sm).is_some()
    }

    /// `isWearingAwkwardGloves()`: gloves tagged `AWKWARD_GLOVES`.
    pub fn is_wearing_awkward_gloves(&self, sm: &ScriptManager) -> bool {
        self.hands_item(sm)
            .and_then(|id| self.items.item(id))
            .is_some_and(|i| super::items::has_tag(sm, &i.borrow(), "AWKWARD_GLOVES"))
    }

    /// `getClimbingFailChanceFloat()`: the skills and bonuses less the
    /// penalties, then the whole part of the square root.
    pub fn climbing_fail_chance_float(&self, cx: &mut StateCx<'_, I>) -> f32 {
        let (sm, chars) = (cx.sm, cx.chars);
        let mut fail = 0.0f32;
        fail += self.climb_perk(chars, "Fitness") as f32 * 2.0;
        fail += self.climb_perk(chars, "Strength") as f32 * 2.0;
        fail += self.climb_perk(chars, "Nimble") as f32 * 2.0;
        fail += self.climb_moodle(sm, "ENDURANCE") as f32 * -5.0;
        fail += self.climb_moodle(sm, "DRUNK") as f32 * -8.0;
        fail += self.climb_moodle(sm, "HEAVY_LOAD") as f32 * -8.0;
        fail += self.climb_moodle(sm, "PAIN") as f32 * -5.0;
        if self.climb_trait(sm, "OBESE") {
            fail += -25.0;
        } else if self.climb_trait(sm, "OVERWEIGHT") {
            fail += -15.0;
        }
        if self.climb_trait(sm, "CLUMSY") {
            fail /= 2.0;
        }
        if self.is_wearing_awkward_gloves(sm) {
            fail /= 2.0;
        } else if self.is_wearing_gloves(sm) {
            fail += 4.0;
        }
        if self.climb_trait(sm, "ALL_THUMBS") {
            fail += -4.0;
        } else if self.climb_trait(sm, "DEXTROUS") {
            fail += 4.0;
        }
        if self.climb_trait(sm, "BURGLAR") {
            fail += 4.0;
        }
        if self.climb_trait(sm, "GYMNAST") {
            fail += 4.0;
        }
        fail += self.nearby_zombie_climb_penalty(cx);
        fail = fail.max(0.0);
        (fail as f64).sqrt() as i32 as f32
    }

    /// `getClimbingFailChanceInt()`.
    pub fn climbing_fail_chance_int(&self, cx: &mut StateCx<'_, I>) -> i32 {
        self.climbing_fail_chance_float(cx) as i32
    }

    /// `nearbyZombieClimbPenalty()`: 25 for each zombie on the square
    /// attacking this character, 7 for any other.
    pub fn nearby_zombie_climb_penalty(&self, cx: &mut StateCx<'_, I>) -> f32 {
        let mut penalty = 0.0f32;
        let Some(sq) = self.path.current_square else { return penalty };
        for id in cx.world.zombies_on_square(sq) {
            let attacking = if id == self.id {
                self.vars.target == Some(self.id) && self.is_current_state_id(ids::ATTACK)
            } else {
                cx.world.character_mut(id).is_some_and(|z| z.vars.target == Some(self.id) && z.is_current_state_id(ids::ATTACK))
            };
            penalty += if attacking { 25.0 } else { 7.0 };
        }
        penalty
    }

    /// `getClimbRopeSpeed(down)`.
    pub fn climb_rope_speed(&self, sm: &ScriptManager, chars: &Characters, down: bool) -> f32 {
        let mut strength = self.climb_perk(chars, "Strength").max(self.climb_perk(chars, "Fitness"))
            - (self.climb_moodle(sm, "DRUNK") + self.climb_moodle(sm, "ENDURANCE") + self.climb_moodle(sm, "PAIN"));
        if !down {
            strength -= self.climb_moodle(sm, "HEAVY_LOAD");
            if self.climb_trait(sm, "OBESE") {
                strength -= 2;
            } else if self.climb_trait(sm, "OVERWEIGHT") {
                strength -= 1;
            }
        }
        if self.climb_trait(sm, "ALL_THUMBS") {
            strength -= 1;
        } else if self.climb_trait(sm, "DEXTROUS") {
            strength += 1;
        }
        if self.climb_trait(sm, "BURGLAR") {
            strength += 1;
        }
        if self.climb_trait(sm, "GYMNAST") {
            strength += 1;
        }
        if self.is_wearing_awkward_gloves(sm) {
            strength /= 2;
        } else if self.is_wearing_gloves(sm) {
            strength += 1;
        }
        strength = strength.clamp(0, 10);
        let mut speed = 0.16f32;
        match strength {
            0 => speed -= 0.12,
            1 => speed -= 0.11,
            2 => speed -= 0.1,
            3 => speed -= 0.09,
            6 => speed += 0.02,
            7 => speed += 0.05,
            8 => speed += 0.07,
            9 => speed += 0.09,
            10 => speed += 0.12,
            _ => {}
        }
        speed * 0.5
    }

    /// `addArmMuscleStrain(painfactor)`: the right arm.
    pub fn add_arm_muscle_strain(&mut self, chars: &Characters, ctx: &dyn SimContext, f: f32) {
        self.add_one_arm_muscle_strain(chars, ctx, f, ["Hand_R", "ForeArm_R", "UpperArm_R"]);
    }

    /// `addLeftArmMuscleStrain(painfactor)`.
    pub fn add_left_arm_muscle_strain(&mut self, chars: &Characters, ctx: &dyn SimContext, f: f32) {
        self.add_one_arm_muscle_strain(chars, ctx, f, ["Hand_L", "ForeArm_L", "UpperArm_L"]);
    }

    fn add_one_arm_muscle_strain(&mut self, chars: &Characters, ctx: &dyn SimContext, mut f: f32, parts: [&str; 3]) {
        if ctx.muscle_strain_factor() > 0.0 {
            f *= 2.5;
            f = (f as f64 * ctx.muscle_strain_factor()) as f32;
            for p in parts {
                let _ = self.add_stiffness_to(chars, p, f);
            }
        }
    }

    /// `addBothArmMuscleStrain(painfactor)`.
    pub fn add_both_arm_muscle_strain(&mut self, chars: &Characters, ctx: &dyn SimContext, f: f32) {
        self.add_arm_muscle_strain(chars, ctx, f);
        self.add_left_arm_muscle_strain(chars, ctx, f);
    }

    /// `setCollidable(collidable)`.
    pub fn set_collidable(&mut self, collidable: bool) {
        self.path.collidable = collidable;
        self.player.collidable = collidable;
    }

    /// `isClimbingRope()`: in `ClimbSheetRopeState` or
    /// `ClimbDownSheetRopeState`.
    pub fn is_climbing_rope(&self) -> bool {
        self.is_current_state_id(ids::CLIMB_SHEET_ROPE) || self.is_current_state_id(ids::CLIMB_DOWN_SHEET_ROPE)
    }

    /// `fallFromRope()`: a character on a rope lets go.
    pub fn fall_from_rope(&mut self) {
        if !self.is_climbing_rope() {
            return;
        }
        self.set_collidable(true);
        self.vars.climbing = false;
        self.vars.falling = true;
        self.clear_var("ClimbRope");
        self.vars.llz = self.vars.z;
    }

    /// `canClimbSheetRope(sq)`: a rope runs up from `sq` to its top, with
    /// no floor in between.
    pub fn can_climb_sheet_rope(&self, sq: Option<(i32, i32, i32)>, cx: &StateCx<'_, I>) -> bool {
        let Some(mut p) = sq else { return false };
        let start_z = p.2;
        while let Some(s) = cx.world.sheet_rope_square(p) {
            if !s.sheet_rope || !s.can_climb_here {
                return false;
            }
            if s.solid_floor && p.2 > start_z {
                return false;
            }
            if s.top {
                return true;
            }
            // getGridSquare(x, y, z + 1.0f)
            p.2 += 1;
        }
        false
    }

    /// `canClimbDownSheetRope(sq)`: a rope runs down from `sq` to a floor
    /// below it.
    pub fn can_climb_down_sheet_rope(&self, sq: Option<(i32, i32, i32)>, cx: &StateCx<'_, I>) -> bool {
        let Some(mut p) = sq else { return false };
        let start_z = p.2;
        while let Some(s) = cx.world.sheet_rope_square(p) {
            if !s.sheet_rope || !s.can_climb_here {
                return false;
            }
            if s.solid_floor {
                return p.2 < start_z;
            }
            p.2 -= 1;
        }
        false
    }

    /// `canClimbDownSheetRopeInCurrentSquare()`.
    pub fn can_climb_down_sheet_rope_in_current_square(&self, cx: &StateCx<'_, I>) -> bool {
        self.can_climb_down_sheet_rope(self.path.current_square, cx)
    }

    /// `climbSheetRope()`: drops the heavy hand items, then
    /// `EventClimbRope`.
    pub fn climb_sheet_rope(&mut self, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        if !self.can_climb_sheet_rope(self.path.current_square, cx) {
            return;
        }
        self.drop_heavy_items_in(cx);
        if cx.has_host_error() { return; }
        self.state.params.clear(ids::CLIMB_SHEET_ROPE);
        self.report_event("EventClimbRope");
    }

    /// `climbDownSheetRope()`: drops the heavy hand items, then
    /// `EventClimbDownRope`.
    pub fn climb_down_sheet_rope(&mut self, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        if !self.can_climb_down_sheet_rope(self.path.current_square, cx) {
            return;
        }
        self.drop_heavy_items_in(cx);
        if cx.has_host_error() { return; }
        self.state.params.clear(ids::CLIMB_DOWN_SHEET_ROPE);
        self.report_event("EventClimbDownRope");
    }

    /// `IsoPlayer.canClimbOverWall(dir)`: not sprinting, and the world's
    /// part ([`crate::character::state::StateWorld::can_climb_over_wall`]).
    pub fn can_climb_over_wall(&self, dir: IsoDirections, cx: &StateCx<'_, I>) -> bool {
        if self.is_sprinting(cx.sm, cx.ctx) {
            return false;
        }
        cx.world.can_climb_over_wall(self, dir)
    }

    /// `IsoPlayer.climbOverWall(dir)`: drops the heavy hand items, rolls
    /// the climb (`ClimbOverWallState.setParams`), then `EventClimbWall`.
    pub fn climb_over_wall(&mut self, dir: IsoDirections, cx: &mut StateCx<'_, I>) -> bool {
    if cx.has_host_error() { return false; }
        if !self.can_climb_over_wall(dir, cx) {
            return false;
        }
        self.drop_heavy_items_in(cx);
        if cx.has_host_error() { return false; }
        climb_wall::set_params(self, dir, cx);
        self.report_event("EventClimbWall");
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pz_inventory::ItemClass;

    use super::*;
    use crate::character::floor_drop::tests::TestFloor;
    use crate::character::state::{SheetRopeSquare, StateWorld};
    use crate::character::tests::{fixture, give_item, player, Recorder};
    use crate::character::CharacterKind;

    /// Zombies on (4, 5, 0); a wall north of it; a rope on (4, 5) from
    /// z 0 (a floor) to its top on z 2.
    #[derive(Default)]
    struct Climbs {
        zombies: Vec<GameCharacter<()>>,
        rope: HashMap<(i32, i32, i32), SheetRopeSquare>,
        floor: TestFloor,
    }

    impl StateWorld<()> for Climbs {
        fn zombies_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
            if sq == (4, 5, 0) {
                self.zombies.iter().map(|z| z.id).collect()
            } else {
                Vec::new()
            }
        }
        fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<()>> {
            self.zombies.iter_mut().find(|z| z.id == id)
        }
        fn can_climb_over_wall(&self, chr: &GameCharacter<()>, dir: IsoDirections) -> bool {
            chr.path.current_square == Some((4, 5, 0)) && dir == IsoDirections::N
        }
        fn sheet_rope_square(&self, sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
            self.rope.get(&sq).copied()
        }
        fn floor(&mut self) -> Option<&mut dyn pz_inventory::floor_drop::FloorWorld> {
            Some(&mut self.floor)
        }
    }

    fn rope() -> HashMap<(i32, i32, i32), SheetRopeSquare> {
        let r = SheetRopeSquare { sheet_rope: true, can_climb_here: true, ..Default::default() };
        HashMap::from([
            ((4, 5, 0), SheetRopeSquare { solid_floor: true, ..r }),
            ((4, 5, 1), r),
            ((4, 5, 2), SheetRopeSquare { top: true, ..r }),
        ])
    }

    fn climber(sm: &ScriptManager, chars: &mut Characters) -> GameCharacter<()> {
        let mut c = player(sm, chars);
        c.id = 1;
        c.path.current_square = Some((4, 5, 0));
        (c.vars.x, c.vars.y) = (4.5, 5.5);
        c
    }

    #[test]
    fn the_rope_speed_follows_the_better_of_strength_and_fitness() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut c = climber(&sm, &mut chars);
        // Level 0: 0.16 - 0.12, halved.
        assert_eq!(c.climb_rope_speed(&sm, &chars, false), (0.16 - 0.12) * 0.5);
        let strength = chars.perks.constant("Strength").unwrap();
        let fitness = chars.perks.constant("Fitness").unwrap();
        c.set_perk_level_debug(&strength, 4, &ctx);
        c.set_perk_level_debug(&fitness, 9, &ctx);
        assert_eq!(c.climb_rope_speed(&sm, &chars, true), (0.16 + 0.09) * 0.5);
        // Levels 4 and 5 keep the base speed.
        c.set_perk_level_debug(&fitness, 5, &ctx);
        assert_eq!(c.climb_rope_speed(&sm, &chars, false), 0.16 * 0.5);
    }

    #[test]
    fn nearby_zombies_add_to_the_climb_odds() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let c = climber(&sm, &mut chars);
        let mut far = climber(&sm, &mut chars);
        far.path.current_square = Some((9, 9, 0));
        let mut w = Climbs::default();
        for id in [7, 8] {
            let mut z = GameCharacter::new(&sm, CharacterKind::Zombie);
            z.id = id;
            w.zombies.push(z);
        }
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        // Two idle zombies: 7 each, and sqrt(14) rounds down to 3.
        assert_eq!(c.nearby_zombie_climb_penalty(&mut cx), 14.0);
        assert_eq!(c.climbing_fail_chance_float(&mut cx), 3.0);
        assert_eq!(c.climbing_fail_chance_int(&mut cx), 3);
        // Away from them: no penalty and no skills, 0.
        assert_eq!(far.climbing_fail_chance_int(&mut cx), 0);
    }

    #[test]
    fn a_rope_is_climbed_up_from_its_floor_and_down_from_above() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Climbs { rope: rope(), ..Default::default() };
        let c = climber(&sm, &mut chars);
        let cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        assert!(c.can_climb_sheet_rope(Some((4, 5, 0)), &cx));
        assert!(c.can_climb_sheet_rope(Some((4, 5, 1)), &cx));
        assert!(!c.can_climb_sheet_rope(Some((5, 5, 0)), &cx));
        assert!(!c.can_climb_sheet_rope(None, &cx));
        assert!(c.can_climb_down_sheet_rope(Some((4, 5, 2)), &cx));
        // Standing on the floor at its foot: nowhere to climb down to.
        assert!(!c.can_climb_down_sheet_rope(Some((4, 5, 0)), &cx));
        assert!(!c.can_climb_down_sheet_rope_in_current_square(&cx));
        let _ = cx;
        // A floor half way up blocks the climb.
        w.rope.get_mut(&(4, 5, 1)).unwrap().solid_floor = true;
        let cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        assert!(!c.can_climb_sheet_rope(Some((4, 5, 0)), &cx));
    }

    #[test]
    fn climbing_a_rope_or_a_wall_drops_heavy_hand_items_first() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        for (wall, event) in [(false, "EventClimbRope"), (true, "EventClimbWall")] {
            let mut w = Climbs { rope: rope(), ..Default::default() };
            let mut c = climber(&sm, &mut chars);
            give_item(&sm, &mut c, ItemClass::Container, "Duffel", 1, |_| {}, |_| {});
            give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
            c.set_primary_hand_item(&sm, Some(2));
            c.set_secondary_hand_item(&sm, Some(1));
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
            if wall {
                // Not a wall to the east.
                assert!(!c.climb_over_wall(IsoDirections::E, &mut cx));
                assert!(c.climb_over_wall(IsoDirections::N, &mut cx));
                assert_eq!(climb_wall::END_Y.peek(&c.state.params), Some(4));
            } else {
                c.climb_sheet_rope(&mut cx);
            }
            let _ = cx;
            assert!(c.state.action_context.has_event_occurred(event, None));
            // The bag falls; the spoon stays in hand.
            assert_eq!(w.floor.put.iter().map(|d| (d.0, d.1)).collect::<Vec<_>>(), [((4, 5, 0), 1)]);
            assert_eq!((c.items.primary_hand_item, c.items.secondary_hand_item), (Some(2), None));
        }
        // No rope down from the floor: nothing happens, nothing drops.
        let mut w = Climbs { rope: rope(), ..Default::default() };
        let mut c = climber(&sm, &mut chars);
        give_item(&sm, &mut c, ItemClass::Container, "Duffel", 1, |_| {}, |_| {});
        c.set_secondary_hand_item(&sm, Some(1));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.climb_down_sheet_rope(&mut cx);
        let _ = cx;
        assert!(w.floor.put.is_empty());
        assert!(!c.state.action_context.has_event_occurred("EventClimbDownRope", None));
    }

    #[test]
    fn failed_drop_aborts_climb_and_preserves_unexecuted_path_requests() {
        use crate::character::path_mover::PathMoverRequest;
        use pz_inventory::floor_drop::RuntimeHostError;
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Climbs { rope: rope(), ..Default::default() };
        w.floor.fail_at = Some(1);
        let mut c = climber(&sm, &mut chars);
        give_item(&sm, &mut c, ItemClass::Container, "Duffel", 1, |_| {}, |_| {});
        give_item(&sm, &mut c, ItemClass::Container, "Bag", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(1));
        c.set_secondary_hand_item(&sm, Some(2));
        c.path.requests = vec![PathMoverRequest::ClimbOverWall(IsoDirections::N), PathMoverRequest::ClimbOverWall(IsoDirections::N)];
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.run_path_requests(&mut cx);
        assert_eq!(cx.state_result().map_err(|error| error.host().cloned().expect("original Host domain")), Err(RuntimeHostError::Borrowed("actual floor allocation")));
        assert_eq!(c.path.requests.len(), 1);
        assert!(!c.state.action_context.has_event_occurred("EventClimbWall", None));
        assert!(climb_wall::END_Y.peek(&c.state.params).is_none());
        assert_eq!(c.items.primary_hand_item, None);
        assert_eq!(c.items.secondary_hand_item, Some(2));
        assert!(ctx.events.borrow().is_empty());
        drop(cx);
        assert_eq!(w.floor.attempts, 1);
        assert!(w.floor.put.is_empty());
    }

    #[test]
    fn letting_go_of_a_rope_starts_a_fall() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Climbs { rope: rope(), ..Default::default() };
        let mut c = climber(&sm, &mut chars);
        // Not on a rope: nothing.
        c.vars.climbing = true;
        c.fall_from_rope();
        assert!(c.vars.climbing && !c.vars.falling);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.change_state(Some(crate::character::states::climb_rope::climb_sheet_rope()), &mut cx);
        let _ = cx;
        assert!(c.is_climbing_rope());
        c.vars.z = 1.25;
        c.fall_from_rope();
        assert!(!c.vars.climbing && c.vars.falling && c.path.collidable);
        assert_eq!(c.vars.llz, 1.25);
    }

    #[test]
    fn arm_strain_follows_the_sandbox_factor() {
        let (sm, mut chars) = crate::character::thermoregulator::body_tests::body_fixture();
        let ctx = Recorder::default();
        let mut c = climber(&sm, &mut chars);
        c.add_both_arm_muscle_strain(&chars, &ctx, 2.0);
        let stiff = |c: &GameCharacter<()>, f: &str| {
            let i = chars.sim.body_part_types.index(f).unwrap();
            c.body_damage.parts[i].stiffness
        };
        // 2 * 2.5 * 1.0 on each part of both arms, nothing elsewhere.
        for f in ["Hand_L", "ForeArm_L", "UpperArm_L", "Hand_R", "ForeArm_R", "UpperArm_R"] {
            assert_eq!(stiff(&c, f), 5.0, "{f}");
        }
        assert_eq!(stiff(&c, "Head"), 0.0);
    }
}
