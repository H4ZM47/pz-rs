//! The save thumbnail's read-back (`SavefileThumbnail.TakeScreenShotDrawer`
//! on the render thread, after the thumbnail's world render at zoom 1).
//!
//! With the offscreen world buffer (the usual case) the original runs
//! `createWithRenderShader`: a 256x256 `TextureFBO` gets one quad of the
//! offscreen buffer's texel square `sx..sx + 256`, `sy..sy + 256` (`sx =
//! screenLeft + screenWidth / 2 - 128`, the same for `sy` from the top),
//! drawn 1:1 with the `screen` shader (`WeatherShader`, as the frame's
//! screen pass grades the world) without blending, depth or stencil, and
//! `glReadPixels(0, 0, 256, 256)` reads all of it. `sy` is taken as a GL
//! row although it was measured from the top, so the square is the
//! player's view mirrored about its middle row; for a full-screen view of
//! even height that is exactly the middle 256x256 of the screen. Without
//! the offscreen buffer it reads the back buffer's rectangle instead
//! (`Core.TakeScreenshot(256, 256, GL_BACK)`). Either way the rows come
//! back bottom-up, as `glReadPixels` returns them.

use super::*;
use pz_render::ScreenParams;

/// `SavefileThumbnail.WIDTH` / `HEIGHT`.
pub const THUMBNAIL_SIZE: u32 = 256;

