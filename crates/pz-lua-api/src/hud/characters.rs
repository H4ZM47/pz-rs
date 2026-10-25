//! The HUD inputs the characters thread owns ([`HudPlayers`],
//! [`HudClimate`], [`HudMoodles`], [`HudBodyDamage`]; [`HudGameTime`] is
//! [`SharedGameTime`]) over the bound characters: the local players in
//! [`crate::bind_players`], their [`Moodles`](pz_characters::character::Moodles)
//! and `BodyDamage`, and the shared `GameTime` / `ClimateManager`
//! ([`crate::bind_climate::shared_game_time`]).
//!
//! [`install_character_hud_hosts`] puts them in the [`super::host`] slots
//! (`register_all` does, after `hud::register`).

use super::host::{
    with_host, BodyPartView, HudBodyDamage, HudClimate, HudMoodles, HudPlayers, PlayerScreen, SharedGameTime,
};
use crate::bind_character_sim::{with_sim_context, CharacterObject, LuaGameCharacterRef};
use crate::bind_players;
use pz_scripts::objects::items::MoodleType;
use pz_kahlua::Value;
use pz_scripts::registry::RegistryKey;
use std::rc::Rc;

/// `IsoPlayer`'s statics from [`crate::bind_players`].
pub struct CharacterPlayers;

impl HudPlayers for CharacterPlayers {
    fn has_player_instance(&self) -> bool {
        bind_players::player_instance().is_some()
    }
    fn num_players(&self) -> i32 {
        bind_players::num_players()
    }
    fn player_exists(&self, i: usize) -> bool {
        bind_players::player(i).is_some()
    }
    fn all_players_dead(&self) -> bool {
        bind_players::all_players_dead()
    }
    fn player_screen(&self, i: usize) -> Option<PlayerScreen> {
        let p = bind_players::player(i)?;
        let p = p.borrow();
        Some(PlayerScreen {
            x: p.vars.x,
            y: p.vars.y,
            z: p.vars.z,
            // `IsoMovingObject.offsetX` / `offsetY`: 0 for characters (only
            // set for objects placed with a sprite offset).
            offset_x: 0.0,
            offset_y: 0.0,
            // `getUserNameHeight()`: 0 unless `GameClient.client` (the
            // name tag is a multiplayer `TextDrawObject`, not ported).
            user_name_height: 0,
        })
    }
    fn player_dead(&self, i: usize) -> bool {
        bind_players::player(i).is_some_and(|p| p.borrow().is_dead())
    }
}

/// `ClimateManager.getInstance()` (the shared `GameTime`'s) for a player.
pub struct CharacterClimate;

impl HudClimate for CharacterClimate {
    fn air_temperature_for_character(&self, player: usize) -> f32 {
        let chr = bind_players::player(player);
        with_sim_context(|ctx| match &chr {
            Some(p) => p.borrow().environment.air_temperature_for_character(ctx, false),
            // The original's `plr.square` is a NullPointerException here;
            // `Clock` only asks with a `clockPlayer`, so this is unreachable
            // from the HUD. `getTemperature()` keeps a stray call harmless.
            None => ctx.climate().map_or(0.0, |c| c.get_temperature()),
        })
    }
}

/// `players[player].getMoodles()`.
pub struct CharacterMoodles;

/// The `MoodleType` `moodle` names (its id without namespace), as
/// `MoodlesUI.wiggle` resolves it.
fn moodle_key(moodle: &str) -> Option<Rc<RegistryKey>> {
    let sm = crate::bind_registry::shared_script_manager();
    MoodleType::values(&sm)
        .into_iter()
        .find(|k| k.path().eq_ignore_ascii_case(moodle))
}

fn moodles_of<R>(player: usize, f: impl FnOnce(&LuaGameCharacterRef) -> R) -> Option<R> {
    bind_players::player(player).map(|p| f(&p))
}

