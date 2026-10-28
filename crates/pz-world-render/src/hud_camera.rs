//! The iso cameras as the HUD reads them: `IsoCamera.getOffX/getOffY` (of
//! `IsoPlayer.getPlayerIndex()`), `IsoCamera.cameras[player].getOffX/getOffY`,
//! `IsoCamera.frameState.playerIndex` and `Core.getZoom(player)`.
//!
//! The engine owns a [`SharedCameraFrame`], sets each player's [`Camera`]
//! (`PlayerCamera`: its offsets already truncated to whole pixels with the
//! right-click offset added, as `getOffX` returns them) and the player
//! indices every frame; its `HudCamera` adapter reads them from here.
//! The gizmo scene also reads `PlayerCamera`'s right-click, target and
//! deferred offsets ([`PlayerCameraOffsets`]) and `frameState`'s camera
//! character and zoom ([`CameraFrame::frame_camera`]).

use std::cell::RefCell;
use std::rc::Rc;

use pz_iso_render::render_info::MAX_PLAYERS;
use pz_iso_render::Camera;

/// The `PlayerCamera` offsets beside the ones a [`Camera`] holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerCameraOffsets {
    /// `rightClickX/Y`.
    pub right_click: (f32, f32),
    /// `getTOffX/Y()`; `None` while the camera is settled (`tOffX ==
    /// offX`), where it equals `getOffX/Y()`.
    pub t_off: Option<(f32, f32)>,
    /// `deferedX/Y`.
    pub defered: (f32, f32),
}

/// `IsoCamera.cameras` and the player indices for this frame (owner:
/// world-render thread).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraFrame {
    /// Each player's camera; `None` for players without a view.
    pub cameras: [Option<Camera>; MAX_PLAYERS],
    /// Each player's other `PlayerCamera` offsets.
    pub offsets: [PlayerCameraOffsets; MAX_PLAYERS],
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

    /// `IsoCamera.getOffX()`, `getOffY()`: the current player's.
    pub fn current_offset(&self) -> (f32, f32) {
        self.offset(self.player_index)
    }

    /// `IsoCamera.getTOffX(player)`, `getTOffY(player)`.
    pub fn t_offset(&self, player: usize) -> (f32, f32) {
        self.offsets
            .get(player)
            .and_then(|o| o.t_off)
            .unwrap_or_else(|| self.offset(player))
    }

    /// `IsoCamera.frameState`'s `camCharacterX/Y/Z` and `zoom`: the frame
    /// player's camera position and zoom (zero and 1 without one).
    pub fn frame_camera(&self) -> ([f32; 3], f32) {
        self.camera(self.frame_player_index)
            .map_or(([0.0; 3], 1.0), |c| ([c.x, c.y, c.z], c.zoom))
    }

    /// `Core.getZoom(player)`: 1 without an offscreen buffer (no camera).
    pub fn zoom(&self, player: usize) -> f32 {
        self.camera(player).map_or(1.0, |c| c.zoom)
    }
}

/// The frame's cameras, shared between the engine and the HUD host.
pub type SharedCameraFrame = Rc<RefCell<CameraFrame>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_frame_cameras() {
        let mut f = CameraFrame::default();
        // No cameras yet: Core.getZoom without a buffer, zero offsets.
        assert_eq!(f.zoom(0), 1.0);
        assert_eq!(f.current_offset(), (0.0, 0.0));
        let p0 = Camera::centred(100.0, 200.0, 0.0, 1280, 720, 1.5);
        let p1 = Camera::centred(50.0, 60.0, 1.0, 640, 360, 0.75);
        f.cameras[0] = Some(p0);
        f.cameras[1] = Some(p1);
        f.player_index = 1;
        assert_eq!(
            (f.zoom(0), f.zoom(1), f.zoom(2), f.zoom(9)),
            (1.5, 0.75, 1.0, 1.0)
        );
        assert_eq!(f.current_offset(), (p1.off_x as f32, p1.off_y as f32));
        assert_eq!(f.offset(0), (p0.off_x as f32, p0.off_y as f32));
        assert_eq!(f.offset(3), (0.0, 0.0));
        assert_eq!(f.t_offset(0), f.offset(0));
        f.offsets[0].t_off = Some((5.0, 6.0));
        assert_eq!(f.t_offset(0), (5.0, 6.0));
        f.frame_player_index = 1;
        assert_eq!(f.frame_camera(), ([50.0, 60.0, 1.0], 0.75));
    }
}
