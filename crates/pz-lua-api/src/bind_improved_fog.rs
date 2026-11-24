//! `zombie.iso.weather.fog.ImprovedFog` static Lua bindings (used by
//! `NewFogDebug.lua` and climate tools), backed by `pz_iso_render::fog::ImprovedFog`.

use std::cell::RefCell;

use pz_iso_render::fog::ImprovedFog;
use pz_kahlua::Thread;

use crate::expose::{bool_arg, expose_class, int_arg, num_arg};

pub const IMPROVED_FOG_CLASS: &str = "zombie.iso.weather.fog.ImprovedFog";

/// Full static state of `zombie.iso.weather.fog.ImprovedFog` including the
/// renderer state and debug-editor overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct ImprovedFogState {
    pub fog: ImprovedFog,
    pub min_x_offset: i32,
    pub max_x_offset: i32,
    pub max_y_offset: i32,
    pub render_x_rows_from_center: i32,
    pub alpha_circle_alpha: f32,
    pub alpha_circle_rad: f32,
    pub draw_debug_colors: bool,
    pub enable_editing: bool,
    pub render_current_layer_only: bool,
    pub render_end_only: bool,
    pub render_only_one_row: bool,
}

impl Default for ImprovedFogState {
    fn default() -> Self {
        Self {
            fog: ImprovedFog::default(),
            min_x_offset: -2,
            max_x_offset: 12,
            max_y_offset: -5,
            render_x_rows_from_center: 1,
            alpha_circle_alpha: 0.225,
            alpha_circle_rad: 2.25,
            draw_debug_colors: false,
            enable_editing: false,
            render_current_layer_only: false,
            render_end_only: false,
            render_only_one_row: false,
        }
    }
}

thread_local! {
    static STATE: RefCell<ImprovedFogState> = RefCell::new(ImprovedFogState::default());
}

/// Snapshot current `ImprovedFogState` for rendering or tests.
pub fn snapshot() -> ImprovedFogState {
    STATE.with(|s| s.borrow().clone())
}

/// Reset `ImprovedFog` statics to defaults.
pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = ImprovedFogState::default());
}

