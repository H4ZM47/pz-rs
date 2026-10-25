//! Corpse rot, the corpse registry, corpse counts and flies on
//! hand-built squares.

use super::corpse_count::get_corpse_count;
use super::flies_sound::{
    EmitterId, FliesEnv, FliesPlayer, FliesSound, FliesSoundHooks, FliesWorld,
};
use super::*;
use crate::chunk::{IsoChunk, IsoGridSquare};
use crate::isoregion::IsoRegions;
use crate::java::JavaLongHashMap;
use crate::meta_grid::IsoMetaGrid;
use crate::save::visual::{HumanVisual, MovingBase};
use crate::sprite::{test_tables, IsoSpriteManager};
use crate::square_mutation::{
    DoorTables, NoMutationHooks, PathfindSquareHooks, RenderSquareHooks, SquareMutationHooks,
};
use crate::GameRand;

/// `java.util.HashMap<Long, _>`'s key order over 20000 seeded puts and
/// removes, hashed every 97 steps, against the JDK's (run by a local
/// harness): list bins, resizes, and (third run) tree bins.
#[test]
fn long_hash_map_matches_jdk() {
    let want: [(usize, u64); 3] = [
        (1978, 3747404802781115502),
        (11524, 16873740733146024426),
        (281, 16367519587564365615),
    ];
    for (run, &(size, hash)) in want.iter().enumerate() {
        let mut s = 0x9E3779B97F4A7C15u64.wrapping_add(run as u64);
        let mut m = JavaLongHashMap::new();
        let mut h = 0xcbf29ce484222325u64;
        let span: u64 = [3000, 70000, 400][run];
        for i in 0..20000 {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let r = s >> 33;
            let mut key = (r % span) as i16 as i64;
            if run == 2 {
                key = ((r % span) * 64) as i64;
            }
            if (r >> 20).is_multiple_of(3) {
                m.remove(key);
            } else {
                m.insert(key);
            }
            if (i as u32).is_multiple_of(97) {
                for k in m.order() {
                    h ^= k as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
        }
        assert_eq!((m.len(), h), (size, hash), "run {run}");
    }
}

/// Every `next(max)` bound asked, answering from `answers` (then 1).
#[derive(Default)]
struct Script {
    answers: Vec<i32>,
    asked: Vec<i32>,
}
impl GameRand for Script {
    fn next(&mut self, max: i32) -> i32 {
        self.asked.push(max);
        if self.answers.is_empty() {
            1.min(max - 1).max(0)
        } else {
            self.answers.remove(0)
        }
    }
}

/// Render invalidations and flies changes.
#[derive(Default)]
struct R(Vec<String>);
impl RenderSquareHooks for R {
    fn invalidate_render_chunk_level(&mut self, wx: i32, wy: i32, level: i32, flags: u64) {
        self.0.push(format!("render {wx},{wy},{level} {flags}"));
    }
    fn flies_changed(&mut self, p: SquarePos, flies: bool) {
        self.0
            .push(format!("flies {},{},{} {flies}", p.0, p.1, p.2));
    }
}

struct Ages(f64, Vec<String>);
impl SquareMutationHooks for Ages {
    fn world_age_hours(&self) -> Option<f64> {
        Some(self.0)
    }
    fn corpse_added_to_world(&mut self, p: SquarePos, _: &mut IsoDeadBody) {
        self.1.push(format!("added {},{},{}", p.0, p.1, p.2));
    }
    fn corpse_removed_from_world(&mut self, p: SquarePos, _: &mut IsoDeadBody) {
        self.1.push(format!("removed {},{},{}", p.0, p.1, p.2));
    }
}

struct Hooks {
    created: usize,
    visual: usize,
}
impl CorpseHooks for Hooks {
    fn create_item(&mut self, _: &str, _: &mut dyn GameRand) -> Option<InventoryItem> {
        self.created += 1;
        None
    }
    fn corpse_visual_changed(&mut self, _: SquarePos, _: &IsoDeadBody) {
        self.visual += 1;
    }
}

struct W {
    cell: IsoCell,
    meta: IsoMetaGrid,
    m: IsoSpriteManager,
    regions: IsoRegions,
    doors: DoorTables,
}

impl W {
    /// Chunk (0, 0), level 0, every square present, counts loaded.
    fn new() -> W {
        let mut cell = IsoCell::default();
        let mut ch = IsoChunk::new(0, 0);
        for i in 0..64 {
            ch.set_square(i % 8, i / 8, 0, Some(IsoGridSquare::new(i % 8, i / 8, 0)));
        }
        cell.chunks.insert((0, 0), ch);
        chunk_loaded(&mut cell, 0, 0);
        let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
        meta.create_cell(0, 0);
        W {
            cell,
            meta,
            m: IsoSpriteManager::new(test_tables()),
            regions: IsoRegions::new(13),
            doors: DoorTables {
                x_offset: [[[0; 4]; 2]; 2],
                y_offset: [[[0; 4]; 2]; 2],
            },
        }
    }

    fn env<'a>(
        &'a mut self,
        rand: &'a mut dyn GameRand,
        world: &'a mut dyn SquareMutationHooks,
        path: &'a mut dyn PathfindSquareHooks,
        render: &'a mut dyn RenderSquareHooks,
    ) -> MutationEnv<'a> {
        MutationEnv {
            cell: &mut self.cell,
            meta: &self.meta,
            sprites: &mut self.m,
            regions: &mut self.regions,
            rand,
            doors: &self.doors,
            world,
            pathfind: path,
            render,
            num_players: 1,
        }
    }
}

