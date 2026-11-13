//! A rendered frame: the surface, the vertex streams, the frame's passes.
//!
//! State: presentation (reads the state, never writes it)
//!
//! A frame: the surface texture (`acquire`), the vertex streams (`stream`), `frame` / `frame_capture` and the frame's
//! passes (`frame_once`: the atlas regions, the quads into runs, the scene, shadows and ordering-table stage, the 2D
//! layer, the scaled scene's copy, the capture), the quads after the first pass (`draw_rest`, `draw_runs`, `Run`).

use super::atlas_cache::Region;
use super::{Draw, FrameStats, GpuModel, Instance, PsxQuad, Renderer, Target, atlas_cache};
use crate::output::layout::{Clip, Fit};

impl Renderer {
    /// The next frame's surface texture, acquired before anything of the frame is written (instances, vertices) and
    /// kept until a frame presents it. False when there is none, and the caller skips the frame's drawing: `Occluded`
    /// (a hidden or minimised window), `Timeout`, `Validation`, or `Outdated` / `Lost`, after which the surface is
    /// configured again for the next try (a lost surface is not recreated). A run of failures is logged once, as it
    /// starts (a warning for `Lost` and `Validation`), and its end once. The windowless renderer always has its
    /// texture.
    pub fn acquire(&mut self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(o) = &self.offscreen {
            self.out = Some(Target::Texture(o.target().clone()));
            self.presented = true;
            return true;
        }
        if self.out.is_none() {
            // the wait for a surface texture (vsync) is not the renderer's own time
            let t = if self.perf { crate::perf_now() } else { 0.0 };
            let Some(surface) = &self.surface else {
                self.presented = false;
                return false;
            };
            let failure = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                    self.out = Some(Target::Surface(t));
                    None
                }
                f @ (wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost) => {
                    surface.configure(&self.device, &self.config);
                    Some(if matches!(f, wgpu::CurrentSurfaceTexture::Lost) { "lost" } else { "outdated" })
                }
                wgpu::CurrentSurfaceTexture::Occluded => Some("occluded"),
                wgpu::CurrentSurfaceTexture::Timeout => Some("timeout"),
                wgpu::CurrentSurfaceTexture::Validation => Some("validation error"),
            };
            if self.perf {
                self.stats.wait_ms += (crate::perf_now() - t) * 1000.0;
            }
            if failure != self.acquire_failure {
                match (failure, self.acquire_failure) {
                    (Some(f @ ("lost" | "validation error")), _) => log::warn!("no surface texture: {f}; frames skipped"),
                    (Some(f), _) => log::info!("no surface texture: {f}; frames skipped"),
                    (None, Some(f)) => log::info!("surface texture acquired again (after {f})"),
                    (None, None) => {}
                }
                self.acquire_failure = failure;
            }
            self.presented = self.out.is_some();
        }
        self.presented
    }

    /// `data` in stream `k`, the buffer grown (to a power of two) when too small; None when empty.
    fn stream(&mut self, k: usize, data: &[u8]) -> Option<wgpu::Buffer> {
        if data.is_empty() {
            return None;
        }
        let n = data.len() as u64;
        if self.streams[k].as_ref().is_none_or(|b| b.size() < n) {
            self.streams[k] = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vertex stream"),
                size: n.next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let b = self.streams[k].clone()?;
        self.queue.write_buffer(&b, 0, data);
        Some(b)
    }

    /// Draw a list of (model, instance) pairs and present.
    /// Draw and present; with `capture` the frame is also returned as RGBA8 (desktop only).
    pub fn frame(&mut self, items: &[(&GpuModel, &Instance)], clear: [f64; 3]) {
        self.frame_capture(items, &[], &[], clear, false);
    }

    /// `overlay`: screen-space quads (x0, y0, x1, y1 in pixels from the top-left, sRGB colour with alpha).
    /// `fx`: PS1 quads drawn after the models in the given order (back to front, as an ordering table).
    pub fn frame_capture(
        &mut self,
        items: &[(&GpuModel, &Instance)],
        fx: &[PsxQuad],
        overlay: &[([f32; 4], [f32; 4])],
        clear: [f64; 3],
        capture: bool,
    ) -> Option<(u32, u32, Vec<u8>)> {
        // OKB_SPRITES=ab: a captured frame (not over a kept afterimage) drawn from VRAM, then from the atlas
        if self.sprites_ab && capture && self.afterimage == 0 {
            self.sprites_old = true;
            let old = self.frame_once(items, fx, overlay, clear, true);
            self.sprites_old = false;
            let new = self.frame_once(items, fx, overlay, clear, true);
            if let (Some((w, _, a)), Some((_, _, b))) = (&old, &new) {
                let diff: Vec<usize> = (0..a.len() / 4).filter(|&k| a[4 * k..4 * k + 4] != b[4 * k..4 * k + 4]).collect();
                match diff.first() {
                    Some(&k) => log::warn!("sprite A/B: {} pixels differ, first at ({}, {})", diff.len(), k as u32 % w, k as u32 / w),
                    None => log::info!("sprite A/B: identical"),
                }
            }
            self.scaled_at = None;
            self.backdrop = None;
            return new;
        }
        let out = self.frame_once(items, fx, overlay, clear, capture);
        self.scaled_at = None; // set per frame
        self.backdrop = None;
        out
    }

    fn frame_once(
        &mut self,
        items: &[(&GpuModel, &Instance)],
        fx: &[PsxQuad],
        overlay: &[([f32; 4], [f32; 4])],
        clear: [f64; 3],
        capture: bool,
    ) -> Option<(u32, u32, Vec<u8>)> {
        // the surface texture first: nothing of a frame that cannot be shown is written
        if !self.acquire() {
            return None;
        }
        let out = self.out.take()?;
        let t0 = if self.perf { crate::perf_now() } else { 0.0 };
        // performance overlay: the GPU times read back since the last frame, a free readback slot for this one
        let slot = if self.perf { self.poll_timer() } else { None };
        // atlas quads: their regions (baked on first use), then the new texels to the GPU
        let screen = std::mem::take(&mut self.screen);
        let regions: Vec<Option<Region>> = fx
            .iter()
            .chain(&screen)
            .map(|q| match q.sheet {
                Some(s) if q.textured && !self.sprites_old => atlas_cache::rect_for(&q.uv).and_then(|rect| self.atlas.get(s, rect)),
                _ => None,
            })
            .collect();
        self.flush_atlas();
        // RENDER QUALITY (engine addition): the 3D part drawn into a scaled target, not over a kept afterimage
        let scaled_at = self.scaled_at;
        let keep = self.afterimage != 0 && self.config.usage.contains(wgpu::TextureUsages::COPY_DST);
        let use_lo = scaled_at.is_some() && self.render_scale != 1.0 && !keep;
        let mut hud_v = u32::MAX;
        // effect quads → vertices and runs of (pipeline, first, count); pipelines 5.. are the atlas ones
        let lin_flag = if self.config.format.is_srgb() { 1.0 } else { 0.0 };
        let mut fxv: Vec<u8> = Vec::new();
        let mut runs: Vec<Run> = Vec::new();
        // the ambient fill's split: the vertex where the quads after it start (no run spans it)
        let split = self.ambient.filter(|a| a.at <= fx.len() && self.vp.fit == Fit::Fill);
        let mut split_v = u32::MAX;
        let push = |q: &PsxQuad, reg: Option<Region>, pipe: usize, pass: u32, fxv: &mut Vec<u8>, runs: &mut Vec<Run>, barrier: [u32; 2]| {
            let blend = ((q.tpage >> 5) & 3) as u32;
            let flags = q.textured as u32 | (q.semi as u32) << 1 | pass << 2 | blend << 4;
            let (info, pipe) = match (reg, q.sheet) {
                (Some(r), _) => {
                    let [u, v, w, h] = r.rect.map(u32::from);
                    ([r.origin[0] as u32 | (r.origin[1] as u32) << 16, u | v << 8 | r.layer << 16, flags | 64, w | h << 16], pipe + 5)
                }
                // an atlas quad drawn from the VRAM copy (OKB_SPRITES=old, or not bakeable)
                (None, Some(s)) => ([(s.page.tpage() | (q.tpage & 0x60)) as u32, s.pal.clut() as u32, flags, 0], pipe),
                (None, None) => ([q.tpage as u32, q.clut as u32, flags, 0], pipe),
            };
            let first = (fxv.len() / 56) as u32;
            for k in [0usize, 1, 2, 1, 3, 2] {
                fxv.extend_from_slice(bytemuck::cast_slice(&q.pos[k]));
                fxv.extend_from_slice(bytemuck::cast_slice(&q.uv[k]));
                let c = q.rgb[k];
                fxv.extend_from_slice(bytemuck::cast_slice(&[c[0] as f32, c[1] as f32, c[2] as f32, lin_flag]));
                fxv.extend_from_slice(bytemuck::cast_slice(&info));
            }
            match runs.last_mut() {
                Some(r) if r.pipe == pipe && r.clip == q.clip && r.first + r.count == first && !barrier.contains(&first) => r.count += 6,
                _ => runs.push(Run { pipe, first, count: 6, clip: q.clip }),
            }
        };
        for (i, (q, &reg)) in fx.iter().zip(&regions).enumerate() {
            if split.is_some_and(|a| a.at == i) {
                split_v = (fxv.len() / 56) as u32;
            }
            if use_lo && scaled_at == Some(i) {
                hud_v = (fxv.len() / 56) as u32;
            }
            let barrier = [split_v, hud_v];
            if !q.semi {
                push(q, reg, 0, 0, &mut fxv, &mut runs, barrier);
            } else {
                if q.textured {
                    push(q, reg, 0, 2, &mut fxv, &mut runs, barrier); // texels without the STP bit stay opaque
                }
                push(q, reg, 1 + ((q.tpage >> 5) & 3) as usize, 1, &mut fxv, &mut runs, barrier);
            }
        }
        // the screen quads (touch controls): their own stream and runs, drawn last over the whole window
        let (mut scr_v, mut scr_runs): (Vec<u8>, Vec<Run>) = (Vec::new(), Vec::new());
        for (q, &reg) in screen.iter().zip(&regions[fx.len()..]) {
            if !q.semi {
                push(q, reg, 0, 0, &mut scr_v, &mut scr_runs, [u32::MAX; 2]);
            } else {
                if q.textured {
                    push(q, reg, 0, 2, &mut scr_v, &mut scr_runs, [u32::MAX; 2]);
                }
                push(q, reg, 1 + ((q.tpage >> 5) & 3) as usize, 1, &mut scr_v, &mut scr_runs, [u32::MAX; 2]);
            }
        }
        if split.is_some_and(|a| a.at == fx.len()) {
            split_v = (fxv.len() / 56) as u32;
        }
        if use_lo && scaled_at == Some(fx.len()) {
            hud_v = (fxv.len() / 56) as u32;
        }
        // AFTERIMAGE: a full-screen subtractive F4 of (16,16,16) before the scene darkens the kept image
        let (mut prev, mut pre_runs): (Vec<u8>, Vec<Run>) = (Vec::new(), Vec::new());
        if matches!(self.afterimage, 1 | 2) && self.config.usage.contains(wgpu::TextureUsages::COPY_DST) {
            let q = PsxQuad {
                pos: [[-1.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0], [-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0]],
                uv: [[0.0; 2]; 4],
                // the PS1 adds / subtracts 16 per frame in gamma space (white / black within 16 frames); an sRGB target
                // blends in linear light, where 72 gives the same 16-frame time to white / black
                rgb: [[if self.manual_srgb { 16 } else { 72 }; 3]; 4],
                tpage: 0x40,
                clut: 0,
                textured: false,
                semi: true,
                sheet: None,
                clip: Clip::No,
            };
            push(&q, None, if self.afterimage == 2 { 2 } else { 3 }, 1, &mut prev, &mut pre_runs, [u32::MAX; 2]);
        }
        // the fight backdrop: the frame cleared with the bottom colour (DRAWENV), rows 0..h filled with the top colour
        // (ClearImage) before the ordering table; above the 480 rows (a view taller than 4:3) the top colour goes on
        let mut clear = clear;
        if let Some(b) = self.backdrop {
            // the 15-bit frame buffer: 8-bit values truncated to 5 bits, shown as c << 3 | c >> 2
            let rgb = |c: [u8; 3]| c.map(|v| (v >> 3) << 3 | v >> 5);
            let lin = |v: u8| if lin_flag > 0.5 { (v as f64 / 255.0).powf(2.2) } else { v as f64 / 255.0 };
            clear = rgb(b.bottom).map(lin);
            if b.rows > 0 {
                // rows ≤ 480 (`backdrop` clamps): beyond the 480 rows of a tall view the bottom colour goes on too
                let y = (240 - b.rows) as f32 / 240.0 * self.vp.fov3d()[1];
                let q = PsxQuad {
                    pos: [[-1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0], [-1.0, y, 1.0, 1.0], [1.0, y, 1.0, 1.0]],
                    uv: [[0.0; 2]; 4],
                    rgb: [rgb(b.top); 4],
                    tpage: 0,
                    clut: 0,
                    textured: false,
                    semi: false,
                    sheet: None,
                    clip: Clip::No,
                };
                push(&q, None, 0, 0, &mut prev, &mut pre_runs, [u32::MAX; 2]);
            }
        }
        let pre_buf = self.stream(1, &prev);
        let fx_buf = self.stream(0, &fxv);
        let vr = self.vp.view_rect();
        let (sw, sh) = (vr[2] as f32, vr[3] as f32);
        let lin = |c: f32| if self.config.format.is_srgb() { c.powf(2.2) } else { c };
        let mut verts: Vec<f32> = Vec::with_capacity(overlay.len() * 36 + 36);
        // 4:3 mode: the window is cleared black and the 4:3 area filled with the clear colour (the attachment's
        // clear value is linear, as the vertex colour here)
        let fill_clear = self.vp.fit == Fit::Classic && !(keep && self.persist_used);
        if fill_clear {
            let col = [clear[0] as f32, clear[1] as f32, clear[2] as f32, 1.0];
            for (x, y) in [(-1.0f32, 1.0f32), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                verts.extend_from_slice(&[x, y]);
                verts.extend_from_slice(&col);
            }
        }
        let first_overlay = (verts.len() / 6) as u32;
        for (r, c) in overlay {
            let (x0, y0) = (r[0] / sw * 2.0 - 1.0, 1.0 - r[1] / sh * 2.0);
            let (x1, y1) = (r[2] / sw * 2.0 - 1.0, 1.0 - r[3] / sh * 2.0);
            let col = [lin(c[0]), lin(c[1]), lin(c[2]), c[3]];
            for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y0), (x1, y1), (x0, y1)] {
                verts.extend_from_slice(&[x, y]);
                verts.extend_from_slice(&col);
            }
        }
        let overlay_buf = self.stream(2, bytemuck::cast_slice(&verts));
        // AFTERIMAGE: draw into the kept image (no clear), copied to the screen afterwards
        if !keep {
            self.persist = None;
        } else if self.persist.is_none() {
            self.persist = Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("afterimage"),
                size: wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let fresh_persist = keep && !self.persist_used;
        self.persist_used = keep;
        let view = match (&self.persist, keep) {
            (Some(t), true) => t.create_view(&Default::default()),
            _ => out.texture().create_view(&Default::default()),
        };
        // the ambient fill copies its rect from the target (the kept image, or the surface when it allows copies;
        // otherwise the bars keep the clear colour)
        let can_copy = keep || self.config.usage.contains(wgpu::TextureUsages::COPY_SRC);
        let amb = split.filter(|a| can_copy && a.rect[2] > 0 && a.rect[3] > 0);
        if let Some(a) = amb {
            self.amb_texture(a.rect[2], a.rect[3]);
        }
        let amb_v = amb.map(|a| self.ambient_verts(a.rect)).unwrap_or_default();
        let amb_buf = self.stream(3, bytemuck::cast_slice(&amb_v));
        // runs before the split go into the first pass (the capped 3D view: its band as viewport and scissor)
        let n1 = if split.is_some() { runs.iter().position(|r| r.first >= split_v).unwrap_or(runs.len()) } else { runs.len() };
        let whole = self.vp.scissor().unwrap_or([0, 0, self.config.width, self.config.height]);
        let scene = split.filter(|a| a.scene).map(|a| a.rect);
        let area = scene.unwrap_or(whole);
        if use_lo {
            self.lo_target();
        } else if self.render_scale == 1.0 {
            // back at 100 %: the target's memory given back (kept while an afterimage only forces 100 % for a while)
            self.lo = None;
        }
        let lo = if use_lo { self.lo.as_ref() } else { None };
        // the runs before the 2D layer go into the scaled scene, the rest is drawn at the window's resolution
        let k1 = if lo.is_some() { runs.iter().position(|r| r.first >= hud_v).unwrap_or(runs.len()).min(n1) } else { n1 };
        let ov = first_overlay..(verts.len() / 6) as u32;
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let (target, depth, area, core) = match lo {
                Some(t) => {
                    (&t.view, &t.depth, t.rect(area, self.config.width, self.config.height), t.rect(self.vp.core, self.config.width, self.config.height))
                }
                None => (&view, &self.depth, area, self.vp.core),
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if keep && !fresh_persist {
                            wgpu::LoadOp::Load
                        } else {
                            let c = if self.vp.fit == Fit::Classic { [0.0; 3] } else { clear };
                            wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 })
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().filter(|_| slot.is_some()).map(|t| wgpu::RenderPassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: (amb.is_none() && lo.is_none()).then_some(1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The PS1 has no depth buffer: the stage draw (the routine by M.98, the far walls and the debris of
            // the stage draw) links whole objects at fixed far OT slots (M.8 + 0x203C..0x2078 = 0x80F..0x81E), the fighter
            // shadows at 0x810, while fighter polygons (0x10 + avg SZ / 4, at most 0x80F) and
            // the effects (OT[otz], culled from 0x80F) sort in front of them. So the stage and the shadows keep their
            // own depth order, then the depth is reset to the far plane: the fighters and effects are drawn over the
            // stage everywhere. A stage in OT order (`OtDraw`) has no depth order at all: its units in the order
            // given, the shadows over them, then the units of slot 0x80F.
            if self.vp.scissor().is_some() || scene.is_some() {
                let [x, y, w, h] = area;
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
            }
            if let (true, Some(buf)) = (fill_clear, &overlay_buf) {
                pass.set_pipeline(&self.flat);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..first_overlay, 0..1);
            }
            if let Some(buf) = &pre_buf {
                pass.set_bind_group(0, &self.vram_bind, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                for r in &pre_runs {
                    pass.set_pipeline(&self.psx[r.pipe]);
                    pass.draw(r.first..r.first + r.count, 0..1);
                }
            }
            let models = |pass: &mut wgpu::RenderPass<'_>, stage: bool| {
                pass.set_pipeline(&self.pipeline);
                for (m, inst) in items.iter().filter(|(_, i)| !i.shadow.get() && i.stage.get() == stage && i.ot.get().is_none()) {
                    if m.lit.is_some() {
                        // a lightmapped mesh or a sky box (`lightmapped`)
                        self.draw_lit(pass, m, inst);
                        pass.set_pipeline(&self.pipeline);
                        continue;
                    }
                    pass.set_bind_group(0, &inst.bind, &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    if inst.semi.get() {
                        for k in 0..2 {
                            for &Draw { first, count, tex, hidden, wall, over, .. } in &m.draws {
                                if !hidden && !wall.is_some_and(|k| self.walls_hidden & (1 << k) != 0) {
                                    pass.set_pipeline(&self.semi_pipes[over as usize][k]);
                                    pass.set_bind_group(1, if tex == 0 { &self.white_stp } else { &m.textures[tex - 1] }, &[]);
                                    pass.draw_indexed(first..first + count, 0, 0..1);
                                }
                            }
                        }
                        pass.set_pipeline(&self.pipeline);
                        continue;
                    }
                    for &Draw { first, count, tex, hidden, wall, .. } in &m.draws {
                        if hidden || wall.is_some_and(|k| self.walls_hidden & (1 << k) != 0) {
                            continue;
                        }
                        if let Some(p) = m.pal.get(tex.wrapping_sub(1)).and_then(Option::as_ref) {
                            // a paletted texture (`set_paletted`): its own pipeline for this draw
                            pass.set_pipeline(&self.pal_pipe);
                            pass.set_bind_group(1, &p.bind, &[]);
                            pass.draw_indexed(first..first + count, 0, 0..1);
                            pass.set_pipeline(&self.pipeline);
                            continue;
                        }
                        let bg = if tex == 0 { &self.white } else { &m.textures[tex - 1] };
                        pass.set_bind_group(1, bg, &[]);
                        pass.draw_indexed(first..first + count, 0, 0..1);
                    }
                }
            };
            // the stage in OT order: each draw over what is there; semi-transparent polygons in the two passes of the
            // drawn-over semi pipelines (texels without STP opaque, then the STP texels added)
            let ot = |pass: &mut wgpu::RenderPass<'_>, late: bool| {
                for (m, inst) in items.iter() {
                    let Some(o) = inst.ot.get().filter(|o| o.late == late) else { continue };
                    pass.set_bind_group(0, &inst.bind, &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    let (a, b) = o.range.unwrap_or((0, m.draws.len() as u32));
                    for &Draw { first, count, tex, hidden, semi, .. } in &m.draws[a as usize..b as usize] {
                        if hidden {
                            continue;
                        }
                        if semi {
                            for k in 0..2 {
                                pass.set_pipeline(&self.semi_pipes[1][k]);
                                pass.set_bind_group(1, if tex == 0 { &self.white_stp } else { &m.textures[tex - 1] }, &[]);
                                pass.draw_indexed(first..first + count, 0, 0..1);
                            }
                        } else if let Some(p) = m.pal.get(tex.wrapping_sub(1)).and_then(Option::as_ref) {
                            pass.set_pipeline(&self.ot_pipes[1]);
                            pass.set_bind_group(1, &p.bind, &[]);
                            pass.draw_indexed(first..first + count, 0, 0..1);
                        } else {
                            pass.set_pipeline(&self.ot_pipes[0]);
                            pass.set_bind_group(1, if tex == 0 { &self.white } else { &m.textures[tex - 1] }, &[]);
                            pass.draw_indexed(first..first + count, 0, 0..1);
                        }
                    }
                }
            };
            models(&mut pass, true);
            ot(&mut pass, false);
            // fighter shadows: over the stage (depth-tested against it, not written; over all of a stage in OT order,
            // as slot 0x810), under the fighters
            let ot_stage = items.iter().any(|(_, i)| i.ot.get().is_some());
            pass.set_pipeline(&self.shadow_pipes[ot_stage as usize]);
            pass.set_bind_group(1, &self.white, &[]);
            for (m, inst) in items.iter().filter(|(_, i)| i.shadow.get()) {
                pass.set_bind_group(0, &inst.bind, &[]);
                pass.set_vertex_buffer(0, m.vbuf.slice(..));
                pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                for &Draw { first, count, shadow, hidden, .. } in &m.draws {
                    if shadow || (inst.shadow_all.get() && !hidden) {
                        pass.draw_indexed(first..first + count, 0, 0..1);
                    }
                }
            }
            ot(&mut pass, true);
            pass.set_pipeline(&self.depth_reset);
            pass.draw(0..3, 0..1);
            models(&mut pass, false);
            if let Some(buf) = &fx_buf {
                self.draw_runs(&mut pass, buf, &runs[..k1], area, core);
            }
            if amb.is_none() && lo.is_none() {
                self.draw_rest(&mut pass, scene, whole, fx_buf.as_ref(), &runs[n1..], overlay_buf.as_ref(), ov.clone());
            }
        }
        if let Some(t) = lo {
            // RENDER QUALITY: the scaled scene over the whole window, then the 2D layer at the window's resolution
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.lo_pipe);
                pass.set_bind_group(0, &t.bind, &[]);
                pass.set_vertex_buffer(0, t.quad.slice(..));
                pass.draw(0..6, 0..1);
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().filter(|_| slot.is_some()).map(|t| wgpu::RenderPassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: amb.is_none().then_some(1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if self.vp.scissor().is_some() || scene.is_some() {
                let [x, y, w, h] = area;
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
            }
            if let Some(buf) = &fx_buf {
                self.draw_runs(&mut pass, buf, &runs[k1..n1], area, self.vp.core);
            }
            if amb.is_none() {
                self.draw_rest(&mut pass, scene, whole, fx_buf.as_ref(), &runs[n1..], overlay_buf.as_ref(), ov.clone());
            }
        }
        // the ambient fill: the rect copied out, then drawn scaled up and blurred around it; the quads after the
        // split (HUD, fades, text) on top
        if let (Some(a), Some(ab), Some(t)) = (amb, &amb_buf, &self.amb) {
            let target = match (&self.persist, keep) {
                (Some(t), true) => t,
                _ => out.texture(),
            };
            let [x, y, w, h] = a.rect;
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                t.copy.as_image_copy(),
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            {
                // the blur at 1/4 size (the bars show it scaled up)
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.small_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.amb_blur);
                pass.set_bind_group(0, &t.copy_bind, &[]);
                pass.set_vertex_buffer(0, ab.slice(..));
                pass.draw(0..6, 0..1);
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().filter(|_| slot.is_some()).map(|t| wgpu::RenderPassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.amb_pipe);
            pass.set_bind_group(0, &t.small_bind, &[]);
            pass.set_vertex_buffer(0, ab.slice(..));
            pass.draw(6..(amb_v.len() / 4) as u32, 0..1);
            if let Some(buf) = &fx_buf {
                self.draw_runs(&mut pass, buf, &runs[n1..], whole, self.vp.core);
            }
            if let Some(buf) = &overlay_buf {
                let [x, y, w, h] = whole;
                pass.set_scissor_rect(x, y, w, h);
                pass.set_pipeline(&self.flat);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(ov, 0..1);
            }
        }
        if let (Some(i), Some(t)) = (slot, &self.timer) {
            enc.resolve_query_set(&t.set, 0..2, &t.resolve, 0);
            enc.copy_buffer_to_buffer(&t.resolve, 0, &t.read[i].0, 0, 16);
        }
        if let (true, Some(t)) = (keep, &self.persist) {
            enc.copy_texture_to_texture(
                t.as_image_copy(),
                out.texture().as_image_copy(),
                wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
            );
        }
        if let Some(buf) = self.stream(4, &scr_v) {
            let target = out.texture().create_view(&Default::default());
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let all = [0, 0, self.config.width, self.config.height];
            self.draw_runs(&mut pass, &buf, &scr_runs, all, all);
        }
        let (w, h) = (self.config.width, self.config.height);
        let row = (w * 4).div_ceil(256) * 256;
        let readback = (capture && self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)).then(|| {
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (row * h) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            enc.copy_texture_to_buffer(
                out.texture().as_image_copy(),
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            buf
        });
        // the windowless renderer: the frame copied into its next readback buffer (`offscreen`)
        #[cfg(not(target_arch = "wasm32"))]
        let video = self.offscreen.as_mut().and_then(|o| o.copy(&mut enc, out.texture()));
        #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
        let submitted = self.queue.submit([enc.finish()]);
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(o), Some(k)) = (&mut self.offscreen, video) {
            o.submitted(k, submitted);
        }
        if let Target::Surface(t) = out {
            self.queue.present(t);
        }
        // what the frames released is freed once the device is polled
        let _ = self.device.poll(wgpu::PollType::Poll);
        if let (Some(i), Some(t)) = (slot, &self.timer) {
            let (b, st) = &t.read[i];
            st.store(1, std::sync::atomic::Ordering::Release);
            let st = st.clone();
            b.slice(..).map_async(wgpu::MapMode::Read, move |r| st.store(if r.is_ok() { 2 } else { 0 }, std::sync::atomic::Ordering::Release));
        }
        if self.perf {
            // the counts of what was drawn: model primitives, quad runs, the 2D overlay
            // (+ 1: the depth reset between the stage and the fighters)
            let mut draws = (runs.len() + pre_runs.len()) as u32 + overlay_buf.is_some() as u32 + fill_clear as u32 + 2 * amb.is_some() as u32 + 1;
            let mut tris = (fxv.len() / 56 + prev.len() / 56) as u32 / 3 + (verts.len() / 6) as u32 / 3;
            for (m, inst) in items {
                let ot = inst.ot.get();
                let (a, b) = ot.and_then(|o| o.range).unwrap_or((0, m.draws.len() as u32));
                for &Draw { count, hidden, wall, shadow, semi, .. } in &m.draws[a as usize..b as usize] {
                    let shown = if inst.shadow.get() {
                        shadow || (inst.shadow_all.get() && !hidden)
                    } else {
                        !hidden && (ot.is_some() || !wall.is_some_and(|k| self.walls_hidden & (1 << k) != 0))
                    };
                    if shown {
                        // a semi-transparent instance or polygon draws twice (fs_stp_opaque, fs_stp_add)
                        let n = 1 + ((inst.semi.get() || (ot.is_some() && semi)) && !inst.shadow.get()) as u32;
                        draws += n;
                        tris += n * count / 3;
                    }
                }
            }
            self.stats = FrameStats { draws, quads: fx.len() as u32, tris, uploads: self.uploads, cpu_ms: (crate::perf_now() - t0) * 1000.0, ..self.stats };
        }
        self.uploads = 0;
        let buf = readback?;
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let data = buf.slice(..).get_mapped_range().ok()?;
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let line = &data[(y * row) as usize..(y * row + w * 4) as usize];
            for px in line.as_chunks::<4>().0 {
                if bgra {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        Some((w, h, rgba))
    }

    /// The quads after the first pass's part and the overlay, over the whole drawn area (`whole`; a capped 3D view's
    /// band was the viewport before).
    fn draw_rest(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        scene: Option<[u32; 4]>,
        whole: [u32; 4],
        fx_buf: Option<&wgpu::Buffer>,
        runs: &[Run],
        overlay_buf: Option<&wgpu::Buffer>,
        ov: std::ops::Range<u32>,
    ) {
        if scene.is_some() {
            let [x, y, w, h] = whole;
            pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, w, h);
        }
        if let Some(buf) = fx_buf {
            self.draw_runs(pass, buf, runs, whole, self.vp.core);
        }
        if let Some(buf) = overlay_buf {
            // the runs leave their last clip rect as scissor
            let [x, y, w, h] = whole;
            pass.set_scissor_rect(x, y, w, h);
            pass.set_pipeline(&self.flat);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(ov, 0..1);
        }
    }

    /// Effect / HUD quad runs with their pipelines and textures, each scissored by its `Clip` (`No`: `area`;
    /// `Outside`: drawn once per non-empty strip of `area` around `core`, the core 4:3 rectangle in the target).
    fn draw_runs(&self, pass: &mut wgpu::RenderPass<'_>, buf: &wgpu::Buffer, runs: &[Run], area: [u32; 4], core: [u32; 4]) {
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.set_blend_constant(wgpu::Color { r: 0.5, g: 0.5, b: 0.5, a: 0.5 });
        let (mut bound, mut cut) = (None, None);
        for r in runs {
            let atlas = r.pipe >= 5;
            if bound != Some(atlas) {
                pass.set_bind_group(0, if atlas { &self.atlas_bind } else { &self.vram_bind }, &[]);
                bound = Some(atlas);
            }
            let ([ax, ay, aw, ah], [cx, cy, cw, ch]) = (area, core);
            let (ar, ab, cr, cb) = (ax + aw, ay + ah, cx + cw, cy + ch);
            let rects = match r.clip {
                Clip::No => [area, [0; 4], [0; 4], [0; 4]],
                Clip::Core => [core, [0; 4], [0; 4], [0; 4]],
                Clip::Outside => [
                    [ax, ay, cx.saturating_sub(ax), ah],
                    [cr, ay, ar.saturating_sub(cr), ah],
                    [cx, ay, cw, cy.saturating_sub(ay)],
                    [cx, cb, cw, ab.saturating_sub(cb)],
                ],
            };
            pass.set_pipeline(if atlas { &self.sprite[r.pipe - 5] } else { &self.psx[r.pipe] });
            for sc in rects.into_iter().filter(|s| s[2] > 0 && s[3] > 0) {
                if cut != Some(sc) {
                    pass.set_scissor_rect(sc[0], sc[1], sc[2], sc[3]);
                    cut = Some(sc);
                }
                pass.draw(r.first..r.first + r.count, 0..1);
            }
        }
    }
}

/// A run of effect / HUD quad vertices drawn with one pipeline (0..5 VRAM, 5.. atlas).
#[derive(Clone, Copy)]
struct Run {
    pipe: usize,
    first: u32,
    count: u32,
    clip: Clip,
}
