//! `TextureCombiner.combine` on wgpu: the `SmartTexture` command lists
//! that build character, clothing and item textures (skin, clothing
//! masks, tints and hues, blood, dirt, holes, patches, decals).
//!
//! As the original: the result is an RGBA8 texture the size of the
//! largest input rounded up to powers of two, taken from a pool of
//! textures per size (`CombinerFBO`), cleared to transparent black, then
//! each command is a quad over the whole result (or its `addRect`
//! rectangle in a 256 x 256 space) drawn with its `SmartShader` program and
//! `glBlendFuncSeparate`, sampling its texture with that texture's own
//! filter and wrap and its mask with `GL_NEAREST`. Results are sampled
//! `GL_LINEAR` without mip maps; the first texture of each size keeps
//! `GL_REPEAT` wrapping, later ones are set to `GL_CLAMP_TO_EDGE`
//! (`createTexture`). The combiner's framebuffer is bottom-up like GL's, so
//! texture rows map to the same rows of the result.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use pz_render::combiner::{
    command_quad, resulting_size, Combiner, CombinerCommand, CombinerShader,
};
use pz_render::{flags, Texture, TextureManager};

use crate::{gl, Gpu};

/// The `Params` uniform block of combiner.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
struct Params {
    hue_change: f32,
    r: f32,
    g: f32,
    b: f32,
    intensity: f32,
    blood_dark: f32,
    cutoff_min: f32,
    cutoff_max: f32,
    mask_padding_radius: f32,
    pad: [f32; 3],
}

const PARAMS_STRIDE: u64 = 256;

impl Params {
    fn new(cmd: &CombinerCommand) -> Self {
        let v = |name: &str| cmd.uniform(name).unwrap_or(0.0);
        Params {
            hue_change: v("HueChange"),
            r: cmd.uniform("R").unwrap_or(1.0),
            g: cmd.uniform("G").unwrap_or(1.0),
            b: cmd.uniform("B").unwrap_or(1.0),
            intensity: v("intensity"),
            blood_dark: v("bloodDark"),
            cutoff_min: v("cutoffMin"),
            cutoff_max: v("cutoffMax"),
            mask_padding_radius: v("maskPaddingRadius"),
            pad: [0.0; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CombinerVertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

/// A combiner pipeline: the program and the blend function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CombinerKey {
    shader: CombinerShader,
    blend: [u32; 4],
}

/// `CombinerFBO`: the textures of one size not in use.
struct CombinerFbo {
    width: u32,
    height: u32,
    textures: Vec<Texture>,
}

/// The combiner's pipelines, pool and render textures.
pub(crate) struct CombinerState {
    module: wgpu::ShaderModule,
    uniform_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pipelines: HashMap<CombinerKey, wgpu::RenderPipeline>,
    pool: Vec<CombinerFbo>,
    /// Result textures by texture id.
    targets: HashMap<u32, wgpu::Texture>,
    created: u32,
}

/// `TextureCombiner.fboPool` keeps at most this many free textures a size.
const POOL_LIMIT: usize = 100;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl CombinerState {
    pub(crate) fn new(device: &wgpu::Device, texture_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SmartShader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("combiner.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SmartShader params"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SmartShader"),
            bind_group_layouts: &[
                Some(&uniform_layout),
                Some(texture_layout),
                Some(texture_layout),
            ],
            immediate_size: 0,
        });
        CombinerState {
            module,
            uniform_layout,
            layout,
            pipelines: HashMap::new(),
            pool: Vec::new(),
            targets: HashMap::new(),
            created: 0,
        }
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, key: CombinerKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let fs = match key.shader {
            CombinerShader::Blit => "fs_blit",
            CombinerShader::HueChange => "fs_hue_change",
            CombinerShader::OverlayMask => "fs_overlay_mask",
            CombinerShader::DirtMask => "fs_dirt_mask",
            CombinerShader::BodyMask => "fs_body_mask",
            CombinerShader::BodyMaskHue => "fs_body_mask_hue",
            CombinerShader::BodyMaskTint => "fs_body_mask_tint",
            CombinerShader::AddHole => "fs_add_hole",
            CombinerShader::RemoveHole => "fs_remove_hole",
        };
        let [sc, dc, sa, da] = key.blend;
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: gl::blend_factor(sc),
                dst_factor: gl::blend_factor(dc),
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: gl::blend_factor(sa),
                dst_factor: gl::blend_factor(da),
                operation: wgpu::BlendOperation::Add,
            },
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(key.shader.name()),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CombinerVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                })],
            },
            // glDisable(GL_CULL_FACE), no depth or stencil.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, p);
    }
}