fn body(x: f32, y: f32, zombie: bool) -> IsoDeadBody {
    IsoDeadBody {
        base: MovingBase {
            offset_x: 0.0,
            offset_y: 0.0,
            x,
            y,
            z: 0.0,
            dir: 0,
            mod_data: None,
        },
        female: false,
        was_zombie: zombie,
        animal: None,
        object_id: (-1, 4),
        persistent_outfit_id: 0,
        desc: None,
        visual: CorpseVisual::Human(Box::new(HumanVisual::new(4))),
        container: Some(crate::objects::ItemContainer::new(Some("inventorymale"))),
        worn_items: vec![("Base.Tshirt".into(), 1)],
        attached_items: vec![],
        death_time: -1.0,
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
    }
}

fn settings() -> CorpseSettings {
    CorpseSettings {
        hours_for_corpse_removal: 30.0,
        season: "Late Summer".into(),
        ..Default::default()
    }
}

fn rot(b: &IsoDeadBody) -> i8 {
    match &b.visual {
        CorpseVisual::Human(v) => v.zombie_rot_stage,
        _ => unreachable!(),
    }
}

#[test]
fn add_corpse_registers_counts_and_clamps_death_time() {
    let mut w = W::new();
    let (mut rand, mut path, mut render) = (Script::default(), NoMutationHooks, R::default());
    let mut world = Ages(7.5, Vec::new());
    {
        let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
        env.cell.dead_bodies.last_id = 100;
        env.add_corpse((3, 3, 0), body(3.5, 3.5, true));
        env.add_corpse((3, 3, 0), body(3.2, 3.7, false));
    }
    let sq = w.cell.get_grid_square(3, 3, 0).unwrap();
    assert_eq!(
        sq.static_moving
            .iter()
            .map(|b| (b.object_id.0, b.death_time))
            .collect::<Vec<_>>(),
        vec![(101, 7.5), (102, 7.5)]
    );
    assert_eq!(w.cell.dead_bodies.ids(), vec![101, 102]);
    assert_eq!(get_corpse_count(&w.cell, &w.meta, 0, 0, 0, None, None), 2);
    assert_eq!(world.1, vec!["added 3,3,0", "added 3,3,0"]);
    {
        let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
        env.remove_corpse((3, 3, 0), 0).unwrap();
    }
    assert_eq!(w.cell.dead_bodies.ids(), vec![102]);
    assert_eq!(get_corpse_count(&w.cell, &w.meta, 0, 0, 0, None, None), 1);
}

