//! `ModelOutlines`: the coloured outlines around highlighted characters
//! (aim targets, debug highlights, picked characters), and the per-object
//! highlight state `IsoObject` keeps for them.
//!
//! A highlighted character's meshes are drawn first as a solid silhouette
//! into a screen-sized mask (`ModelSlotRenderData.performRenderCharacterOutline`
//! with `aim_outline_solid`); consecutive characters with the same colour
//! and side of the player share one mask. When the colour or side changes,
//! and at the end of the player's world render (`endFrameMain`), the mask
//! is thickened (`aim_outline_h`, `aim_outline_v`) and drawn over the world
//! where the mask itself is empty (`aim_outline_blit`), so only the ring
//! around the silhouettes shows. For outlines behind the local player, the
//! player is drawn into the mask before the blit so the ring does not
//! cover it.
//!
//! The draws come out as [`DrawCmd::OutlineBegin`] .. [`DrawCmd::OutlineBlit`]
//! for the GPU backend.

use pz_render::model::ModelDraw;
use pz_render::{gl, DrawCmd};

/// `ColorInfo.setABGR`: bytes times `0.003921569f` (not divided by 255).
pub fn abgr_to_color(abgr: u32) -> [f32; 4] {
    let ch = |shift: u32| 0.003_921_569f32 * ((abgr >> shift) & 0xFF) as f32;
    [ch(0), ch(8), ch(16), ch(24)]
}

/// `IsoObject`'s outline highlight fields: `isOutlineHighlight`,
/// `isOutlineHlAttached`, `isOutlineHlBlink` (a bit per player) and
/// `outlineHighlightCol` (an ABGR colour per player).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineHighlight {
    pub highlight: u8,
    pub attached: u8,
    pub blink: u8,
    pub col: [u32; 4],
}

impl Default for OutlineHighlight {
    fn default() -> Self {
        // `new int[4]` filled with white (`-1`).
        OutlineHighlight {
            highlight: 0,
            attached: 0,
            blink: 0,
            col: [u32::MAX; 4],
        }
    }
}

impl OutlineHighlight {
    /// `setOutlineHighlight(boolean)`: every player.
    pub fn set_all(&mut self, on: bool) {
        self.highlight = if on { 0xFF } else { 0 };
    }

    /// `setOutlineHighlight(playerIndex, boolean)`.
    pub fn set(&mut self, player: usize, on: bool) {
        if on {
            self.highlight |= 1 << player;
        } else {
            self.highlight &= !(1 << player);
        }
    }

    /// `isOutlineHighlight(playerIndex)` (with the
    /// `renderContainerHighlight` debug option on, its default).
    pub fn is_on(&self, player: usize) -> bool {
        self.highlight & (1 << player) != 0
    }

    /// `setOutlineHighlightCol(r, g, b, a)`: every player.
    pub fn set_col_all(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.col = [pz_render::color_to_abgr(r, g, b, a); 4];
    }

    /// `setOutlineHighlightCol(playerIndex, r, g, b, a)`.
    pub fn set_col(&mut self, player: usize, r: f32, g: f32, b: f32, a: f32) {
        self.col[player] = pz_render::color_to_abgr(r, g, b, a);
    }

    /// `outlineColor.setABGR(getOutlineHighlightCol(playerIndex))`.
    pub fn color(&self, player: usize) -> [f32; 4] {
        abgr_to_color(self.col[player])
    }

    /// `ModelSlotRenderData.initModel`/`init` for a character: an outline
    /// request when it is highlighted for the player, coloured and sided
    /// as `init` sets them.
    pub fn character_request(
        &self,
        player: usize,
        x: f32,
        y: f32,
        prone: bool,
        cam_x: f32,
        cam_y: f32,
    ) -> Option<OutlineRequest> {
        self.is_on(player).then(|| OutlineRequest {
            color: self.color(player),
            behind_player: behind_player(x, y, prone, cam_x, cam_y),
        })
    }

    /// `unsetOutlineHighlight`.
    pub fn unset(&mut self) {
        self.highlight = 0;
        self.blink = 0;
        self.attached = 0;
    }
}

/// `ModelSlotRenderData.outlineBehindPlayer`: the character is prone or
/// nearer the top of the screen than the camera character
/// (`x + y < camCharacterX + camCharacterY`).
pub fn behind_player(x: f32, y: f32, prone: bool, cam_x: f32, cam_y: f32) -> bool {
    prone || x + y < cam_x + cam_y
}

/// `PZMath.lerp(0.5, 0.2, zoom / 2.5)`: the thickening radius in pixels
/// of the screen size.
pub fn radius(zoom: f32) -> f32 {
    0.5 + (0.2 - 0.5) * (zoom / 2.5)
}

/// A character's outline request, as `ModelSlotRenderData.init` reads it:
/// the colour (`setABGR` of the highlight colour) and which side of the
/// player it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutlineRequest {
    pub color: [f32; 4],
    pub behind_player: bool,
}

/// `ModelOutlines.instance` for one player's world render.
#[derive(Clone, Debug)]
pub struct ModelOutlines {
    /// `Core.width`/`height`: the mask's size and the viewport the passes
    /// cover (one player's screen).
    pub width: u32,
    pub height: u32,
    /// `SpriteRenderer.getPlayerZoomLevel()`.
    pub zoom: f32,
    pub dirty: bool,
    color: [f32; 4],
    behind_player: bool,
    /// `setPlayerRenderData`: the local player's meshes.
    player: Option<Vec<ModelDraw>>,
}

