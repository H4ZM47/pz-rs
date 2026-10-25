//! `ClimbSheetRopeState` and `ClimbDownSheetRopeState` (climbing a sheet
//! rope up or down), with the `IsoGameCharacter` climbing code they use:
//! `climbSheetRope`, `climbDownSheetRope`, `canClimbSheetRope`,
//! `canClimbDownSheetRope`, `getClimbingFailChanceFloat`,
//! `nearbyZombieClimbPenalty`, `getClimbRopeSpeed`, `fallFromRope`,
//! `isClimbingRope`, `isWearingGloves` and the arm muscle strain.
//!
//! The squares are the world's ([`StateWorld::sheet_rope_square`],
//! [`StateWorld::chunk_levels`], [`StateWorld::sheet_rope_exit`]). Not
//! here: `getSpriteDef().animFrameIncrease` (the sprite renderer's),
//! `setLlz` in `fallFromRope` (TODO(falling): the fall itself is not
//! ported), the debug overlay and the network `processOnEnter/Exit`.
//!
//! [`StateWorld::sheet_rope_square`]: crate::character::state::StateWorld::sheet_rope_square
//! [`StateWorld::chunk_levels`]: crate::character::state::StateWorld::chunk_levels
//! [`StateWorld::sheet_rope_exit`]: crate::character::state::StateWorld::sheet_rope_exit

use pz_world::java::fastfloor;
use pz_world::square::IsoDirections;

use super::ids;
use crate::character::items::{body_location_key, has_tag};
use crate::character::state::{Param, SheetRopeExit, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::GameCharacter;

/// `ClimbSheetRopeState.ClimbSpeed`.
pub const CLIMB_SPEED: f32 = 0.16;
/// `ClimbSheetRopeState.FallChanceBase`.
pub const FALL_CHANCE_BASE: f32 = 1.0;
/// `ClimbSheetRopeState.FallChanceScale`.
const FALL_CHANCE_SCALE: f32 = 100.0;
/// `ClimbDownSheetRopeState.ClimbDownFallChanceScale`.
const CLIMB_DOWN_FALL_CHANCE_SCALE: f32 = 300.0;

/// `ClimbSheetRopeState.SPEED`, `CLIMB`.
pub const UP_SPEED: Param<f32> = Param::new(ids::CLIMB_SHEET_ROPE, "speed", || 0.0);
pub const UP_CLIMB: Param<bool> = Param::new(ids::CLIMB_SHEET_ROPE, "climb", || false);
/// `ClimbDownSheetRopeState.SPEED`, `CLIMB`.
pub const DOWN_SPEED: Param<f32> = Param::new(ids::CLIMB_DOWN_SHEET_ROPE, "speed", || 0.0);
pub const DOWN_CLIMB: Param<bool> = Param::new(ids::CLIMB_DOWN_SHEET_ROPE, "climb", || false);

/// `ClimbSheetRopeState.ClimbData` (kept on the character from its first
/// rope climb on).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimbData {
    pub target_climb_height: i32,
    pub fall_chance: f32,
    pub idealx: f32,
    pub idealy: f32,
    pub target_fall_height: f32,
    /// `climbTargetIsoObject`.
    pub climb_target: Option<pz_pathfind::mover::ObjectRef>,
    /// `targetGridSquare`.
    pub target_square: Option<(i32, i32, i32)>,
    /// `exitBlocked` (`None`: `null`).
    pub exit_blocked: Option<SheetRopeExit>,
}

impl Default for ClimbData {
    fn default() -> Self {
        ClimbData {
            target_climb_height: 0,
            fall_chance: 0.0,
            idealx: 0.0,
            idealy: 0.0,
            target_fall_height: f32::MAX,
            climb_target: None,
            target_square: None,
            exit_blocked: None,
        }
    }
}

/// `ClimbSheetRopeState`.
pub struct ClimbSheetRopeState;
/// `ClimbDownSheetRopeState`.
pub struct ClimbDownSheetRopeState;

/// `ClimbSheetRopeState.instance()`.
pub fn climb_sheet_rope<I: 'static>() -> StateRef<I> {
    static S: ClimbSheetRopeState = ClimbSheetRopeState;
    &S
}