#[test]
fn zombie_corpse_rots_spawns_maggot_draws_and_is_removed() {
    let mut w = W::new();
    let (mut rand, mut path, mut render) = (Script::default(), NoMutationHooks, R::default());
    let mut world = Ages(0.0, Vec::new());
    let s = settings();
    let mut hooks = Hooks {
        created: 0,
        visual: 0,
    };
    w.env(&mut rand, &mut world, &mut path, &mut render)
        .add_corpse((2, 2, 0), body(2.5, 2.5, true));
    rand.asked.clear();
    render.0.clear();
    let step = |w: &mut W,
                rand: &mut Script,
                world: &mut Ages,
                render: &mut R,
                hooks: &mut Hooks,
                age: f64| {
        let mut path = NoMutationHooks;
        let mut env = w.env(rand, world, &mut path, render);
        update_bodies(&mut env, hooks, &s, age)
    };
    // Stage -1 -> 1: two decay steps of maggot draws (5, then 10; the
    // season names never equal "Summer").
    step(&mut w, &mut rand, &mut world, &mut render, &mut hooks, 5.0);
    assert_eq!(rand.asked, vec![100, 5, 10, 5, 10]);
    assert_eq!(rot(get_dead_body(&w.cell, (2, 2, 0)).unwrap()), 1);
    assert_eq!(render.0, vec!["render 0,0,0 2"]);
    rand.asked.clear();
    step(&mut w, &mut rand, &mut world, &mut render, &mut hooks, 9.9);
    assert!(rand.asked.is_empty());
    step(&mut w, &mut rand, &mut world, &mut render, &mut hooks, 15.0);
    assert_eq!(rand.asked, vec![100, 5, 10]);
    assert_eq!(rot(get_dead_body(&w.cell, (2, 2, 0)).unwrap()), 2);
    rand.asked.clear();
    step(&mut w, &mut rand, &mut world, &mut render, &mut hooks, 25.0);
    assert_eq!(rot(get_dead_body(&w.cell, (2, 2, 0)).unwrap()), 3);
    rand.asked.clear();
    // Three stages: the skeleton roll fails, and at 30 hours it is gone.
    step(&mut w, &mut rand, &mut world, &mut render, &mut hooks, 30.5);
    assert_eq!(rand.asked, vec![7]);
    assert!(get_dead_body(&w.cell, (2, 2, 0)).is_none());
    assert!(w.cell.dead_bodies.ids().is_empty());
    assert_eq!(get_corpse_count(&w.cell, &w.meta, 0, 0, 0, None, None), 0);
    assert_eq!(world.1.last().unwrap(), "removed 2,2,0");
    assert_eq!(hooks.created, 0);
}

#[test]
fn skeleton_stays_one_stage_longer() {
    let mut w = W::new();
    let (mut rand, mut path, mut render) = (Script::default(), NoMutationHooks, R::default());
    let mut world = Ages(0.0, Vec::new());
    let s = CorpseSettings {
        maggot_spawn: 3,
        ..settings()
    };
    let mut hooks = Hooks {
        created: 0,
        visual: 0,
    };
    w.env(&mut rand, &mut world, &mut path, &mut render)
        .add_corpse((2, 2, 0), body(2.5, 2.5, true));
    {
        let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
        update_bodies(&mut env, &mut hooks, &s, 25.0);
    }
    // stages == 3, `Rand.Next(7) == 0`, skin `Rand.Next(1, 3)`.
    rand.answers = vec![0, 1];
    rand.asked.clear();
    {
        let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
        update_bodies(&mut env, &mut hooks, &s, 31.0);
    }
    assert_eq!(rand.asked, vec![7, 2]);
    let b = get_dead_body(&w.cell, (2, 2, 0)).unwrap();
    assert!(b.is_skeleton() && b.worn_items.is_empty());
    let CorpseVisual::Human(v) = &b.visual else {
        unreachable!()
    };
    assert_eq!((v.skin_texture, v.hair_model.as_deref()), (2, Some("")));
    {
        let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
        update_bodies(&mut env, &mut hooks, &s, 39.0);
    }
    assert!(get_dead_body(&w.cell, (2, 2, 0)).is_some());
    {
        let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
        update_bodies(&mut env, &mut hooks, &s, 40.0);
    }
    assert!(get_dead_body(&w.cell, (2, 2, 0)).is_none());
}

