//! The Java HUD widgets `UIManager.init()` builds in game
//! (`SpeedControls`, ...) and the `UIManager` statics that hold them, on
//! the pz-ui extension slot.
//!
//! Inputs from other workstreams come through the traits in [`host`].
//! [`init_ui`] is `UIManager.init()` (call it instead of
//! `pz_ui::UIManager::init` once Lua is up).

pub mod host;
pub mod progress_bar;
pub mod speed_controls;

use crate::bind_ui::{element_of, UIELEMENT, UIMANAGER};
use crate::expose::{expose_class, int_arg, num_arg};
use host::with_host;
use progress_bar::{ActionProgressBar, ACTION_PROGRESS_BAR};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_ui::{with_ui, UIElement, UIManager};
use speed_controls::{SpeedControls, SPEED_CONTROLS};
use std::rc::Rc;

fn el_value(e: Option<Rc<UIElement>>) -> Value {
    e.map(|e| Value::Object(e as Rc<dyn LuaObject>))
        .unwrap_or_default()
}

/// `UIManager.init()`.
pub fn init_ui(t: &mut Thread) {
    UIManager::init();
    let (debug, game_mode) = {
        let core = crate::bind_core::shared_core();
        let c = core.borrow();
        (c.debug, c.game_mode.clone())
    };
    let sc = SpeedControls::create(debug);
    with_ui(|m| m.speed_controls = Some(sc.clone()));
    // setPicked(null), setLastPicked(null), rightDownObject = null:
    // TODO(world): picking is not ported.
    if !with_host(|h| h.players.has_player_instance()) {
        return;
    }
    if game_mode != "LastStand" && !with_host(|h| h.network.is_client()) {
        with_ui(|m| m.ui.push(sc.clone()));
    }
    if with_host(|h| h.network.is_server()) {
        return;
    }
    // TODO(ui): ObjectTooltip, Clock, UIDebugConsole and MoodlesUI
    // (their own PRs).
    UIManager::resize();
    for i in 0..4 {
        let bar = ActionProgressBar::create(0, 0);
        bar.set_render_this_player_only(i as i32);
        with_ui(|m| {
            m.progress_bar[i] = Some(bar.clone());
            m.ui.push(bar.clone());
        });
        bar.ext::<ActionProgressBar>().unwrap().set_value(1.0);
        bar.set_visible(false);
    }
    with_ui(|m| {
        m.player_inventory_ui = None;
        m.player_loot_ui = None;
    });
    UIManager::trigger(t, "OnCreateUI", &[]);
}

/// `UIManager.getProgressBar(n)`.
pub fn progress_bar(n: usize) -> Option<Rc<UIElement>> {
    with_ui(|m| m.progress_bar.get(n).cloned().flatten())
}

/// `UIManager.trySetProgressBarValue(n, value)`.
pub fn try_set_progress_bar_value(n: usize, v: f32) -> bool {
    match progress_bar(n) {
        Some(b) => {
            b.ext::<ActionProgressBar>().unwrap().set_value(v);
            true
        }
        None => false,
    }
}

/// `IngameState`'s per-player `UIManager.getProgressBar(n).update(n)`.
pub fn update_progress_bar(n: usize) {
    if let Some(b) = progress_bar(n) {
        b.ext::<ActionProgressBar>().unwrap().update_bar(&b, n);
    }
}

/// `IngameState`'s per-player progress bar render (the UI pass skips
/// game-world elements).
pub fn render_progress_bar(t: &mut Thread, n: usize) {
    if let Some(b) = progress_bar(n) {
        if b.is_visible() {
            b.render(t);
        }
    }
}

fn speed_controls(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
    let e = element_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("Expected SpeedControls"))?;
    if e.ext::<SpeedControls>().is_none() {
        return Err(LuaError::runtime(format!(
            "Expected SpeedControls but got {}",
            e.class_name()
        )));
    }
    Ok(e)
}

