//! Running [`SmartTextureJob`]s through a `TextureCombiner` backend
//! ([`pz_render::combiner::Combiner`], e.g. `pz_gpu::Gpu`), as
//! `ModelInstanceTextureCreator.render` and `EquippedTextureCreator.render`
//! call `SmartTexture.calculate`.

use pz_render::combiner::{Combiner, CombinerCommand as GpuCommand, CombinerShader};
use pz_render::{Texture, TextureManager};

use crate::texture_creator::{CombinerCommand, SmartTextureJob, TexRef};

/// `TextureCombinerShaderParam` evaluated as `combine` does
/// (`Rand.Next(min, max)`). Every parameter `SmartTexture` makes has
/// `min == max`, where `Rand.Next` returns `min` without drawing a number.
fn param_value(min: f32, max: f32) -> f32 {
    if min == max {
        min
    } else {
        // Not reached by the game's command lists; the midpoint stands in
        // for the random draw.
        (min + max) * 0.5
    }
}

/// One command with its textures resolved. `resolve` gives a file's
/// texture or an earlier job's result; missing textures become the error
/// texture (`TextureCombinerCommand.requireNonNull`).
pub fn gpu_command(
    c: &CombinerCommand,
    resolve: &mut dyn FnMut(&TexRef) -> Option<Texture>,
    error: &Texture,
) -> GpuCommand {
    let shader = CombinerShader::from_name(c.shader.program()).expect("SmartShader program");
    GpuCommand {
        tex: resolve(&c.tex).unwrap_or_else(|| error.clone()),
        mask: c
            .mask
            .as_ref()
            .map(|m| resolve(m).unwrap_or_else(|| error.clone())),
        shader,
        params: c
            .params
            .iter()
            .map(|p| (p.name.clone(), param_value(p.min, p.max)))
            .collect(),
        blend: c.blend,
        rect: c.rect,
    }
}

/// Runs the jobs in order (`calculate` for each), then releases the
/// intermediate results — the full character and item textures and the
/// textures holes were cut from, i.e. every result a later job reads — to
/// the combiner's pool, as `render` releases its `tempTextures`. Returns
/// each job's texture; released ones are `None`.
///
/// `file` loads a texture file (`Texture.getSharedTexture(name,
/// ModelManager.getTextureFlags())`).
pub fn run_jobs(
    jobs: &[SmartTextureJob],
    file: &mut dyn FnMut(&mut TextureManager, &str) -> Option<Texture>,
    combiner: &mut dyn Combiner,
    tm: &mut TextureManager,
) -> Vec<Option<Texture>> {
    let mut done: Vec<Option<Texture>> = Vec::with_capacity(jobs.len());
    let mut temp = vec![false; jobs.len()];
    for job in jobs {
        let error = tm.error_texture();
        let mut cmds = Vec::with_capacity(job.commands.len());
        for c in &job.commands {
            for r in std::iter::once(&c.tex).chain(c.mask.as_ref()) {
                if let TexRef::Job(i) = r {
                    if let Some(t) = temp.get_mut(*i) {
                        *t = true;
                    }
                }
            }
            let mut resolve = |r: &TexRef| match r {
                TexRef::File(p) => file(tm, p),
                TexRef::Job(i) => done.get(*i).cloned().flatten(),
            };
            cmds.push(gpu_command(c, &mut resolve, &error));
        }
        done.push(Some(combiner.combine(&cmds, tm)));
    }
    for (i, t) in temp.iter().enumerate() {
        if *t {
            if let Some(tex) = done[i].take() {
                combiner.release(tex);
            }
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture_creator::{ShaderParam, SmartShader};

    /// Records the command lists and hands out numbered textures.
    #[derive(Default)]
    struct Recorder {
        jobs: Vec<Vec<GpuCommand>>,
        released: Vec<String>,
    }

    impl Combiner for Recorder {
        fn combine(&mut self, cmds: &[GpuCommand], tm: &mut TextureManager) -> Texture {
            self.jobs.push(cmds.to_vec());
            tm.render_texture(&format!("job{}", self.jobs.len() - 1), 4, 4, 16)
        }

        fn release(&mut self, tex: Texture) {
            self.released.push(tex.name.to_string());
        }
    }

    fn cmd(tex: TexRef, mask: Option<TexRef>, shader: SmartShader) -> CombinerCommand {
        CombinerCommand {
            tex,
            mask,
            shader,
            params: vec![ShaderParam {
                name: "intensity".into(),
                min: 0.3,
                max: 0.3,
            }],
            blend: [770, 771, 1, 771],
            rect: None,
        }
    }

    #[test]
    fn jobs_chain_and_temporaries_return_to_the_pool() {
        let mut tm = TextureManager::new();
        let jobs = vec![
            SmartTextureJob {
                commands: vec![cmd(TexRef::File("a.png".into()), None, SmartShader::Blit)],
            },
            SmartTextureJob {
                commands: vec![cmd(
                    TexRef::Job(0),
                    Some(TexRef::File("missing.png".into())),
                    SmartShader::AddHole,
                )],
            },
            SmartTextureJob {
                commands: vec![cmd(TexRef::Job(1), None, SmartShader::DirtMask)],
            },
        ];
        let mut rec = Recorder::default();
        let mut file = |tm: &mut TextureManager, p: &str| (p == "a.png").then(|| tm.white());
        let out = run_jobs(&jobs, &mut file, &mut rec, &mut tm);
        assert_eq!(rec.released, ["job0", "job1"]);
        assert!(out[0].is_none() && out[1].is_none());
        assert_eq!(out[2].as_ref().unwrap().name.as_ref(), "job2");
        assert_eq!(rec.jobs[1][0].tex.name.as_ref(), "job0");
        assert_eq!(
            rec.jobs[1][0].mask.as_ref().unwrap().name.as_ref(),
            "EngineErrorTexture"
        );
        assert_eq!(rec.jobs[1][0].shader, CombinerShader::AddHole);
        assert_eq!(rec.jobs[2][0].params, [("intensity".to_string(), 0.3)]);
    }
}
