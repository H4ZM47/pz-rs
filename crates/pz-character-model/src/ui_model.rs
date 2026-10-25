//! `AnimatedModel` as `UI3DModel` uses it: a character drawn inside a UI
//! rectangle with its own orthographic camera (`UIModelCamera`), lit by the
//! two fill lights of an outdoor `AnimatedModel` with no square lights, and
//! animated by an anim set and action group of its own (`player-avatar`)
//! instead of a character's state machine.
//!
//! * [`UiModelCamera`]: `UIModelCamera.Begin` (projection, model view and
//!   the `glViewport` of `DoRender(x, y, w, h, sizeV, angle)`), with the
//!   viewport folded into the MVP for a full-target draw plus a scissor box
//!   ([`UiModelCamera::target_mvp`]);
//! * [`ui_model_lights`]: `DrawCharSingular`'s lights for that model;
//! * [`AvatarAnimation`]: `updateInternal`'s animation half
//!   (`ActionContext`, `AdvancedAnimator`, `AnimationPlayer`) and
//!   `setModelData`'s reset;
//! * [`ui_model_draws`]: `DoRender` + `Render` + `DrawChar` for a built
//!   [`ModelSlot`] as draw commands.

use std::rc::Rc;

use pz_anim::{AdvancedAnimator, AnimationPlayer, AnimationSet, MapVariableSource};
use pz_characters::character::state::action::{ActionContext, ActionGroup};
use pz_iso_render::joml::Matrix4f;
use pz_iso_render::models::encode_light;
use pz_model::SkinningData;
use pz_render::model::ModelLight;
use pz_render::{gl, DrawCmd};

use crate::model_tree::ModelSlot;
use crate::render::{render_slot, ModelResources, SlotUniforms};

/// `AnimatedModel.UIModelCamera` with what `DoRender(x, y, w, h, sizeV,
/// animPlayerAngle)` sets and what `Begin` reads from the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiModelCamera {
    /// `glViewport(x, y, w, h)`: GL window coordinates (y from the bottom).
    pub viewport: [i32; 4],
    pub size_v: f32,
    pub anim_player_angle: f32,
    /// `AnimatedModel.isometric`.
    pub isometric: bool,
    /// `AnimatedModel.flipY`.
    pub flip_y: bool,
    /// `stateInfoRender().offset` (`setOffset` at `renderMain`).
    pub offset: [f32; 3],
}

impl UiModelCamera {
    /// `Begin()`'s projection and model-view matrices.
    pub fn matrices(&self) -> (Matrix4f, Matrix4f) {
        let x_scale = self.viewport[2] as f32 / self.viewport[3] as f32;
        let s = self.size_v;
        let p = if self.flip_y {
            Matrix4f::set_ortho(-s * x_scale, s * x_scale, s, -s, -100.0, 100.0)
        } else {
            Matrix4f::set_ortho(-s * x_scale, s * x_scale, -s, s, -100.0, 100.0)
        };
        let f = 2048.0f32.sqrt();
        let p = p.scale(-f, f, f);
        let mv = if self.isometric {
            Matrix4f::identity()
                .rotate(0.523_598_8, 1.0, 0.0, 0.0)
                .rotate(self.anim_player_angle + 0.785_398_2, 0.0, 1.0, 0.0)
        } else {
            Matrix4f::identity().rotate(self.anim_player_angle, 0.0, 1.0, 0.0)
        };
        let o = self.offset;
        (p, mv.translate(o[0], o[1], o[2]))
    }

    /// `projection * modelView` drawn through the camera's viewport, as an
    /// MVP for a draw over the whole `target_w` x `target_h` target: the
    /// viewport transform composed into clip space (depth untouched).
    /// Together with a scissor box on [`Self::scissor`] this rasterises as
    /// `glViewport(x, y, w, h)` does.
    pub fn target_mvp(&self, target_w: i32, target_h: i32) -> Matrix4f {
        let (p, mv) = self.matrices();
        viewport_matrix(self.viewport, target_w, target_h).mul(&p.mul(&mv))
    }

    /// The viewport as a top-left-origin scissor box (`x, y, w, h`) on a
    /// target `target_h` pixels high.
    pub fn scissor(&self, target_h: i32) -> [i32; 4] {
        let [x, y, w, h] = self.viewport;
        [x, target_h - y - h, w, h]
    }
}

