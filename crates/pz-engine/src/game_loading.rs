//! `zombie.gameStates.LoadingQueueState` and `GameLoadingState`: the
//! screens between the main menu and the game, and the world load.
//!
//! The original runs `GameLoadingState`'s loader (`runInner`) on its own
//! thread, which waits twice for the main thread's asset loads
//! (`waitForAssetLoadingToFinish1/2`, released by `update`). The Lua state
//! is single threaded here, so each part of `runInner` runs in line in the
//! `update` that releases its lock, in the original's order of operations
//! and events.

use crate::game_state::{state, GameState, StateAction, StateCx, StateRef};
use crate::ingame::IngameState;
use crate::iso_world::{IsoWorld, WORLD_VERSION};
use pz_render::{DrawList, Rgba};
use pz_text::UIFont;
use pz_ui::TextAlign;
use std::time::Instant;

/// `SoundManager.instance.setMusicState(name)`.
fn set_music_state(name: &str) {
    pz_lua_api::bind_audio::sound_system().borrow_mut().with(|m, _| m.set_music_state(name));
}

/// `Translator.getText(key)`.
fn text(key: &str) -> String {
    pz_lua_api::bind_text::with_translator(|t| t.get_text(key, &[])).unwrap_or_else(|| key.to_string())
}

fn draw_centre(font: UIFont, x: f64, y: f64, s: &str, r: f32, g: f32, b: f32, a: f32) {
    pz_ui::draw::draw_string(Some(font), x, y, 1.0, s, Rgba(r, g, b, a), TextAlign::Centre, false);
}

fn draw_left(font: UIFont, x: f64, y: f64, s: &str, r: f32, g: f32, b: f32, a: f32) {
    pz_ui::draw::draw_string(Some(font), x, y, 1.0, s, Rgba(r, g, b, a), TextAlign::Left, false);
}

/// `SpriteRenderer.instance.renderi(null, 0, 0, w, h, 0, 0, 0, 1, null)`
/// and the queued UI draws, lowered into the frame's list.
fn flush_ui(out: &mut DrawList) {
    pz_ui::with_ui(|m| m.sprite.lower_into(out));
    pz_ui::with_ui(|m| m.sprite.clear());
}

// ---- LoadingQueueState --------------------------------------------------------------

/// `zombie.gameStates.LoadingQueueState`: in a single-player game it hands
/// over to `GameLoadingState` on its first update (the queue is the
/// server's login queue).
#[derive(Default)]
pub struct LoadingQueueState {
    /// `cancel`.
    pub cancel: bool,
}

impl GameState for LoadingQueueState {
    fn class_name(&self) -> &'static str {
        "zombie.gameStates.LoadingQueueState"
    }

    fn enter(&mut self, _cx: &mut StateCx<'_>) {
        self.cancel = false;
        // TODO(port): `GameWindow.activatedJoyPad`'s A button.
        set_music_state("Loading");
        // TODO(network): `GameClient.sendLoginQueueRequest()`.
    }

    fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
        if self.cancel {
            return Some(state(crate::main_screen::MainScreenState::default()));
        }
        Some(state(GameLoadingState::default()))
    }

    fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        out.renderi(None, 0, 0, cx.screen_w, cx.screen_h, Rgba(0.0, 0.0, 0.0, 1.0));
        // `placeInQueue >= 0` (a server queue) draws the menu background,
        // the UI and the queue's UI.
    }

    fn update(&mut self, _cx: &mut StateCx<'_>) -> StateAction {
        // TODO(network): the client's queue.
        StateAction::Continue
    }
}

// ---- GameLoadingState ----------------------------------------------------------------

/// Where the in-line loader is (`runInner` and its two waits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loader {
    /// Not started (`loader == null`).
    None,
    /// Waiting on `assetLock1`.
    WaitAssets1,
    /// Waiting on `assetLock2`.
    WaitAssets2,
    /// `runInner` returned (or threw).
    Finished,
}