/// `ClimbDownSheetRopeState.instance()`.
pub fn climb_down_sheet_rope<I: 'static>() -> StateRef<I> {
    static S: ClimbDownSheetRopeState = ClimbDownSheetRopeState;
    &S
}

/// What `getClimbingFailChanceFloat` and `getClimbRopeSpeed` read of the
/// character (perk levels, moodle levels, traits, gloves).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClimbSkills {
    pub fitness: i32,
    pub strength: i32,
    pub nimble: i32,
    pub endurance: i32,
    pub drunk: i32,
    pub heavy_load: i32,
    pub pain: i32,
    pub obese: bool,
    pub overweight: bool,
    pub clumsy: bool,
    pub all_thumbs: bool,
    pub dextrous: bool,
    pub burglar: bool,
    pub gymnast: bool,
    /// `isWearingGloves()`.
    pub gloves: bool,
    /// `isWearingAwkwardGloves()`.
    pub awkward_gloves: bool,
}

impl ClimbSkills {
    /// `getClimbingFailChanceFloat()` with the nearby-zombie penalty
    /// already summed.
    pub fn fail_chance(&self, zombie_penalty: f32) -> f32 {
        let mut f = 0.0f32;
        f += self.fitness as f32 * 2.0;
        f += self.strength as f32 * 2.0;
        f += self.nimble as f32 * 2.0;
        f += self.endurance as f32 * -5.0;
        f += self.drunk as f32 * -8.0;
        f += self.heavy_load as f32 * -8.0;
        f += self.pain as f32 * -5.0;
        if self.obese {
            f += -25.0;
        } else if self.overweight {
            f += -15.0;
        }
        if self.clumsy {
            f /= 2.0;
        }
        if self.awkward_gloves {
            f /= 2.0;
        } else if self.gloves {
            f += 4.0;
        }
        if self.all_thumbs {
            f += -4.0;
        } else if self.dextrous {
            f += 4.0;
        }
        if self.burglar {
            f += 4.0;
        }
        if self.gymnast {
            f += 4.0;
        }
        f += zombie_penalty;
        f = f.max(0.0);
        ((f as f64).sqrt() as i32) as f32
    }

