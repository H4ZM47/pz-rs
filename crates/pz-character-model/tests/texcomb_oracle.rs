//! Parity of the smart-texture jobs and their execution against the
//! original `ModelInstanceTextureCreator.render` + `TextureCombiner.combine`:
//! a body with blood and dirt, partially masked; a tinted texture-only
//! T-shirt with blood showing a hole through; a jacket model with a hue,
//! a decal, blood, dirt, two patches and two holes (chained jobs); a
//! partially masked plain trousers model.
//!
//! The oracle harness (outside the repo) runs the original with GL
//! shimmed and writes `texcomb.txt`: each combine's size and each draw's
//! program, textures (with their filters), uniforms set, blend function,
//! quad and projection. Point `PZ_TEXCOMB_ORACLE` at that file and
//! `PZ_GAME_DIR` at the game (texture sizes); without them this test does
//! nothing.

use std::path::PathBuf;
use std::rc::Rc;

use pz_character_model::combine::run_jobs;
use pz_character_model::data::ClothingDecal;
use pz_character_model::mask::{BloodBodyPartType as B, CharacterMask, Part, BLOOD_PARTS};
use pz_character_model::texture_creator::{render_textures, CharacterTextureData, ItemTextureData};
use pz_character_model::TextureSource;
use pz_render::combiner::{command_quad, resulting_size, Combiner, CombinerCommand};
use pz_render::{flags, Texture, TextureManager};

/// Formats each combine as the oracle does.
struct Recorder<'a> {
    lines: Vec<String>,
    tm_filters: &'a dyn Fn(&TextureManager, &Texture) -> &'static str,
}

impl Combiner for Recorder<'_> {
    fn combine(&mut self, cmds: &[CombinerCommand], tm: &mut TextureManager) -> Texture {
        let (w, h) = resulting_size(cmds);
        let k = self
            .lines
            .iter()
            .filter(|l| l.starts_with("combine "))
            .count();
        self.lines.push(format!("combine {k} {w} {h}"));
        for c in cmds {
            let q = command_quad(c, w, h);
            let quad: Vec<String> = q
                .iter()
                .map(|v| format!("{:?},{:?},{:?},{:?}", v.pos[0], v.pos[1], v.uv[0], v.uv[1]))
                .collect();
            let params: String = c
                .params
                .iter()
                .map(|(n, v)| format!("{n}={v:?};"))
                .collect();
            let mask = match &c.mask {
                // Masks are bound GL_NEAREST.
                Some(m) => format!("{}#9728/9728@1", m.name),
                None => "-".into(),
            };
            self.lines.push(format!(
                "draw {} tex={}#{}@0 mask={mask} params={params} blend={},{},{},{} quad={} z=0.0,0.0,0.0,0.0 color=1.0,1.0,1.0,1.0 ortho={:?},{:?},-1.0,1.0",
                c.shader.name(),
                c.tex.name,
                (self.tm_filters)(tm, &c.tex),
                c.blend[0],
                c.blend[1],
                c.blend[2],
                c.blend[3],
                quad.join(";"),
                2.0 / w as f32,
                -2.0 / h as f32,
            ));
        }
        tm.render_texture(&format!("job{k}"), w as i32, h as i32, flags::FBO)
    }

    fn release(&mut self, _tex: Texture) {}
}

fn mask(hidden: &[Part]) -> CharacterMask {
    let mut m = CharacterMask::default();
    m.set_all_visible(true);
    for &p in hidden {
        m.set_part_visible(p, false);
    }
    m
}

fn item(instance: Option<usize>, mask: CharacterMask, folder: &str, base: &str) -> ItemTextureData {
    ItemTextureData {
        instance,
        mask,
        mask_folder: folder.into(),
        base_texture: base.into(),
        tint: [1.0; 3],
        hue: 0.0,
        decal: None,
        blood: [0.0; BLOOD_PARTS],
        dirt: [0.0; BLOOD_PARTS],
        basic: [0.0; BLOOD_PARTS],
        denim: [0.0; BLOOD_PARTS],
        leather: [0.0; BLOOD_PARTS],
        hole: [0.0; BLOOD_PARTS],
    }
}

