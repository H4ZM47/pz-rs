//! Parity with the original classes, from a dump a local harness makes by
//! running them headless (`BaseVehicle.updateTransform`, `updateLights`,
//! `doDamageOverlay`, `VehicleModelCamera` + `Model.DrawVehicle`'s
//! centre-of-mass offset and `targetDepth`, the shadow polygon, and
//! `TorchInfo.set(part)` for the headlights) on vehicles of several
//! scripts. The harness and its output are not part of this repository:
//! point `PZ_VEHRENDER_ORACLE` at the dump (and `PZ_GAME_DIR` at the
//! game); without them this test does nothing.

mod common;

use std::rc::Rc;

use pz_iso_render::joml::Matrix4f;
use pz_iso_render::models::ModelCamera;
use pz_iso_render::Camera;
use pz_model::assets::MediaIndex;
use pz_vehicle_render::lights::vehicle_torches;
use pz_vehicle_render::models::MediaVehicleModels;
use pz_vehicle_render::render::{vehicle_slot, VehicleFrame};
use pz_vehicle_render::VehicleModelSet;
use pz_vehicles::jmath::{Matrix3f, Vector3f};

fn f(h: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(h, 16).unwrap())
}

fn floats(s: &[&str]) -> Vec<f32> {
    s.iter().map(|h| f(h)).collect()
}

fn cols(m: &Matrix4f) -> Vec<f32> {
    m.to_array().to_vec()
}

#[derive(Default)]
struct Case {
    lines: Vec<String>,
}

impl Case {
    fn get(&self, tag: &str) -> Vec<Vec<String>> {
        self.lines
            .iter()
            .filter(|l| l.split(' ').next() == Some(tag))
            .map(|l| l.split(' ').skip(1).map(str::to_string).collect())
            .collect()
    }
}

struct Tally {
    ok: usize,
    bad: Vec<String>,
}

impl Tally {
    fn eq(&mut self, what: &str, rust: &[f32], java: &[f32]) {
        let same = rust.len() == java.len()
            && rust
                .iter()
                .zip(java)
                .all(|(a, b)| a.to_bits() == b.to_bits() || (*a == 0.0 && *b == 0.0));
        if same {
            self.ok += 1;
        } else {
            self.bad
                .push(format!("{what}\n  rust {rust:?}\n  java {java:?}"));
        }
    }
}