#[test]
fn player_corpses_wait_for_the_server_option() {
    let mut w = W::new();
    let (mut rand, mut path, mut render) = (Script::default(), NoMutationHooks, R::default());
    let mut world = Ages(0.0, Vec::new());
    let mut hooks = Hooks {
        created: 0,
        visual: 0,
    };
    w.env(&mut rand, &mut world, &mut path, &mut render)
        .add_corpse((2, 2, 0), body(2.5, 2.5, false));
    let mut env = w.env(&mut rand, &mut world, &mut path, &mut render);
    update_bodies(&mut env, &mut hooks, &settings(), 100.0);
    assert!(get_dead_body(&w.cell, (2, 2, 0)).is_some());
}

#[derive(Default)]
struct Sound(Vec<String>, u64);
impl FliesSoundHooks for Sound {
    fn sickness_from_corpses_rate(&self, n: i32) -> f32 {
        if n > 2 {
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
    fn play_sound_looped_impl(&mut self, e: EmitterId, name: &str) {
        self.0.push(format!("play{e} {name}"));
    }
    fn set_volume_all(&mut self, e: EmitterId, v: f32) {
        self.0.push(format!("vol{e} {v}"));
    }
    fn set_pos(&mut self, e: EmitterId, x: f32, y: f32, z: f32) {
        self.0.push(format!("pos{e} {x},{y},{z}"));
    }
    fn stop_all(&mut self, e: EmitterId) {
        self.0.push(format!("stop{e}"));
    }
}

#[test]
fn flies_follow_the_corpse_count_and_the_player() {
    let mut w = W::new();
    let (mut rand, mut path, mut render) = (Script::default(), NoMutationHooks, R::default());
    let mut world = Ages(0.0, Vec::new());
    for p in [(1, 1, 0), (5, 2, 0), (6, 6, 0)] {
        w.env(&mut rand, &mut world, &mut path, &mut render)
            .add_corpse(p, body(p.0 as f32 + 0.5, p.1 as f32 + 0.5, true));
    }
    let mut flies = FliesSound::new();
    flies.chunk_loaded(0, 0);
    let mut sound = Sound::default();
    let env = FliesEnv {
        decaying_corpse_health_impact: 3,
        zombie_health_impact: false,
        food_sickness_decrease: 0.5,
        thirty_fps_multiplier: 50.0,
        num_players: 1,
    };
    let player = |x: i32, y: i32| FliesPlayer {
        index: 0,
        xi: x,
        yi: y,
        zi: 0,
        square: Some((x, y, 0)),
        deaf: false,
    };
    let mut r = R::default();
    let run = |w: &mut W, r: &mut R, flies: &mut FliesSound, sound: &mut Sound, p: FliesPlayer| {
        let mut fw = FliesWorld {
            cell: &mut w.cell,
            meta: &w.meta,
            render: r,
            hooks: sound,
        };
        flies.update(&mut fw, &[Some(p)], &env);
    };
    run(&mut w, &mut r, &mut flies, &mut sound, player(4, 4));
    // The middle (3 / 2 = 1) of the corpse squares, row by row: (5, 2).
    assert!(w.cell.get_grid_square(5, 2, 0).unwrap().has_flies);
    assert_eq!(r.0, vec!["render 0,0,0 64", "flies 5,2,0 true"]);
    assert_eq!(
        sound.0,
        vec!["emitter1 5,2,0", "play1 CorpseFlies", "vol1 0", "vol1 0.5"]
    );
    sound.0.clear();
    // Fading in: done after the second frame (no last volume set).
    run(&mut w, &mut r, &mut flies, &mut sound, player(4, 4));
    assert!(sound.0.is_empty());
    // Away from the chunks: deref, fade out, stop.
    r.0.clear();
    run(&mut w, &mut r, &mut flies, &mut sound, player(100, 100));
    assert!(!w.cell.get_grid_square(5, 2, 0).unwrap().has_flies);
    assert_eq!(r.0, vec!["render 0,0,0 128", "flies 5,2,0 false"]);
    assert_eq!(sound.0, vec!["vol1 0.5"]);
    run(&mut w, &mut r, &mut flies, &mut sound, player(100, 100));
    assert_eq!(sound.0, vec!["vol1 0.5", "stop1"]);
    assert!(flies.playing().is_empty());
}
