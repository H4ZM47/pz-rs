//! Parity against the original Java ragdoll code.
//!
//! A local harness (kept outside the repo) runs the game's
//! `AnimationPlayer`, `RagdollController` and `RagdollBuilder` against the
//! shipped physics library: a character plays a death clip, blends into
//! its ragdoll on a flat floor, settles, and its final pose is frozen and
//! replayed as `DeadBodyAtlas` does (`DeathPose`). It writes
//! `skeleton.txt` (skeleton and clips) and one log per scenario. Point
//! `PZ_RAGDOLL_ORACLE` at a scenario log, `PZ_RAGDOLL_SCRIPTS` at
//! `media/scripts/ragdolls`, and optionally `PZ_RAGDOLL_DEFINES` at the
//! `defineRagdoll*` arrays the game uploaded; without them this test does
//! nothing. The same scenario runs here with pz-bullet as the physics.

#![allow(clippy::needless_range_loop)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use pz_anim::clip::AnimClip;
use pz_anim::math::Matrix4f;
use pz_anim::AnimationPlayer;
use pz_bullet::{JavaCallbacks, PzBullet};
use pz_model::{AnimationClip, Keyframe, Quat, SkinningData};
use pz_ragdoll::controller::RagdollController;
use pz_ragdoll::{
    corpse, RagdollBuilder, RagdollCharacter, RagdollDebugOptions, RagdollEnv, RagdollFrame,
    RagdollGlobals, RagdollScripts, Vec3,
};

fn f(s: &str) -> f32 {
    s.parse().unwrap()
}

