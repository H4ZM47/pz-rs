//! Engine-owned ragdoll runtime (`pz-ragdoll` over the shared `PzBullet` world):
//!
//! - Loads `media/scripts/ragdolls/*.txt` into [`RagdollScripts`] and uploads
//!   them to [`PzBullet`] (`RagdollScripts::to_bullet`).
//! - Lazily initializes [`RagdollBuilder`] from `Skinned/MaleBody` +
//!   `Bob/Bob_TPose` (or directly via [`WorldRagdolls::init_builder_with`] in
//!   tests).
//! - Owns [`RagdollGlobals`] (synced from `Core.getMaxActiveRagdolls()` and
//!   exposed to `SimContext::active_ragdoll_simulations`).
//! - Drives per-character `RagdollController` updates via
//!   `AnimationPlayer::update_with_ragdoll` and freezes ragdoll poses into
//!   `IsoDeadBody.ragdoll` on corpse creation.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::clip::AnimClip;
use pz_anim::skeleton::Skeleton;
use pz_anim::AnimationPlayer;
use pz_bullet::PzBullet;
use pz_characters::character::firearm::{CharacterRagdoll, TargetSkeleton};
use pz_characters::character::state::StateCx;
use pz_lua_api::bind_character_sim::LuaGameCharacter;
use pz_ragdoll::{
    RagdollBuilder, RagdollCharacter, RagdollController, RagdollDebugOptions, RagdollEnv,
    RagdollFrame, RagdollGlobals, RagdollScripts, Vec3,
};
use pz_world::save::visual::BoneTransform as SavedBoneTransform;

use crate::iso_world::IsoWorld;
use crate::ui_models::Assets;

thread_local! {
    /// Current `RagdollGlobals.number_of_active_simulations` for
    /// `SimContext::active_ragdoll_simulations` without re-borrowing `IsoWorld`.
    static ACTIVE_SIMULATIONS: Cell<i32> = const { Cell::new(0) };
}

/// `RagdollController.getNumberOfActiveSimulations()` as read by `SimContext`.
pub fn active_ragdoll_simulations() -> i32 {
    ACTIVE_SIMULATIONS.with(Cell::get)
}

fn publish_active_simulations(count: i32) {
    ACTIVE_SIMULATIONS.with(|c| c.set(count));
}

/// Engine-owned ragdoll script tables, T-pose builder, and active simulation
/// counter sharing the world's single [`PzBullet`] instance.
pub struct WorldRagdolls {
    bullet: Rc<RefCell<PzBullet>>,
    pub scripts: RagdollScripts,
    builder: Option<RagdollBuilder>,
    builder_attempted: bool,
    pub globals: RagdollGlobals,
    pub debug: RagdollDebugOptions,
}

impl WorldRagdolls {
    /// Reads every `*.txt` script under `<game_dir>/media/scripts/ragdolls` in
    /// sorted path order into a [`RagdollScripts`].
    pub fn load_scripts(game_dir: &Path) -> RagdollScripts {
        let mut scripts = RagdollScripts::default();
        let dir = game_dir.join("media").join("scripts").join("ragdolls");
        let mut files: Vec<PathBuf> = Vec::new();
        collect_txt(&dir, &mut files);
        files.sort();
        for path in files {
            if let Ok(text) = std::fs::read_to_string(&path) {
                scripts.load_file(&text);
            }
        }
        scripts
    }

    /// Initializes ragdoll scripts, uploads them into the shared [`PzBullet`]
    /// world, and seeds [`RagdollGlobals`] from `Core::get_max_active_ragdolls`.
    pub fn init(game_dir: &Path, bullet: Rc<RefCell<PzBullet>>) -> Self {
        let scripts = Self::load_scripts(game_dir);
        if !scripts.constraints.is_empty()
            || !scripts.anchors.is_empty()
            || !scripts.body_part_infos.is_empty()
            || !scripts.body_dynamics.is_empty()
        {
            scripts.to_bullet(&mut bullet.borrow_mut(), false);
        }
        let max_active = pz_lua_api::bind_core::shared_core()
            .borrow()
            .get_max_active_ragdolls();
        publish_active_simulations(0);
        Self {
            bullet,
            scripts,
            builder: None,
            builder_attempted: false,
            globals: RagdollGlobals {
                number_of_active_simulations: 0,
                max_active_ragdolls: max_active,
            },
            debug: RagdollDebugOptions::default(),
        }
    }