/// `zombie.gameStates.GameLoadingState`.
pub struct GameLoadingState {
    loader: Loader,
    /// `newGame` (static).
    pub new_game: bool,
    start_time: Instant,
    /// `worldVersionError`.
    pub world_version_error: bool,
    /// `unexpectedError`.
    pub unexpected_error: bool,
    /// `gameLoadingString`.
    pub game_loading_string: String,
    /// `playerWrongIP`.
    pub player_wrong_ip: bool,
    showed_ui: bool,
    showed_click_to_skip: bool,
    player_created: bool,
    /// `done`.
    pub done: bool,
    /// `convertingWorld`, `convertingFileCount`, `convertingFileMax`.
    pub converting_world: bool,
    pub converting_file_count: i32,
    pub converting_file_max: i32,
    wait_for_asset_loading_to_finish1: bool,
    wait_for_asset_loading_to_finish2: bool,
    time: f32,
    force_done: bool,
    stage: i32,
    loading_dot_tick: f32,
    loading_dot: &'static str,
    click_to_skip_alpha: f32,
    click_to_skip_fade_in: bool,
    quick_tips_timer: f32,
    quick_tips_text: Option<String>,
    quick_tips_list: Vec<String>,
    quick_tips_list_joke: Vec<String>,
}

/// `QUICK_TIP_MAX_TIMER`.
const QUICK_TIP_MAX_TIMER: f32 = 720.0;
/// `totalTime`.
const TOTAL_TIME: f32 = 33.0;
/// `BOTTOM_SCREEN`.
const BOTTOM_SCREEN: f32 = 40.0;

impl Default for GameLoadingState {
    fn default() -> Self {
        GameLoadingState {
            loader: Loader::None,
            new_game: true,
            start_time: Instant::now(),
            world_version_error: false,
            unexpected_error: false,
            game_loading_string: String::new(),
            player_wrong_ip: false,
            showed_ui: false,
            showed_click_to_skip: false,
            player_created: false,
            done: false,
            converting_world: false,
            converting_file_count: -1,
            converting_file_max: -1,
            wait_for_asset_loading_to_finish1: false,
            wait_for_asset_loading_to_finish2: false,
            time: 0.0,
            force_done: false,
            stage: 0,
            loading_dot_tick: 0.0,
            loading_dot: "",
            click_to_skip_alpha: 1.0,
            click_to_skip_fade_in: false,
            quick_tips_timer: QUICK_TIP_MAX_TIMER,
            quick_tips_text: None,
            quick_tips_list: Vec::new(),
            quick_tips_list_joke: Vec::new(),
        }
    }
}

impl GameLoadingState {
    /// `ZomboidFileSystem.getCurrentSaveDir()`.
    fn current_save_dir(cx: &StateCx<'_>) -> std::path::PathBuf {
        let core = pz_lua_api::bind_core::shared_core();
        let c = core.borrow();
        cx.launch.cache_dir.join("Saves").join(&c.game_mode).join(&c.game_save_world)
    }

    /// `loadQuickTipList()`.
    fn load_quick_tip_list(&mut self) {
        self.quick_tips_list.clear();
        self.quick_tips_list_joke.clear();
        pz_lua_api::bind_text::with_translator(|t| {
            for key in t.get_ui().keys() {
                if key.starts_with("UI_quick_tip_joke") {
                    self.quick_tips_list_joke.push(key.clone());
                } else if key.starts_with("UI_quick_tip") {
                    self.quick_tips_list.push(key.clone());
                }
            }
        });
    }

    /// `SendDone()` (single player).
    fn send_done(&mut self) {
        println!("game loading took {} seconds", (self.start_time.elapsed().as_millis() + 999) / 1000);
        self.done = true;
        // TODO(port): `GameKeyboard.noEventsWhileLoading = false`.
    }