#[test]
fn vehicle_render_state_matches_the_original() {
    let Some(game) = common::game_dir() else {
        return;
    };
    let Some(path) = std::env::var_os("PZ_VEHRENDER_ORACLE") else {
        eprintln!("PZ_VEHRENDER_ORACLE not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut cases: Vec<Case> = Vec::new();
    for l in text.lines() {
        if l.starts_with("C ") {
            cases.push(Case::default());
        }
        if let Some(c) = cases.last_mut() {
            c.lines.push(l.to_string());
        }
    }
    let sm = common::scripts(&game);
    let media = MediaIndex::scan(&game).unwrap();
    let load = |_: &str, _: i32| None;
    let source = MediaVehicleModels::new(&media, sm.clone(), true, &load);
    let mut t = Tally {
        ok: 0,
        bad: Vec::new(),
    };
    for c in &cases {
        let head = &c.get("C")[0];
        let id: i32 = head[0].parse().unwrap();
        let script = &head[1];
        let (x, y) = (f(&head[2]), f(&head[3]));
        let v = common::vehicle(script, x, y, f(&head[4]));
        {
            let tr = floats(&c.get("T")[0].iter().map(String::as_str).collect::<Vec<_>>());
            let mut st = v.st_mut();
            st.jni_transform.basis = Matrix3f {
                m00: tr[0],
                m01: tr[1],
                m02: tr[2],
                m10: tr[3],
                m11: tr[4],
                m12: tr[5],
                m20: tr[6],
                m21: tr[7],
                m22: tr[8],
            };
            st.jni_transform.origin = Vector3f::new(tr[9], tr[10], tr[11]);
            let p = &c.get("P")[0];
            st.x = f(&p[0]);
            st.y = f(&p[1]);
            st.z = f(&p[2]);
            let col: Vec<f32> = p[5..8]
                .iter()
                .map(|s| f(s.trim_start_matches("col=")))
                .collect();
            st.color_hue = col[0];
            st.color_saturation = col[1];
            st.color_value = col[2];
            st.rust = f(p[4].trim_start_matches("rust="));
            st.vehicle_id = id as i16;
            st.poly_dirty = true;
            for w in c.get("W") {
                let i: usize = w[0].parse().unwrap();
                st.wheel_info[i].steering = f(&w[1]);
                st.wheel_info[i].rotation = f(&w[2]);
                st.wheel_info[i].suspension_length = f(&w[3]);
            }
            st.headlights_on = true;
            st.stoplights_on = id % 2 == 0;
            st.window_lights_on = id >= 3;
        }
        v.set_blood_intensity("Front", 0.4);
        v.set_blood_intensity("Left", 0.9);
        for p in c.get("PART") {
            let part = v.part_by_id(&p[0]).unwrap();
            let field = |k: &str| p.iter().find_map(|s| s.strip_prefix(k).map(str::to_string));
            let item = field("item=").unwrap();
            part.st_mut().item = if item == "-" {
                None
            } else {
                Some(common::item("Base.Part", item.parse().unwrap()))
            };
            part.st_mut().condition = field("cond=").unwrap().parse().unwrap();
            if let Some(d) = field("door=") {
                part.st_mut().door.as_mut().unwrap().open = d == "1";
            }
            if let Some(w) = field("window=") {
                part.st_mut().window.as_mut().unwrap().open = w == "1";
            }
            if let Some(l) = field("light=") {
                let l: Vec<&str> = l.split(',').collect();
                part.create_spot_light_color(
                    f(l[0]),
                    f(l[1]),
                    f(l[2]),
                    f(l[3]),
                    f(l[4]),
                    l[5].parse().unwrap(),
                    f(l[7]),
                    f(l[8]),
                    f(l[9]),
                );
                part.set_light_active(l[6] == "1");
            }
        }
        if f(&c.get("BAT")[0][0]) > 0.0 {
            let mut tpl = pz_inventory::ItemTemplate::new(
                pz_inventory::ItemClass::Drainable,
                "Base",
                "CarBattery1",
            );
            tpl.use_delta = 0.1;
            let mut i = pz_inventory::InventoryItem::create(Rc::new(tpl), -1);
            i.uses = 10;
            v.battery().unwrap().st_mut().item = Some(pz_vehicles::part::PartItem::Item(Rc::new(
                std::cell::RefCell::new(i),
            )));
        }
        v.create_physics(false);
        // createPhysics may move the transform; restore the dumped one.
        {
            let tr = floats(&c.get("T")[0].iter().map(String::as_str).collect::<Vec<_>>());
            let mut st = v.st_mut();
            st.jni_transform.basis = Matrix3f {
                m00: tr[0],
                m01: tr[1],
                m02: tr[2],
                m10: tr[3],
                m11: tr[4],
                m12: tr[5],
                m20: tr[6],
                m21: tr[7],
                m22: tr[8],
            };
            st.jni_transform.origin = Vector3f::new(tr[9], tr[10], tr[11]);
            st.poly_dirty = true;
        }
        let mut no_tex = |_: &str, _: i32| None;
        let mut set = VehicleModelSet::create(&v, &source, &mut no_tex, &|_, _| None);
        let ms = &c.get("MS")[0];
        set.instance.scale = f(&ms[1]);
        set.update_transform(&v);
        let row = |r: &Vec<String>, from: usize| {
            floats(&r[from..].iter().map(String::as_str).collect::<Vec<_>>())
        };
        t.eq(
            &format!("{script} renderTransform"),
            &cols(&set.render_transform),
            &row(&c.get("RT")[0], 0),
        );
        t.eq(
            &format!("{script} vehicleTransform"),
            &cols(&set.vehicle_transform),
            &row(&c.get("VT")[0], 0),
        );
        let mts = c.get("MT");
        assert_eq!(mts.len(), set.parts.len(), "{script} part models");
        for (m, p) in mts.iter().zip(&set.parts) {
            assert_eq!(p.part.id().as_deref(), Some(m[1].as_str()));
            t.eq(
                &format!("{script} {} renderTransform", m[1]),
                &cols(&p.render_transform),
                &row(m, 3),
            );
        }
        set.instance.update_lights(&v);
        set.instance.do_damage_overlay(&v, true);
        let inst = &set.instance;
        for i in c.get("I") {
            let arr = match i[0].as_str() {
                "textureUninstall1" => inst.texture_uninstall1,
                "textureUninstall2" => inst.texture_uninstall2,
                "textureLightsEnables1" => inst.texture_lights_enables1,
                "textureLightsEnables2" => inst.texture_lights_enables2,
                "textureDamage1Enables1" => inst.texture_damage1_enables1,
                "textureDamage1Enables2" => inst.texture_damage1_enables2,
                "textureDamage2Enables1" => inst.texture_damage2_enables1,
                "textureDamage2Enables2" => inst.texture_damage2_enables2,
                "matrixBlood1Enables1" => inst.matrix_blood1_enables1,
                "matrixBlood1Enables2" => inst.matrix_blood1_enables2,
                "matrixBlood2Enables1" => inst.matrix_blood2_enables1,
                "matrixBlood2Enables2" => inst.matrix_blood2_enables2,
                other => panic!("{other}"),
            };
            t.eq(&format!("{script} {}", i[0]), &arr, &row(&i, 1));
        }
        let is = row(&c.get("IS")[0], 0);
        t.eq(
            &format!("{script} rust/ref/paint"),
            &[
                inst.texture_rust_a,
                inst.ref_windows,
                inst.ref_body,
                inst.pain_color[0],
                inst.pain_color[1],
                inst.pain_color[2],
            ],
            &is,
        );
        // The DrawVehicle camera.
        for cam in c.get("CAM") {
            let ci = &cam[0];
            let (cx, cy, cz) = (f(&cam[1]), f(&cam[2]), f(&cam[3]));
            let (w, h): (i32, i32) = (cam[4].parse().unwrap(), cam[5].parse().unwrap());
            let camera = Camera::centred(cx, cy, cz, w, h, 1.0);
            let mc = ModelCamera::new(&camera, w as f32, h as f32);
            let frame = VehicleFrame {
                alpha: 1.0,
                ambient: [1.0; 3],
                lights: Vec::new(),
                sky_texture_shift: 0.0,
                sky_textures: [None, None],
                square_light: [1.0; 3],
                seen: true,
                slope: None,
                default_shadow: None,
            };
            let slot = vehicle_slot(&v, &frame, &mc);
            let pick = |tag: &str| c.get(tag).into_iter().find(|r| &r[0] == ci).unwrap();
            t.eq(
                &format!("{script} cam {ci} projection"),
                &cols(&slot.projection),
                &row(&pick("CP"), 1),
            );
            t.eq(
                &format!("{script} cam {ci} model-view"),
                &cols(&slot.model_view),
                &row(&pick("CMV"), 1),
            );
            let cd = row(&pick("CD"), 1);
            t.eq(
                &format!("{script} cam {ci} com/square depth/targetDepth"),
                &[slot.center_of_mass_y, slot.square_depth, slot.target_depth],
                &[cd[0], cd[1], cd[3]],
            );
            t.eq(
                &format!("{script} cam {ci} model-view x renderTransform"),
                &cols(&slot.model_view.mul(&set.render_transform)),
                &row(&pick("CX"), 1),
            );
        }
        let sh = row(&c.get("SH")[0], 0);
        let p = v.poly();
        let _ = p;
        let sc = v.st().shadow_coord;
        t.eq(
            &format!("{script} shadow polygon"),
            &[sc.x1, sc.y1, sc.x2, sc.y2, sc.x3, sc.y3, sc.x4, sc.y4],
            &sh,
        );
        let torches = vehicle_torches(&v);
        let java = c.get("TORCH");
        assert_eq!(torches.len(), java.len(), "{script} torches");
        for (r, j) in torches.iter().zip(&java) {
            let jv = floats(&[
                &j[1], &j[2], &j[3], &j[4], &j[5], &j[6], &j[7], &j[8], &j[9], &j[10], &j[12],
            ]);
            t.eq(
                &format!("{script} torch {}", j[0]),
                &[
                    r.x, r.y, r.z, r.r, r.g, r.b, r.angle_x, r.angle_y, r.dist, r.strength, r.dot,
                ],
                &jv,
            );
            assert_eq!(
                (r.cone, r.focusing),
                (j[11] == "true", j[13].parse().unwrap()),
                "{script} torch {}",
                j[0]
            );
        }
    }
    eprintln!("{} values match over {} vehicles", t.ok, cases.len());
    assert!(
        t.bad.is_empty(),
        "{} mismatches:\n{}",
        t.bad.len(),
        t.bad.join("\n")
    );
}
