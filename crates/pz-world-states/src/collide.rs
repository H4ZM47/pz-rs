//! The mover-aware collision hooks of [`StateWorld`]: `testCollideAdjacent`
//! with the special objects (pz-world's `CollideEnv`), the hoppable checks
//! after a square-edge collision, the vegetation rustle of
//! `getGlobalMovementMod`, character fire and the path-finding host hooks.
//!
//! [`StateWorld`]: pz_characters::character::state::StateWorld

use pz_characters::character::states::ids;
use pz_characters::{CharacterKind, GameCharacter};
use pz_pathfind::behavior::PathFindBehavior2;
use pz_pathfind::mover::{MoverId, ObjectRef};
use pz_world::fire::manager::{FireFlags, FireHooks, FireMutation, FireSettings, IsoFireManager};
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::square::{
    CollideAdjacent, CollideEnv, CollideMover, CollidedFlags, EdgeFacing, HoppableAction,
    HoppableHooks, MoverKind, MoverStates,
};
use pz_world::{IsoCell, IsoDirections, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};

use crate::{StatesWorldAdapter, Thumper};

/// The squares and tables, read only (what the path-finding host builds
/// its `MainThreadMap` from).
#[derive(Clone, Copy)]
pub struct WorldView<'w> {
    pub cell: &'w IsoCell,
    pub meta: &'w IsoMetaGrid,
    pub sprites: &'w IsoSpriteManager,
}

/// The fire simulation parts the engine owns (the cell, meta grid and
/// sprites are the adapter's).
pub struct StatesFire<'e> {
    pub manager: &'e mut IsoFireManager,
    pub hooks: &'e mut dyn FireHooks,
    /// The square mutations a fire started by a character can lead to.
    pub mutation: FireMutation<'e>,
    pub flags: FireFlags,
    pub settings: FireSettings,
}

/// `RenderEffectType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderEffectType {
    HitTreeShudder,
    VegetationRustle,
    HitDoor,
}

/// A new `ObjectRenderEffects` (`getNew`): its type, `totalMaxTime`,
/// `isTree` and `isBig`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NewRenderEffect {
    pub kind: RenderEffectType,
    pub total_max_time: f32,
    pub is_tree: bool,
    pub is_big: bool,
}

/// A character as `testCollideAdjacent` and the hoppable checks read it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CollideMoverInfo {
    pub id: i32,
    pub kind: MoverKind,
    /// `IsoPlayer.isNoClip()`.
    pub no_clip: bool,
    /// `IsoZombie.crawling`.
    pub crawling: bool,
    /// `collidedN/S/W/E`.
    pub collided: CollidedFlags,
    pub states: MoverStates,
    /// `IsoAnimal.adef.canClimbFences`.
    pub can_climb_fences: bool,
    /// The square of `getSitOnFurnitureObject()`.
    pub sit_on_square: Option<(i32, i32, i32)>,
    /// `getPathFindBehavior2()`'s sit-on-furniture goal square, while the
    /// character has a path (`hasPath()`).
    pub path_ignore_square: Option<(i32, i32, i32)>,
}

impl CollideMover for CollideMoverInfo {
    fn mover_kind(&self) -> MoverKind {
        self.kind
    }
    fn is_no_clip(&self) -> bool {
        self.no_clip
    }
    fn should_ignore_collision_with_square(&self, sq: Option<(i32, i32, i32)>) -> bool {
        sq.is_some() && (sq == self.sit_on_square || sq == self.path_ignore_square)
    }
    fn is_crawling(&self) -> bool {
        self.crawling
    }
    fn collided(&self) -> CollidedFlags {
        self.collided
    }
    fn states(&self) -> MoverStates {
        self.states
    }
    fn can_climb_fences(&self) -> bool {
        self.can_climb_fences
    }
}

/// The mover kind of a character kind (`IsoSurvivor` is no `IsoPlayer`).
pub fn mover_kind(kind: CharacterKind) -> MoverKind {
    match kind {
        CharacterKind::Zombie => MoverKind::Zombie,
        CharacterKind::Player => MoverKind::Player,
        CharacterKind::Animal => MoverKind::Animal,
        CharacterKind::Survivor => MoverKind::Other,
    }
}