    /// `runInner` up to `waitForAssetLoadingToFinish2`.
    fn run_inner_1(&mut self, cx: &mut StateCx<'_>) -> Result<(), String> {
        let core = pz_lua_api::bind_core::shared_core();
        let mode_dir = cx.launch.cache_dir.join("Saves").join(core.borrow().game_mode.clone());
        let _ = std::fs::create_dir(&mode_dir);
        // TODO(port): `BaseVehicle.LoadAllVehicleTextures()`;
        // `TutorialManager.instance = new TutorialManager()`.
        pz_lua_api::bind_climate::install_game_time(std::rc::Rc::new(std::cell::RefCell::new(pz_core::game_time::GameTime::default())));
        pz_lua_api::bind_climate::new_climate_manager(&mut cx.lua.thread);
        let spawn_region = cx.world.as_ref().map(|w| w.spawn_region.clone()).unwrap_or_default();
        let mut world = IsoWorld::new(&cx.launch.game_dir, &cx.launch.cache_dir, Self::current_save_dir(cx));
        world.spawn_region = spawn_region;
        world.new_game_setup = cx.launch.start.new_game_setup();
        let r = world.init(cx.lua);
        *cx.world = Some(world);
        r?;
        // TODO(network): `GameWindow.serverDisconnected`; `playerWrongIP`.
        if self.world_version_error {
            return Ok(());
        }
        println!("triggerEvent OnGameTimeLoaded");
        cx.lua.trigger_event("OnGameTimeLoaded", &[]);
        // TODO(port): `SGlobalObjects.initSystems()` / `CGlobalObjects.initSystems()`
        // (the Lua global object systems), `IsoObjectPicker.Instance.Init()`,
        // `TutorialManager.instance.init()` / `CreateQuests()`.
        let saved = cx.world.as_ref().map_or(-1, |w| w.saved_world_version);
        let save_dir = Self::current_save_dir(cx);
        let new_game = !save_dir.join("map_ver.bin").exists();
        if new_game || saved != WORLD_VERSION {
            if !new_game {
                self.game_loading_string = "Saving converted world.".into();
            }
            // TODO(port): `GameWindow.save(true)` (`world_save::save_game`
            // with the engine's `SaveGameHooks`), which writes `map_ver.bin`
            // and the rest of a new world.
        }
        // `ChatUtility.InitAllowedChatIcons()` (TODO(port):
        // `Texture.collectAllIcons`), then `ChatManager.init(true, player)`.
        pz_lua_api::chat::set_allowed_chat_icons(Default::default(), Default::default());
        if let Some(p) = pz_lua_api::bind_players::player_instance() {
            if let Err(e) = pz_lua_api::chat::with_chat_manager(|m| m.init(true, p)) {
                eprintln!("ChatManager.init: {e:?}");
            }
        }
        // TODO(port): `Bullet.startLoadingPhysicsMeshes()`.
        for t in [
            "media/textures/NewShadow.png",
            "media/wallcutaways.png",
            "media/windowframe_cutaways.png",
            "media/windowframe_cutaways_2.png",
            "media/windowframe_cutaways_3.png",
        ] {
            let _ = pz_render::get_shared_texture(t);
        }
        println!("bWaitForAssetLoadingToFinish2 start");
        self.wait_for_asset_loading_to_finish2 = true;
        Ok(())
    }

    /// `runInner` after `waitForAssetLoadingToFinish2`.
    fn run_inner_2(&mut self, _cx: &mut StateCx<'_>) {
        println!("bWaitForAssetLoadingToFinish2 end");
        // TODO(port): `FBORenderChunkManager.instance.gameLoaded()`,
        // `Bullet.initPhysicsMeshes()`, `RBTableStoryBase.initStories()`,
        // `IsoPlayer.visitAllPlayers(ECSEntity::onGameLoadingStateEnter)`.
        self.player_created = true;
        self.game_loading_string.clear();
        self.send_done();
    }

    /// The loader's `run()`: `runInner` throwing sets `unexpectedError`;
    /// `finally` releases the UI.
    fn loader_result(&mut self, r: Result<(), String>) {
        if let Err(e) = r {
            self.unexpected_error = true;
            eprintln!("ERROR: GameLoadingState: {e}");
        }
        if self.unexpected_error || !self.wait_for_asset_loading_to_finish2 {
            self.loader = Loader::Finished;
            pz_ui::with_ui(|m| m.suspend = false);
        }
    }