impl ModelOutlines {
    pub fn new(width: u32, height: u32, zoom: f32) -> Self {
        ModelOutlines {
            width,
            height,
            zoom,
            dirty: false,
            color: [0.0; 4],
            behind_player: false,
            player: None,
        }
    }

    /// `startFrame` (from `startFrameMain`, before `IsoWorld.render`).
    pub fn start_frame(&mut self) {
        self.dirty = false;
        self.player = None;
    }

    /// `setPlayerRenderData`: the local player drawn this frame without
    /// an outline of its own.
    pub fn set_player_render_data(&mut self, draws: Vec<ModelDraw>) {
        self.player = Some(draws);
    }

    /// `beginRenderOutline`: flushes the pending outline when the colour
    /// or side changes (unless drawing the player into the mask), and
    /// says whether the mask must be cleared (the first silhouette of a
    /// new outline).
    pub fn begin_render_outline(
        &mut self,
        color: [f32; 4],
        behind: bool,
        player_to_mask: bool,
        out: &mut Vec<DrawCmd>,
    ) -> bool {
        if !(player_to_mask || !self.dirty || self.color == color && behind == self.behind_player) {
            self.end_frame(out);
            self.dirty = false;
        }
        self.color = color;
        self.behind_player = behind;
        if self.dirty {
            return false;
        }
        self.dirty = true;
        true
    }

    /// `performRenderCharacterOutline(bPlayerToMask, color, behind)`: the
    /// meshes as a solid silhouette (`u_color` = colour with alpha 1) into
    /// the mask.
    pub fn render_character_outline(
        &mut self,
        draws: &[ModelDraw],
        color: [f32; 4],
        behind: bool,
        player_to_mask: bool,
        out: &mut Vec<DrawCmd>,
    ) {
        let clear = self.begin_render_outline(color, behind, player_to_mask, out);
        out.push(DrawCmd::OutlineBegin {
            width: self.width,
            height: self.height,
            clear,
        });
        for d in draws {
            out.push(DrawCmd::OutlineModel {
                draw: Box::new(d.clone()),
                color: [color[0], color[1], color[2]],
            });
        }
        out.push(DrawCmd::OutlineEnd);
    }

    /// `endFrame`: thickens the pending mask and draws the ring over the
    /// world (with the local player masked out when the outline is behind
    /// it). Nothing when no outline is pending.
    pub fn end_frame(&mut self, out: &mut Vec<DrawCmd>) {
        if !self.dirty {
            return;
        }
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        out.push(DrawCmd::OutlineThicken {
            color: self.color,
            radius: radius(self.zoom),
            resolution: [self.width as f32, self.height as f32],
        });
        if self.behind_player {
            if let Some(p) = self.player.take() {
                let (color, behind) = (self.color, self.behind_player);
                self.render_character_outline(&p, color, behind, true, out);
                self.player = Some(p);
            }
        }
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        out.push(DrawCmd::OutlineBlit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(out: &[DrawCmd]) -> Vec<&'static str> {
        out.iter()
            .filter_map(|c| match c {
                DrawCmd::OutlineBegin { clear: true, .. } => Some("begin+clear"),
                DrawCmd::OutlineBegin { clear: false, .. } => Some("begin"),
                DrawCmd::OutlineEnd => Some("end"),
                DrawCmd::OutlineThicken { .. } => Some("thicken"),
                DrawCmd::OutlineBlit => Some("blit"),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn same_colour_shares_a_mask_and_a_change_flushes() {
        let mut o = ModelOutlines::new(640, 480, 1.0);
        o.start_frame();
        let mut out = Vec::new();
        let red = [1.0, 0.0, 0.0, 1.0];
        let cyan = [0.0, 1.0, 1.0, 0.65];
        o.render_character_outline(&[], red, false, false, &mut out);
        o.render_character_outline(&[], red, false, false, &mut out);
        o.render_character_outline(&[], cyan, false, false, &mut out);
        o.render_character_outline(&[], cyan, true, false, &mut out);
        o.end_frame(&mut out);
        assert_eq!(
            kinds(&out),
            [
                "begin+clear",
                "end",
                "begin",
                "end",
                "thicken",
                "blit",
                "begin+clear",
                "end",
                "thicken",
                "blit",
                "begin+clear",
                "end",
                "thicken",
                "blit"
            ]
        );
    }

    #[test]
    fn the_player_masks_outlines_behind_it() {
        let mut o = ModelOutlines::new(640, 480, 1.0);
        o.start_frame();
        o.set_player_render_data(Vec::new());
        let mut out = Vec::new();
        o.render_character_outline(&[], [1.0, 0.0, 0.0, 1.0], true, false, &mut out);
        o.end_frame(&mut out);
        assert_eq!(
            kinds(&out),
            ["begin+clear", "end", "thicken", "begin", "end", "blit"]
        );
        // In front of the player: no mask.
        let mut out = Vec::new();
        o.start_frame();
        o.set_player_render_data(Vec::new());
        o.render_character_outline(&[], [1.0, 0.0, 0.0, 1.0], false, false, &mut out);
        o.end_frame(&mut out);
        assert_eq!(kinds(&out), ["begin+clear", "end", "thicken", "blit"]);
    }

    #[test]
    fn colours_round_trip_through_abgr() {
        let mut h = OutlineHighlight::default();
        h.set_col_all(1.0, 0.0, 0.0, 0.65);
        h.set(1, true);
        assert!(h.is_on(1) && !h.is_on(0));
        let c = h.color(1);
        assert_eq!(c, [1.0, 0.0, 0.0, 0.647_058_84]);
        assert_eq!(radius(1.0), 0.38);
    }
}
