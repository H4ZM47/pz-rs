//! The per-frame part of `FBORenderCell.performRenderTiles` that follows
//! the chunk composite: the moving objects (players, corpses, items,
//! zombies, animals, vehicles) drawn straight into the offscreen buffer
//! every frame, and their drop shadows, which are collected while the
//! objects draw and drawn per level inside the translucent loop.
//!
//! The objects come in as what each one's `render` queues: a character's
//! draws are its `ModelSlotRenderData` (a [`pz_render::model::ModelDraw`]
//! per mesh, as `DrawCmd::Model`) and its shadow request
//! (`renderShadow` → `FBORenderShadows.addShadow`); corpses and placed
//! items are whatever their atlases draw.

use pz_render::DrawCmd;

use crate::models::ModelCamera;
use crate::outlines::{ModelOutlines, OutlineRequest};
use crate::shadows::{ShadowParams, Shadows, SlopedSurface};

/// A shadow an object adds while it renders.
#[derive(Clone, Debug, PartialEq)]
pub enum ShadowRequest {
    /// `addShadow(x, y, z, forward, w, fm, bm, r, g, b, a, bAnimal)`.
    Character {
        pos: [f32; 3],
        forward: [f32; 2],
        params: ShadowParams,
        /// The square's light colour (`lighting[playerIndex].lightInfo()`).
        rgb: [f32; 3],
        alpha: f32,
        animal: bool,
        slope: Option<SlopedSurface>,
    },
    /// The explicit-quad form (vehicles; corners front-left, front-right,
    /// back-right, back-left) with its texture (`None`: the drop shadow).
    Quad {
        origin: [f32; 3],
        corners: [[f32; 2]; 4],
        rgba: [f32; 4],
        texture: Option<pz_render::Texture>,
        vehicle: bool,
        slope: Option<SlopedSurface>,
    },
}

impl ShadowRequest {
    fn add(&self, shadows: &mut Shadows, cam: &ModelCamera) {
        match self {
            ShadowRequest::Character {
                pos,
                forward,
                params,
                rgb,
                alpha,
                animal,
                slope,
            } => {
                shadows.add_character_shadow(
                    cam,
                    *pos,
                    *forward,
                    *params,
                    *rgb,
                    *alpha,
                    *animal,
                    slope.as_ref(),
                );
            }
            ShadowRequest::Quad {
                origin,
                corners,
                rgba,
                texture,
                vehicle,
                slope,
            } => {
                if let Some(t) = texture.clone().or_else(|| shadows.drop_shadow.clone()) {
                    shadows.add_shadow(cam, *origin, *corners, *rgba, t, *vehicle, slope.as_ref());
                }
            }
        }
    }
}

/// One object's `renderShadow` + `render`.
#[derive(Clone, Debug, Default)]
pub struct ObjectRender {
    pub shadow: Option<ShadowRequest>,
    pub draws: Vec<DrawCmd>,
    /// `ModelSlotRenderData.characterOutline`: the character's meshes (its
    /// [`DrawCmd::Model`] draws) go into the outline mask first.
    pub outline: Option<OutlineRequest>,
    /// The player whose view this is (`IsoPlayer.players[playerIndex]`):
    /// drawn without an outline, it masks outlines behind it
    /// (`ModelOutlines.setPlayerRenderData`).
    pub local_player: bool,
}

impl ObjectRender {
    /// The character's meshes (`modelData`). A vehicle's meshes count too,
    /// untextured (solid): build 42 never outlines vehicles (their
    /// `ModelSlotRenderData` ignores the highlight), but a caller may ask
    /// for it.
    fn meshes(&self) -> Vec<pz_render::model::ModelDraw> {
        self.draws
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Model(d) => Some((**d).clone()),
                DrawCmd::VehicleModel(v) => Some(pz_render::model::ModelDraw { texture: None, ..v.base.clone() }),
                _ => None,
            })
            .collect()
    }

    fn render(&self, cam: &ModelCamera, shadows: &mut Shadows, outlines: &mut ModelOutlines, out: &mut Vec<DrawCmd>) {
        if let Some(s) = &self.shadow {
            s.add(shadows, cam);
        }
        // renderCharacter: the outline, the model, then the player's mask
        // data.
        if let Some(o) = &self.outline {
            outlines.render_character_outline(&self.meshes(), o.color, o.behind_player, false, out);
        }
        out.extend(self.draws.iter().cloned());
        if self.local_player && self.outline.is_none() {
            outlines.set_player_render_data(self.meshes());
        }
    }
}

