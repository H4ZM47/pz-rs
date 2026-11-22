//! The static `zombie.iso.IsoUtils` conversions between screen pixels and
//! world tiles that Lua calls (`ISCoordConversion.ToWorld` / `ToScreen`).

use super::host::with_host;
use crate::expose::{expose_class, int_arg, num_arg};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};

fn tile_scale() -> f32 {
    crate::bind_core::shared_core().borrow().tile_scale as f32
}

/// `IsoUtils.XToIso/YToIso(index, x, y, z)`: the screen position plus
/// `IsoCamera.getOffX/Y(index)`, in tiles.
pub fn to_iso(index: usize, x: f32, y: f32, z: f32) -> (f32, f32) {
    let (ox, oy) = with_host(|h| h.camera.camera_offset(index));
    let (px, py) = (x + ox, y + oy);
    let ts = tile_scale();
    ((px + 2.0 * py) / (64.0 * ts) + 3.0 * z, (px - 2.0 * py) / (-64.0 * ts) + 3.0 * z)
}

/// `IsoUtils.XToScreen/YToScreen(x, y, z, screenZ)`.
pub fn to_screen(x: f32, y: f32, z: f32, screen_z: i32) -> (f32, f32) {
    let ts = tile_scale();
    let sx = x * (32.0 * ts) - y * (32.0 * ts);
    let sy = y * (16.0 * ts) + x * (16.0 * ts) + (screen_z as f32 - z) * (96.0 * ts);
    (sx, sy)
}

/// The arguments of `XToIso/YToIso`: `(index, x, y, z)`, or `(x, y, z)`
/// with `IsoPlayer.getPlayerIndex()`. `IsoCamera.cameras` has four slots.
fn to_iso_args(c: &NativeCall<'_>) -> LuaResult<(usize, f32, f32, f32)> {
    let (index, first) = if c.nargs() >= 4 {
        (int_arg(c, 0), 1usize)
    } else {
        (crate::bind_players::player_index(), 0)
    };
    let slot = usize::try_from(index).ok().filter(|&i| i < 4);
    let index = slot.ok_or_else(|| LuaError::Runtime {
        message: format!("Index {index} out of bounds for length 4"),
        class: "java.lang.ArrayIndexOutOfBoundsException",
    })?;
    let f = |i: usize| num_arg(c, first + i) as f32;
    Ok((index, f(0), f(1), f(2)))
}

fn screen_args(c: &NativeCall<'_>) -> (f32, f32) {
    let f = |i| num_arg(c, i) as f32;
    to_screen(f(0), f(1), f(2), int_arg(c, 3))
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoUtils", None);
    b.static_fn("XToIso", |c| {
        let (i, x, y, z) = to_iso_args(c)?;
        Ok(c.push(Value::Num(to_iso(i, x, y, z).0 as f64)))
    });
    b.static_fn("YToIso", |c| {
        let (i, x, y, z) = to_iso_args(c)?;
        Ok(c.push(Value::Num(to_iso(i, x, y, z).1 as f64)))
    });
    b.static_fn("XToScreen", |c| Ok(c.push(Value::Num(screen_args(c).0 as f64))));
    b.static_fn("YToScreen", |c| Ok(c.push(Value::Num(screen_args(c).1 as f64))));
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudCamera, HudHost};
    use super::super::test_util::*;

    struct Cam;
    impl HudCamera for Cam {
        fn camera_offset(&self, p: usize) -> (f32, f32) {
            if p == 1 {
                (1000.0, 500.0)
            } else {
                (0.0, 0.0)
            }
        }
    }

    #[test]
    fn iso_utils_converts_with_the_camera_offset() {
        install_textures(&[]);
        install(HudHost { camera: Box::new(Cam), ..HudHost::default() });
        let mut t = thread();
        // Tile scale 2: (200, 100) + offsets (1000, 500) = (1200, 600);
        // x = (1200 + 1200) / 128 = 18.75, y = (1200 - 1200) / -128 = 0,
        // plus 3 per floor.
        assert_eq!(eval(&mut t, "IsoUtils.XToIso(1, 200, 100, 0)").as_num(), Some(18.75));
        assert_eq!(eval(&mut t, "IsoUtils.YToIso(1, 200, 100, 1)").as_num(), Some(3.0));
        // Without an index: IsoPlayer.getPlayerIndex(), 0 here.
        assert_eq!(eval(&mut t, "IsoUtils.XToIso(256, 0, 0)").as_num(), Some(2.0));
        // XToScreen = (x - y) * 64, YToScreen = (x + y) * 32 + (cz - z) * 192.
        assert_eq!(eval(&mut t, "IsoUtils.XToScreen(3, 1, 0, 0)").as_num(), Some(128.0));
        assert_eq!(eval(&mut t, "IsoUtils.YToScreen(3, 1, 1, 0)").as_num(), Some(-64.0));
        // ISCoordConversion.ToWorld's playerIndex 0, and an index past the cameras.
        assert_eq!(eval(&mut t, "IsoUtils.YToIso(0, 128, 0, 0)").as_num(), Some(-1.0));
        run(&mut t, "ok = pcall(IsoUtils.XToIso, 4, 0, 0, 0)");
        assert!(matches!(t.globals().get_str("ok"), pz_kahlua::Value::Bool(false)));
    }
}