    /// Initializes the [`RagdollBuilder`] from an explicit skeleton and T-pose
    /// clip (also uploads `self.scripts` if not already uploaded).
    pub fn init_builder_with(&mut self, skeleton: &Skeleton, t_pose: &AnimClip) {
        let mut b = self.bullet.borrow_mut();
        let builder = RagdollBuilder::initialize(
            &mut b,
            skeleton,
            t_pose,
            &self.scripts.body_dynamics,
        );
        self.builder = Some(builder);
        self.builder_attempted = true;
    }

    /// Lazily initializes [`RagdollBuilder`] from the game's `Skinned/MaleBody`
    /// skeleton and `Bob/Bob_TPose` animation clip.
    pub(crate) fn ensure_builder(&mut self, assets: &Assets) -> bool {
        if self.builder.is_some() {
            return true;
        }
        if self.builder_attempted {
            return false;
        }
        self.builder_attempted = true;
        if let Some((skeleton, t_pose)) = assets.ragdoll_tpose() {
            self.init_builder_with(&skeleton, &t_pose);
            true
        } else {
            false
        }
    }

    pub fn has_builder(&self) -> bool {
        self.builder.is_some()
    }

    /// Current `WorldSimulation.getBulletFrameNo()`.
    pub fn bullet_frame_no(&self) -> i32 {
        pz_vehicles::world_sim::state().bullet_frame_no as i32
    }

    /// Runs `AnimationPlayer::update_with_ragdoll` for `chr`, syncing
    /// `chr.ragdoll` (`CharacterRagdoll::track`) and `ACTIVE_SIMULATIONS`.
    /// Returns `false` if `RagdollBuilder` is not yet initialized (caller
    /// falls back to `player.update`).
    pub fn update_player_with_ragdoll(
        &mut self,
        player: &mut AnimationPlayer,
        controller: &mut Option<RagdollController>,
        chr: &mut LuaGameCharacter,
        cx: StateCx<'_, pz_lua_api::bind_characters::LuaItem>,
        dt: f32,
        multiplier: f32,
    ) -> bool {
        let Some(builder) = self.builder.as_ref() else {
            return false;
        };
        self.globals.max_active_ragdolls = pz_lua_api::bind_core::shared_core()
            .borrow()
            .get_max_active_ragdolls();
        chr.update_ragdoll_variables(cx.ctx);

        let bullet_frame_no = self.bullet_frame_no();
        let vars = ChrVarProxy::snapshot(chr, player);
        let vehicle_contact = chr.is_zombie().then(|| zombie_vehicle_contact(chr)).flatten();
        let is_zombie = chr.is_zombie();
        {
            let mut bullet = self.bullet.borrow_mut();
            let mut character = CharacterRagdoll::new(chr, cx);
            let mut frame = RagdollFrame {
                env: RagdollEnv {
                    bullet: &mut bullet,
                    globals: &mut self.globals,
                    builder,
                    character: &mut character,
                    bullet_frame_no,
                    time_delta: dt,
                    debug: self.debug,
                },
                controller,
                multiplier,
            };
            player.update_with_ragdoll(dt, Some(&vars), &mut frame);
            if is_zombie {
                if let Some(ctrl) = frame.controller.as_mut() {
                    ctrl.vehicle_collision(&mut frame.env, vehicle_contact);
                }
            }
        }
        CharacterRagdoll::track(chr, player, controller.as_ref());
        publish_active_simulations(self.globals.number_of_active_simulations);
        true
    }