#[test]
fn matches_texture_combiner_oracle() {
    let (Some(oracle), Some(game)) = (
        std::env::var_os("PZ_TEXCOMB_ORACLE").map(PathBuf::from),
        std::env::var_os("PZ_GAME_DIR").map(PathBuf::from),
    ) else {
        eprintln!("PZ_TEXCOMB_ORACLE / PZ_GAME_DIR not set; skipping");
        return;
    };
    let expected = std::fs::read_to_string(oracle).unwrap();

    let mut chr = CharacterTextureData {
        mask: mask(&[Part::Torso, Part::LeftLeg]),
        mask_folder: "media/textures/Body/Masks".into(),
        base_texture: "media/textures/Body/MaleBody01a.png".into(),
        blood: [0.0; BLOOD_PARTS],
        dirt: [0.0; BLOOD_PARTS],
        hole_mask: [false; BLOOD_PARTS],
    };
    chr.blood[B::HandL.index()] = 1.0;
    chr.blood[B::Head.index()] = 0.8;
    chr.dirt[B::HandL.index()] = 0.6;
    chr.dirt[B::HandR.index()] = 0.9;
    chr.hole_mask[B::TorsoLower.index()] = true;

    let body_masks = "media/textures/Body/Masks";
    let mut shirt = item(
        None,
        mask(&[Part::LeftArm]),
        body_masks,
        "media/textures/Clothes/Shirt_Tshirt_Textures/TShirt_Black.png",
    );
    shirt.tint = [0.8, 0.3, 0.3];
    shirt.blood[B::TorsoUpper.index()] = 0.5;
    shirt.hole[B::TorsoLower.index()] = -1.0;
    let mut jacket = item(
        Some(1),
        mask(&[]),
        "none",
        "media/textures/Clothes/JacketPadded/JacketPadded_CamoTree.png",
    );
    jacket.hue = 0.2;
    jacket.decal = Some(Rc::new(ClothingDecal {
        name: None,
        texture: Some("shirtdecals/spiffo7".into()),
        x: 102,
        y: 118,
        width: 52,
        height: 52,
    }));
    jacket.blood[B::TorsoUpper.index()] = 1.0;
    jacket.blood[B::UpperArmL.index()] = 0.7;
    jacket.dirt[B::ForeArmR.index()] = 1.0;
    jacket.dirt[B::TorsoLower.index()] = 0.6;
    jacket.basic[B::UpperArmR.index()] = 1.0;
    jacket.denim[B::UpperLegR.index()] = 1.0;
    jacket.hole[B::TorsoLower.index()] = 1.0;
    jacket.hole[B::UpperArmL.index()] = 1.0;
    let trousers = item(
        Some(2),
        mask(&[Part::RightLeg]),
        body_masks,
        "media/textures/Clothes/Male_Pants.png",
    );
    let r = render_textures(&chr, &[shirt, jacket, trousers]);

    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(std::env::temp_dir().join("pz-texcomb-oracle"));
    let mut tm = TextureManager::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
    // Named by the path asked for, as the original's shared textures are.
    let mut file = |tm: &mut TextureManager, p: &str| {
        tm.shared_texture_flags(&fs, p, model_flags).map(|mut t| {
            t.name = p.into();
            t
        })
    };
    let filters = |tm: &TextureManager, t: &Texture| match t.id.and_then(|id| tm.data(id)) {
        Some(d) if d.min_nearest() => "9728/9728",
        _ => "9729/9729",
    };
    let mut rec = Recorder {
        lines: Vec::new(),
        tm_filters: &filters,
    };
    run_jobs(&r.out.jobs, &mut file, &mut rec, &mut tm);
    let mut lines = rec.lines;
    lines.push(format!("body job{}", r.body));
    let job = |i: usize| match r.instances.iter().find(|(n, _)| *n == i) {
        Some((_, TextureSource::Smart(j))) => format!("job{j}"),
        other => format!("{other:?}"),
    };
    let white = if r.white_tint.contains(&1) {
        "1.0,1.0,1.0"
    } else {
        "-"
    };
    lines.push(format!("jacket {} tint {white}", job(1)));
    lines.push(format!("trousers {}", job(2)));

    let want: Vec<&str> = expected
        .lines()
        .filter(|l| !l.starts_with("rendered ") && !l.starts_with("GL "))
        .collect();
    for (i, (a, b)) in lines.iter().zip(&want).enumerate() {
        assert_eq!(a, b, "line {i}");
    }
    assert_eq!(lines.len(), want.len());
}
