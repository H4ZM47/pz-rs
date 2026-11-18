//! `LuaManager.GlobalObject.screenToIsoX/Y` and `isoToScreenX/Y`: window
//! pixels to world tiles and back, through a player's camera.

use super::host::with_host;
use crate::expose::{global, int_arg, num_arg};
use pz_kahlua::{Thread, Value};

fn tile_scale() -> f32 {
    crate::bind_core::shared_core().borrow().tile_scale as f32
}

/// `IsoCamera.getScreenLeft(player)` / `getScreenTop(player)`.
fn screen_left_top(player: usize) -> (f32, f32) {
    let (l, t, _, _) = super::world_markers::screen_rect(player);
    (l as f32, t as f32)
}

fn player_arg(c: &pz_kahlua::NativeCall<'_>) -> usize {
    int_arg(c, 0).clamp(0, 3) as usize
}

/// `screenToIsoX/Y(player, x, y, z)`: `PlayerCamera.XToIso` / `YToIso` of
/// the position relative to the player's screen, times `Core.getZoom`.
/// `XToIso` truncates the pixel position before it adds `getOffX()`.
pub fn screen_to_iso(player: usize, x: f32, y: f32, z: f32) -> (f32, f32) {
    let (zoom, (ox, oy)) = with_host(|h| (h.camera.zoom(player), h.camera.camera_offset(player)));
    let (l, t) = screen_left_top(player);
    let px = ((x - l) * zoom) as i32 as f32 + ox;
    let py = ((y - t) * zoom) as i32 as f32 + oy;
    let ts = tile_scale();
    ((px + 2.0 * py) / (64.0 * ts) + 3.0 * z, (px - 2.0 * py) / (-64.0 * ts) + 3.0 * z)
}

/// `isoToScreenX/Y(player, x, y, z)`: `IsoUtils.XToScreen` / `YToScreen`
/// of the position plus the camera's `fixJigglyModelsSquareX/Y`, less
/// `getOffX/Y()`, divided by the camera's zoom, plus the screen's left/top.
pub fn iso_to_screen(player: usize, x: f32, y: f32, z: f32) -> (f32, f32) {
    let (zoom, (ox, oy), (jx, jy)) = with_host(|h| {
        (h.camera.zoom(player), h.camera.camera_offset(player), h.camera.fix_jiggly_square(player))
    });
    let ts = tile_scale();
    let (x, y) = (x + jx, y + jy);
    let sx = x * (32.0 * ts) - y * (32.0 * ts);
    let sy = y * (16.0 * ts) + x * (16.0 * ts) - z * (96.0 * ts);
    let (l, t) = screen_left_top(player);
    (l + (sx - ox) / zoom, t + (sy - oy) / zoom)
}

fn args(c: &pz_kahlua::NativeCall<'_>) -> (usize, f32, f32, f32) {
    (player_arg(c), num_arg(c, 1) as f32, num_arg(c, 2) as f32, num_arg(c, 3) as f32)
}

pub fn register(thread: &mut Thread) {
    global(thread, "screenToIsoX", |c| {
        let (p, x, y, z) = args(c);
        Ok(c.push(Value::Num(screen_to_iso(p, x, y, z).0 as f64)))
    });
    global(thread, "screenToIsoY", |c| {
        let (p, x, y, z) = args(c);
        Ok(c.push(Value::Num(screen_to_iso(p, x, y, z).1 as f64)))
    });
    global(thread, "isoToScreenX", |c| {
        let (p, x, y, z) = args(c);
        Ok(c.push(Value::Num(iso_to_screen(p, x, y, z).0 as f64)))
    });
    global(thread, "isoToScreenY", |c| {
        let (p, x, y, z) = args(c);
        Ok(c.push(Value::Num(iso_to_screen(p, x, y, z).1 as f64)))
    });
}