fn hx(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

fn load_skinning(path: &PathBuf) -> SkinningData {
    let text = std::fs::read_to_string(path).unwrap();
    let mut sd = SkinningData::default();
    let mut clip: Option<AnimationClip> = None;
    let m16 = |v: &[&str]| {
        let mut a = [0f32; 16];
        for (i, s) in v.iter().take(16).enumerate() {
            a[i] = f(s);
        }
        Matrix4f::load(&a).to_mat4()
    };
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        match p[0] {
            "bones" => {
                let n: usize = p[1].parse().unwrap();
                sd.bind_pose = vec![Default::default(); n];
                sd.inverse_bind_pose = vec![Default::default(); n];
                sd.bone_offset = vec![Default::default(); n];
                sd.skeleton_hierarchy = vec![-1; n];
            }
            "name" => {
                sd.bone_indices
                    .insert(p[2].to_string(), p[1].parse().unwrap());
            }
            "parent" => {
                sd.skeleton_hierarchy[p[1].parse::<usize>().unwrap()] = p[2].parse().unwrap()
            }
            "bind" => sd.bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "invbind" => sd.inverse_bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "offset" => sd.bone_offset[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "clip" => {
                if let Some(c) = clip.take() {
                    sd.animation_clips.insert(c.name.clone(), c);
                }
                clip = Some(AnimationClip {
                    name: p[1].into(),
                    duration: f(p[2]),
                    keep_last_frame: p[3] == "true",
                    ..Default::default()
                });
            }
            "k" => clip.as_mut().unwrap().keyframes.push(Keyframe {
                bone: p[1].parse().unwrap(),
                time: f(p[2]),
                position: [f(p[3]), f(p[4]), f(p[5])],
                rotation: Quat {
                    x: f(p[6]),
                    y: f(p[7]),
                    z: f(p[8]),
                    w: f(p[9]),
                },
                scale: [f(p[10]), f(p[11]), f(p[12])],
                bone_name: p.get(13).unwrap_or(&"").to_string(),
            }),
            _ => {}
        }
    }
    if let Some(c) = clip.take() {
        sd.animation_clips.insert(c.name.clone(), c);
    }
    sd
}

/// The harness's flat floor: a floor on every square of the chunks in the
/// active window, uploaded when the world asks for a level.
struct Floor {
    map_x: i32,
    map_y: i32,
    grid_w: i32,
    pending: HashSet<(i32, i32)>,
}

fn flat_chunk(wx: i32, wy: i32) -> Vec<u8> {
    let mut b = vec![8u8];
    for v in [wx as i16, wy as i16, 0, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    for y in 0..8u8 {
        for x in 0..8u8 {
            b.extend([x, y, 1, 7]);
        }
    }
    b.extend([0xff, 0xff]);
    b
}

impl JavaCallbacks for Floor {
    fn update_physics_for_level_if_needed(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool {
        let inside = wx >= self.map_x
            && wx < self.map_x + self.grid_w
            && wy >= self.map_y
            && wy < self.map_y + self.grid_w;
        if inside && level == 0 && self.pending.remove(&(wx, wy)) {
            to_bullet.push(flat_chunk(wx, wy));
            return true;
        }
        false
    }

    fn on_vehicle_constraint_impulse(&mut self, _c: i32, _a: i32, _b: i32, _imp: f32) {}
}

/// The harness's stand-in character (as plain as the Java one).
#[derive(Default)]
struct FakeChar {
    id: i32,
    x: f32,
    y: f32,
    z: f32,
    fwd: f32,
    fall_on_front: bool,
    ragdoll_fall: bool,
    on_floor: bool,
    hit_reaction: bool,
}

impl FakeChar {
    fn anim_angle(&self, p: &AnimationPlayer) -> f32 {
        if p.bone_transforms_need_first_frame() {
            self.fwd
        } else {
            p.angle()
        }
    }
}

impl RagdollCharacter for FakeChar {
    fn id(&self) -> i32 {
        self.id
    }
    fn position(&self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
    fn forward_direction_angle(&self) -> f32 {
        self.fwd
    }
    fn can_ragdoll(&self) -> bool {
        true
    }
    fn set_ragdoll_fall(&mut self, v: bool) {
        self.ragdoll_fall = v;
    }
    fn set_use_physic_hit_reaction(&mut self, v: bool) {
        self.hit_reaction = v;
    }
    fn is_fall_on_front(&self) -> bool {
        self.fall_on_front
    }
    fn set_fall_on_front(&mut self, v: bool) {
        self.fall_on_front = v;
    }
    fn set_on_floor(&mut self, v: bool) {
        self.on_floor = v;
    }
    fn do_deferred_movement_from_ragdoll(&mut self, _p: &AnimationPlayer, m: Vec3) {
        self.x += m.x;
        self.y += m.y;
        self.z += m.z;
    }
}

#[derive(Default)]
struct Diff {
    max: f32,
    first_bad: Option<String>,
    exact: usize,
    total: usize,
}

impl Diff {
    fn cmp(&mut self, what: &str, ours: f32, theirs: f32) {
        self.total += 1;
        if ours.to_bits() == theirs.to_bits() || (ours.is_nan() && theirs.is_nan()) {
            self.exact += 1;
            return;
        }
        let e = (ours - theirs).abs();
        if e > self.max || e.is_nan() {
            self.max = if e.is_nan() { f32::INFINITY } else { e };
        }
        if self.first_bad.is_none() {
            self.first_bad = Some(format!("{what}: ours {ours:e} java {theirs:e}"));
        }
    }
    fn flag(&mut self, what: &str, ours: &str, theirs: &str) {
        self.total += 1;
        if ours == theirs {
            self.exact += 1;
        } else if self.first_bad.is_none() {
            self.first_bad = Some(format!("{what}: ours {ours} java {theirs}"));
        }
    }
}

#[test]
fn matches_java_oracle() {
    let (Ok(log), Ok(scripts_dir)) = (
        std::env::var("PZ_RAGDOLL_ORACLE"),
        std::env::var("PZ_RAGDOLL_SCRIPTS"),
    ) else {
        eprintln!("PZ_RAGDOLL_ORACLE / PZ_RAGDOLL_SCRIPTS not set; skipping");
        return;
    };
    let log = PathBuf::from(log);
    let sd = Rc::new(load_skinning(&log.with_file_name("skeleton.txt")));
    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<Vec<&str>> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split(' ').collect())
        .collect();
    let line = |tag: &str| lines.iter().find(|l| l[0] == tag).unwrap().clone();

    // RagdollScript: the game's script files, checked against what the game uploaded.
    let mut scripts = RagdollScripts::default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&scripts_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    for p in &files {
        scripts.load_file(&std::fs::read_to_string(p).unwrap());
    }
    if let Ok(defines) = std::env::var("PZ_RAGDOLL_DEFINES") {
        for l in std::fs::read_to_string(defines)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let p: Vec<&str> = l.split(' ').collect();
            let java: Vec<u32> = p[3..].iter().map(|s| hx(s).to_bits()).collect();
            let ours: Vec<u32> = match p[0] {
                "defineRagdollBodyDynamics" => scripts.body_dynamics_params(),
                "defineRagdollConstraints" => scripts.constraint_params(),
                "defineRagdollAnchors" => scripts.anchor_params(),
                _ => scripts.body_part_info_params(),
            }
            .iter()
            .map(|v| v.to_bits())
            .collect();
            assert_eq!(ours, java, "{} differs", p[0]);
        }
        eprintln!("ragdoll script arrays match the game's uploads");
    }

    let sc = line("scenario");
    let clip = sc[1].to_string();
    let (dir_x, dir_y) = (hx(sc[2]), hx(sc[3]));
    let rs: i32 = sc[4].parse().unwrap();
    let ramp: i32 = sc[5].parse().unwrap();
    let frames: i32 = sc[6].parse().unwrap();
    let deferred = sc
        .get(8)
        .copied()
        .filter(|d| *d != "-")
        .map(|d| sd.bone_indices[d] as usize);
    let gt = line("gametime");
    let (time_delta, multiplier) = (hx(gt[1]), hx(gt[2]));

    let mut bullet = PzBullet::new();
    bullet.init_pz_bullet();
    scripts.to_bullet(&mut bullet, false);
    let (cx0, cy0) = (1300, 1200);
    let grid_w = 13;
    let mut floor = Floor {
        map_x: cx0 - grid_w / 2,
        map_y: cy0 - grid_w / 2,
        grid_w,
        pending: HashSet::new(),
    };
    bullet
        .init_world(-250, -250, 250, 250, -250 * 256, -250 * 256, false)
        .unwrap();
    bullet
        .activate_chunk_map(0, floor.map_x, floor.map_y, grid_w)
        .unwrap();
    for x in floor.map_x..floor.map_x + grid_w {
        for y in floor.map_y..floor.map_y + grid_w {
            floor.pending.insert((x, y));
        }
    }
    for x in floor.map_x..floor.map_x + grid_w {
        for y in floor.map_y..floor.map_y + grid_w {
            bullet.set_chunk_min_max_level(x, y, 0, 0).unwrap();
        }
    }

    // RagdollBuilder.Initialize.
    let mut probe = AnimationPlayer::new(sd.clone());
    let tpose: AnimClip = (*probe.clip("Bob_TPose").unwrap()).clone();
    let builder = RagdollBuilder::initialize(
        &mut bullet,
        probe.skeleton(),
        &tpose,
        &scripts.body_dynamics,
    );
    let b = line("builder");
    let hier: Vec<i32> = b[3..].iter().map(|s| s.parse().unwrap()).collect();
    assert_eq!(builder.bone_hierarchy, hier, "bone hierarchy");
    let t = line("tpose");
    let tp: Vec<u32> = t[2..].iter().map(|s| hx(s).to_bits()).collect();
    assert_eq!(
        builder
            .bone_transform_data
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        tp,
        "T-pose"
    );
    let v = line("vehdyn");
    let vd: Vec<u32> = v[2..].iter().map(|s| hx(s).to_bits()).collect();
    assert_eq!(
        builder
            .vehicle_body_dynamics_params
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        vd,
        "vehicle dynamics"
    );

    let mut ch = FakeChar {
        id: 42,
        x: (cx0 * 8) as f32 + 4.25,
        y: (cy0 * 8) as f32 + 3.5,
        z: 0.0,
        fwd: pz_anim::math::Vector2::direction(dir_x, dir_y),
        ..Default::default()
    };
    let mut player = AnimationPlayer::new(sd.clone());
    player.set_target_and_current_direction(dir_x, dir_y);
    let mut globals = RagdollGlobals::default();
    let mut slot: Option<RagdollController> = None;
    let mut frame_no = -1;
    let (mut a, mut r) = (None, None);
    let mut d = Diff::default();
    let mut d_bones = Diff::default();
    let mut idx = lines.iter().position(|l| l[0] == "frame").unwrap();
    for fr in 0..frames {
        if fr == 0 {
            a = player.play(&clip, false, false, -1.0);
            let t = player.track_mut(a.unwrap()).unwrap();
            t.set_blend_weight(1.0);
            if deferred.is_some() {
                t.set_deferred_bone(deferred, pz_anim::BoneAxis::Y);
            }
        }
        if fr == rs {
            r = player.play("Ragdoll", false, true, 5.0);
            let t = player.track_mut(r.unwrap()).unwrap();
            t.set_blend_weight(0.0);
            if deferred.is_some() {
                t.set_deferred_bone(deferred, pz_anim::BoneAxis::Y);
            }
        }
        if let Some(rid) = r {
            let w = 1f32.min((fr - rs + 1) as f32 / ramp as f32);
            if let Some(t) = player.track_mut(rid) {
                t.set_blend_weight(w);
            }
            if let Some(t) = a.and_then(|a| player.track_mut(a)) {
                t.set_blend_weight(1.0 - w);
            }
        }
        if fr > 0 {
            for _ in 0..2 {
                bullet.step_simulation(0.01, 0, 0.0, &mut floor).unwrap();
            }
            frame_no += 1;
        }
        let dt = hx(lines[idx][2]);
        {
            let mut host = RagdollFrame {
                env: RagdollEnv {
                    bullet: &mut bullet,
                    globals: &mut globals,
                    builder: &builder,
                    character: &mut ch,
                    bullet_frame_no: frame_no,
                    time_delta,
                    debug: RagdollDebugOptions::default(),
                },
                controller: &mut slot,
                multiplier,
            };
            player.update_with_ragdoll(dt, None, &mut host);
        }
        let tag = format!("frame {fr}");
        assert_eq!(lines[idx][0], "frame");
        let pl = &lines[idx + 1];
        d.cmp(&format!("{tag} angle"), player.angle(), hx(pl[1]));
        d.cmp(
            &format!("{tag} rendered angle"),
            player.rendered_angle(),
            hx(pl[2]),
        );
        d.cmp(
            &format!("{tag} ragdoll weight"),
            player.ragdoll_animation_weight(),
            hx(pl[3]),
        );
        let flags = format!(
            "{} {} {} {}",
            player.is_ragdolling(),
            player.is_ragdoll_simulation_active(),
            player.is_fully_ragdolling(),
            player.track_count()
        );
        d.flag(&format!("{tag} player flags"), &flags, &pl[4..8].join(" "));
        let c = &lines[idx + 2];
        for (i, (o, j)) in [ch.x, ch.y, ch.z].iter().zip(&c[1..4]).enumerate() {
            d.cmp(&format!("{tag} char pos {i}"), *o, hx(j));
        }
        d.flag(
            &format!("{tag} char flags"),
            &format!("{} {} {}", ch.fall_on_front, ch.ragdoll_fall, ch.on_floor),
            &c[4..7].join(" "),
        );
        let k = &lines[idx + 3];
        match (&slot, k[1]) {
            (None, "none") => {}
            (Some(rc), _) if k[1] != "none" => {
                let st = rc.state();
                d.flag(
                    &format!("{tag} ctrl flags"),
                    &format!(
                        "{} {} {} {} {}",
                        rc.is_initialized(),
                        rc.is_first_frame(frame_no),
                        rc.simulation_state(),
                        rc.is_upright(),
                        rc.is_on_back()
                    ),
                    &k[1..6].join(" "),
                );
                let ours = [
                    rc.head_position().x,
                    rc.head_position().y,
                    rc.head_position().z,
                    rc.pelvis_position().x,
                    rc.pelvis_position().y,
                    rc.pelvis_position().z,
                    rc.desired_character_position().x,
                    rc.desired_character_position().y,
                    rc.desired_character_position().z,
                    st.simulation_timeout,
                ];
                for (i, o) in ours.iter().enumerate() {
                    d.cmp(&format!("{tag} ctrl value {i}"), *o, hx(k[6 + i]));
                }
                d.flag(
                    &format!("{tag} ctrl state"),
                    &format!(
                        "{} {} {}",
                        st.is_simulating, st.is_simulation_movement, st.is_calculated
                    ),
                    &k[16..19].join(" "),
                );
                let ours = [
                    st.simulation_direction.x,
                    st.simulation_direction.y,
                    st.pelvis_direction.x,
                    st.pelvis_direction.y,
                    st.pelvis_direction.z,
                    st.simulation_rendered_angle,
                    st.simulation_character_forward_angle,
                ];
                for (i, o) in ours.iter().enumerate() {
                    d.cmp(&format!("{tag} ctrl dir {i}"), *o, hx(k[19 + i]));
                }
            }
            _ => d.flag(
                &format!("{tag} controller"),
                &format!("{:?}", slot.is_some()),
                k[1],
            ),
        }
        let nb = player.num_bones();
        for bi in 0..nb {
            let bl = &lines[idx + 4 + bi * 2];
            let (p, q, s) = player.bone_transform_at(bi).prs();
            for (i, o) in [p.x, p.y, p.z, q.x, q.y, q.z, q.w, s.x, s.y, s.z]
                .iter()
                .enumerate()
            {
                d_bones.cmp(&format!("{tag} bone {bi} prs {i}"), *o, hx(bl[2 + i]));
            }
            let ml = &lines[idx + 5 + bi * 2];
            for (i, o) in player.model_transform(bi).store().iter().enumerate() {
                d_bones.cmp(&format!("{tag} bone {bi} model {i}"), *o, hx(ml[2 + i]));
            }
        }
        idx += 4 + nb * 2;
    }
    eprintln!(
        "state: {}/{} exact, max err {:e}, first: {:?}",
        d.exact, d.total, d.max, d.first_bad
    );
    eprintln!(
        "bones: {}/{} exact, max err {:e}, first: {:?}",
        d_bones.exact, d_bones.total, d_bones.max, d_bones.first_bad
    );

    // The corpse: IsoDeadBody's frozen pose, posed again by the atlas.
    let died = corpse::died_bone_transforms(&player, true, true);
    let angle = ch.anim_angle(&player);
    let cl = &lines[idx];
    assert_eq!(cl[0], "corpse");
    let mut dc = Diff::default();
    dc.cmp("corpse angle", angle, hx(cl[1]));
    let mut p2 = AnimationPlayer::new(sd.clone());
    let (dx, dy) = (((angle as f64).cos()) as f32, ((angle as f64).sin()) as f32);
    p2.set_target_and_current_direction(dx, dy);
    corpse::set_death_pose(&mut p2, &died);
    p2.update(0.0, None);
    let dl = &lines[idx + 1];
    dc.cmp("deathpose angle", p2.angle(), hx(dl[1]));
    dc.flag("deathpose tracks", &p2.track_count().to_string(), dl[3]);
    let nb = p2.num_bones();
    for bi in 0..nb {
        let ml = &lines[idx + 2 + bi];
        for (i, o) in p2.model_transform(bi).store().iter().enumerate() {
            dc.cmp(&format!("deathpose bone {bi} model {i}"), *o, hx(ml[2 + i]));
        }
    }
    let skin = p2.skin_transforms(None);
    for bi in 0..skin.len() {
        let ml = &lines[idx + 2 + nb + bi];
        for (i, o) in skin[bi].store().iter().enumerate() {
            dc.cmp(&format!("deathpose skin {bi} {i}"), *o, hx(ml[2 + i]));
        }
    }
    eprintln!(
        "corpse: {}/{} exact, max err {:e}, first: {:?}",
        dc.exact, dc.total, dc.max, dc.first_bad
    );
    // The replay of the frozen pose must be exact given the same input.
    let _ = &mut probe;
    assert!(
        d.first_bad.is_none() || d.max < 1e-2,
        "ragdoll state diverges"
    );
}