/// Register the HUD classes and the `UIManager` getters (after
/// `bind_ui::register`).
pub fn register(thread: &mut Thread) {
    let m = expose_class(thread, UIMANAGER, None);
    m.static_fn("init", |c| {
        init_ui(c.thread);
        Ok(0)
    });
    m.static_fn("getSpeedControls", |c| {
        Ok(c.push(el_value(with_ui(|m| m.speed_controls.clone()))))
    });
    m.static_fn("setSpeedControls", |c| {
        let e = element_of(&c.arg(0));
        with_ui(|m| m.speed_controls = e);
        Ok(0)
    });
    m.static_fn("getClock", |c| {
        Ok(c.push(el_value(with_ui(|m| m.clock.clone()))))
    });
    m.static_fn("getToolTip", |c| {
        Ok(c.push(el_value(with_ui(|m| m.tool_tip.clone()))))
    });
    m.static_fn("getDebugConsole", |c| {
        Ok(c.push(el_value(with_ui(|m| m.debug_console.clone()))))
    });
    m.static_fn("getMoodleUI", |c| {
        let i = num_arg(c, 0) as i32;
        let v = with_ui(|m| {
            usize::try_from(i)
                .ok()
                .and_then(|i| m.moodle_ui.get(i).cloned())
        });
        match v {
            Some(e) => Ok(c.push(el_value(e))),
            None => Err(index_error(i)),
        }
    });
    m.static_fn("getProgressBar", |c| {
        let i = int_arg(c, 0);
        let v = with_ui(|m| {
            usize::try_from(i)
                .ok()
                .and_then(|i| m.progress_bar.get(i).cloned())
        });
        match v {
            Some(e) => Ok(c.push(el_value(e))),
            None => Err(index_error(i)),
        }
    });

    m.static_fn("trySetProgressBarValue", |c| {
        let i = int_arg(c, 0);
        let v = num_arg(c, 1) as f32;
        let r = usize::try_from(i)
            .ok()
            .filter(|&i| i < 4)
            .map(|i| try_set_progress_bar_value(i, v));
        match r {
            Some(r) => Ok(c.push(r)),
            None => Err(index_error(i)),
        }
    });

    let p = expose_class(thread, ACTION_PROGRESS_BAR, Some(UIELEMENT));
    fn bar(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
        let e =
            element_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("Expected ActionProgressBar"))?;
        if e.ext::<ActionProgressBar>().is_none() {
            return Err(LuaError::runtime(format!(
                "Expected ActionProgressBar but got {}",
                e.class_name()
            )));
        }
        Ok(e)
    }
    p.method("setValue", |c| {
        let e = bar(c)?;
        e.ext::<ActionProgressBar>()
            .unwrap()
            .set_value(num_arg(c, 1) as f32);
        Ok(0)
    });
    p.method("getValue", |c| {
        let e = bar(c)?;
        let v = e.ext::<ActionProgressBar>().unwrap().get_value();
        Ok(c.push(v as f64))
    });
    // `update()` (UIElement) and `update(int nPlayer)`.
    p.method("update", |c| {
        let e = bar(c)?;
        if c.nargs() >= 2 {
            let n = int_arg(c, 1);
            let n = usize::try_from(n).map_err(|_| index_error(n))?;
            e.ext::<ActionProgressBar>().unwrap().update_bar(&e, n);
        } else {
            e.update(c.thread);
        }
        Ok(0)
    });

    let b = expose_class(thread, SPEED_CONTROLS, Some(UIELEMENT));
    b.field("PauseSpeed", 0.0)
        .field("PlaySpeed", 1.0)
        .field("FastForwardSpeed", 2.0)
        .field("FasterForwardSpeed", 3.0)
        .field("WaitSpeed", 4.0);
    b.method("getCurrentGameSpeed", |c| {
        let e = speed_controls(c)?;
        let v = e.ext::<SpeedControls>().unwrap().get_current_game_speed();
        Ok(c.push(v as f64))
    });
    b.method("SetCurrentGameSpeed", |c| {
        let e = speed_controls(c)?;
        let v = int_arg(c, 1);
        e.ext::<SpeedControls>()
            .unwrap()
            .set_current_game_speed(&e, v);
        Ok(0)
    });
    b.method("SetCorrectIconStates", |c| {
        let e = speed_controls(c)?;
        e.ext::<SpeedControls>()
            .unwrap()
            .set_correct_icon_states(&e);
        Ok(0)
    });
    b.method("Pause", |c| {
        let e = speed_controls(c)?;
        e.ext::<SpeedControls>().unwrap().pause(&e);
        Ok(0)
    });
    b.method("stepForward", |c| {
        let e = speed_controls(c)?;
        e.ext::<SpeedControls>().unwrap().step_forward(&e);
        Ok(0)
    });
    b.method("isPaused", |c| {
        let e = speed_controls(c)?;
        let v = e.ext::<SpeedControls>().unwrap().is_paused();
        Ok(c.push(v))
    });
}

