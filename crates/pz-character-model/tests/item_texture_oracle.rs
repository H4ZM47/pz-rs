//! Parity of the item smart textures against the original: the jobs of
//! `EquippedTextureCreator` (held items: tint with and without a `TINT`
//! mask, weapon blood, fluid; a plain instance and one already showing an
//! `ItemSmartTexture`), `WorldItemAtlas.RenderJob.checkSmartTexture` and
//! `ItemModelRenderer.checkSmartTexture`, executed as
//! `TextureCombiner.combine` executes them.
//!
//! The oracle harness (outside the repo) runs the original classes with
//! GL shimmed and writes `itemtex.txt`: per case, each combine's size and
//! each draw's program, textures (with their filters), uniforms, blend
//! function, quad and projection. Point `PZ_ITEMTEX_ORACLE` at that file
//! and `PZ_GAME_DIR` at the game (texture sizes and which masks exist);
//! without them this test does nothing.

use std::path::{Path, PathBuf};

use pz_character_model::combine::run_jobs;
use pz_character_model::item_texture::{
    atlas_job, equipped_job, item_renderer_job, ItemTextureParams,
};
use pz_character_model::texture_creator::SmartTextureJob;
use pz_character_model::ItemTextureInit;
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
        self.lines.push(format!("combine 0 {w} {h}"));
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
        tm.render_texture("job", w as i32, h as i32, flags::FBO)
    }

    fn release(&mut self, _tex: Texture) {}
}

const KNIFE: &str = "media/textures/weapons/1handed/HuntingKnife.png";
const BOTTLE: &str = "media/textures/WorldItems/PopBottle_Generic.png";
const MUG: &str = "media/textures/Mug_Green.png";
const WHITE: [f32; 3] = [1.0; 3];

fn init(blood: f32, tint: [f32; 3], fluid: f32, fluid_tint: Option<[f32; 3]>) -> ItemTextureInit {
    ItemTextureInit {
        blood_level: blood,
        tint,
        fluid_level: fluid,
        fluid_tint,
    }
}

fn params(
    tex: &str,
    tint: [f32; 3],
    tint_mask: Option<&str>,
    blood: f32,
    fluid: f32,
    fluid_mask: Option<&str>,
    fluid_tint: [f32; 3],
) -> ItemTextureParams {
    ItemTextureParams {
        model_texture: Some(tex.into()),
        tint,
        tint_mask: tint_mask.map(str::to_string),
        blood_level: blood,
        fluid_level: fluid,
        fluid_mask: fluid_mask.map(str::to_string),
        fluid_tint,
    }
}

#[test]
fn item_smart_textures_match_the_original() {
    let (Some(oracle), Some(game)) = (
        std::env::var_os("PZ_ITEMTEX_ORACLE").map(PathBuf::from),
        std::env::var_os("PZ_GAME_DIR").map(PathBuf::from),
    ) else {
        eprintln!("PZ_ITEMTEX_ORACLE / PZ_GAME_DIR not set; skipping");
        return;
    };
    let expected = std::fs::read_to_string(oracle).unwrap();
    let exists = |n: &str| Path::new(&game).join(n).is_file();
    let mut cases: Vec<(String, Option<SmartTextureJob>)> = Vec::new();
    let eq = [
        ("knife blood", KNIFE, false, init(0.6, WHITE, 0.0, None)),
        (
            "knife tint",
            KNIFE,
            false,
            init(0.0, [0.5, 0.8, 0.3], 0.0, None),
        ),
        (
            "knife tint blood over 1",
            KNIFE,
            false,
            init(1.7, [0.5, 0.8, 0.3], 0.0, None),
        ),
        (
            "bottle tintmask blood fluid",
            BOTTLE,
            false,
            init(0.3, [0.2, 0.4, 0.9], 0.4, Some([0.1, 0.6, 0.2])),
        ),
        (
            "mug fluid plain",
            MUG,
            false,
            init(0.0, WHITE, 0.75, Some([0.8, 0.2, 0.1])),
        ),
        (
            "mug fluid smart",
            MUG,
            true,
            init(0.0, WHITE, 0.75, Some([0.8, 0.2, 0.1])),
        ),
        (
            "mug tint fluid",
            MUG,
            false,
            init(0.0, [0.9, 0.9, 0.5], 1.3, Some([0.3, 0.5, 0.9])),
        ),
        (
            "knife clean smart",
            KNIFE,
            true,
            init(0.0, WHITE, 0.0, None),
        ),
    ];
    for (name, tex, smart, i) in eq {
        let job = equipped_job(tex, &i, smart, &exists);
        cases.push((
            format!(
                "equipped {name} rendered=true smart={}",
                smart || job.is_some()
            ),
            job,
        ));
    }
    let bottle_tint = Some("media/textures/WorldItems/PopBottle_GenericTINT.png");
    let wi = [
        (
            "knife blood",
            params(KNIFE, WHITE, None, 0.45, 0.0, None, WHITE),
        ),
        (
            "bottle tintmask",
            params(BOTTLE, [0.2, 0.4, 0.9], bottle_tint, 0.0, 0.0, None, WHITE),
        ),
        (
            "bottle tintmask blood",
            params(BOTTLE, [0.2, 0.4, 0.9], bottle_tint, 0.7, 0.0, None, WHITE),
        ),
        (
            "mug fluid",
            params(
                MUG,
                WHITE,
                None,
                0.0,
                0.75,
                Some("media/textures/Mug_GreenFLUIDTINT.png"),
                [0.8, 0.2, 0.1],
            ),
        ),
        (
            "bottle all",
            params(
                BOTTLE,
                [0.6, 0.6, 0.2],
                bottle_tint,
                0.2,
                0.3,
                Some("media/textures/WorldItems/PopBottle_GenericFLUIDTINT.png"),
                [0.1, 0.6, 0.2],
            ),
        ),
        ("mug plain", params(MUG, WHITE, None, 0.0, 0.0, None, WHITE)),
    ];
    for (name, p) in wi {
        let job = atlas_job(&p, &exists);
        cases.push((format!("atlas {name} used={}", job.is_some()), job));
        let job = item_renderer_job(&p);
        cases.push((format!("renderer {name} used={}", job.is_some()), job));
    }

    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(std::env::temp_dir().join("pz-itemtex-oracle"));
    let mut tm = TextureManager::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
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
    let mut lines = Vec::new();
    for (title, job) in &cases {
        lines.push(format!("== {title}"));
        if let Some(j) = job {
            let mut rec = Recorder {
                lines: Vec::new(),
                tm_filters: &filters,
            };
            run_jobs(std::slice::from_ref(j), &mut file, &mut rec, &mut tm);
            lines.extend(rec.lines);
        }
    }
    let want: Vec<&str> = expected.lines().collect();
    for (i, (a, b)) in lines.iter().zip(&want).enumerate() {
        assert_eq!(a, b, "line {i}");
    }
    assert_eq!(lines.len(), want.len());
}