impl CollideMoverInfo {
    /// The collision view of a character. `path` is its
    /// `PathFindBehavior2` when it has a path (`hasPath()`); the
    /// sit-on-furniture square and `adef.canClimbFences` are the owner's.
    pub fn of<I: 'static>(chr: &GameCharacter<I>, path: Option<&PathFindBehavior2>) -> Self {
        let st = |id| chr.is_current_state_id(id);
        let path_ignore_square = path.and_then(|b| {
            b.goal_sit_on_furniture_object()
                .map(|o| (o.x, o.y, o.z))
                .filter(|&sq| b.should_ignore_collision_with_square(sq))
        });
        CollideMoverInfo {
            id: chr.id,
            kind: mover_kind(chr.kind),
            no_clip: chr.vars.no_clip,
            crawling: chr.vars.crawling,
            collided: CollidedFlags {
                n: chr.vars.collided_n,
                s: chr.vars.collided_s,
                w: chr.vars.collided_w,
                e: chr.vars.collided_e,
            },
            states: MoverStates {
                attack: st(ids::ATTACK),
                stagger_back: st(ids::STAGGER_BACK),
                climb_over_fence: st(ids::CLIMB_OVER_FENCE),
                climb_through_window: st(ids::CLIMB_THROUGH_WINDOW),
                crawling_zombie_turn: st(ids::CRAWLING_ZOMBIE_TURN),
                animal_attack: st(ids::ANIMAL_ATTACK),
                animal_climb_over_fence: st(ids::ANIMAL_CLIMB_OVER_FENCE),
            },
            can_climb_fences: false,
            sit_on_square: None,
            path_ignore_square,
        }
    }
}

/// The kind encoded in a character's `MoverId` (`PathMover::mover_id`).
fn kind_of_mover(m: MoverId) -> MoverKind {
    match m.0 >> 32 {
        3 => MoverKind::Zombie,
        1 => MoverKind::Player,
        4 => MoverKind::Animal,
        _ => MoverKind::Other,
    }
}

struct AdapterHoppableHooks<'s, 'a> {
    adapter: &'s StatesWorldAdapter<'a>,
    thumper: Option<Thumper>,
}

impl HoppableHooks for AdapterHoppableHooks<'_, '_> {
    fn thumpable_for(&self, sq: &IsoGridSquare, obj: &IsoObject) -> bool {
        let r = ObjectRef {
            x: sq.x,
            y: sq.y,
            z: sq.z,
            id: obj.id,
        };
        self.adapter
            .thumpable_for(r, self.thumper.as_ref())
            .is_some()
    }
    fn bent_fences_enabled(&self) -> bool {
        self.adapter.engine.bent_fences_enabled()
    }
    fn is_unbent_object(&self, sq: &IsoGridSquare, obj: &IsoObject, facing: EdgeFacing) -> bool {
        if let Some(bf) = self.adapter.engine.bent_fences() {
            return self.adapter.is_unbent_object(&bf, (sq, obj), facing);
        }
        self.adapter
            .env()
            .sprite_name(obj)
            .is_some_and(|n| self.adapter.engine.is_unbent_fence(n, facing))
    }
}