    /// Releases a character's [`RagdollController`] from the shared Bullet
    /// world before dropping or replacing its [`AnimationPlayer`].
    pub fn release_controller(
        &mut self,
        player: &mut AnimationPlayer,
        controller: &mut Option<RagdollController>,
        chr_id: i32,
    ) {
        if controller.is_none() {
            return;
        }
        let Some(builder) = self.builder.as_ref() else {
            *controller = None;
            return;
        };
        let bullet_frame_no = self.bullet_frame_no();
        let mut bullet = self.bullet.borrow_mut();
        let mut dummy = ReleaseCharacter(chr_id);
        let mut frame = RagdollFrame {
            env: RagdollEnv {
                bullet: &mut bullet,
                globals: &mut self.globals,
                builder,
                character: &mut dummy,
                bullet_frame_no,
                time_delta: 0.0,
                debug: self.debug,
            },
            controller,
            multiplier: 1.0,
        };
        player.release_ragdoll_controller(Some(&mut frame));
        publish_active_simulations(self.globals.number_of_active_simulations);
    }

    /// Resets ragdoll counters on `IngameState.exit` (call before
    /// `WorldVehicles::reset()` destroys the Bullet world).
    pub fn reset(&mut self) {
        self.globals.number_of_active_simulations = 0;
        self.builder = None;
        self.builder_attempted = false;
        publish_active_simulations(0);
    }
}

/// Resolves `RagdollController.vehicleCollision(zombie, collidedVehicle)` inputs:
/// `Some((is_collided, touching))` when a live cell vehicle contacts `chr`, or
/// `None` when no live vehicle is in contact (clearing `was_contacting_vehicle`
/// and resetting ragdoll body dynamics in Bullet).
pub(crate) fn zombie_vehicle_contact(chr: &LuaGameCharacter) -> Option<(bool, bool)> {
    let vehicles = pz_vehicles::manager::cell_vehicles();
    if vehicles.is_empty() {
        return None;
    }
    let (x, y, z) = (chr.vars.x, chr.vars.y, chr.vars.z);
    let flagged = chr.vars.vehicle_collision || chr.vars.collided_with_vehicle;
    let mut best: Option<(bool, bool)> = None;
    for v in &vehicles {
        if (v.z() - z).abs() >= 1.0 {
            continue;
        }
        let touching = if v.script().is_some() {
            v.poly_plus_radius().contains_point(x, y) || v.closest_point_on_poly(x, y).0 <= 0.25
        } else {
            let dx = v.x() - x;
            let dy = v.y() - y;
            dx * dx + dy * dy <= 4.0
        };
        if touching {
            return Some((flagged || touching, true));
        }
        if flagged && best.is_none() {
            let dx = v.x() - x;
            let dy = v.y() - y;
            if dx * dx + dy * dy <= 25.0 {
                best = Some((true, false));
            }
        }
    }
    best
}

/// Packs an [`AnimationPlayer`]'s current bone transforms into a
/// [`TargetSkeleton`] (`BallisticsTarget.updateSkeleton`).
pub fn pack_target_skeleton(player: &AnimationPlayer) -> TargetSkeleton {
    use pz_characters::character::firearm::ballistics::pack_bone_transforms;
    use pz_ragdoll::skeleton_bone::{self as sb, COUNT};

    let skel = player.skeleton();
    let mut bones = Vec::with_capacity(COUNT);
    for i in 0..COUNT {
        let entry = sb::find_bone(skel, i).map(|idx| {
            let mut bt = player.bone_transform_at(idx);
            let p = bt.position();
            let r = bt.rotation();
            ([p.x, p.y, p.z], [r.x, r.y, r.z, r.w])
        });
        bones.push(entry);
    }
    let mut packed = vec![0.0f32; COUNT * 7];
    pack_bone_transforms(&bones, &mut packed);
    TargetSkeleton {
        bone_count: COUNT as i32,
        angle: player.rendered_angle(),
        packed,
    }
}

/// Freezes a dying character's ragdoll pose into `IsoDeadBody.ragdoll`
/// (`pz_ragdoll::corpse::died_bone_transforms`), returning `None` when the
/// character did not ragdoll or has no usable pose.
pub fn freeze_corpse_ragdoll(
    player: &AnimationPlayer,
    ragdoll_fall: bool,
) -> Option<Vec<SavedBoneTransform>> {
    let usable = pz_ragdoll::corpse::can_use_current_pose_for_corpse(player, false, true);
    let died = pz_ragdoll::corpse::died_bone_transforms(player, ragdoll_fall, usable);
    if died.is_empty() {
        return None;
    }
    let saved = died
        .into_iter()
        .enumerate()
        .map(|(idx, mut bt)| {
            let (p, r, s) = bt.prs();
            SavedBoneTransform {
                bone: idx as i32,
                values: [p.x, p.y, p.z, r.x, r.y, r.z, r.w, s.x, s.y, s.z],
            }
        })
        .collect();
    Some(saved)
}