/// Expose `zombie.iso.weather.fog.ImprovedFog`.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, IMPROVED_FOG_CLASS, Some("java.lang.Object"));

    // Float getters / setters
    for (get_name, set_name, slot) in [
        ("getBaseAlpha", "setBaseAlpha", 0),
        ("getSecondLayerAlpha", "setSecondLayerAlpha", 1),
        ("getTopAlphaHeight", "setTopAlphaHeight", 2),
        ("getBottomAlphaHeight", "setBottomAlphaHeight", 3),
        ("getOctaves", "setOctaves", 4),
        ("getAlphaCircleAlpha", "setAlphaCircleAlpha", 5),
        ("getAlphaCircleRad", "setAlphaCircleRad", 6),
        ("getColorR", "setColorR", 7),
        ("getColorG", "setColorG", 8),
        ("getColorB", "setColorB", 9),
    ] {
        b.static_fn(get_name, move |c| {
            let v = STATE.with(|s| {
                let s = s.borrow();
                match slot {
                    0 => s.fog.base_alpha,
                    1 => s.fog.second_layer_alpha,
                    2 => s.fog.top_alpha_height,
                    3 => s.fog.bottom_alpha_height,
                    4 => s.fog.octaves,
                    5 => s.alpha_circle_alpha,
                    6 => s.alpha_circle_rad,
                    7 => s.fog.color[0],
                    8 => s.fog.color[1],
                    _ => s.fog.color[2],
                }
            });
            Ok(c.push(v as f64))
        });
        b.static_fn(set_name, move |c| {
            let v = num_arg(c, 0) as f32;
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                match slot {
                    0 => s.fog.base_alpha = v,
                    1 => s.fog.second_layer_alpha = v,
                    2 => s.fog.top_alpha_height = v,
                    3 => s.fog.bottom_alpha_height = v,
                    4 => s.fog.octaves = v,
                    5 => s.alpha_circle_alpha = v,
                    6 => s.alpha_circle_rad = v,
                    7 => s.fog.color[0] = v,
                    8 => s.fog.color[1] = v,
                    _ => s.fog.color[2] = v,
                }
            });
            Ok(0)
        });
    }

    // Integer getters / setters
    for (get_name, set_name, slot) in [
        ("getMinXOffset", "setMinXOffset", 0),
        ("getMaxXOffset", "setMaxXOffset", 1),
        ("getMaxYOffset", "setMaxYOffset", 2),
        ("getRenderEveryXRow", "setRenderEveryXRow", 3),
        ("getRenderXRowsFromCenter", "setRenderXRowsFromCenter", 4),
    ] {
        b.static_fn(get_name, move |c| {
            let v = STATE.with(|s| {
                let s = s.borrow();
                match slot {
                    0 => s.min_x_offset,
                    1 => s.max_x_offset,
                    2 => s.max_y_offset,
                    3 => s.fog.render_every_x_row,
                    _ => s.render_x_rows_from_center,
                }
            });
            Ok(c.push(v))
        });
        b.static_fn(set_name, move |c| {
            let v = int_arg(c, 0);
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                match slot {
                    0 => s.min_x_offset = v,
                    1 => s.max_x_offset = v,
                    2 => s.max_y_offset = v,
                    3 => s.fog.render_every_x_row = v.max(1),
                    _ => s.render_x_rows_from_center = v,
                }
            });
            Ok(0)
        });
    }

    // Boolean getters / setters
    for (get_name, set_name, slot) in [
        ("isHighQuality", "setHighQuality", 0),
        ("isDrawDebugColors", "setDrawDebugColors", 1),
        ("isEnableEditing", "setEnableEditing", 2),
        ("isRenderCurrentLayerOnly", "setRenderCurrentLayerOnly", 3),
        ("isRenderEndOnly", "setRenderEndOnly", 4),
        ("isRenderOnlyOneRow", "setRenderOnlyOneRow", 5),
    ] {
        b.static_fn(get_name, move |c| {
            let v = STATE.with(|s| {
                let s = s.borrow();
                match slot {
                    0 => s.fog.high_quality,
                    1 => s.draw_debug_colors,
                    2 => s.enable_editing,
                    3 => s.render_current_layer_only,
                    4 => s.render_end_only,
                    _ => s.render_only_one_row,
                }
            });
            Ok(c.push(v))
        });
        b.static_fn(set_name, move |c| {
            let v = bool_arg(c, 0);
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                match slot {
                    0 => s.fog.high_quality = v,
                    1 => s.draw_debug_colors = v,
                    2 => s.enable_editing = v,
                    3 => s.render_current_layer_only = v,
                    4 => s.render_end_only = v,
                    _ => s.render_only_one_row = v,
                }
            });
            Ok(0)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    #[test]
    fn improved_fog_getters_and_setters_from_lua() {
        reset();
        let mut t = Thread::new(TableRef::new());
        register(&mut t);
        let f = load_string(
            r#"
            ImprovedFog.setEnableEditing(true)
            ImprovedFog.setBaseAlpha(0.75)
            ImprovedFog.setColorR(0.4)
            ImprovedFog.setColorG(0.5)
            ImprovedFog.setColorB(0.6)
            ImprovedFog.setOctaves(4)
            ImprovedFog.setRenderEveryXRow(2)
            ImprovedFog.setMinXOffset(-4)
            ImprovedFog.setHighQuality(false)
            return ImprovedFog.isEnableEditing(),
                   ImprovedFog.getBaseAlpha(),
                   ImprovedFog.getColorR(),
                   ImprovedFog.getColorG(),
                   ImprovedFog.getColorB(),
                   ImprovedFog.getOctaves(),
                   ImprovedFog.getRenderEveryXRow(),
                   ImprovedFog.getMinXOffset(),
                   ImprovedFog.isHighQuality()
            "#,
            Some("improved_fog_test.lua"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.pcall(&f, &[]).unwrap();
        assert!(res[0].truthy());
        assert!((res[1].as_num().unwrap() - 0.75).abs() < 1e-4);
        assert!((res[2].as_num().unwrap() - 0.4).abs() < 1e-4);
        assert!((res[3].as_num().unwrap() - 0.5).abs() < 1e-4);
        assert!((res[4].as_num().unwrap() - 0.6).abs() < 1e-4);
        assert_eq!(res[5].as_num(), Some(4.0));
        assert_eq!(res[6].as_num(), Some(2.0));
        assert_eq!(res[7].as_num(), Some(-4.0));
        assert!(!res[8].truthy());
        reset();
    }
}