impl StatesWorldAdapter<'_> {
    /// The mover-aware collision queries over the cell.
    pub fn collide_env(&self) -> CollideEnv<'_> {
        CollideEnv::new(self.cell, self.sprites, self.meta)
    }

    /// The read-only squares and tables.
    pub fn world_view(&self) -> WorldView<'_> {
        WorldView {
            cell: self.cell,
            meta: self.meta,
            sprites: self.sprites,
        }
    }

    /// The collision view of the character `mover`: the engine's
    /// [`CollideMoverInfo`], else what [`crate::StatesEngine::character`]
    /// and the `MoverId` tell.
    pub fn collide_mover_info(&self, mover: MoverId) -> CollideMoverInfo {
        let id = mover.0 as u32 as i32;
        if let Some(m) = self.engine.collide_mover(id) {
            return m;
        }
        let t = self.engine.character(id);
        CollideMoverInfo {
            id,
            kind: kind_of_mover(mover),
            crawling: t.is_some_and(|t| t.crawling),
            ..Default::default()
        }
    }

    /// `square.testCollideAdjacent(mover, dx, dy, dz)` from `from`, with
    /// the special object hit. `None` when there is no square at `from`.
    pub fn test_collide_adjacent_mover(
        &self,
        mover: &dyn CollideMover,
        from: (i32, i32, i32),
        d: (i32, i32, i32),
    ) -> Option<CollideAdjacent> {
        let env = self.collide_env();
        let sq_owner = self.cell.get_grid_square(from.0, from.1, from.2)?;
        Some(env.test_collide_adjacent(&sq_owner, Some(mover), d.0, d.1, d.2))
    }

    /// `checkBreakHoppable()`, `checkHitHoppable()` and
    /// `checkBreakBendableFence(current)` of `mover` after a collision
    /// from `last`.
    pub fn hoppable_actions_for(
        &self,
        mover: &CollideMoverInfo,
        last: (i32, i32, i32),
    ) -> Vec<HoppableAction> {
        let Some(sq_owner) = self.cell.get_grid_square(last.0, last.1, last.2) else {
            return Vec::new();
        };
        let sq = sq_owner.borrow();
        let mut thumper = self.engine.character(mover.id);
        if thumper.is_none() && mover.kind == MoverKind::Zombie {
            thumper = Some(Thumper {
                id: mover.id,
                zombie: true,
                crawling: mover.crawling,
                current_square: Some(last),
                ..Default::default()
            });
        }
        let hooks = AdapterHoppableHooks {
            adapter: self,
            thumper,
        };
        drop(sq);
        self.collide_env()
            .hoppable_actions(mover, &sq_owner, &hooks)
    }

    /// The trees and bushes past the first object of a square with a tree
    /// or a bush (`getGlobalMovementMod`'s rustle targets), with
    /// `isBig` for trees (`size > 4`); empty when it has neither.
    pub fn rustle_targets(&self, sq: (i32, i32, i32)) -> Vec<(ObjectRef, bool, bool)> {
        let Some(s_owner) = self.cell.get_grid_square(sq.0, sq.1, sq.2) else {
            return Vec::new();
        };
        let s = s_owner.borrow();
        let is_bush = |o: &IsoObject| {
            o.sprite.is_some_and(|i| {
                let spr = &self.sprites.sprites[i];
                spr.tileset_name.as_deref() == Some("f_bushes_1")
                    || spr.properties.get(&self.sprites.alias, "Bush").is_some()
            })
        };
        let tree = self.sprites.tables.ty.tree;
        if !(s.has_type(tree) || s.objects.iter().any(|o| is_bush(&o.borrow()))) {
            return Vec::new();
        }
        s.objects
            .iter()
            .map(|o| o.borrow())
            .skip(1)
            .filter_map(|o| {
                let r = ObjectRef {
                    x: s.x,
                    y: s.y,
                    z: s.z,
                    id: o.id,
                };
                match &o.kind {
                    ObjectKind::Tree(t) => Some((r, true, t.size > 4)),
                    _ if is_bush(&o) => Some((r, false, false)),
                    _ => None,
                }
            })
            .collect()
    }
}