/// Minimal [`RagdollCharacter`] used when releasing an already-detached
/// `RagdollController` from Bullet on character removal/unload.
struct ReleaseCharacter(i32);

impl RagdollCharacter for ReleaseCharacter {
    fn id(&self) -> i32 {
        self.0
    }
    fn position(&self) -> Vec3 {
        Vec3::default()
    }
    fn forward_direction_angle(&self) -> f32 {
        0.0
    }
    fn can_ragdoll(&self) -> bool {
        false
    }
    fn set_ragdoll_fall(&mut self, _v: bool) {}
    fn set_use_physic_hit_reaction(&mut self, _v: bool) {}
    fn is_fall_on_front(&self) -> bool {
        false
    }
    fn set_fall_on_front(&mut self, _v: bool) {}
    fn set_on_floor(&mut self, _v: bool) {}
    fn do_deferred_movement_from_ragdoll(&mut self, _player: &AnimationPlayer, _m: Vec3) {}
}

/// Read-only snapshot of a character's animation variables so
/// `AnimationPlayer::update_with_ragdoll` can read `vars` while
/// `CharacterRagdoll` holds `&mut LuaGameCharacter`.
struct ChrVarProxy {
    vars: pz_anim::MapVariableSource,
}

impl ChrVarProxy {
    fn snapshot(chr: &LuaGameCharacter, player: &AnimationPlayer) -> Self {
        use pz_anim::AnimationVariableSource;
        let mut vars = pz_anim::MapVariableSource::new();
        for name in [
            "canRagdoll",
            "usePhysicHitReaction",
            "hitreaction",
            "HitReaction",
            "bDead",
            "bFalling",
            "bOnFloor",
            "fallOnFront",
            "isRagdoll",
        ] {
            if let Some(v) = chr.get_variable_named(name) {
                vars.set_variable(name, v);
            }
        }
        // Preserve any custom `m_TrackTimeToVariable` variables referenced by
        // active animation tracks so non-ragdolling tracks keep reading them.
        for track in player.tracks() {
            if let Some(name) = track.track_time_to_variable.as_deref().filter(|s| !s.is_empty()) {
                if let Some(v) = chr.get_variable_named(name) {
                    vars.set_variable(name, v);
                }
            }
        }
        Self { vars }
    }
}

impl pz_anim::AnimationVariableSource for ChrVarProxy {
    fn get_variable(
        &self,
        handle: &pz_anim::AnimationVariableHandle,
    ) -> Option<pz_anim::VariableValue> {
        self.vars.get_variable(handle)
    }
}

fn collect_txt(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                collect_txt(&p, out);
            } else if p.extension().is_some_and(|e| e == "txt") {
                out.push(p);
            }
        }
    }
}

impl IsoWorld {
    /// Initializes [`WorldRagdolls`] sharing the Bullet physics world created
    /// by [`IsoWorld::init_vehicles`].
    pub fn init_ragdolls(&mut self) {
        let Some(bullet) = self.vehicles_runtime.as_ref().map(|v| v.bullet_rc()) else {
            return;
        };
        self.ragdolls = Some(WorldRagdolls::init(&self.game_dir, bullet));
    }

    /// Releases all live ragdoll controllers from `WorldModels` and resets
    /// [`WorldRagdolls`] before `WorldVehicles::reset()` destroys Bullet.
    pub fn reset_ragdolls(&mut self) {
        if let (Some(models), Some(ragdolls)) = (self.models.as_mut(), self.ragdolls.as_mut()) {
            models.release_all_ragdolls(ragdolls);
        }
        if let Some(mut r) = self.ragdolls.take() {
            r.reset();
        }
    }
}

#[cfg(test)]
mod tests;