    /// `getClimbRopeSpeed(down)`.
    pub fn rope_speed(&self, down: bool) -> f32 {
        let mut s = self.strength.max(self.fitness) - (self.drunk + self.endurance + self.pain);
        if !down {
            s -= self.heavy_load;
            if self.obese {
                s -= 2;
            } else if self.overweight {
                s -= 1;
            }
        }
        if self.all_thumbs {
            s -= 1;
        } else if self.dextrous {
            s += 1;
        }
        if self.burglar {
            s += 1;
        }
        if self.gymnast {
            s += 1;
        }
        if self.awkward_gloves {
            s /= 2;
        } else if self.gloves {
            s += 1;
        }
        let s = s.clamp(0, 10);
        let mut speed = CLIMB_SPEED;
        match s {
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
}

impl<I: 'static> GameCharacter<I> {
    /// The perks, moodles, traits and gloves of [`ClimbSkills`].
    pub fn climb_skills(&self, cx: &StateCx<'_, I>) -> ClimbSkills {
        let (sm, chars) = (cx.sm, cx.chars);
        let perk = |n: &str| self.perk_level(chars.perks.constant(n).as_ref());
        let moodle = |n: &str| self.moodles.moodle_level_of(sm, n).unwrap_or(0);
        let tr = |n: &str| self.has_trait(sm, n).unwrap_or(false);
        let hands = self
            .worn_item(body_location_key(sm, "HANDS").as_ref())
            .ok()
            .flatten();
        let awkward = hands
            .and_then(|id| self.items.item(id))
            .is_some_and(|i| has_tag(sm, &i.borrow(), "AWKWARD_GLOVES"));
        ClimbSkills {
            fitness: perk("Fitness"),
            strength: perk("Strength"),
            nimble: perk("Nimble"),
            endurance: moodle("ENDURANCE"),
            drunk: moodle("DRUNK"),
            heavy_load: moodle("HEAVY_LOAD"),
            pain: moodle("PAIN"),
            obese: tr("OBESE"),
            overweight: tr("OVERWEIGHT"),
            clumsy: tr("CLUMSY"),
            all_thumbs: tr("ALL_THUMBS"),
            dextrous: tr("DEXTROUS"),
            burglar: tr("BURGLAR"),
            gymnast: tr("GYMNAST"),
            gloves: hands.is_some(),
            awkward_gloves: awkward,
        }
    }

    /// `nearbyZombieClimbPenalty()`: 25 for each zombie on the square
    /// attacking this character, 7 for every other.
    pub fn nearby_zombie_climb_penalty(&self, cx: &mut StateCx<'_, I>) -> f32 {
        let mut p = 0.0;
        let Some(cur) = self.path.current_square else {
            return p;
        };
        for id in cx.world.zombies_on_square(cur) {
            let attacking = cx.world.character_mut(id).is_some_and(|z| {
                z.vars.target == Some(self.id) && z.is_current_state_id(ids::ATTACK)
            });
            p += if attacking { 25.0 } else { 7.0 };
        }
        p
    }

    /// `getClimbingFailChanceFloat()`.
    pub fn climbing_fail_chance_float(&self, cx: &mut StateCx<'_, I>) -> f32 {
        let penalty = self.nearby_zombie_climb_penalty(cx);
        self.climb_skills(cx).fail_chance(penalty)
    }

    /// `getClimbingFailChanceInt()`.
    pub fn climbing_fail_chance_int(&self, cx: &mut StateCx<'_, I>) -> i32 {
        self.climbing_fail_chance_float(cx) as i32
    }

    /// `getClimbRopeSpeed(down)`.
    pub fn climb_rope_speed(&self, down: bool, cx: &StateCx<'_, I>) -> f32 {
        self.climb_skills(cx).rope_speed(down)
    }

    /// `isClimbingRope()`.
    pub fn is_climbing_rope(&self) -> bool {
        self.is_current_state_id(ids::CLIMB_SHEET_ROPE)
            || self.is_current_state_id(ids::CLIMB_DOWN_SHEET_ROPE)
    }

    /// `fallFromRope()`.
    pub fn fall_from_rope(&mut self) {
        if !self.is_climbing_rope() {
            return;
        }
        self.path.collidable = true;
        self.vars.climbing = false;
        self.vars.falling = true;
        self.clear_var("ClimbRope");
        // setLlz(getZ()): TODO(falling).
    }

    /// `addArmMuscleStrain` and `addLeftArmMuscleStrain`.
    pub fn add_both_arm_muscle_strain(&mut self, cx: &StateCx<'_, I>, f: f32) {
        let factor = cx.ctx.muscle_strain_factor();
        for parts in [
            ["Hand_R", "ForeArm_R", "UpperArm_R"],
            ["Hand_L", "ForeArm_L", "UpperArm_L"],
        ] {
            if factor > 0.0 {
                let v = ((f * 2.5) as f64 * factor) as f32;
                for name in parts {
                    if let Ok(i) = cx.chars.sim.body_part_types.index(name) {
                        if let Some(p) = self.body_damage.parts.get_mut(i) {
                            p.add_stiffness(v);
                        }
                    }
                }
            }
        }
    }

    /// `canClimbSheetRope(sq)`.
    pub fn can_climb_sheet_rope(&self, sq: Option<(i32, i32, i32)>, cx: &StateCx<'_, I>) -> bool {
        let Some(mut sq) = sq else { return false };
        let start_z = sq.2;
        while let Some(s) = cx.world.sheet_rope_square(sq) {
            if !s.sheet_rope || !s.can_climb_here {
                return false;
            }
            if s.solid_floor && sq.2 > start_z {
                return false;
            }
            if s.top {
                return true;
            }
            sq.2 += 1;
        }
        false
    }

    /// `canClimbDownSheetRope(sq)`.
    pub fn can_climb_down_sheet_rope(
        &self,
        sq: Option<(i32, i32, i32)>,
        cx: &StateCx<'_, I>,
    ) -> bool {
        let Some(mut sq) = sq else { return false };
        let start_z = sq.2;
        while let Some(s) = cx.world.sheet_rope_square(sq) {
            if !s.sheet_rope || !s.can_climb_here {
                return false;
            }
            if s.solid_floor {
                return sq.2 < start_z;
            }
            sq.2 -= 1;
        }
        false
    }

    /// `climbSheetRope()`.
    pub fn climb_sheet_rope(&mut self, cx: &mut StateCx<'_, I>) {
        if !self.can_climb_sheet_rope(self.path.current_square, cx) {
            return;
        }
        self.drop_heavy_items_in(cx);
        self.state.params.clear(ids::CLIMB_SHEET_ROPE);
        self.report_event("EventClimbRope");
    }

    /// `climbDownSheetRope()`.
    pub fn climb_down_sheet_rope(&mut self, cx: &mut StateCx<'_, I>) {
        if !self.can_climb_down_sheet_rope(self.path.current_square, cx) {
            return;
        }
        self.drop_heavy_items_in(cx);
        self.state.params.clear(ids::CLIMB_DOWN_SHEET_ROPE);
        self.report_event("EventClimbDownRope");
    }
}

/// `ClimbSheetRopeState.createClimbData(chr)`.
fn create_climb_data<I: 'static>(chr: &mut GameCharacter<I>) {
    if chr.vars.climb_data.is_none() {
        chr.vars.climb_data = Some(ClimbData::default());
    }
}