impl Gpu {
    /// `TextureCombiner.createTexture`: a free texture of this size from
    /// the pool, or a new one.
    fn combiner_texture(&mut self, width: u32, height: u32, tm: &mut TextureManager) -> Texture {
        let fbo = self
            .combiner
            .pool
            .iter_mut()
            .position(|f| f.width == width && f.height == height);
        let (tex, clamp) = match fbo {
            None => {
                self.combiner.pool.push(CombinerFbo {
                    width,
                    height,
                    textures: Vec::new(),
                });
                (None, false)
            }
            Some(i) => (self.combiner.pool[i].textures.pop(), true),
        };
        let tex = match tex {
            Some(t) => t,
            None => {
                // new Texture(width, height, TextureFlags.FBO)
                self.combiner.created += 1;
                let t = tm.render_texture(
                    &format!("TextureCombiner#{}", self.combiner.created),
                    width as i32,
                    height as i32,
                    flags::FBO,
                );
                let id = t.id.expect("render texture id");
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("TextureCombiner"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                self.views
                    .insert(id, texture.create_view(&Default::default()));
                self.combiner.targets.insert(id, texture);
                t
            }
        };
        // GL_LINEAR both ways, no mip maps; GL_REPEAT (generateHwId without
        // CLAMP_TO_EDGE) unless reset to GL_CLAMP_TO_EDGE.
        let id = tex.id.expect("render texture id");
        let sampler = self.sampler((false, false, false, clamp));
        let view = &self.views[&id];
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        self.textures.insert(id, bg);
        self.nearest.remove(&id);
        tex
    }

    /// `TextureCombiner.combine(cmdList)`.
    pub fn combine_textures(
        &mut self,
        cmds: &[CombinerCommand],
        tm: &mut TextureManager,
    ) -> Texture {
        let (width, height) = resulting_size(cmds);
        let result = self.combiner_texture(width, height, tm);
        let result_id = result.id.expect("render texture id");
        // `Texture.bind` of a texture that is not ready binds the error
        // texture.
        let mut resolve = |gpu: &mut Gpu, t: &Texture| -> Option<u32> {
            let id =
                t.id.filter(|&id| tm.data(id).is_some_and(|d| d.ready))
                    .or_else(|| tm.error_texture().id)?;
            gpu.ensure_texture(id, tm);
            Some(id)
        };
        let mut draws = Vec::new();
        let mut vertices: Vec<CombinerVertex> = Vec::new();
        let mut params: Vec<Params> = Vec::new();
        for cmd in cmds {
            let Some(tex) = resolve(self, &cmd.tex) else {
                continue;
            };
            // The mask is bound with GL_NEAREST both ways.
            let mask = match &cmd.mask {
                Some(m) => resolve(self, m),
                None => None,
            };
            if let Some(m) = mask {
                self.ensure_nearest(m);
            }
            let key = CombinerKey {
                shader: cmd.shader,
                blend: cmd.blend,
            };
            self.combiner.ensure_pipeline(&self.device, key);
            // ortho2D(0, w, h, 0) into a bottom-up framebuffer: GL's clip y
            // flipped.
            let first = vertices.len() as u32;
            for v in command_quad(cmd, width, height) {
                vertices.push(CombinerVertex {
                    pos: [
                        2.0 * v.pos[0] / width as f32 - 1.0,
                        2.0 * v.pos[1] / height as f32 - 1.0,
                    ],
                    uv: v.uv,
                });
            }
            params.push(Params::new(cmd));
            draws.push((key, tex, mask, first, params.len() as u32 - 1));
        }
        let white = self.white.clone().expect("white texture");
        if vertices.is_empty() {
            vertices.push(CombinerVertex::zeroed());
        }
        let vbuf = self.buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let mut indices: Vec<u32> = (0..draws.len() as u32)
            .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
            .collect();
        if indices.is_empty() {
            indices.push(0);
        }
        let ibuf = self.buffer(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
        let mut pbytes = vec![0u8; params.len().max(1) * PARAMS_STRIDE as usize];
        for (i, p) in params.iter().enumerate() {
            let at = i * PARAMS_STRIDE as usize;
            pbytes[at..at + std::mem::size_of::<Params>()].copy_from_slice(bytemuck::bytes_of(p));
        }
        let pbuf = self.buffer(&pbytes, wgpu::BufferUsages::UNIFORM);
        let pbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SmartShader params"),
            layout: &self.combiner.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &pbuf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
                }),
            }],
        });
        let target = self.views[&result_id].clone();
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("TextureCombiner"),
            });
        {
            // TextureFBO.startDrawing(true, true): cleared to (0, 0, 0, 0).
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TextureCombiner"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for (i, (key, tex, mask, first, p)) in draws.iter().enumerate() {
                let (Some(diffuse), Some(pipeline)) =
                    (self.textures.get(tex), self.combiner.pipelines.get(key))
                else {
                    continue;
                };
                let mask_bg = mask.and_then(|m| self.nearest.get(&m)).unwrap_or(&white);
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &pbind, &[*p * PARAMS_STRIDE as u32]);
                pass.set_bind_group(1, diffuse, &[]);
                pass.set_bind_group(2, mask_bg, &[]);
                debug_assert_eq!(*first, i as u32 * 4);
                pass.draw_indexed(i as u32 * 6..i as u32 * 6 + 6, 0, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
        result
    }

    /// `TextureCombiner.releaseTexture`.
    pub fn release_combined(&mut self, tex: Texture) {
        let Some(id) = tex.id else { return };
        if let Some(f) = self
            .combiner
            .pool
            .iter_mut()
            .find(|f| f.width == tex.width && f.height == tex.height)
        {
            if f.textures.len() < POOL_LIMIT {
                f.textures.push(tex);
                return;
            }
        }
        // Texture.destroy
        self.combiner.targets.remove(&id);
        self.textures.remove(&id);
        self.views.remove(&id);
        self.nearest.remove(&id);
    }

    /// A combined texture's pixels, RGBA, in GL row order (the first row is
    /// texture v = 0, the first row of the images it was made from).
    pub fn read_combined(&self, tex: &Texture) -> Option<Vec<u8>> {
        let texture = self.combiner.targets.get(&tex.id?)?;
        let (w, h) = (texture.width(), texture.height());
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("combiner readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        Some(out)
    }
}

impl Combiner for Gpu {
    fn combine(&mut self, cmds: &[CombinerCommand], tm: &mut TextureManager) -> Texture {
        self.combine_textures(cmds, tm)
    }

    fn release(&mut self, tex: Texture) {
        self.release_combined(tex)
    }
}

#[cfg(test)]
#[path = "combiner_tests.rs"]
mod tests;