impl StatesWorldAdapter<'_> {
    /// `square.hasNonHoppableWall(isNorth)`: a window on that edge, or a
    /// wall there that is not a hoppable or tall hoppable fence.
    pub fn has_non_hoppable_wall(&self, sq: &IsoGridSquare, north: bool) -> bool {
        let m = &*self.sprites;
        let t = &m.tables;
        for o in &sq.objects {
            let o = o.borrow();
            let Some(si) = o.sprite else { continue };
            let sp = &m.sprites[si];
            let p = &sp.properties;
            if p.has(t.flag.WallSE) {
                continue;
            }
            if let ObjectKind::Window(w) = &o.kind {
                if w.north == north {
                    return true;
                }
            }
            let has = |n: &str| t.flag_from_string(n).is_some_and(|f| p.has(f));
            let hit = if north {
                (sp.cut_n || has("WallN") || has("WallNTrans"))
                    && !has("HoppableN")
                    && !has("TallHoppableN")
            } else {
                (sp.cut_w || has("WallW") || has("WallWTrans"))
                    && !has("HoppableW")
                    && !has("TallHoppableW")
            };
            if hit {
                return true;
            }
        }
        false
    }

    /// `square.isPlayerAbleToHopWallTo(dir, oppositeSq)`.
    pub fn is_player_able_to_hop_wall_to(
        &self,
        sq: &IsoGridSquare,
        dir: IsoDirections,
        opp: &IsoGridSquare,
    ) -> bool {
        let adjacent = (sq.x, sq.y, sq.z) == (opp.x, opp.y, opp.z)
            || IsoDirections::ALL.iter().any(|&d| {
                sq.get_adjacent_square(d, self.cell).is_some_and(|a| {
                    let a = a.borrow();
                    (a.x, a.y, a.z) == (opp.x, opp.y, opp.z)
                })
            });
        if !adjacent {
            return false;
        }
        let m = &*self.sprites;
        let ty = &m.tables.ty;
        let above = |s: &IsoGridSquare| self.cell.get_grid_square(s.x, s.y, s.z + 1);
        if sq.has_stairs(m) && sq.has_type(ty.stairsTN) || sq.has_type(ty.stairsTW) {
            if let (Some(a), Some(oa)) = (above(sq), above(opp)) {
                return !match dir {
                    IsoDirections::N => self.has_non_hoppable_wall(&a.borrow(), true),
                    IsoDirections::S => self.has_non_hoppable_wall(&oa.borrow(), true),
                    IsoDirections::W => self.has_non_hoppable_wall(&a.borrow(), false),
                    IsoDirections::E => self.has_non_hoppable_wall(&oa.borrow(), false),
                    _ => false,
                };
            }
            return true;
        }
        !match dir {
            IsoDirections::N => self.has_non_hoppable_wall(sq, true),
            IsoDirections::S => self.has_non_hoppable_wall(opp, true),
            IsoDirections::W => self.has_non_hoppable_wall(sq, false),
            IsoDirections::E => self.has_non_hoppable_wall(opp, false),
            _ => false,
        }
    }

    /// `climbOverFence(dir)`'s checks for a character at `t` on `from`
    /// (`canClimbThroughHelper`, `isPlayerAbleToHopWallTo`) and what
    /// `ClimbOverFenceState.setParams` reads of the square beyond.
    /// TODO(bent fences): `BentFences.checkDamageHoppableFence` (only with
    /// bent fences enabled).
    pub fn fence_climb_for(
        &self,
        t: &Thumper,
        from: (i32, i32, i32),
        dir: IsoDirections,
    ) -> Option<pz_characters::character::state::FenceClimb> {
        let sq_owner = self.cell.get_grid_square(from.0, from.1, from.2)?;
        let sq = sq_owner.borrow();
        let opp_owner = sq.get_adjacent_square(dir, self.cell);
        let opp = opp_owner.as_ref().map(|s| s.borrow());
        let opp = opp.as_deref();
        let north = matches!(dir, IsoDirections::N | IsoDirections::S);
        if !self.can_climb_through_helper(t, &sq, opp, north) {
            return None;
        }
        let opp = opp?;
        if !self.is_player_able_to_hop_wall_to(&sq, dir, opp) {
            return None;
        }
        let m = &*self.sprites;
        Some(pz_characters::character::state::FenceClimb {
            counter: opp.has(m.tables.flag.solidtrans),
            solid_floor: opp.treat_as_solid_floor(m),
            sheet_rope: self.env().can_climb_down_sheet_rope(Some(opp)),
        })
    }

    /// The hoppable checks of `mover` (character `chr`) after a collision
    /// from `last`.
    pub fn hoppable_actions_dyn(
        &self,
        mover: &dyn CollideMover,
        chr: i32,
        last: (i32, i32, i32),
    ) -> Vec<HoppableAction> {
        let Some(sq_owner) = self.cell.get_grid_square(last.0, last.1, last.2) else {
            return Vec::new();
        };
        let mut thumper = self.engine.character(chr);
        if thumper.is_none() {
            // A real game always has the colliding character registered; a
            // missing one means the engine hook is not wired.
            warn_once(&MISSING_CHARACTER, || {
                format!(
                    "hoppable_actions_dyn: engine has no character {chr}; using a default thumper"
                )
            });
        }
        if thumper.is_none() && mover.mover_kind() == MoverKind::Zombie {
            thumper = Some(Thumper {
                id: chr,
                zombie: true,
                crawling: mover.is_crawling(),
                current_square: Some(last),
                ..Default::default()
            });
        }
        let hooks = AdapterHoppableHooks {
            adapter: self,
            thumper,
        };
        self.collide_env()
            .hoppable_actions(mover, &sq_owner, &hooks)
    }
}

static MISSING_CHARACTER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Logs `msg` once per flag in debug builds.
fn warn_once(flag: &std::sync::atomic::AtomicBool, msg: impl FnOnce() -> String) {
    if cfg!(debug_assertions) && !flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!("warning: {}", msg());
    }
}