fn data<I: 'static>(chr: &GameCharacter<I>) -> ClimbData {
    chr.vars.climb_data.unwrap_or_default()
}

fn set_data<I: 'static>(chr: &mut GameCharacter<I>, d: ClimbData) {
    chr.vars.climb_data = Some(d);
}

/// `ClimbSheetRopeState.setIdealDirection(chr)`.
pub fn set_ideal_direction<I: 'static>(chr: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
    let mut d = data(chr);
    // The original dereferences the current square.
    let s = chr
        .path
        .current_square
        .and_then(|sq| cx.world.sheet_rope_square(sq))
        .unwrap_or_default();
    if s.n {
        chr.set_forward_iso_direction(IsoDirections::N);
        (d.idealx, d.idealy) = (0.54, 0.39);
    }
    if s.s {
        chr.set_forward_iso_direction(IsoDirections::S);
        (d.idealx, d.idealy) = (0.118, 0.5756);
    }
    if s.w {
        chr.set_forward_iso_direction(IsoDirections::W);
        (d.idealx, d.idealy) = (0.4, 0.7);
    }
    if s.e {
        chr.set_forward_iso_direction(IsoDirections::E);
        (d.idealx, d.idealy) = (0.5417, 0.3144);
    }
    set_data(chr, d);
}

/// `ClimbSheetRopeState.applyIdealDirection(chr)`: a quarter of the way
/// to the ideal place in the square.
pub fn apply_ideal_direction<I: 'static>(chr: &mut GameCharacter<I>) {
    let d = data(chr);
    let mut ox = chr.vars.x - fastfloor(chr.vars.x) as f32;
    let mut oy = chr.vars.y - fastfloor(chr.vars.y) as f32;
    if ox != d.idealx {
        let dif = (d.idealx - ox) / 4.0;
        ox += dif;
        chr.set_x(fastfloor(chr.vars.x) as f32 + ox);
    }
    if oy != d.idealy {
        let dif = (d.idealy - oy) / 4.0;
        oy += dif;
        chr.set_y(fastfloor(chr.vars.y) as f32 + oy);
    }
    chr.vars.next_x = chr.vars.x;
    chr.vars.next_y = chr.vars.y;
}

/// `fallChanceCalculation(chr)` of either state (`scale` 100 up, 300
/// down); `climbRopeTime` grows by the multiplier.
fn fall_chance_calculation<I: 'static>(
    chr: &mut GameCharacter<I>,
    scale: f32,
    cx: &mut StateCx<'_, I>,
) -> f32 {
    let mut d = data(chr);
    d.fall_chance = chr.climbing_fail_chance_float(cx) + FALL_CHANCE_BASE;
    let m = cx.ctx.multiplier();
    chr.vars.climb_rope_time += m;
    d.fall_chance *= scale;
    d.fall_chance /= if m < 1.0 { 1.0 } else { (m as i32) as f32 };
    set_data(chr, d);
    d.fall_chance
}