/// Everything drawn after the chunk composite, in `performRenderTiles`'
/// groups.
#[derive(Clone, Debug, Default)]
pub struct FrameObjects {
    /// `renderPlayers`: each on-screen player's shadow, then its model.
    pub players: Vec<ObjectRender>,
    /// `renderCorpseShadows`: corpses on squares without stairs.
    pub corpse_shadows: Vec<ShadowRequest>,
    /// `renderMannequinShadows` (and mannequins drawn every frame).
    pub mannequins: Vec<ObjectRender>,
    /// `renderCorpsesInWorld`: dead bodies and prone zombies.
    pub corpses: Vec<DrawCmd>,
    /// `renderItemsInWorld`: placed items.
    pub items: Vec<DrawCmd>,
    /// `renderMovingObjects`: every other moving object (zombies, animals,
    /// vehicles, ...), in the cell's object-list order.
    pub moving: Vec<ObjectRender>,
}

impl FrameObjects {
    /// The draws from `renderPlayers` to `renderMovingObjects`, adding the
    /// shadows to `shadows` as they go (and dropping outlines).
    pub fn render(&self, cam: &ModelCamera, shadows: &mut Shadows, out: &mut Vec<DrawCmd>) {
        let mut outlines = ModelOutlines::new(1, 1, 1.0);
        self.render_outlined(cam, shadows, &mut outlines, out);
    }

    /// [`render`](Self::render) with the frame's `ModelOutlines`: outlined
    /// characters draw into its mask as they render.
    pub fn render_outlined(&self, cam: &ModelCamera, shadows: &mut Shadows, outlines: &mut ModelOutlines, out: &mut Vec<DrawCmd>) {
        for p in &self.players {
            p.render(cam, shadows, outlines, out);
        }
        for s in &self.corpse_shadows {
            s.add(shadows, cam);
        }
        for m in &self.mannequins {
            m.render(cam, shadows, outlines, out);
        }
        out.extend(self.corpses.iter().cloned());
        out.extend(self.items.iter().cloned());
        for m in &self.moving {
            m.render(cam, shadows, outlines, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;
    use pz_render::Texture;

    fn shadow(z: f32) -> ShadowRequest {
        ShadowRequest::Character {
            pos: [10.5, 10.5, z],
            forward: [0.0, 1.0],
            params: ShadowParams::default(),
            rgb: [1.0; 3],
            alpha: 1.0,
            animal: false,
            slope: None,
        }
    }

    #[test]
    fn objects_draw_in_group_order_and_collect_shadows() {
        let cam = Camera::player(10.5, 10.5, 0.0, 640, 480, 1.0);
        let mc = ModelCamera::new(&cam, 640.0, 480.0);
        let mark = |n: u32| DrawCmd::StencilMask(n);
        let objects = FrameObjects {
            players: vec![ObjectRender {
                shadow: Some(shadow(0.0)),
                draws: vec![mark(1)],
                ..Default::default()
            }],
            corpse_shadows: vec![shadow(1.0)],
            mannequins: vec![],
            corpses: vec![mark(2)],
            items: vec![mark(3)],
            moving: vec![ObjectRender {
                shadow: Some(shadow(0.0)),
                draws: vec![mark(4)],
                ..Default::default()
            }],
        };
        let mut shadows = Shadows::default();
        let mut tex = Texture::empty("NewShadow");
        tex.id = Some(1);
        shadows.clear(Some(tex));
        let mut out = Vec::new();
        objects.render(&mc, &mut shadows, &mut out);
        let marks: Vec<u32> = out
            .iter()
            .map(|c| {
                if let DrawCmd::StencilMask(n) = c {
                    *n
                } else {
                    0
                }
            })
            .collect();
        assert_eq!(marks, [1, 2, 3, 4]);
        assert_eq!(shadows.shadows.len(), 3);
        assert_eq!(shadows.shadows[1].oz, 1.0);
    }
}
