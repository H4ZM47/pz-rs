//! The HUD's camera reads (`hud::host::HudCamera`) from the iso cameras:
//! `IsoCamera.getOffX/getOffY` (of `IsoPlayer.getPlayerIndex()`),
//! `IsoCamera.cameras[player].getOffX/getOffY`,
//! `IsoCamera.frameState.playerIndex` and `Core.getZoom(player)`.
//!
//! The engine owns a [`SharedCameraFrame`], sets each player's
//! [`Camera`] (`PlayerCamera`: its offsets already truncated to whole
//! pixels with the right-click offset added, as `getOffX` returns them)
//! and the player indices every frame, and installs it once with
//! [`install_hud_camera`].

use std::cell::RefCell;
use std::rc::Rc;

use pz_iso_render::render_info::MAX_PLAYERS;
use pz_iso_render::Camera;
use pz_lua_api::hud::host::{with_host, HudCamera};

/// `IsoCamera.cameras` and the player indices for this frame (owner:
/// world-render thread).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraFrame {
    /// Each player's camera; `None` for players without a view.
    pub cameras: [Option<Camera>; MAX_PLAYERS],
    /// `IsoPlayer.getPlayerIndex()`: the current player (`IsoPlayer.instance`).
    pub player_index: usize,
    /// `IsoCamera.frameState.playerIndex`: the player whose frame is being
    /// updated or rendered.
    pub frame_player_index: usize,
}

impl CameraFrame {
    fn camera(&self, player: usize) -> Option<&Camera> {
        self.cameras.get(player).and_then(Option::as_ref)
    }

    /// `IsoCamera.getOffX(player)`, `getOffY(player)`.
    pub fn offset(&self, player: usize) -> (f32, f32) {
        self.camera(player)
            .map_or((0.0, 0.0), |c| (c.off_x as f32, c.off_y as f32))
    }

    /// `Core.getZoom(player)`: 1 without an offscreen buffer (no camera).
    pub fn zoom(&self, player: usize) -> f32 {
        self.camera(player).map_or(1.0, |c| c.zoom)
    }
}

/// The frame's cameras, shared between the engine and the HUD host.
pub type SharedCameraFrame = Rc<RefCell<CameraFrame>>;

/// [`HudCamera`] over a [`SharedCameraFrame`].
pub struct WorldHudCamera(pub SharedCameraFrame);

impl HudCamera for WorldHudCamera {
    fn off_x(&self) -> f32 {
        let f = self.0.borrow();
        f.offset(f.player_index).0
    }

    fn off_y(&self) -> f32 {
        let f = self.0.borrow();
        f.offset(f.player_index).1
    }

    fn zoom(&self, player: usize) -> f32 {
        self.0.borrow().zoom(player)
    }

    fn camera_offset(&self, player: usize) -> (f32, f32) {
        self.0.borrow().offset(player)
    }

    fn frame_player_index(&self) -> usize {
        self.0.borrow().frame_player_index
    }
}

/// Installs `frame` as the HUD's camera (`HudHost::camera`), leaving the
/// other HUD inputs as they are.
pub fn install_hud_camera(frame: SharedCameraFrame) {
    with_host(|h| h.camera = Box::new(WorldHudCamera(frame)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_frame_cameras() {
        let frame = SharedCameraFrame::default();
        install_hud_camera(frame.clone());
        let read = |f: &dyn Fn(&dyn HudCamera) -> f32| with_host(|h| f(&*h.camera));
        // No cameras yet: Core.getZoom without a buffer, zero offsets.
        assert_eq!(read(&|c| c.zoom(0)), 1.0);
        assert_eq!(read(&|c| c.off_x()), 0.0);
        let p0 = Camera::centred(100.0, 200.0, 0.0, 1280, 720, 1.5);
        let p1 = Camera::centred(50.0, 60.0, 1.0, 640, 360, 0.75);
        {
            let mut f = frame.borrow_mut();
            f.cameras[0] = Some(p0);
            f.cameras[1] = Some(p1);
            f.player_index = 1;
            f.frame_player_index = 0;
        }
        assert_eq!(read(&|c| c.zoom(0)), 1.5);
        assert_eq!(read(&|c| c.zoom(1)), 0.75);
        assert_eq!(read(&|c| c.zoom(2)), 1.0);
        // getOffX()/getOffY() follow IsoPlayer.getPlayerIndex().
        assert_eq!(read(&|c| c.off_x()), p1.off_x as f32);
        assert_eq!(read(&|c| c.off_y()), p1.off_y as f32);
        let off0 = with_host(|h| h.camera.camera_offset(0));
        assert_eq!(off0, (p0.off_x as f32, p0.off_y as f32));
        assert_eq!(with_host(|h| h.camera.camera_offset(3)), (0.0, 0.0));
        assert_eq!(with_host(|h| h.camera.frame_player_index()), 0);
        frame.borrow_mut().frame_player_index = 1;
        assert_eq!(with_host(|h| h.camera.frame_player_index()), 1);
    }
}