    /// `getNewQuickTip()`.
    fn new_quick_tip(&self) -> Option<String> {
        if self.quick_tips_list.is_empty() || self.quick_tips_list_joke.is_empty() {
            return None;
        }
        let mut tip = &self.quick_tips_list[pz_core::random::next(self.quick_tips_list.len() as i32) as usize];
        // `Rand.NextBool(13)`.
        if pz_core::random::next(13) == 0 {
            tip = &self.quick_tips_list_joke[pz_core::random::next(self.quick_tips_list_joke.len() as i32) as usize];
        }
        Some(text(tip))
    }

    /// `doQuickTips()`.
    fn do_quick_tips(&mut self, cx: &StateCx<'_>) {
        if self.new_game {
            return;
        }
        if self.quick_tips_timer > QUICK_TIP_MAX_TIMER {
            self.quick_tips_text = self.new_quick_tip();
            self.quick_tips_timer = 0.0;
        }
        self.quick_tips_timer += cx.multiplier;
        if let Some(t) = self.quick_tips_text.as_deref().filter(|t| !t.is_empty()) {
            draw_centre(UIFont::NewMedium, cx.screen_w as f64 / 2.0, (cx.screen_h as f32 - BOTTOM_SCREEN) as f64, t, 0.5, 0.5, 0.5, 1.0);
        }
    }

    /// One intro text's fade: in over a second from `start`, full until
    /// `full_end`, out over two seconds until `end`; the stage moves on at
    /// `end`.
    fn intro_alpha(&mut self, start: f32, full_end: f32, end: f32) -> f32 {
        let pos = self.time;
        let mut del = 0.0;
        if pos > start && pos < start + 1.0 {
            del = pos - start;
        }
        if pos >= start + 1.0 && pos <= full_end {
            del = 1.0;
        }
        if pos > full_end && pos < end {
            del = 1.0 - (pos - full_end) / 2.0;
        }
        if pos >= end {
            self.stage += 1;
        }
        del
    }
}