/// The fall check and arm strain both states run each frame (`strain` is
/// 0.02 up, 0.007 down).
fn climb_frame<I: 'static>(
    chr: &mut GameCharacter<I>,
    scale: f32,
    strain: f64,
    cx: &mut StateCx<'_, I>,
) {
    fall_chance_calculation(chr, scale, cx);
    let d = data(chr);
    let can_fall_check = chr.vars.climb_rope_time > d.fall_chance * 10.0;
    // Rand.NextBool((int)fallChance).
    let fall = cx.ctx.rand_next(d.fall_chance as i32) == 0;
    let rope_here = chr
        .path
        .current_square
        .and_then(|s| cx.world.sheet_rope_square(s))
        .is_some_and(|s| s.sheet_rope);
    if !rope_here {
        chr.path.collidable = true;
        chr.vars.climbing = false;
        chr.vars.falling = true;
        chr.clear_var("ClimbRope");
    } else if can_fall_check && !cx.ctx.easy_climbing() && fall && !chr.vars.remote {
        chr.fall_from_rope();
    }
    let k = chr.climb_skills(cx);
    let skill_factor = (k.nimble + k.strength.max(k.fitness) * 2) as f32 / 3.0;
    let m = cx.ctx.multiplier();
    let pain = (strain * m as f64 * (k.heavy_load + 1) as f64) as f32
        * ((15.0 - skill_factor) / 10.0)
        * (m / 0.8);
    chr.add_both_arm_muscle_strain(cx, pain);
    if chr.is_player() && chr.is_local_player(cx.ctx) {
        chr.player.dirty_recalc_grid_stack_time = 2.0;
    }
}

impl ClimbSheetRopeState {
    /// `setParams(owner, stage)`.
    fn set_params<I: 'static>(owner: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
        if !owner.vars.remote {
            let speed = owner.climb_rope_speed(false, cx);
            UP_SPEED.set(&mut owner.state.params, speed);
            UP_CLIMB.set(&mut owner.state.params, owner.vars.climbing);
        }
    }

    /// `calculateClimb(chr)`.
    fn calculate_climb<I: 'static>(chr: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        set_ideal_direction(chr, cx);
        apply_ideal_direction(chr);
        let cur = chr.path.current_square.unwrap_or((
            fastfloor(chr.vars.x),
            fastfloor(chr.vars.y),
            fastfloor(chr.vars.z),
        ));
        let (_, max_level) = cx.world.chunk_levels(cur);
        let (x, y) = (fastfloor(chr.vars.x), fastfloor(chr.vars.y));
        for z in fastfloor(chr.vars.z)..=max_level {
            let top = cx.world.sheet_rope_square((x, y, z)).is_some_and(|s| s.top);
            if !top {
                continue;
            }
            let mut d = data(chr);
            d.target_square = Some((x, y, z));
            d.target_climb_height = z;
            set_data(chr, d);
            let dir = chr.forward_iso_direction();
            let exit = cx.world.sheet_rope_exit(chr, (x, y, z), dir);
            let mut d = data(chr);
            if let SheetRopeExit::OpenWindow(o) | SheetRopeExit::WindowFrame(o) = exit {
                d.climb_target = Some(o);
            }
            d.exit_blocked = Some(exit);
            set_data(chr, d);
            break;
        }
        fall_chance_calculation(chr, FALL_CHANCE_SCALE, cx);
    }

    /// `finishClimbing(chr)`.
    fn finish_climbing<I: 'static>(chr: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let d = data(chr);
        chr.set_z(d.target_climb_height as f32);
        if let Some(sq) = d.target_square {
            chr.path.current_square = Some(sq);
        }
        chr.path.collidable = true;
        match d.exit_blocked {
            Some(SheetRopeExit::Blocked) => chr.climb_down_sheet_rope(cx),
            Some(SheetRopeExit::OpenWindow(_)) => {
                if let Some(o) = d.climb_target {
                    chr.climb_through_window(o, cx);
                }
            }
            Some(SheetRopeExit::WindowFrame(_)) => {
                if let Some(o) = d.climb_target {
                    chr.climb_through_window_frame(o, cx);
                }
            }
            Some(SheetRopeExit::Fence) => {
                let dir = chr.forward_iso_direction();
                chr.do_climb_over_fence(dir, cx);
            }
            Some(SheetRopeExit::Undefined) | None => {}
        }
    }
}