impl Gpu {
    /// `createWithRenderShader(player)`: the 256x256 square at texel
    /// (`sx`, `sy`) of the offscreen buffer (GL rows, bottom-up) drawn
    /// through the `screen` shader and read back, RGBA rows bottom-up.
    /// `Ok(None)` when there is no offscreen buffer (nothing has drawn into
    /// one yet). Texels outside the buffer repeat its edge (clamp to edge).
    pub fn read_offscreen_thumbnail(
        &mut self,
        sx: i32,
        sy: i32,
        params: ScreenParams,
    ) -> Result<Option<Vec<u8>>, GpuError> {
        if let Some(e @ GpuError::DeviceLost(_)) = self.faults.get() {
            return Err(e);
        }
        let Some((ow, oh)) = self.offscreen.as_ref().map(|o| (o.width, o.height)) else {
            return Ok(None);
        };
        let size = THUMBNAIL_SIZE;
        let tex = |d: &wgpu::Device, format, usage, label| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let Some((color, depth)) = self.allocate("thumbnail FBO", |d| {
            (
                tex(d, state::OFFSCREEN_COLOR_FORMAT, usage, "thumbnail"),
                tex(
                    d,
                    state::DEPTH_STENCIL_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT,
                    "thumbnail depth",
                ),
            )
        }) else {
            return Err(self
                .faults
                .get()
                .unwrap_or_else(|| GpuError::Allocation("thumbnail FBO".into())));
        };
        // glDisable(SCISSOR, STENCIL, BLEND, ALPHA_TEST, DEPTH, CULL).
        let key = PipeKey {
            blend: None,
            stencil: None,
            depth: None,
            program: Program::Screen,
            target: TargetKind::Offscreen,
            ..GlState::default().pipe_key()
        };
        self.ensure_pipeline(key);
        let (fw, fh) = (size as f32, size as f32);
        let (tw, th) = (ow as f32, oh as f32);
        let (u0, u1) = (sx as f32 / tw, (sx + size as i32) as f32 / tw);
        let (v0, v1) = (sy as f32 / th, (sy + size as i32) as f32 / th);
        // `projection.setOrtho2D(0, 256, 256, 0)`; a texture target, so the
        // projection is flipped to keep GL's row order in memory.
        let mut mvp = gl::ortho2d(0.0, fw, fh, 0.0);
        for col in &mut mvp {
            col[1] = -col[1];
        }
        let mut g = Globals::zeroed();
        g.mvp = mvp;
        g.alpha_func = 8;
        g.clip_z = 0.5;
        // `startRenderThread`: the offscreen size and its texture size.
        g.screen = [
            [params.desaturation, params.timer, tw, th],
            [tw, th, 0.0, 0.0],
        ];
        let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
            pos: [x, y],
            uv: [u, v],
            color: 0xFFFF_FFFF,
            uv1: [0.0; 2],
            uv2: [0.0; 2],
            depth: [0.0; 2],
        };
        // `vbor.addQuad(0, 0, u0, v1, 0, 256, u0, v0, 256, 256, u1, v0, 256, 0, u1, v1)`.
        let verts = [
            corner(0.0, 0.0, u0, v1),
            corner(0.0, fh, u0, v0),
            corner(fw, fh, u1, v0),
            corner(fw, 0.0, u1, v1),
        ];
        let vbuf = self.buffer(bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
        let ibuf = self.buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            wgpu::BufferUsages::INDEX,
        );
        let mut gbytes = vec![0u8; GLOBALS_STRIDE as usize];
        gbytes[..std::mem::size_of::<Globals>()].copy_from_slice(bytemuck::bytes_of(&g));
        let gbuf = self.buffer(&gbytes, wgpu::BufferUsages::UNIFORM);
        let gbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("thumbnail globals"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &gbuf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Globals>() as u64),
                }),
            }],
        });
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("thumbnail"),
            });
        {
            let o = self.offscreen.as_ref().expect("offscreen buffer");
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("thumbnail"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    // A new texture: GL leaves it undefined, the quad covers it.
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, fw, fh, 0.0, 1.0);
            pass.set_pipeline(&self.pipelines[&key]);
            pass.set_bind_group(0, &gbind, &[0]);
            pass.set_bind_group(1, &o.bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..1);
        }
        self.queue.submit([enc.finish()]);
        // Memory row 0 is GL row 0 here, so the rows already run bottom-up.
        self.read_texture(&color, size, size).map(Some)
    }

    /// `Core.TakeScreenshot(x, y, w, h, GL_BACK)` on the headless target:
    /// the GL window rectangle (origin bottom-left) as RGBA rows bottom-up;
    /// pixels outside the target are zero. `Ok(None)` for a window surface,
    /// whose back buffer cannot be read.
    pub fn read_main_rect(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<Option<Vec<u8>>, GpuError> {
        let Target::Headless { .. } = &self.target else {
            return Ok(None);
        };
        let top_down = self.try_read_pixels()?;
        let (w, h) = (self.width as i32, self.height as i32);
        let mut out = vec![0u8; (width * height * 4) as usize];
        for r in 0..height as i32 {
            // GL row y + r is memory row h - 1 - (y + r).
            let gy = y + r;
            if gy < 0 || gy >= h {
                continue;
            }
            let src_row = (h - 1 - gy) as usize;
            for c in 0..width as i32 {
                let gx = x + c;
                if gx < 0 || gx >= w {
                    continue;
                }
                let s = (src_row * w as usize + gx as usize) * 4;
                let d = ((r as u32 * width + c as u32) * 4) as usize;
                out[d..d + 4].copy_from_slice(&top_down[s..s + 4]);
            }
        }
        Ok(Some(out))
    }

    /// Copies an RGBA8 texture's memory rows (first row first) to the CPU.
    fn read_texture(&self, texture: &wgpu::Texture, w: u32, h: u32) -> Result<Vec<u8>, GpuError> {
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("thumbnail readback"),
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
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let polled = self.device.poll(wgpu::PollType::wait_indefinitely());
        let lost = |why: String| self.faults.get().unwrap_or(GpuError::DeviceLost(why));
        match rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(lost(format!("readback map: {e}"))),
            Err(_) => return Err(lost(format!("readback never mapped ({polled:?})"))),
        }
        let data = slice
            .get_mapped_range()
            .map_err(|e| lost(format!("readback range: {e:?}")))?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "thumbnail_tests.rs"]
mod tests;
