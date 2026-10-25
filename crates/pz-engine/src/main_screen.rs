//! `zombie.gameStates.MainScreenState`: the title screen's own drawing
//! (background, lightning, logo) around the Lua main menu.

use crate::game_state::{GameState, StateAction, StateCx};
use pz_lua_api::bind_video::{VideoRef, VideoTexture};
use pz_lua_api::LuaManager;
use pz_render::{DrawList, Rgba, Texture};
use pz_ui::UIManager;

const GL_SRC_ALPHA: u32 = 770;
const GL_ONE_MINUS_SRC_ALPHA: u32 = 771;
const GL_ONE: u32 = 1;

/// `MainScreenState.ScreenElement`.
pub struct ScreenElement {
    pub tex: Option<Texture>,
    pub x: i32,
    pub y: i32,
    pub alpha: f32,
    pub target_alpha: f32,
    pub alpha_step: f32,
}

impl ScreenElement {
    fn update(&mut self, multiplier: f32) {
        if self.alpha < self.target_alpha {
            self.alpha = (self.alpha + self.alpha_step * multiplier).min(self.target_alpha);
        } else if self.alpha > self.target_alpha {
            self.alpha = (self.alpha - self.alpha_step * multiplier).max(self.target_alpha);
        }
    }
}

#[derive(Default)]
pub struct MainScreenState {
    pub elements: Vec<ScreenElement>,
    pub lightning_delta: f32,
    pub lightning_target_delta: f32,
    pub lightning_full_timer: f32,
    pub lightning_count: f32,
    pub lightning_time: f32,
    pub light_off_count: f32,
    pub show_logo: bool,
    pub fade_alpha: f32,
    pub total_scale: f32,
    /// `videoTex`: the title video (`title_screen_background.bik`).
    pub video_tex: Option<VideoRef>,
}

fn tex(path: &str) -> Option<Texture> {
    pz_render::get_shared_texture(path)
}

impl MainScreenState {
    /// `enter()`.
    pub fn enter(&mut self, lua: &mut LuaManager, screen_w: i32, screen_h: i32) {
        self.elements.clear();
        self.total_scale = screen_h as f32 / 1080.0;
        let logo = tex("media/ui/PZ_Logo.png");
        let lw = logo.as_ref().map_or(0, |t| t.width) as f32;
        let y = (screen_h as f32 * 0.7) as i32;
        self.elements.push(ScreenElement {
            x: screen_w / 2 - (lw * self.total_scale) as i32 / 2,
            y: y - (350.0 * self.total_scale) as i32,
            tex: logo,
            alpha: 0.0,
            target_alpha: 1.0,
            alpha_step: 0.2 * 0.9,
        });
        lua.trigger_event("OnMainMenuEnter", &[]);
    }

    /// `update()`.
    pub fn update(&mut self, lua: &mut LuaManager, multiplier: f32) {
        lua.trigger_event("OnFETick", &[0.0.into()]);
        for e in &mut self.elements {
            e.update(multiplier);
        }
    }