impl GameState for GameLoadingState {
    fn class_name(&self) -> &'static str {
        "zombie.gameStates.GameLoadingState"
    }

    fn enter(&mut self, cx: &mut StateCx<'_>) {
        self.load_quick_tip_list();
        // TODO(port): `WorldMapImages.Reset()`, `WorldMapVisited.Reset()`.
        pz_lua_api::bind_video::release_all_video_textures();
        // TODO(port): `GameWindow.loadedAsClient` / `okToSaveOnExit = false`.
        self.showed_ui = false;
        // TODO(port): `ChunkMapFilenames.instance.clear()`.
        let core = pz_lua_api::bind_core::shared_core();
        println!("Savefile name is \"{}\"", core.borrow().game_save_world);
        self.game_loading_string.clear();
        cx.lua.load_dir_base("server");
        // TODO(port): `LuaManager.finishChecksum()`.
        pz_lua_api::bind_registry::shared_script_manager().loaded_after_lua();
        // TODO(port): `Core.initFBOs()`, `initShaders()`, `SkyBox.getInstance()`,
        // `IsoPuddles.getInstance()`, `IsoWater.getInstance()`.
        let (save_world, no_save, tutorial, last_stand, mode) = {
            let c = core.borrow();
            (c.game_save_world.clone(), c.is_no_save(), c.tutorial, c.is_last_stand(), c.game_mode.clone())
        };
        if save_world.is_empty() {
            // The original shows this and exits the process.
            println!("No savefile directory was specified.  It's a bug.");
            self.unexpected_error = true;
            return;
        }
        let save_dir = Self::current_save_dir(cx);
        if !save_dir.exists() && !no_save {
            println!("The savefile directory doesn't exist.  It's a bug.");
            self.unexpected_error = true;
            return;
        }
        if !no_save {
            if let Err(e) = pz_lua_api::world_save::ensure_subdirectories_exist(&save_dir) {
                eprintln!("ERROR: {e}");
            }
        }
        if !(tutorial || last_stand || mode == "Multiplayer") {
            let ini = format!("{save_world}\r\n{mode}\r\n");
            if let Err(e) = std::fs::write(cx.launch.cache_dir.join("latestSave.ini"), ini) {
                eprintln!("ERROR: latestSave.ini: {e}");
            }
        }
        self.done = false;
        self.force_done = false;
        // TODO(port): `IsoChunkMap.CalcChunkWidth()` is the streamer's
        // (`IsoWorld.init`); `Core.setInitialSize()`, `LosUtil.init`.
        self.time = 0.0;
        self.stage = 0;
        self.click_to_skip_alpha = 1.0;
        self.click_to_skip_fade_in = false;
        self.start_time = Instant::now();
        // TODO(port): `SoundManager.instance.Purge()`.
        set_music_state("Loading");
        cx.lua.trigger_event("OnPreMapLoad", &[]);
        self.new_game = !save_dir.join("map_ver.bin").exists();
        self.world_version_error = false;
        self.unexpected_error = false;
        self.player_created = false;
        self.converting_world = false;
        self.converting_file_count = 0;
        self.converting_file_max = -1;
        if !self.new_game {
            self.stage = -1;
            // TODO(port): `screenFader.startFadeFromBlack()`.
        }
        // TODO(port): `WorldDictionary.setIsNewGame(newGame)`,
        // `GameKeyboard.noEventsWhileLoading = true`.
        // `loader.start()`: `runInner` waits on `assetLock1` first.
        self.wait_for_asset_loading_to_finish1 = true;
        self.loader = Loader::WaitAssets1;
        pz_ui::with_ui(|m| m.suspend = true);
    }

    fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
        Some(state(IngameState::default()))
    }

    fn exit(&mut self, cx: &mut StateCx<'_>) {
        // TODO(port): the blocking fade to black (`screenFader`), which
        // renders frames from inside `exit`.
        pz_lua_api::hud::init_ui(&mut cx.lua.thread);
        let p0 = pz_lua_api::bind_players::player(0).map(|p| pz_lua_api::bind_character_sim::character_value(&p)).unwrap_or_default();
        cx.lua.trigger_event("OnCreatePlayer", &[0.0.into(), p0]);
        self.loader = Loader::None;
        self.done = false;
        self.stage = 0;
        // TODO(port): `IsoCamera.SetCharacterToFollow(IsoPlayer.getInstance())`.
        pz_lua_api::bind_audio::sound_system().borrow_mut().with(|m, ctx| m.stop_music(Some(""), ctx.backend));
        // TODO(port): `AmbientStreamManager.instance.init()`; the asleep
        // player's fade and game speed.
        let core = pz_lua_api::bind_core::shared_core();
        let no_save = core.borrow().is_no_save();
        let mods = pz_lua_api::savefile::active_mods_by_id("currentGame");
        // TODO(port): `checkMissingMods` / `checkMissingMaps`.
        pz_lua_api::savefile::set_active_mods("loaded", mods.clone());
        let path = Self::current_save_dir(cx).join("mods.txt");
        pz_lua_api::savefile::write_active_mods(no_save, &path.to_string_lossy(), &mods);
        println!("Game Mode: {}", core.borrow().game_mode);
        println!("Sandbox Options:");
        if let Ok(pz_kahlua::Value::Object(o)) = pz_lua_api::bind_sandbox::instance(&mut cx.lua.thread) {
            if let Some(s) = o.as_any().downcast_ref::<pz_lua_api::bind_sandbox::SandboxObj>() {
                let s = s.0.borrow();
                for i in 0..s.num_options() {
                    if let Some(o) = s.option_by_index(i) {
                        println!("{} {}", o.name(), o.config.get_value_as_string());
                    }
                }
            }
        }
        // TODO(port): `GameWindow.okToSaveOnExit = true`.
    }

    fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        let font_small = pz_ui::draw::font_height(Some(UIFont::NewSmall)) as f32;
        self.loading_dot_tick += cx.multiplier;
        if self.loading_dot_tick > 20.0 {
            self.loading_dot = ".";
        }
        if self.loading_dot_tick > 40.0 {
            self.loading_dot = "..";
        }
        if self.loading_dot_tick > 60.0 {
            self.loading_dot = "...";
        }
        if self.loading_dot_tick > 80.0 {
            self.loading_dot = "";
            self.loading_dot_tick = 0.0;
        }
        self.time += cx.time_delta;
        let (mut alpha1, mut alpha2, mut alpha3) = (0.0, 0.0, 0.0);
        if self.stage == 0 {
            alpha1 = self.intro_alpha(0.0, 5.0, 7.0);
        }
        if self.stage == 1 {
            alpha2 = self.intro_alpha(7.0, 13.0, 15.0);
        }
        if self.stage == 2 {
            alpha3 = self.intro_alpha(15.0, 31.0, TOTAL_TIME);
        }
        let (sw, sh) = (cx.screen_w, cx.screen_h);
        out.renderi(None, 0, 0, sw, sh, Rgba(0.0, 0.0, 0.0, 1.0));
        if self.stage == -1 {
            // TODO(port): `renderProgressIndicator()` (the walking
            // `AnimatedTexture`) and the `ScreenFader`.
        }
        let (cxm, cym) = (sw as f64 / 2.0, sh as f64 / 2.0);
        if self.unexpected_error {
            let medium = pz_ui::draw::font_height(Some(UIFont::Medium)) as f64;
            let small = pz_ui::draw::font_height(Some(UIFont::Small)) as f64;
            let dy = medium + 8.0 + small + 2.0 + small;
            let top = cym - dy / 2.0;
            draw_centre(UIFont::Medium, cxm, top, &text("UI_GameLoad_UnexpectedError1"), 0.8, 0.1, 0.1, 1.0);
            draw_centre(UIFont::Small, cxm, top + medium + 8.0, &text("UI_GameLoad_UnexpectedError2"), 1.0, 1.0, 1.0, 1.0);
            let console = cx.launch.cache_dir.join("console.txt");
            draw_centre(UIFont::Small, cxm, top + medium + 8.0 + small + 2.0, &console.to_string_lossy(), 1.0, 1.0, 1.0, 1.0);
            pz_ui::UIManager::render(&mut cx.lua.thread);
            flush_ui(out);
            return;
        }
        if self.world_version_error {
            // TODO(port): `WorldConverter.convertingVersion` picks the text.
            draw_centre(UIFont::Small, cxm, (sh - 100) as f64, &text("UI_CorruptedWorldVersion"), 0.8, 0.1, 0.1, 1.0);
        } else if self.converting_world {
            draw_centre(UIFont::Small, cxm, (sh - 100) as f64, &text("UI_ConvertWorld"), 0.5, 0.5, 0.5, 1.0);
            if self.converting_file_max != -1 {
                let y = (sh - 100) as f64 + pz_ui::draw::font_height(Some(UIFont::Small)) as f64 + 8.0;
                let s = format!("{} / {}", self.converting_file_count, self.converting_file_max);
                draw_centre(UIFont::Small, cxm, y, &s, 0.5, 0.5, 0.5, 1.0);
            }
        }
        let last_stand = pz_lua_api::bind_core::shared_core().borrow().is_last_stand();
        if !self.player_created && self.new_game && !last_stand {
            let loading = text("UI_Loading").replace('.', "");
            let y = (sh as f32 - BOTTOM_SCREEN - font_small - 5.0) as f64;
            draw_centre(UIFont::NewSmall, cxm, y, &loading, 0.5, 0.5, 0.5, 1.0);
            let x = sw / 2 + pz_ui::draw::measure_string_x(Some(UIFont::Small), &loading) / 2 + 1;
            draw_left(UIFont::NewSmall, x as f64, y, self.loading_dot, 0.5, 0.5, 0.5, 1.0);
        }
        self.do_quick_tips(cx);
        let intro_y = cym - (pz_ui::draw::font_height(Some(UIFont::Intro)) / 2) as f64;
        for (stage, key, a) in [(0, "UI_Intro1", alpha1), (1, "UI_Intro2", alpha2), (2, "UI_Intro3", alpha3)] {
            if self.stage == stage {
                draw_centre(UIFont::Intro, (sw / 2) as f64, intro_y.floor(), &text(key), 1.0, 1.0, 1.0, a);
            }
        }
        if pz_lua_api::bind_core::shared_core().borrow().get_debug() {
            self.showed_click_to_skip = true;
        }
        let mode = pz_lua_api::bind_core::shared_core().borrow().game_mode.clone();
        if self.done && self.player_created && (!self.new_game || self.time >= TOTAL_TIME || last_stand || mode == "Tutorial") {
            let step = cx.multiplier / 30.0;
            if self.click_to_skip_fade_in {
                self.click_to_skip_alpha += step;
                if self.click_to_skip_alpha > 1.0 {
                    self.click_to_skip_alpha = 1.0;
                    self.click_to_skip_fade_in = false;
                }
            } else {
                self.showed_click_to_skip = true;
                self.click_to_skip_alpha -= step;
                if self.click_to_skip_alpha < 0.25 {
                    self.click_to_skip_fade_in = true;
                }
            }
            // TODO(port): the joypad's "press A" prompt
            // (`GameWindow.activatedJoyPad`).
            let y = (sh as f32 - BOTTOM_SCREEN - font_small - 5.0) as f64;
            draw_centre(UIFont::NewLarge, cxm.floor(), y, &text("UI_ClickToSkip"), 1.0, 1.0, 1.0, self.click_to_skip_alpha);
        }
        // TODO(port): `ActiveMods.renderUI()`.
        flush_ui(out);
    }

    fn update(&mut self, cx: &mut StateCx<'_>) -> StateAction {
        // TODO(port): `OutfitManager.isLoadingClothingItems()` (loads are
        // synchronous here); `OutfitManager.debugOutfits()` in debug.
        if self.wait_for_asset_loading_to_finish1 {
            self.wait_for_asset_loading_to_finish1 = false;
            if self.loader == Loader::WaitAssets1 {
                let r = self.run_inner_1(cx);
                if self.wait_for_asset_loading_to_finish2 && r.is_ok() {
                    self.loader = Loader::WaitAssets2;
                }
                self.loader_result(r);
            }
        }
        // TODO(port): `ModelManager.isLoadingAnimations()`,
        // `GameWindow.fileSystem.hasWork()`; then every
        // `RuntimeAnimationScript.exec()`.
        if self.wait_for_asset_loading_to_finish2 {
            self.wait_for_asset_loading_to_finish2 = false;
            if self.loader == Loader::WaitAssets2 {
                self.run_inner_2(cx);
                self.loader_result(Ok(()));
            }
        }
        if self.unexpected_error || self.player_wrong_ip {
            if !self.showed_ui {
                self.showed_ui = true;
                pz_lua_api::bind_players::set_player_instance(None);
                pz_lua_api::bind_players::set_player(0, None);
                pz_ui::with_ui(|m| m.ui.clear());
                // TODO(port): `LuaEventManager.Reset()`.
                let f = cx.lua.thread.globals().get_str("ISGameLoadingUI_OnGameLoadingUI");
                if let Err((e, tr)) = cx.lua.thread.pcall(&f, &["".into()]) {
                    eprintln!("ISGameLoadingUI_OnGameLoadingUI: {e:?}\n{tr}");
                }
                pz_ui::with_ui(|m| m.suspend = false);
            }
            // TODO(port): Escape quits the process.
            return StateAction::Remain;
        }
        if !self.done {
            return StateAction::Remain;
        }
        if cx.world.as_ref().and_then(|w| w.streamer.as_ref()).is_some_and(|s| s.is_busy()) {
            return StateAction::Remain;
        }
        if !self.showed_click_to_skip {
            return StateAction::Remain;
        }
        if pz_ui::with_ui(|m| m.mouse.is_button_down(0)) || cx.launch.start.click_to_skip() {
            self.force_done = true;
        }
        // TODO(port): `GameWindow.activatedJoyPad.isAPressed()`.
        if self.force_done {
            pz_lua_api::bind_audio::sound_system().borrow_mut().with(|m, ctx| m.play_ui_sound("UIClickToStart", None, ctx));
            self.force_done = false;
            return StateAction::Continue;
        }
        StateAction::Remain
    }
}