fn index_error(i: i32) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length 4"),
        class: "java.lang.ArrayIndexOutOfBoundsException",
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use pz_kahlua::{TableRef, Thread, Value};
    use pz_render::{Texture, TextureManager};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    /// Textures by name with fixed sizes (`name -> (w, h)`), others 24x24.
    pub fn install_textures(sizes: &[(&str, i32, i32)]) {
        let v: Vec<(String, i32, i32, f32, f32)> = sizes
            .iter()
            .map(|(n, w, h)| (n.to_string(), *w, *h, 0.0, 0.0))
            .collect();
        install_textures_ex(&v);
    }

    /// As [`install_textures`] with draw offsets (`offsetX`, `offsetY`).
    pub fn install_textures_ex(sizes: &[(String, i32, i32, f32, f32)]) {
        let sizes: HashMap<String, (i32, i32, f32, f32)> = sizes
            .iter()
            .map(|(n, w, h, x, y)| (n.clone(), (*w, *h, *x, *y)))
            .collect();
        let tm = Rc::new(RefCell::new(TextureManager::new()));
        let cache: RefCell<HashMap<String, Texture>> = RefCell::default();
        pz_render::install_shared_texture_loader(move |name| {
            if let Some(t) = cache.borrow().get(name) {
                return Some(t.clone());
            }
            let (w, h, ox, oy) = sizes.get(name).copied().unwrap_or((24, 24, 0.0, 0.0));
            let px = vec![
                255;
                ((w.max(1) as u32).next_power_of_two() * (h.max(1) as u32).next_power_of_two() * 4)
                    as usize
            ];
            let mut t = tm
                .borrow_mut()
                .texture_from_pixels(name, w.max(1), h.max(1), px, 0);
            t.width = w as u32;
            t.height = h as u32;
            t.offset_x = ox;
            t.offset_y = oy;
            // The oracles' textures cover their whole image.
            (t.x_start, t.y_start, t.x_end, t.y_end) = (0.0, 0.0, 1.0, 1.0);
            cache.borrow_mut().insert(name.to_string(), t.clone());
            Some(t)
        });
    }

    /// `tex`/`fake` lines of an oracle file.
    pub fn oracle_textures_ex(text: &str) -> Vec<(String, i32, i32, f32, f32)> {
        let mut v: Vec<(String, i32, i32, f32, f32)> = oracle_textures(text)
            .into_iter()
            .map(|(n, w, h)| (n, w, h, 0.0, 0.0))
            .collect();
        for l in text.lines().filter_map(|l| l.strip_prefix("fake ")) {
            let p: Vec<&str> = l.split(' ').collect();
            v.push((
                p[0].to_string(),
                p[1].parse().unwrap(),
                p[2].parse().unwrap(),
                p[3].parse().unwrap(),
                p[4].parse().unwrap(),
            ));
        }
        v
    }

    /// The draw list with UVs as the oracle's `drawuv` lines.
    pub fn drawuv_lines() -> Vec<String> {
        pz_ui::with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    pz_ui::UiDrawCmd::Quad {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        color,
                        uv: Some(uv),
                    } => Some(format!(
                        "drawuv {} {} {} {} {} {} {} {} {} {} {} {} {}",
                        texture
                            .as_ref()
                            .map_or("-".to_string(), |t| t.name.to_string()),
                        jf(*x),
                        jf(*y),
                        jf(*w),
                        jf(*h),
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3),
                        jf(uv[0]),
                        jf(uv[1]),
                        jf(uv[2]),
                        jf(uv[3])
                    )),
                    _ => None,
                })
                .collect()
        })
    }

    /// Line-by-line comparison where numeric tokens may differ by `tol`
    /// (the original truncates texture coordinates to 1/1000).
    #[allow(dead_code)]
    pub fn assert_lines_close(got: &[String], want: &[String], tol: f32) {
        assert_eq!(
            got.len(),
            want.len(),
            "line count\n got {got:#?}\nwant {want:#?}"
        );
        for (g, w) in got.iter().zip(want) {
            let (gt, wt): (Vec<&str>, Vec<&str>) = (g.split(' ').collect(), w.split(' ').collect());
            let same = gt.len() == wt.len()
                && gt.iter().zip(&wt).all(|(a, b)| a == b || matches!((a.parse::<f32>(), b.parse::<f32>()), (Ok(x), Ok(y)) if (x - y).abs() <= tol));
            assert!(same, "\n got {g}\nwant {w}");
        }
    }

    pub fn thread() -> Thread {
        pz_ui::UIManager::reset();
        pz_ui::with_ui(|m| m.time_override_ms = Some(1_000_000));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_ui::register(&mut t);
        super::register(&mut t);
        t
    }

    pub fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    pub fn eval(t: &mut Thread, expr: &str) -> Value {
        run(t, &format!("__r = {expr}"));
        t.globals().get_str("__r")
    }

    /// Java's `Float.toString` for the values the oracles print.
    pub fn jf(v: f32) -> String {
        if v.fract() == 0.0 && v.abs() < 1e7 {
            format!("{v:.1}")
        } else {
            format!("{v}")
        }
    }

    /// The oracle output `name` under `PZ_HUD_ORACLE_DIR`, if set.
    pub fn oracle(name: &str) -> Option<String> {
        let dir = std::env::var("PZ_HUD_ORACLE_DIR").ok()?;
        Some(std::fs::read_to_string(std::path::Path::new(&dir).join(name)).expect("oracle file"))
    }

    /// `tex <name> <w> <h>` lines of an oracle file as texture sizes.
    pub fn oracle_textures(text: &str) -> Vec<(String, i32, i32)> {
        text.lines()
            .filter_map(|l| l.strip_prefix("tex "))
            .map(|l| {
                let p: Vec<&str> = l.rsplitn(3, ' ').collect();
                (
                    p[2].to_string(),
                    p[1].parse().unwrap(),
                    p[0].parse().unwrap(),
                )
            })
            .collect()
    }

    /// The draw list as the oracle's `draw` lines (integer rectangles;
    /// the oracle's float `drawf` lines are normalised the same way by
    /// [`normalise_oracle`]).
    pub fn draw_lines() -> Vec<String> {
        pz_ui::with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    pz_ui::UiDrawCmd::Quad {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        color,
                        ..
                    } => Some(format!(
                        "draw {} {} {} {} {} {} {} {} {}",
                        texture
                            .as_ref()
                            .map_or("-".to_string(), |t| t.name.to_string()),
                        *x as i32,
                        *y as i32,
                        *w as i32,
                        *h as i32,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    _ => None,
                })
                .collect()
        })
    }

    /// Oracle lines with `drawf` turned into `draw` with integer rectangles.
    pub fn normalise_oracle(line: &str) -> String {
        match line.strip_prefix("drawf ") {
            Some(r) => {
                let p: Vec<&str> = r.split(' ').collect();
                let i = |s: &str| s.parse::<f32>().unwrap() as i32;
                format!(
                    "draw {} {} {} {} {} {}",
                    p[0],
                    i(p[1]),
                    i(p[2]),
                    i(p[3]),
                    i(p[4]),
                    p[5..].join(" ")
                )
            }
            None => line.to_string(),
        }
    }

    /// The UI draw list as (texture name or "", x, y, w, h, alpha).
    pub fn quads() -> Vec<(String, f32, f32, f32, f32, f32)> {
        pz_ui::with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    pz_ui::UiDrawCmd::Quad {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        color,
                        ..
                    } => Some((
                        texture
                            .as_ref()
                            .map_or(String::new(), |t| t.name.to_string()),
                        *x,
                        *y,
                        *w,
                        *h,
                        color.3,
                    )),
                    _ => None,
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::host::{install, HudHost, HudPlayers};
    use super::test_util::*;
    use super::*;

    struct OnePlayer;
    impl HudPlayers for OnePlayer {
        fn has_player_instance(&self) -> bool {
            true
        }
        fn player_exists(&self, i: usize) -> bool {
            i == 0
        }
    }

    fn sc() -> Rc<UIElement> {
        with_ui(|m| m.speed_controls.clone()).unwrap()
    }

    #[test]
    fn layout_and_menu_init() {
        install_textures(&[]);
        install(HudHost::default());
        let mut t = thread();
        init_ui(&mut t);
        // Built in the main menu too, but only added with a player.
        let e = sc();
        assert!(with_ui(|m| m.ui.is_empty()));
        // 24 px icons + 6: buttons at 1, 33, 65, 97, 129 (step off in release).
        let xs: Vec<f64> = e.get_controls().iter().map(|b| b.get_x()).collect();
        assert_eq!(xs, [33.0, 65.0, 97.0, 129.0, 161.0]);
        assert_eq!((e.get_width(), e.get_height()), (191.0, 30.0));
        assert_eq!(
            eval(&mut t, "UIManager.getSpeedControls():getCurrentGameSpeed()").as_num(),
            Some(1.0)
        );
        assert_eq!(eval(&mut t, "SpeedControls.WaitSpeed").as_num(), Some(4.0));
    }

    #[test]
    fn speed_state_machine() {
        install_textures(&[]);
        install(HudHost {
            players: Box::new(OnePlayer),
            ..HudHost::default()
        });
        let mut t = thread();
        run(&mut t, "fired = 0; Events = nil");
        init_ui(&mut t);
        let e0 = sc();
        assert!(with_ui(|m| m.ui.iter().any(|u| Rc::ptr_eq(u, &e0))));
        let gt = crate::bind_climate::shared_game_time();
        let mult = || gt.borrow().multiplier;
        let clicked = || sc().get_clicked_value().map(|s| s.to_string());
        run(
            &mut t,
            "UIManager.getSpeedControls():ButtonClicked('Fast Forward x 2')",
        );
        assert_eq!(
            (
                eval(&mut t, "UIManager.getSpeedControls():getCurrentGameSpeed()").as_num(),
                mult()
            ),
            (Some(3.0), 20.0)
        );
        // The multiplier is set after SetCorrectIconStates ran; the icon
        // follows at the next update.
        assert_eq!(clicked(), None);
        sc().update(&mut t);
        assert_eq!(clicked().as_deref(), Some("Fast Forward x 2"));
        // Pause remembers speed 3 and the full multiplier; resume restores it.
        run(
            &mut t,
            "UIManager.getSpeedControls():ButtonClicked('Pause')",
        );
        assert!(eval(&mut t, "UIManager.getSpeedControls():isPaused()").truthy());
        assert_eq!((mult(), clicked().as_deref()), (1.0, Some("Pause")));
        run(
            &mut t,
            "UIManager.getSpeedControls():ButtonClicked('Pause')",
        );
        assert_eq!(
            eval(&mut t, "UIManager.getSpeedControls():getCurrentGameSpeed()").as_num(),
            Some(3.0)
        );
        // multiBeforePause is getMultiplier() read after both
        // setMultiplier(1) calls: 1 * 0.8, as in the original.
        assert_eq!(mult(), 0.8);
        run(
            &mut t,
            "UIManager.getSpeedControls():SetCurrentGameSpeed(1)",
        );
        assert_eq!((mult(), clicked().as_deref()), (1.0, Some("Play")));
        run(&mut t, "UIManager.getSpeedControls():ButtonClicked('Wait')");
        sc().update(&mut t);
        assert_eq!((mult(), clicked().as_deref()), (40.0, Some("Wait")));
        // Step forward: pause, then run one update and pause again.
        let e = sc();
        e.ext::<SpeedControls>().unwrap().step_forward(&e);
        e.ext::<SpeedControls>().unwrap().step_forward(&e);
        assert_eq!(
            e.ext::<SpeedControls>().unwrap().get_current_game_speed(),
            1
        );
        e.update(&mut t);
        assert_eq!(
            e.ext::<SpeedControls>().unwrap().get_current_game_speed(),
            1
        );
        e.update(&mut t);
        assert!(e.ext::<SpeedControls>().unwrap().is_paused());
    }

    #[test]
    fn render_draws_backgrounds_and_lit_icon() {
        install_textures(&[]);
        install(HudHost {
            players: Box::new(OnePlayer),
            ..HudHost::default()
        });
        let mut t = thread();
        init_ui(&mut t);
        let e = sc();
        e.set_x(1000.0);
        e.set_y(80.0);
        // The icon state is set after the children draw, so the first
        // frame after a change shows it from the next render on.
        e.update(&mut t);
        with_ui(|m| m.sprite.clear());
        e.render(&mut t);
        let q = quads();
        // Per button: black 0.75 background, then the icon at +3, +3.
        assert_eq!(q.len(), 10);
        assert_eq!(q[0], (String::new(), 1033.0, 80.0, 30.0, 30.0, 0.75));
        assert_eq!(
            q[1],
            (
                "media/ui/speedControls/Pause_Off.png".into(),
                1036.0,
                83.0,
                24.0,
                24.0,
                0.85
            )
        );
        // Play is the current speed: its highlight at full alpha.
        assert_eq!(
            q[3],
            (
                "media/ui/speedControls/Play_On.png".into(),
                1068.0,
                83.0,
                24.0,
                24.0,
                1.0
            )
        );
    }

    /// Parity with the original `SpeedControls` (oracle `speed.txt`):
    /// layout, the speed/multiplier/icon state after each click and its
    /// update, and the draw list of each state.
    #[test]
    fn matches_speed_controls_oracle() {
        let Some(text) = oracle("speed.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        let texs = oracle_textures(&text);
        let refs: Vec<(&str, i32, i32)> =
            texs.iter().map(|(n, w, h)| (n.as_str(), *w, *h)).collect();
        install_textures(&refs);
        install(HudHost::default());
        let mut t = thread();
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .multiplier = 1.0;
        let e = SpeedControls::create(false);
        let s = e.ext::<SpeedControls>().unwrap();
        let mut got = vec![format!(
            "size {} {}",
            jf(e.get_width() as f32),
            jf(e.get_height() as f32)
        )];
        for c in e.get_controls() {
            let name = c.ext::<speed_controls::HUDButton>().unwrap().name.clone();
            got.push(format!(
                "child {} {} {} {} {}",
                name,
                jf(c.get_x() as f32),
                jf(c.get_y() as f32),
                jf(c.get_width() as f32),
                jf(c.get_height() as f32)
            ));
        }
        e.set_x(1000.0);
        e.set_y(80.0);
        let state = |label: &str| {
            let tm = crate::bind_climate::shared_game_time()
                .borrow()
                .get_true_multiplier();
            format!(
                "state {label} speed {} mult {} clicked {}",
                s.get_current_game_speed(),
                jf(tm),
                e.get_clicked_value().as_deref().unwrap_or("null")
            )
        };
        for step in [
            "update",
            "Pause",
            "Pause",
            "Fast Forward x 1",
            "Fast Forward x 2",
            "Wait",
            "Play",
            "Pause",
        ] {
            if step == "update" {
                e.update(&mut t);
            } else {
                e.button_clicked(Some(Rc::from(step)));
            }
            got.push(state(step));
            with_ui(|m| m.sprite.clear());
            e.update(&mut t);
            e.render(&mut t);
            got.push(state(&format!("{step}+update")));
            got.push(format!("case {step}"));
            got.extend(draw_lines());
        }
        let b = e.control(2).unwrap();
        with_ui(|m| m.sprite.clear());
        b.on_mouse_move(&mut t, 0.0, 0.0);
        b.on_mouse_down(&mut t, 1.0, 1.0);
        e.render(&mut t);
        got.push("case hover-press".into());
        got.extend(draw_lines());
        with_ui(|m| m.sprite.clear());
        b.on_mouse_up(&mut t, 1.0, 1.0);
        e.update(&mut t);
        got.push(state("released"));
        e.render(&mut t);
        got.push("case released".into());
        got.extend(draw_lines());
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("tex "))
            .map(normalise_oracle)
            .collect();
        assert_eq!(got, want);
    }

    struct PlayerOnScreen;
    impl HudPlayers for PlayerOnScreen {
        fn has_player_instance(&self) -> bool {
            true
        }
        fn player_exists(&self, i: usize) -> bool {
            i == 0
        }
        fn player_screen(&self, i: usize) -> Option<host::PlayerScreen> {
            (i == 0).then_some(host::PlayerScreen {
                x: 120.5,
                y: 87.25,
                z: 1.0,
                offset_x: 64.0,
                offset_y: 192.0,
                user_name_height: 0,
            })
        }
    }
    struct Cam;
    impl host::HudCamera for Cam {
        // IsoCamera.getOffX() truncates PlayerCamera's offsets.
        fn off_x(&self) -> f32 {
            1234.0
        }
        fn off_y(&self) -> f32 {
            5678.0
        }
    }

    #[test]
    fn progress_bars_from_init_and_lua() {
        install_textures(&[("BuildBar_Bkg", 120, 16)]);
        install(HudHost {
            players: Box::new(OnePlayer),
            ..HudHost::default()
        });
        let mut t = thread();
        init_ui(&mut t);
        // Four hidden bars at value 1, one per player, in the UI list.
        for i in 0..4 {
            let b = progress_bar(i).unwrap();
            assert!(!b.is_visible() && b.get_render_this_player_only() == i as i32);
            assert_eq!(b.ext::<ActionProgressBar>().unwrap().get_value(), 1.0);
        }
        assert_eq!(with_ui(|m| m.ui.len()), 5);
        assert!(eval(&mut t, "UIManager.trySetProgressBarValue(1, 0.5)").truthy());
        assert_eq!(
            eval(&mut t, "UIManager.getProgressBar(1):getValue()").as_num(),
            Some(0.5)
        );
        run(&mut t, "UIManager.getProgressBar(1):setValue(0.25)");
        assert_eq!(
            progress_bar(1)
                .unwrap()
                .ext::<ActionProgressBar>()
                .unwrap()
                .get_value(),
            0.25
        );
        run(
            &mut t,
            "ok = pcall(function() UIManager.getProgressBar(4) end)",
        );
        assert!(!eval(&mut t, "ok").truthy());
        // The UI pass skips it (follows the game world); IngameState draws it.
        let b = progress_bar(1).unwrap();
        b.set_visible(true);
        with_ui(|m| m.sprite.clear());
        UIManager::render(&mut t);
        assert!(drawuv_lines().is_empty());
        render_progress_bar(&mut t, 1);
        assert_eq!(drawuv_lines().len(), 2);
    }

    /// Parity with the original `ActionProgressBar` (oracle
    /// `progress.txt`): render at values 0..1 and scales 1 and 2, the
    /// show/hide countdown with the screen position, the infinite sweep
    /// (texture coordinates included, truncated to 1/1000 as the original).
    #[test]
    fn matches_progress_bar_oracle() {
        let Some(text) = oracle("progress.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_ex(&oracle_textures_ex(&text));
        install(HudHost {
            players: Box::new(PlayerOnScreen),
            camera: Box::new(Cam),
            ..HudHost::default()
        });
        let mut t = thread();
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .fps_multiplier = 1.0;
        let core = crate::bind_core::shared_core();
        let e = ActionProgressBar::create(0, 0);
        let bar = e.ext::<ActionProgressBar>().unwrap();
        let mut got = vec![format!(
            "size {} {}",
            jf(e.get_width() as f32),
            jf(e.get_height() as f32)
        )];
        e.set_x(400.0);
        e.set_y(300.0);
        let draws = |got: &mut Vec<String>, label: String| {
            got.push(format!("case {label}"));
            got.extend(drawuv_lines());
            with_ui(|m| m.sprite.clear());
        };
        with_ui(|m| m.sprite.clear());
        for scale in 1..=2 {
            core.borrow_mut().set_option_action_progress_bar_size(scale);
            for v in [0.0f32, 0.25, 0.5, 0.999, 1.0] {
                bar.set_value(v);
                e.render(&mut t);
                draws(&mut got, format!("scale {scale} value {}", jf(v)));
            }
        }
        core.borrow_mut().set_option_action_progress_bar_size(1);
        e.set_visible(false);
        let values = [1.0, 0.3, 0.6, 1.0, 1.0, 1.0, f32::INFINITY, f32::INFINITY];
        for (i, v) in values.iter().enumerate() {
            bar.set_value(*v);
            bar.update_bar(&e, 0);
            got.push(format!(
                "update {i} visible {} delay {} pos {} {} {} {}",
                e.is_visible(),
                bar.delay_hide.get(),
                jf(e.get_x() as f32),
                jf(e.get_y() as f32),
                jf(e.get_width() as f32),
                jf(e.get_height() as f32)
            ));
        }
        for i in (0..60).step_by(7) {
            bar.update_bar(&e, 0);
            e.render(&mut t);
            draws(&mut got, format!("infinite {i}"));
        }
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("tex ") && !l.starts_with("fake "))
            .map(normalise_oracle)
            .collect();
        assert_eq!(got, want);
    }
}