/// Clip space of a `glViewport(x, y, w, h)` mapped into the clip space of
/// the full `tw` x `th` viewport.
pub fn viewport_matrix(vp: [i32; 4], tw: i32, th: i32) -> Matrix4f {
    let [x, y, w, h] = vp.map(|v| v as f32);
    let (tw, th) = (tw as f32, th as f32);
    let sx = w / tw;
    let sy = h / th;
    let tx = (2.0 * x + w) / tw - 1.0;
    let ty = (2.0 * y + h) / th - 1.0;
    Matrix4f::from_cols([
        [sx, 0.0, 0.0, 0.0],
        [0.0, sy, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [tx, ty, 0.0, 1.0],
    ])
}

/// `DrawCharSingular`'s lights for an `AnimatedModel` that was never given
/// square lights (`setLights`) or an ambient (`setAmbient`): every result
/// light has radius 0, `outside` is true, the ambient is white and the
/// lights' origin is (0, 0, 0), leaving the two fill lights (3 and 4).
pub fn ui_model_lights(anim_player_angle: f32) -> [ModelLight; 5] {
    let none = ModelLight {
        direction: [0.0, 1.0, 0.0],
        colour: [0.0; 3],
    };
    let mut l = [none; 5];
    let fill = [1.7 / 4.0; 3];
    let o = [0.0; 3];
    let a = Some(anim_player_angle);
    l[3] = encode_light([-2.0, -2.0, 1.0], fill, 5000.0, o, a);
    l[4] = encode_light([2.0, 2.0, 1.0], fill, 5000.0, o, a);
    l
}

/// `AnimatedModel.DoRender(x, y, w, h, sizeV, angle)` for a ready model:
/// the depth clear, the viewport (as a scissor box) and each instance's
/// `DrawCharSingular` (`targetDepth` 0.5, white ambient times 0.45, alpha
/// and tint 1, `finalScale` 1), state saved and restored around the
/// draws. `target` is the window size (`Core.width`, `Core.height`).
pub fn ui_model_draws(
    slot: &ModelSlot,
    player: &AnimationPlayer,
    cam: &UiModelCamera,
    target: (i32, i32),
    res: &dyn ModelResources,
) -> Vec<DrawCmd> {
    let u = SlotUniforms {
        mvp: cam.target_mvp(target.0, target.1).m,
        target_depth: 0.5,
        lights: ui_model_lights(cam.anim_player_angle),
        ambient: [0.45; 3],
        tint: [1.0; 3],
        alpha: 1.0,
        final_scale: 1.0,
    };
    let mut out = vec![
        // glClear(GL_DEPTH_BUFFER_BIT) before the attributes are pushed.
        DrawCmd::Clear(gl::DEPTH_BUFFER_BIT),
        DrawCmd::PushAttrib,
        DrawCmd::Clip(Some(cam.scissor(target.1))),
        DrawCmd::DepthMask(true),
    ];
    out.extend(
        render_slot(slot, player, &u, res)
            .into_iter()
            .map(|d| DrawCmd::Model(Box::new(d))),
    );
    // glDepthMask(false), glDepthFunc(GL_ALWAYS), then glPopAttrib.
    out.push(DrawCmd::PopAttrib);
    out
}

/// The animation half of an `AnimatedModel`: the player, the animator over
/// the model's anim set, the action context over its action group, and
/// the model's own variables (`AnimatedModel` is an
/// `AnimationVariableSource`).
pub struct AvatarAnimation {
    pub player: AnimationPlayer,
    pub animator: AdvancedAnimator,
    pub actions: ActionContext,
    pub set: Rc<AnimationSet>,
}

impl AvatarAnimation {
    pub fn new(skinning: Rc<SkinningData>, set: Rc<AnimationSet>) -> Self {
        let mut animator = AdvancedAnimator::new();
        animator.set_anim_set(Some(set.clone()));
        AvatarAnimation {
            player: AnimationPlayer::new(skinning),
            animator,
            actions: ActionContext::new(),
            set,
        }
    }

    /// `setModelData`'s animation part after a reset
    /// (`OnAnimDataChanged`): with `animate` the action group is (re)set and
    /// the animator takes `state` or the action state; then one update with
    /// the fps multiplier at 100 (`dt100`) so the blends finish.
    pub fn reset(
        &mut self,
        group: &Rc<ActionGroup>,
        state: Option<&str>,
        animate: bool,
        vars: &mut MapVariableSource,
        dt100: f32,
    ) {
        self.animator.reset(&mut self.player, vars);
        let state = state.filter(|s| !s.trim().is_empty());
        if animate {
            if !self.actions.group().is_some_and(|g| Rc::ptr_eq(g, group)) {
                self.actions.set_group(group.clone());
            }
            let name = state
                .map(str::to_string)
                .unwrap_or_else(|| self.actions.current_state_name());
            let subs: Vec<String> = self
                .actions
                .child_states()
                .iter()
                .map(|s| s.name().to_string())
                .collect();
            let subs: Vec<&str> = subs.iter().map(String::as_str).collect();
            self.animator
                .set_state(&mut self.player, vars, &name, &subs);
        } else if let Some(s) = state {
            self.animator.set_state(&mut self.player, vars, s, &[]);
        }
        self.animator.update(&mut self.player, vars, dt100);
    }

    /// `ActionContext.reportEvent(event)` (no state).
    pub fn report_event(&mut self, event: &str) {
        self.actions.report_event(None, event);
    }

    /// `updateInternal` from `SetForceDir(angle)` on, `animate` branch:
    /// the action context (state changes go to the animator,
    /// `actionStateChanged`), the animator and the player, then the fired
    /// anim events back to the action context (`OnAnimEvent`).
    pub fn update(&mut self, angle: [f32; 2], vars: &mut MapVariableSource, dt: f32) {
        self.player
            .set_target_and_current_direction(angle[0], angle[1]);
        if self.actions.update(vars) {
            let name = self.actions.current_state_name();
            let subs: Vec<String> = self
                .actions
                .child_states()
                .iter()
                .map(|s| s.name().to_string())
                .collect();
            let subs: Vec<&str> = subs.iter().map(String::as_str).collect();
            self.animator
                .set_state(&mut self.player, vars, &name, &subs);
        }
        self.animator.animate(&mut self.player, vars, dt);
        for e in std::mem::take(&mut self.animator.events) {
            let name = &e.event.event_name;
            if !name.trim().is_empty() {
                self.actions.report_event(e.state_name.as_deref(), name);
            }
        }
    }

    /// The `!animate` branch: the animator at fps multiplier 100, the first
    /// track at `track_time` when set, and `Update(0)`.
    pub fn update_still(
        &mut self,
        angle: [f32; 2],
        vars: &mut MapVariableSource,
        dt100: f32,
        track_time: f32,
    ) {
        self.player
            .set_target_and_current_direction(angle[0], angle[1]);
        self.animator.update(&mut self.player, vars, dt100);
        if track_time > 0.0 {
            if let Some(id) = self.player.tracks().first().map(|t| t.id) {
                if let Some(t) = self.player.track_mut(id) {
                    t.set_current_time_value(track_time);
                }
            }
        }
        self.player.update(0.0, Some(&*vars));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-5 * x.abs().max(y.abs()).max(1.0))
    }

    /// A point projected through the folded MVP over the whole target
    /// lands where the camera's own viewport puts it.
    #[test]
    fn folded_viewport_matches_gl_viewport() {
        let cam = UiModelCamera {
            viewport: [100, 50, 200, 400],
            size_v: 25.0,
            anim_player_angle: 0.7,
            isometric: false,
            flip_y: false,
            offset: [0.0, -0.5, 0.0],
        };
        let (p, mv) = cam.matrices();
        let pm = p.mul(&mv);
        let full = cam.target_mvp(800, 600);
        for pt in [[0.0, 0.0, 0.0], [0.3, 1.2, -0.1], [-0.2, 0.8, 0.4]] {
            let a = pm.project(pt[0], pt[1], pt[2], cam.viewport);
            let c = full.transform([pt[0], pt[1], pt[2], 1.0]);
            let wx = (c[0] / c[3] + 1.0) * 0.5 * 800.0;
            let wy = (c[1] / c[3] + 1.0) * 0.5 * 600.0;
            assert!(close(&a[..2], &[wx, wy]), "{a:?} vs {wx} {wy}");
        }
        assert_eq!(cam.scissor(600), [100, 150, 200, 400]);
    }
}
