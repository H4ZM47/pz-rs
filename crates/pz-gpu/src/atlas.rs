//! Atlas textures (`DeadBodyAtlas`, `WorldItemAtlas`, `ModelManager.bitmap`):
//! colour + 16-bit depth targets kept with the chunk textures, drawn into
//! between `AtlasBegin` and `AtlasEnd`, and drawn from with the
//! `DeadBodyAtlas` shader (`AtlasQuad`).

use pz_render::DrawCmd;

use crate::state::{self, Program};
use crate::{sprite_vertices, ChunkTarget, FrameBuilder, Gpu, PassDesc, PassTarget, Tex2};

impl Gpu {
    /// `new Texture(w, h, 16)` + `new Texture(w, h, 512)`: colour sampled
    /// `GL_LINEAR` without mips (repeat), depth `GL_DEPTH_COMPONENT16`.
    fn ensure_atlas(&mut self, id: u64, width: u32, height: u32) {
        if self
            .chunks
            .get(&id)
            .is_some_and(|c| c.width == width && c.height == height)
        {
            return;
        }
        let tex = |device: &wgpu::Device, format, label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        // An atlas that cannot be allocated is reported and left out; its
        // draws are skipped.
        let Some((color_texture, depth_texture)) = self.allocate(&format!("atlas texture {id} {width}x{height}"), |d| {
            (tex(d, state::CHUNK_COLOR_FORMAT, "atlas"), tex(d, state::CHUNK_DEPTH_FORMAT, "atlas depth"))
        }) else {
            self.chunks.remove(&id);
            return;
        };
        let color = color_texture.create_view(&Default::default());
        let depth = depth_texture.create_view(&Default::default());
        let sampler = self.sampler((false, false, false, false));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let depth_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.chunk_depth_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&depth),
            }],
        });
        let levels = vec![color.clone()];
        self.chunks.insert(
            id,
            ChunkTarget {
                width,
                height,
                color_texture,
                levels,
                color,
                depth,
                composite: [bind.clone(), bind],
                depth_bind,
            },
        );
    }

    pub(crate) fn record_atlas(&mut self, f: &mut FrameBuilder, cmd: &DrawCmd) {
        match cmd {
            DrawCmd::AtlasBegin {
                id,
                width,
                height,
                clear,
            } => {
                self.ensure_atlas(*id, *width, *height);
                f.atlas_stack
                    .push((f.target, f.mvp, f.viewport, f.origin, self.state.clone()));
                f.target = PassTarget::Atlas(*id);
                f.viewport = (*width, *height);
                f.origin = (0, 0);
                f.passes.push(PassDesc {
                    target: f.target,
                    clear: *clear,
                    mipmaps: false,
                    batches: f.batches.len()..f.batches.len(),
                });
            }
            DrawCmd::AtlasEnd => {
                if let Some((target, mvp, viewport, origin, st)) = f.atlas_stack.pop() {
                    f.target = target;
                    f.mvp = mvp;
                    f.viewport = viewport;
                    f.origin = origin;
                    self.state = st;
                }
            }
            DrawCmd::AtlasQuad {
                id,
                pos,
                uv,
                color,
                depth,
            } => {
                if !self.chunks.contains_key(id) {
                    return;
                }
                let g = f.globals(&self.state, f.mvp_for_target(), 0.5);
                let s = pz_render::Sprite {
                    texture: None,
                    x: pos.map(|p| p[0]),
                    y: pos.map(|p| p[1]),
                    u: uv.map(|t| t[0]),
                    v: uv.map(|t| t[1]),
                    color: [*color; 4],
                    flipped: false,
                };
                let mut v = sprite_vertices(&s);
                for (vx, t) in v.iter_mut().zip(uv) {
                    vx.uv = *t;
                    vx.depth = *depth;
                }
                let key = self.key(f, Program::DepthAtlas);
                f.push(key, None, Tex2::Chunk(*id, true), &self.state, g, v);
            }
            _ => {}
        }
    }
}