impl<I: 'static> State<I> for ClimbSheetRopeState {
    fn id(&self) -> StateId {
        ids::CLIMB_SHEET_ROPE
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.hide_weapon_model = true;
        owner.vars.climbing = true;
        owner.set_var_bool("ClimbRope", true);
        Self::set_params(owner, cx);
        create_climb_data(owner);
        Self::calculate_climb(owner, cx);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        apply_ideal_direction(owner);
        let mut speed = owner.climb_rope_speed(false, cx);
        if owner.vars.remote {
            speed = UP_SPEED.get_or(&mut owner.state.params, || speed);
        }
        let d = data(owner);
        let height = owner.vars.z + speed / 10.0 * cx.ctx.multiplier();
        owner.set_z(height);
        // `!(height >= targetFallHeight)`, true for NaN.
        let below_fall_height = !matches!(
            height.partial_cmp(&d.target_fall_height),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        );
        if below_fall_height && height > d.target_climb_height as f32 {
            Self::finish_climbing(owner, cx);
            return;
        }
        climb_frame(owner, FALL_CHANCE_SCALE, 0.02, cx);
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.path.collidable = true;
        owner.vars.ignore_movement = false;
        owner.vars.hide_weapon_model = false;
        owner.vars.climbing = false;
        owner.clear_var("ClimbRope");
        Self::set_params(owner, cx);
    }

    fn is_processed_on_enter(&self) -> bool {
        true
    }

    fn is_processed_on_exit(&self) -> bool {
        true
    }
}

impl ClimbDownSheetRopeState {
    fn set_params<I: 'static>(owner: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
        if !owner.vars.remote {
            let speed = owner.climb_rope_speed(true, cx);
            DOWN_SPEED.set(&mut owner.state.params, speed);
            DOWN_CLIMB.set(&mut owner.state.params, owner.vars.climbing);
        }
    }

    /// `calculateClimbDown(chr)`.
    fn calculate_climb_down<I: 'static>(chr: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        set_ideal_direction(chr, cx);
        apply_ideal_direction(chr);
        let cur = chr.path.current_square.unwrap_or((
            fastfloor(chr.vars.x),
            fastfloor(chr.vars.y),
            fastfloor(chr.vars.z),
        ));
        let (min_level, _) = cx.world.chunk_levels(cur);
        let (x, y) = (fastfloor(chr.vars.x), fastfloor(chr.vars.y));
        let mut z = fastfloor(chr.vars.z);
        while z >= min_level {
            // The original dereferences the square.
            let floor = cx
                .world
                .sheet_rope_square((x, y, z))
                .is_some_and(|s| s.solidtrans || s.solid_floor);
            if floor {
                let mut d = data(chr);
                d.target_square = Some((x, y, z));
                d.target_climb_height = z;
                set_data(chr, d);
                break;
            }
            z -= 1;
        }
        fall_chance_calculation(chr, CLIMB_DOWN_FALL_CHANCE_SCALE, cx);
    }
}

