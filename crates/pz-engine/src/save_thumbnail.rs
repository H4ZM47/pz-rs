//! The renderer's side of `SavefileThumbnail.create(player)`, for
//! `SaveGameHooks::capture_thumbnail`.
//!
//! The engine's hook implementation, on the game thread after `OnSave`:
//! sets the player's zoom (offscreen buffer zoom and target zoom, camera
//! zoom and offscreen size) to 1 and centres the camera; renders the
//! player's world frame into the offscreen buffer as `renderWorld` does
//! (`IsoSprite.globalOffsetX = -1`, clear, the camera fixed at zoom 1,
//! `IsoWorld.render`, `legacyPostRender`, `EndFrame`; no screen pass, no
//! UI) and submits it; then calls [`capture_thumbnail`] (the
//! `TakeScreenShotDrawer`, which runs right after that render on the
//! render thread). It then restores the old zoom, centres the camera again
//! and lets the next frame redraw the screen and UI (the original copies
//! the offscreen buffer aside before the thumbnail render and back after
//! it, then draws the screen pass and the UI once more).

use pz_core::thumbnail::{self, THUMB_SIZE, Thumbnail};
use pz_gpu::Gpu;
use pz_render::ScreenParams;

/// A player's screen area: `IsoCamera.getScreenLeft/Top/Width/Height`
/// (pixels, origin top-left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerScreen {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

/// `TakeScreenShotDrawer.render` for player `player`'s area. With the
/// offscreen buffer: `createWithRenderShader`, the 256x256 square centred
/// on the area (`sx = left + width / 2 - 128`, likewise `sy`) through the
/// `screen` shader with the player's `params`. Without one:
/// `Core.TakeScreenshot(256, 256, GL_BACK)`, the middle of player 0's area
/// (`player0`), at most its size. The pixels are RGBA, rows bottom-up, as
/// [`thumbnail::save_thumbnail`] takes them. `None` (and an error logged)
/// when the read-back fails, or when there is neither an offscreen buffer
/// nor a readable back buffer (a window surface).
pub fn capture_thumbnail(
    gpu: &mut Gpu,
    player: PlayerScreen,
    player0: PlayerScreen,
    params: ScreenParams,
) -> Option<Thumbnail> {
    let half = THUMB_SIZE / 2;
    let sx = player.left + player.width / 2 - half;
    let sy = player.top + player.height / 2 - half;
    let read = match gpu.read_offscreen_thumbnail(sx, sy, params) {
        Ok(Some(rgba)) => Ok(Some(Thumbnail {
            width: THUMB_SIZE,
            height: THUMB_SIZE,
            rgba,
        })),
        Ok(None) => {
            let r = thumbnail::screen_read_rect(
                player0.left,
                player0.top,
                player0.width,
                player0.height,
            );
            if r.width <= 0 || r.height <= 0 {
                return None;
            }
            gpu.read_main_rect(r.x, r.y, r.width as u32, r.height as u32)
                .map(|p| {
                    p.map(|rgba| Thumbnail {
                        width: r.width,
                        height: r.height,
                        rgba,
                    })
                })
        }
        Err(e) => Err(e),
    };
    match read {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ERROR: SavefileThumbnail.create: {e}");
            None
        }
    }
}

#[cfg(test)]
#[path = "save_thumbnail_tests.rs"]
mod tests;