impl HudMoodles for CharacterMoodles {
    fn ui_refresh_needed(&mut self, player: usize) -> bool {
        moodles_of(player, |p| p.borrow_mut().moodles.ui_refresh_needed()).unwrap_or(false)
    }
    fn set_moodles_state_changed(&mut self, player: usize, changed: bool) {
        moodles_of(player, |p| p.borrow_mut().moodles.set_moodles_state_changed(changed));
    }
    fn moodle_level(&self, player: usize, moodle: &str) -> i32 {
        let key = moodle_key(moodle);
        moodles_of(player, |p| p.borrow().moodles.moodle_level(key.as_deref()).unwrap_or(0)).unwrap_or(0)
    }
    fn good_bad_neutral(&self, player: usize, moodle: &str) -> i32 {
        let key = moodle_key(moodle);
        let sm = crate::bind_registry::shared_script_manager();
        moodles_of(player, |p| p.borrow().moodles.good_bad_neutral(&sm, key.as_deref())).unwrap_or(0)
    }
    fn display_string(&self, player: usize, moodle: &str) -> String {
        let key = moodle_key(moodle);
        moodles_of(player, |p| {
            with_sim_context(|ctx| {
                p.borrow()
                    .moodles
                    .moodle_display_string(key.as_deref(), &|k| ctx.get_text(k))
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
    }
    fn description_string(&self, player: usize, moodle: &str) -> String {
        let key = moodle_key(moodle);
        moodles_of(player, |p| {
            with_sim_context(|ctx| {
                p.borrow()
                    .moodles
                    .moodle_description_string(key.as_deref(), &|k| ctx.get_text(k))
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
    }
}

/// A character Lua object's `BodyDamage` (the health panel's
/// `chr.getBodyDamage()`).
///
/// TODO(network): a remote MP player's panel reads
/// `getBodyDamageRemote()`; offline every character is local.
pub struct CharacterBodyDamage;

fn character_of(chr: &Value) -> Option<LuaGameCharacterRef> {
    match chr {
        Value::Object(o) => o.as_any().downcast_ref::<CharacterObject>().map(|o| o.0.clone()),
        _ => None,
    }
}

impl HudBodyDamage for CharacterBodyDamage {
    fn is_female(&self, chr: &Value) -> bool {
        // `getCharacterGender()`: FEMALE without a descriptor.
        character_of(chr).is_none_or(|c| c.borrow().descriptor.as_ref().is_none_or(|d| d.borrow().is_female()))
    }
    fn health(&self, chr: &Value) -> f32 {
        character_of(chr).map_or(100.0, |c| c.borrow().body_damage.health())
    }
    fn body_part(&self, chr: &Value, index: usize) -> BodyPartView {
        let Some(c) = character_of(chr) else {
            return BodyPartView::default();
        };
        let c = c.borrow();
        let Some(p) = c.body_damage.body_part(index) else {
            return BodyPartView::default();
        };
        BodyPartView {
            bandaged: p.bandaged,
            bandage_life: p.bandage_life,
            wound_infection_level: p.wound_infection_level,
            scratched: p.scratched,
            scratch_time: p.scratch_time,
            cut_time: p.cut_time,
            bitten: p.bitten,
            bite_time: p.bite_time,
            deep_wounded: p.deep_wounded,
            deep_wound_time: p.deep_wound_time,
            stitched: p.stitched,
            fracture_time: p.fracture_time,
            splint_factor: p.splint_factor,
            have_glass: p.have_glass,
            have_bullet: p.have_bullet,
            burn_time: p.burn_time,
        }
    }
}

/// Installs [`CharacterPlayers`], [`SharedGameTime`], [`CharacterClimate`],
/// [`CharacterMoodles`] and [`CharacterBodyDamage`], keeping the other
/// threads' inputs.
pub fn install_character_hud_hosts() {
    with_host(|h| {
        h.players = Box::new(CharacterPlayers);
        h.game_time = Box::new(SharedGameTime);
        h.climate = Box::new(CharacterClimate);
        h.moodles = Box::new(CharacterMoodles);
        h.body_damage = Box::new(CharacterBodyDamage);
    });
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudGameTime, HudHost};
    use super::super::moodles::MoodlesUI;
    use super::super::progress_bar::ActionProgressBar;
    use super::super::test_util::*;
    use super::super::{init_ui, set_moodles_character};
    use super::*;
    use crate::bind_character_sim::LuaGameCharacter;
    use pz_characters::character::DefaultSimContext;
    use pz_characters::{CharacterKind, Characters};
    use pz_scripts::registry::builtin::{BuiltinTables, CharacterStatDecl, MoodleStatDecl};
    use pz_scripts::ScriptManager;
    use pz_ui::with_ui;
    use std::cell::RefCell;

    /// A shared `ScriptManager` with invented base moodle types (`Panic`
    /// with a `PANIC` stat and thresholds, `FoodEaten`), and
    /// the `Characters` statics to update them.
    fn setup() -> (Rc<ScriptManager>, Characters) {
        let sm = Rc::new(ScriptManager::new());
        {
            let mut st = sm.statics.items.borrow_mut();
            for (field, id) in [("PANIC", "Panic"), ("FOOD_EATEN", "FoodEaten")] {
                sm.registries.get("moodle_type").unwrap().borrow_mut().register_id(id, true).unwrap();
                st.constants.insert("moodle_type", field, id);
            }
        }
        let mut tables = BuiltinTables::default();
        for f in ["PANIC", "STRESS", "TEMPERATURE", "ZOMBIE_INFECTION"] {
            tables.character_stats.push(CharacterStatDecl {
                field: f.into(),
                id: f.to_lowercase(),
                minimum: 0.0,
                maximum: if f == "PANIC" { 100.0 } else { 1.0 },
                default: 0.0,
            });
        }
        tables.moodle_stats = vec![MoodleStatDecl {
            field: "PANIC".into(),
            moodle_type: "PANIC".into(),
            thresholds: [0.0, 5.0, 10.0, 20.0, 40.0],
        }];
        *sm.builtin_tables.borrow_mut() = tables.clone();
        let chars = Characters {
            sim: pz_characters::SimStatics::from_tables(&tables).unwrap(),
            ..Default::default()
        };
        crate::bind_registry::install_script_manager(sm.clone());
        (sm, chars)
    }

    fn new_player(sm: &ScriptManager, x: f32, y: f32) -> LuaGameCharacterRef {
        let mut c = LuaGameCharacter::new(sm, CharacterKind::Player);
        c.vars.x = x;
        c.vars.y = y;
        Rc::new(RefCell::new(c))
    }

    fn fresh() -> pz_kahlua::Thread {
        bind_players::reset();
        install(HudHost::default());
        install_character_hud_hosts();
        let mut t = thread();
        bind_players::register(&mut t);
        t
    }

    #[test]
    fn players_drive_init_layout_progress_bar_and_globals() {
        install_textures(&[("media/ui/ClockAssets/ClockSmallBackground.png", 81, 32)]);
        let (sm, _) = setup();
        let mut t = fresh();
        {
            let core = crate::bind_core::shared_core();
            core.borrow_mut().set_screen_size(1280, 720);
        }
        // No IsoPlayer instance (the main menu): nothing added.
        init_ui(&mut t);
        assert!(with_ui(|m| m.ui.is_empty()));
        assert_eq!(eval(&mut t, "getNumActivePlayers()").as_num(), Some(1.0));
        assert!(eval(&mut t, "getPlayer()").is_nil());

        let p0 = new_player(&sm, 10.0, 20.0);
        bind_players::set_player(0, Some(p0.clone()));
        bind_players::set_player_instance(Some(p0.clone()));
        assert!(p0.borrow().in_players && p0.borrow().player_instance);
        init_ui(&mut t);
        assert!(!with_ui(|m| m.ui.is_empty()));
        let mu = |i: usize| with_ui(|m| m.moodle_ui[i].clone()).unwrap();
        assert!(mu(0).is_visible() && !mu(1).is_visible());
        let sc = with_ui(|m| m.speed_controls.clone()).unwrap();
        assert!(sc.is_visible());
        assert!(eval(&mut t, "getPlayer() == getSpecificPlayer(0) and getSpecificPlayer(1) == nil").truthy());
        assert!(eval(&mut t, "not pcall(getSpecificPlayer, 4)").truthy());

        // The progress bar follows players[0] on screen.
        let bar = super::super::progress_bar(0).unwrap();
        let ext = bar.ext::<ActionProgressBar>().unwrap();
        ext.set_value(0.5);
        ext.update_bar(&bar, 0);
        let (w, h) = (bar.get_width() as f32, bar.get_height() as f32);
        let sx = pz_render::iso::x_to_screen(10.0, 20.0) - w / 2.0;
        let tile_scale = crate::bind_core::shared_core().borrow().tile_scale;
        let sy = pz_render::iso::y_to_screen(10.0, 20.0, 0.0, 0) - (128 / (2 / tile_scale)) as f32 - h;
        assert!(bar.is_visible());
        assert_eq!((bar.get_x() as f32, bar.get_y() as f32), (sx, sy));
        p0.borrow_mut().vars.x = 11.0;
        ext.update_bar(&bar, 0);
        assert_eq!(bar.get_x() as f32, pz_render::iso::x_to_screen(11.0, 20.0) - w / 2.0);

        // A second player: split-screen layout; dead players hide the
        // speed controls.
        let p1 = new_player(&sm, 0.0, 0.0);
        bind_players::set_player(1, Some(p1.clone()));
        bind_players::set_num_players(2);
        pz_ui::UIManager::resize();
        assert!(mu(1).is_visible());
        assert_eq!(eval(&mut t, "getNumActivePlayers()").as_num(), Some(2.0));
        p0.borrow_mut().health = 0.0;
        pz_ui::UIManager::resize();
        assert!(sc.is_visible());
        p1.borrow_mut().health = 0.0;
        pz_ui::UIManager::resize();
        assert!(!sc.is_visible());
        assert!(with_host(|h| h.players.player_dead(1) && !h.players.player_dead(2)));

        // Leaving a slot clears isLocalPlayer.
        bind_players::set_player(1, None);
        assert!(!p1.borrow().in_players);
        assert!(!with_host(|h| h.players.player_exists(1)));
    }

    /// Clock textures drawn in the lit colour (not the ghost digits).
    fn lit_textures() -> Vec<String> {
        with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    pz_ui::UiDrawCmd::Quad { texture: Some(t), color, .. } if (color.0 - 100.0 / 255.0).abs() < 1e-3 => {
                        Some(t.name.rsplit('/').next().unwrap().trim_end_matches(".png").to_string())
                    }
                    _ => None,
                })
                .collect()
        })
    }

    #[test]
    fn game_time_and_climate_drive_the_clock_and_speed_controls() {
        install_textures(&[]);
        let (sm, _) = setup();
        let mut t = fresh();
        let p0 = new_player(&sm, 0.0, 0.0);
        bind_players::set_player(0, Some(p0.clone()));
        bind_players::set_player_instance(Some(p0.clone()));
        {
            let core = crate::bind_core::shared_core();
            let mut c = core.borrow_mut();
            c.set_option_clock_size(1);
            c.set_option_clock24_hour(true);
            c.set_option_clock_format(1);
            c.set_option_display_as_celsius(true);
        }
        let gt = crate::bind_climate::shared_game_time();
        {
            let mut g = gt.borrow_mut();
            g.time_of_day = 14.5;
            g.day = 3;
            g.month = 6;
            g.fps_multiplier = 2.0;
            g.climate.vars.floats[pz_core::weather::manager::FLOAT_TEMPERATURE].final_value = -4.25;
        }
        assert_eq!(
            with_host(|h| (
                h.game_time.time_of_day(),
                h.game_time.day(),
                h.game_time.month(),
                h.game_time.realworld_seconds_since_last_update()
            )),
            (14.5, 3, 6, 0.016666668 * 2.0)
        );
        assert_eq!(with_host(|h| h.climate.air_temperature_for_character(0)), -4.25);
        // Indoors (no power, ground floor) the climate warms the air.
        {
            let mut p = p0.borrow_mut();
            p.environment.has_current_square = true;
            p.environment.square_in_room = true;
        }
        let q = pz_core::weather::AirTemperatureQuery {
            in_room: true,
            z: 0,
            vehicle_inside_temperature: None,
            hydro_power_on: false,
            heat_source_highest: None,
        };
        let indoor = gt.borrow().climate.get_air_temperature_for_square(Some(&q), false);
        assert!(indoor > -4.25);
        assert_eq!(with_host(|h| h.climate.air_temperature_for_character(0)), indoor);
        p0.borrow_mut().environment.has_current_square = false;

        // The clock reads them: 14:30, -4.2 C, 07/04 (clock format 1:
        // month first).
        init_ui(&mut t);
        let e = with_ui(|m| m.clock.clone()).unwrap();
        let clock = e.ext::<super::super::clock::Clock>().unwrap();
        clock.set_state(true, Some(0), false, false);
        e.set_visible(true);
        with_ui(|m| m.sprite.clear());
        e.render(&mut t);
        let got = lit_textures();
        // Time, temperature (-04.2 C), then the date in the draw order.
        assert_eq!(
            got,
            [
                "ClockDigitsMedium1",
                "ClockDigitsMedium4",
                "ClockDivideMedium",
                "ClockDigitsMedium3",
                "ClockDigitsMedium0",
                "ClockDigitsSmallMinus",
                "ClockDigitsSmall0",
                "ClockDigitsSmall4",
                "ClockDigitsSmallDot",
                "ClockDigitsSmall2",
                "ClockDigitsSmallDot",
                "ClockDigitsSmallC",
                "ClockDigitsSmall0",
                "ClockDigitsSmall7",
                "DateDivideSmall",
                "ClockDigitsSmall0",
                "ClockDigitsSmall4",
            ]
        );

        // SpeedControls set the shared multiplier and read it back.
        run(&mut t, "UIManager.getSpeedControls():ButtonClicked('Fast Forward x 2')");
        assert_eq!(gt.borrow().multiplier, 20.0);
        assert_eq!(with_host(|h| h.game_time.get_true_multiplier()), 20.0);
        assert_eq!(
            eval(&mut t, "UIManager.getSpeedControls():getCurrentGameSpeed()").as_num(),
            Some(3.0)
        );
        SharedGameTime.set_multiplier(1.0);
        assert_eq!(gt.borrow().multiplier, 1.0);
    }

    #[test]
    fn moodles_drive_moodles_ui() {
        install_textures(&[]);
        let (sm, chars) = setup();
        let mut t = fresh();
        let p0 = new_player(&sm, 0.0, 0.0);
        bind_players::set_player(0, Some(p0.clone()));
        bind_players::set_player_instance(Some(p0.clone()));
        init_ui(&mut t);
        let panic = chars.sim.character_stats.constant("PANIC").unwrap();
        {
            let mut p = p0.borrow_mut();
            p.stats.set(&panic, 15.0);
            p.update_moodles(&sm, &chars, &DefaultSimContext).unwrap();
        }
        assert_eq!(with_host(|h| h.moodles.moodle_level(0, "Panic")), 2);
        assert_eq!(with_host(|h| h.moodles.good_bad_neutral(0, "Panic")), 2);
        assert_eq!(with_host(|h| h.moodles.good_bad_neutral(0, "FoodEaten")), 1);
        assert_eq!(with_host(|h| h.moodles.display_string(0, "Panic")), "Moodles_Panic_lvl2");
        assert_eq!(with_host(|h| h.moodles.description_string(0, "Panic")), "Moodles_Panic_desc_lvl2");
        // No player in slot 1: the HUD's defaults.
        assert_eq!(with_host(|h| h.moodles.moodle_level(1, "Panic")), 0);

        // MoodlesUI(0) shows players[0]'s panic moodle and consumes the
        // refresh flag.
        set_moodles_character(0, Some(0));
        let ui = with_ui(|m| m.moodle_ui[0].clone()).unwrap();
        ui.update(&mut t);
        let d = ui.ext::<MoodlesUI>().unwrap().data("Panic").unwrap();
        assert_eq!((d.level, d.good_bad_neutral), (2, 2));
        let food = ui.ext::<MoodlesUI>().unwrap().data("FoodEaten").unwrap();
        assert_eq!(food.level, 0);
        assert!(!p0.borrow_mut().moodles.ui_refresh_needed());

        // A level change reaches the UI on the next update.
        {
            let mut p = p0.borrow_mut();
            p.stats.set(&panic, 30.0);
            p.update_moodles(&sm, &chars, &DefaultSimContext).unwrap();
        }
        ui.update(&mut t);
        assert_eq!(ui.ext::<MoodlesUI>().unwrap().data("Panic").unwrap().level, 3);
    }

    #[test]
    fn body_damage_drives_the_health_panel() {
        install_textures(&[]);
        let (sm, _) = setup();
        let mut t = fresh();
        let p0 = new_player(&sm, 0.0, 0.0);
        {
            let mut p = p0.borrow_mut();
            let decl = |f: &str| pz_scripts::registry::builtin::BodyPartDecl {
                field: f.into(),
                name: f.into(),
                ..Default::default()
            };
            p.body_damage.parts = vec![
                pz_characters::character::BodyPart::new(&decl("Hand_L"), 0),
                pz_characters::character::BodyPart::new(&decl("Hand_R"), 1),
            ];
            let hand = &mut p.body_damage.parts[1];
            hand.bandaged = true;
            hand.bandage_life = 2.5;
            hand.scratched = true;
            hand.scratch_time = 12.0;
            hand.fracture_time = 30.0;
            hand.splint_factor = 1.5;
            hand.have_glass = true;
            p.body_damage.overall_body_health = 55.0;
        }
        bind_players::set_player(0, Some(p0.clone()));
        bind_players::set_player_instance(Some(p0.clone()));
        let chr = crate::bind_character_sim::character_value(&p0);
        // No descriptor: FEMALE (`getCharacterGender`).
        assert!(with_host(|h| h.body_damage.is_female(&chr)));
        let part = with_host(|h| h.body_damage.body_part(&chr, 1));
        assert_eq!(
            part,
            BodyPartView {
                bandaged: true,
                bandage_life: 2.5,
                scratched: true,
                scratch_time: 12.0,
                fracture_time: 30.0,
                splint_factor: 1.5,
                have_glass: true,
                ..BodyPartView::default()
            }
        );
        assert_eq!(with_host(|h| h.body_damage.body_part(&chr, 5)), BodyPartView::default());
        // Not a character: the defaults.
        assert_eq!(with_host(|h| h.body_damage.health(&Value::Num(1.0))), 100.0);

        // ISNewHealthPanel's NewHealthPanel.new(x, y, getPlayer()).
        run(&mut t, "p = NewHealthPanel.new(0, 0, getPlayer())");
        assert_eq!(
            eval(&mut t, "p:getDamageStatusString()").as_str(),
            Some(crate::bind_text::translate("IGUI_health_Severe_damage").as_str())
        );
        p0.borrow_mut().body_damage.overall_body_health = 100.0;
        assert_eq!(
            eval(&mut t, "p:getDamageStatusString()").as_str(),
            Some(crate::bind_text::translate("IGUI_health_ok").as_str())
        );
    }
}