impl<I: 'static> State<I> for ClimbDownSheetRopeState {
    fn id(&self) -> StateId {
        ids::CLIMB_DOWN_SHEET_ROPE
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.hide_weapon_model = true;
        owner.vars.climbing = true;
        owner.set_var_bool("ClimbRope", true);
        Self::set_params(owner, cx);
        create_climb_data(owner);
        Self::calculate_climb_down(owner, cx);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        apply_ideal_direction(owner);
        let mut speed = owner.climb_rope_speed(true, cx);
        if owner.vars.remote {
            speed = DOWN_SPEED.get_or(&mut owner.state.params, || speed);
        }
        let cur = owner.path.current_square.unwrap_or((
            fastfloor(owner.vars.x),
            fastfloor(owner.vars.y),
            fastfloor(owner.vars.z),
        ));
        let (min_level, _) = cx.world.chunk_levels(cur);
        let height = (owner.vars.z - speed / 10.0 * cx.ctx.multiplier()).max(min_level as f32);
        owner.set_z(height);
        let d = data(owner);
        if height <= d.target_climb_height as f32 {
            // finishClimbing(chr).
            owner.set_z(d.target_climb_height as f32);
            owner.state.params.clear(ids::CLIMB_DOWN_SHEET_ROPE);
            owner.clear_var("ClimbRope");
            owner.path.collidable = true;
            return;
        }
        climb_frame(owner, CLIMB_DOWN_FALL_CHANCE_SCALE, 0.007, cx);
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.path.collidable = true;
        owner.vars.ignore_movement = false;
        owner.vars.hide_weapon_model = false;
        owner.vars.climbing = false;
        owner.clear_var("ClimbRope");
        Self::set_params(owner, cx);
    }

    fn is_processed_on_enter(&self) -> bool {
        true
    }

    fn is_processed_on_exit(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::state::{SheetRopeSquare, SquareInfo, StateWorld};
    use crate::character::tests::{fixture, player, Recorder};
    use crate::character::SimContext;

    #[test]
    fn fail_chance_and_rope_speed_follow_the_skills() {
        let k = ClimbSkills::default();
        assert_eq!(k.fail_chance(0.0), 0.0);
        assert_eq!(k.rope_speed(false), (CLIMB_SPEED - 0.12) * 0.5);
        let k = ClimbSkills { fitness: 5, strength: 6, nimble: 2, gloves: true, ..Default::default() };
        // 10 + 12 + 4 + 4 = 30 -> (int)sqrt(30) = 5; strength 6 + gloves.
        assert_eq!(k.fail_chance(0.0), 5.0);
        assert_eq!(k.rope_speed(false), (CLIMB_SPEED + 0.05) * 0.5);
        // Two zombies (7 each), clumsy halves before the bonuses.
        let k = ClimbSkills { fitness: 5, strength: 6, nimble: 2, clumsy: true, burglar: true, ..Default::default() };
        assert_eq!(k.fail_chance(14.0), ((13.0f32 + 4.0 + 14.0).sqrt() as i32) as f32);
        // Heavy load and obesity slow only the climb up; awkward gloves
        // halve the strength.
        let k = ClimbSkills { strength: 8, heavy_load: 2, obese: true, awkward_gloves: true, gloves: true, ..Default::default() };
        // Up: (8 - 2 - 2) / 2 = 2; down: 8 / 2 = 4.
        assert_eq!(k.rope_speed(false), (CLIMB_SPEED - 0.1) * 0.5);
        assert_eq!(k.rope_speed(true), CLIMB_SPEED * 0.5);
        assert_eq!(ClimbSkills { strength: 1, overweight: true, fitness: 1, ..Default::default() }.fail_chance(-100.0), 0.0);
    }

    /// A rope on (5, 5) from level 0 to its top on level 2, with a floor
    /// on 0 and an open window to the north at the top.
    #[derive(Default)]
    struct Rope {
        exits: Vec<(i32, i32, i32)>,
    }

    impl StateWorld<()> for Rope {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Some(SquareInfo { x, y, z, solid_floor: z == 0, ..Default::default() })
        }
        fn sheet_rope_square(&self, sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
            if !(0..=3).contains(&sq.2) {
                return None;
            }
            let rope = sq.0 == 5 && sq.1 == 5 && sq.2 <= 2;
            Some(SheetRopeSquare {
                sheet_rope: rope,
                can_climb_here: true,
                top: rope && sq.2 == 2,
                solid_floor: sq.2 == 0,
                n: rope,
                ..Default::default()
            })
        }
        fn chunk_levels(&self, _sq: (i32, i32, i32)) -> (i32, i32) {
            (0, 3)
        }
        fn sheet_rope_exit(&mut self, _chr: &GameCharacter<()>, target: (i32, i32, i32), dir: IsoDirections) -> SheetRopeExit {
            assert_eq!(dir, IsoDirections::N);
            self.exits.push(target);
            SheetRopeExit::Blocked
        }
    }

    struct Fast(Recorder);
    impl SimContext for Fast {
        fn zomboid_globals(&self) -> &crate::character::ZomboidGlobals {
            &self.0.globals
        }
        fn multiplier(&self) -> f32 {
            4.0
        }
        fn easy_climbing(&self) -> bool {
            true
        }
        fn event(&self, e: crate::character::SimEvent) {
            self.0.event(e)
        }
    }

    #[test]
    fn climbing_a_rope_to_the_top_and_down_again() {
        let (sm, mut chars) = fixture();
        let ctx = Fast(Recorder::default());
        let mut w = Rope::default();
        let mut p = player(&sm, &mut chars);
        p.id = 1;
        (p.vars.x, p.vars.y) = (5.9, 5.9);
        p.path.current_square = Some((5, 5, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        assert!(p.can_climb_sheet_rope(Some((5, 5, 0)), &cx));
        assert!(!p.can_climb_sheet_rope(Some((6, 5, 0)), &cx));
        assert!(!p.can_climb_down_sheet_rope(Some((5, 5, 0)), &cx));
        assert!(p.can_climb_down_sheet_rope(Some((5, 5, 2)), &cx));
        p.climb_sheet_rope(&mut cx);
        let up = climb_sheet_rope::<()>();
        up.enter(&mut p, &mut cx);
        assert!(p.vars.climbing && p.variable_boolean("ClimbRope"));
        assert_eq!(p.vars.dir, IsoDirections::N);
        let d = p.vars.climb_data.unwrap();
        assert_eq!((d.target_climb_height, d.target_square, d.exit_blocked), (2, Some((5, 5, 2)), Some(SheetRopeExit::Blocked)));
        // A quarter of the way to (0.54, 0.39) in the square.
        assert!((p.vars.x - (5.0 + 0.9 + (0.54 - 0.9) / 4.0)).abs() < 1e-5);
        // Fall chance (sqrt(0) + 1) * 100 / 4; the multiplier added once.
        assert_eq!((d.fall_chance, p.vars.climb_rope_time), (25.0, 4.0));
        let speed = p.climb_rope_speed(false, &cx);
        let mut frames = 0;
        while frames < 1000 {
            up.execute(&mut p, &mut cx);
            frames += 1;
            if p.vars.z == 2.0 {
                break;
            }
            p.path.current_square = Some((5, 5, p.vars.z as i32));
        }
        let (mut z, mut expected) = (0.0f32, 0);
        while z <= 2.0 {
            z += speed / 10.0 * 4.0;
            expected += 1;
        }
        assert_eq!(frames, expected);
        // Finished: at the top square, and the blocked exit starts the
        // climb down.
        assert_eq!(p.vars.z, 2.0);
        up.exit(&mut p, &mut cx);
        assert!(!p.vars.climbing);
        let down = climb_down_sheet_rope::<()>();
        down.enter(&mut p, &mut cx);
        assert_eq!(p.vars.climb_data.unwrap().target_climb_height, 0);
        let mut frames = 0;
        while p.variable_boolean("ClimbRope") && frames < 1000 {
            down.execute(&mut p, &mut cx);
            frames += 1;
            p.path.current_square = Some((5, 5, p.vars.z.ceil() as i32));
        }
        assert_eq!(p.vars.z, 0.0);
        assert!(p.path.collidable);
        let _ = cx;
        assert_eq!(w.exits, [(5, 5, 2)]);
    }

    #[test]
    fn leaving_the_rope_makes_the_climber_fall() {
        let (sm, mut chars) = fixture();
        let ctx = Fast(Recorder::default());
        let mut w = Rope::default();
        let mut p = player(&sm, &mut chars);
        (p.vars.x, p.vars.y) = (5.5, 5.5);
        p.path.current_square = Some((5, 5, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        let up = climb_sheet_rope::<()>();
        up.enter(&mut p, &mut cx);
        // Pushed off the rope's square below the top.
        p.path.current_square = Some((6, 5, 0));
        up.execute(&mut p, &mut cx);
        assert!(p.vars.falling && !p.vars.climbing && !p.variable_boolean("ClimbRope"));
    }
}