    /// `render()`: black fill, background, then the UI.
    pub fn render(&mut self, lua: &mut LuaManager, out: &mut DrawList, screen_w: i32, screen_h: i32, multiplier: f32) {
        self.lightning_time += multiplier;
        out.blend_func(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        out.renderi(None, 0, 0, screen_w, screen_h, Rgba(0.0, 0.0, 0.0, 1.0));
        out.blend_func(GL_SRC_ALPHA, GL_SRC_ALPHA);
        self.render_background(out, screen_w, screen_h, multiplier);
        UIManager::render(&mut lua.thread);
        pz_ui::with_ui(|m| m.sprite.lower_into(out));
        pz_ui::with_ui(|m| m.sprite.clear());
        if self.fade_alpha > 0.0 {
            out.renderi(None, 0, 0, screen_w, screen_h, Rgba(0.0, 0.0, 0.0, self.fade_alpha));
        }
    }

    /// `exit()`'s video part: closes the title video.
    pub fn exit_video(&mut self) {
        if let Some(v) = self.video_tex.take() {
            v.close_and_destroy();
        }
    }

    /// `renderVideo()`: the title video scaled to the largest 16:9 box
    /// centred on the screen; false when it cannot play.
    fn render_video(&mut self, out: &mut DrawList, sw: i32, sh: i32) -> bool {
        if self.video_tex.is_none() {
            self.video_tex = VideoTexture::get_or_create("title_screen_background.bik", 1920, 1080, true);
        }
        let Some(v) = self.video_tex.as_ref().filter(|v| v.is_valid()) else { return false };
        v.render_frame();
        let scale = (sw as f32 / 16.0).min(sh as f32 / 9.0);
        let width = (scale * 16.0) as i32;
        let height = (scale * 9.0) as i32;
        let x = (sw - width) / 2;
        let y = (sh - height) / 2;
        out.renderi(Some(&v.texture()), x, y, width, height, Rgba(1.0, 1.0, 1.0, 1.0));
        true
    }

    /// `renderBackground()`: the title video, or the stills when video
    /// effects are off or the video cannot play.
    fn render_background(&mut self, out: &mut DrawList, sw: i32, sh: i32, multiplier: f32) {
        if self.lightning_target_delta == 0.0 && self.lightning_delta != 0.0 && self.lightning_delta < 0.6 && self.lightning_count == 0.0 {
            self.lightning_target_delta = 1.0;
            self.lightning_count = 1.0;
        }
        if self.lightning_target_delta == 1.0
            && self.lightning_delta == 1.0
            && (self.lightning_full_timer > 1.0 && self.lightning_count == 0.0 || self.lightning_full_timer > 10.0)
        {
            self.lightning_target_delta = 0.0;
            self.lightning_full_timer = 0.0;
        }
        if self.lightning_target_delta == 1.0 && self.lightning_delta == 1.0 {
            self.lightning_full_timer += multiplier;
        }
        if self.lightning_delta != self.lightning_target_delta {
            if self.lightning_delta < self.lightning_target_delta {
                self.lightning_delta += 0.17 * multiplier;
                if self.lightning_delta > self.lightning_target_delta {
                    self.lightning_delta = self.lightning_target_delta;
                    if self.lightning_delta == 1.0 {
                        self.show_logo = true;
                    }
                }
            }
            if self.lightning_delta > self.lightning_target_delta {
                self.lightning_delta -= 0.025 * multiplier;
                if self.lightning_count == 0.0 {
                    self.lightning_delta -= 0.1;
                }
                if self.lightning_delta < self.lightning_target_delta {
                    self.lightning_delta = self.lightning_target_delta;
                    self.lightning_count = 0.0;
                }
            }
        }
        // TODO(rand): `Rand.Next(150) == 0` sets lightOffCount (unused here).
        let a = 1.0 - self.lightning_delta * 0.6;
        let video_effects = pz_lua_api::bind_core::shared_core().borrow().get_option_do_video_effects();
        if video_effects && self.render_video(out, sw, sh) {
            return;
        }
        self.render_original_background(out, sw, sh, a);
    }

    /// `renderOriginalBackground(a)`.
    fn render_original_background(&mut self, out: &mut DrawList, sw: i32, sh: i32, a: f32) {
        let (Some(l), Some(l2), Some(l3), Some(l4)) =
            (tex("media/ui/Title.png"), tex("media/ui/Title2.png"), tex("media/ui/Title3.png"), tex("media/ui/Title4.png"))
        else {
            return;
        };
        let he = sh as f32 / 1080.0;
        let wi = l.width as f32 * he;
        let wi2 = l2.width as f32 * he;
        let mut bigger = sw as f32 - (wi + wi2);
        if bigger >= 0.0 {
            bigger = 0.0;
        }
        let he1 = 1024.0 * he;
        let c = |al: f32| Rgba(1.0, 1.0, 1.0, al);
        let b = bigger as i32;
        out.renderi(Some(&l), b, 0, wi as i32, he1 as i32, c(a));
        out.renderi(Some(&l2), b + wi as i32, 0, wi as i32, he1 as i32, c(a));
        out.renderi(Some(&l3), b, he1 as i32, wi as i32, (l3.height as f32 * he) as i32, c(a));
        out.renderi(Some(&l4), b + wi as i32, he1 as i32, wi as i32, (l3.height as f32 * he) as i32, c(a));
        out.blend_func(GL_SRC_ALPHA, GL_ONE);
        let names = ["media/ui/Title_lightning.png", "media/ui/Title_lightning2.png", "media/ui/Title_lightning3.png", "media/ui/Title_lightning4.png"];
        let pos = [(b, 0), (b + wi as i32, 0), (b, he1 as i32), (b + wi as i32, he1 as i32)];
        for (n, (x, y)) in names.iter().zip(pos) {
            if let Some(t) = tex(n) {
                out.renderi(Some(&t), x, y, wi as i32, he1 as i32, c(self.lightning_delta));
            }
        }
        out.blend_func(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    }
}

impl GameState for MainScreenState {
    fn class_name(&self) -> &'static str {
        "zombie.gameStates.MainScreenState"
    }

    fn enter(&mut self, cx: &mut StateCx<'_>) {
        MainScreenState::enter(self, cx.lua, cx.screen_w, cx.screen_h);
    }

    /// `exit()`: TODO(port): the blocking 250 ms fade (`fadeAlpha`, which
    /// renders frames from inside `exit`), `ServerBrowser.Release`, and
    /// the music stop and volume restore; then the video closes.
    fn exit(&mut self, _cx: &mut StateCx<'_>) {
        self.exit_video();
    }

    fn update(&mut self, cx: &mut StateCx<'_>) -> StateAction {
        // TODO(port): `connectToServerState`, `restartDebounceClickTimer`,
        // `DebugFileWatcher`.
        MainScreenState::update(self, cx.lua, cx.multiplier);
        StateAction::Remain
    }

    fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        MainScreenState::render(self, cx.lua, out, cx.screen_w, cx.screen_h, cx.multiplier);
    }
}
