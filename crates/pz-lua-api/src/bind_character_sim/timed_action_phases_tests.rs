//! Authored callback-mutation fixtures over the real resumable player loop.
use super::tests::{run, setup_with};
use super::*;
use pz_characters::character::player::{
    PlayerEvent, PlayerInput, PlayerUpdate, PlayerUpdatePhase, PlayerUpdateStep,
};
use pz_characters::character::state::{ActionGroups, NullWorld, StateCx, StateRegistry};
use pz_core::game_time::GameTime;

struct Keys {
    aim: bool,
    forward: bool,
}
impl PlayerInput for Keys {
    fn is_key_down(&self, key: &str) -> bool {
        (key == "Aim" && self.aim) || (key == "Forward" && self.forward)
    }
    fn which_key_down(&self, key: &str) -> i32 {
        if self.is_key_down(key) {
            if key == "Aim" {
                29
            } else {
                17
            }
        } else {
            0
        }
    }
    fn screen_to_iso(&self, _: i32, _: i32, _: i32, _: f32) -> (f32, f32) {
        (20.0, 0.0)
    }
    fn current_time_ms(&self) -> i64 {
        1000
    }
}

struct Clock;
impl SimContext for Clock {
    fn do_character_stats(&self) -> bool {
        false
    }
    fn multiplier(&self) -> f32 {
        crate::bind_climate::shared_game_time()
            .borrow()
            .get_multiplier(&crate::bind_climate::LuaClimateContext::new())
    }
    fn realworld_seconds_since_last_update(&self) -> f32 {
        crate::bind_climate::shared_game_time()
            .borrow()
            .realworld_seconds_since_last_update()
    }
}

struct Fixture {
    thread: Thread,
    chr: LuaGameCharacterRef,
    registry: StateRegistry<LuaItem>,
    groups: ActionGroups,
    out: PlayerUpdate,
    old_time: Rc<RefCell<GameTime>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        crate::bind_climate::install_game_time(self.old_time.clone());
    }
}
impl Fixture {
    fn new() -> Self {
        let (mut thread, chr, _) = setup_with(ScriptManager::new());
        let sm = shared_script_manager();
        for field in ["OBESE", "OVERWEIGHT", "VERY_UNDERWEIGHT", "EMACIATED"] {
            let id = format!("authored:{field}");
            sm.registries
                .get("character_trait")
                .unwrap()
                .borrow_mut()
                .register_id(&id, false)
                .unwrap();
            sm.statics
                .items
                .borrow_mut()
                .constants
                .insert("character_trait", field, &id);
        }
        chr.borrow_mut().character_traits = pz_characters::CharacterTraits::new(&sm);
        for field in ["ENDURANCE", "FATIGUE"] {
            sm.builtin_tables.borrow_mut().character_stats.push(
                pz_scripts::registry::builtin::CharacterStatDecl {
                    field: field.into(),
                    id: format!("authored_{field}"),
                    minimum: 0.0,
                    maximum: 1.0,
                    default: if field == "ENDURANCE" { 1.0 } else { 0.0 },
                },
            );
        }
        shared_characters().borrow_mut().sim =
            pz_characters::SimStatics::from_tables(&sm.builtin_tables.borrow()).unwrap();
        crate::timed_actions::register(&mut thread);
        crate::bind_states::register(&mut thread, &sm);
        crate::bind_climate::register_with(&mut thread, &[]);
        let old_time = crate::bind_climate::shared_game_time();
        crate::bind_climate::install_game_time(Rc::new(RefCell::new(GameTime::default())));
        chr.borrow_mut().player.closest_zombie = 1.0;
        chr.borrow_mut().player.ticks_since_seen_zombie = 0;
        run(
            &mut thread,
            r#"
            stops=0
            T={}; T.__index=T
            function T:isValid() return true end
            function T:waitToStart() return false end
            function T:start() end
            function T:stop() stops=stops+1 end
            a=setmetatable({character=player,maxTime=100,stopOnAim=true},T)
            a.action=LuaTimedActionNew.new(a,player)
            player:StartAction(a.action)
        "#,
        )
        .unwrap();
        Self {
            thread,
            chr,
            registry: StateRegistry::with_ported(),
            groups: ActionGroups::default(),
            out: PlayerUpdate::default(),
            old_time,
        }
    }
    fn advance(&mut self, phase: PlayerUpdatePhase, input: &dyn PlayerInput) -> PlayerUpdateStep {
        let sm = shared_script_manager();
        let chars = shared_characters();
        let chars = chars.borrow();
        let mut world = NullWorld;
        let mut cx = StateCx::new(&sm, &chars, &Clock, &mut world);
        self.chr
            .borrow_mut()
            .advance_player_update(
                phase,
                input,
                &mut self.registry,
                &mut self.groups,
                &mut self.out,
                &mut cx,
            )
            .unwrap()
    }
    fn cancel(&mut self, step: PlayerUpdateStep, expected: &str) -> PlayerUpdatePhase {
        let PlayerUpdateStep::StopActionQueue { reason, resume } = step else {
            panic!("expected inline cancellation: {step:?}");
        };
        assert_eq!(reason, expected);
        // The callback reenters these exact owners after advance returned.
        assert!(self.chr.try_borrow_mut().is_ok());
        crate::timed_actions::cancel(&mut self.thread, &self.chr, reason);
        resume
    }
}

#[test]
fn timed_action_phases_aim_callback_changes_farming_and_blocks_later_movement() {
    let mut f = Fixture::new();
    run(
        &mut f.thread,
        r#"
        player:setIsFarming(true)
        function a:stop()
            stops=stops+1
            assert(player:getCharacterActions():contains(self.action))
            player:setIsFarming(false)
            player:setVariable('blockMovement',true)
        end
    "#,
    )
    .unwrap();
    let input = Keys {
        aim: true,
        forward: true,
    };
    let first = f.advance(PlayerUpdatePhase::Start, &input);
    assert_eq!(f.chr.borrow().player.ticks_since_seen_zombie, 0);
    assert!(f
        .out
        .events
        .contains(&PlayerEvent::LuaEvent("OnPlayerUpdate")));
    assert!(!f
        .out
        .events
        .iter()
        .any(|e| matches!(e, PlayerEvent::StopActionQueue(_))));
    let open_door = f.chr.borrow().player.time_since_open_door;
    let resume = f.cancel(first, "Aiming");
    let last = f.advance(resume, &input);
    assert!(matches!(last, PlayerUpdateStep::Done)); // Authored fixture has no world square.
    let chr = f.chr.borrow();
    assert!(!chr.player.is_farming && chr.vars.block_movement && chr.vars.is_aiming);
    assert!(
        !chr.player.just_moved,
        "movement ran before Lua changed blockMovement"
    );
    assert_eq!(chr.player.ticks_since_seen_zombie, 1);
    assert_eq!(
        chr.player.time_since_open_door, open_door,
        "input prefix replayed"
    );
    drop(chr);
    run(
        &mut f.thread,
        "assert(stops==1 and player:getCharacterActions():isEmpty())",
    )
    .unwrap();
}

#[test]
fn timed_action_phases_walk_callback_changes_clock_before_same_frame_slowdown() {
    let mut f = Fixture::new();
    f.chr.borrow_mut().player.slow_timer = 2.0;
    f.chr.borrow_mut().player.slow_factor = 0.4;
    run(
        &mut f.thread,
        "function a:stop() stops=stops+1; getGameTime():setMultiplier(3) end",
    )
    .unwrap();
    let input = Keys {
        aim: false,
        forward: true,
    };
    let first = f.advance(PlayerUpdatePhase::Start, &input);
    assert_eq!(
        f.chr.borrow().player.slow_factor,
        0.4,
        "tail ran before cancellation"
    );
    let open_door = f.chr.borrow().player.time_since_open_door;
    let resume = f.cancel(first, "Walking");
    let multiplier = Clock.multiplier();
    assert!(matches!(f.advance(resume, &input), PlayerUpdateStep::Done));
    let chr = f.chr.borrow();
    assert_eq!(chr.player.slow_factor, 0.4 - multiplier / 100.0);
    assert_eq!(chr.player.time_since_open_door, open_door);
    assert!((chr.environment.current_speed - 0.5 * 0.6).abs() < 1e-6);
    drop(chr);
    run(&mut f.thread, "assert(stops==1)").unwrap();
}

#[test]
fn timed_action_phases_two_yields_preserve_input_locals_without_replaying_prefix() {
    let mut f = Fixture::new();
    run(
        &mut f.thread,
        "player:setIsFarming(true); a.action:getTable().stopOnAim=false",
    )
    .unwrap();
    // Constructor snapshots the option; use a fresh action with that option.
    run(
        &mut f.thread,
        "a.action=LuaTimedActionNew.new(a,player); player:StartAction(a.action)",
    )
    .unwrap();
    let input = Keys {
        aim: true,
        forward: true,
    };
    let first = f.advance(PlayerUpdatePhase::Start, &input);
    let open_door = f.chr.borrow().player.time_since_open_door;
    let phase = f.cancel(first, "Aiming");
    run(&mut f.thread, "assert(stops==0)").unwrap();
    let second = f.advance(phase, &input);
    assert_eq!(f.chr.borrow().player.ticks_since_seen_zombie, 1);
    let phase = f.cancel(second, "Walking");
    assert!(matches!(f.advance(phase, &input), PlayerUpdateStep::Done));
    assert_eq!(f.chr.borrow().player.time_since_open_door, open_door);
    assert_eq!(f.chr.borrow().player.ticks_since_seen_zombie, 1);
    run(&mut f.thread, "assert(stops==1)").unwrap();
}

#[test]
fn timed_action_phases_farming_throw_timer_does_not_turn_to_aim_vector() {
    let mut f = Fixture::new();
    f.chr.borrow_mut().player.attack_anim_throw_timer = 2000;
    f.chr.borrow_mut().vars.forward_x = 0.0;
    f.chr.borrow_mut().vars.forward_y = 1.0;
    run(&mut f.thread, "player:setIsFarming(true); a.stopOnAim=false; a.action=LuaTimedActionNew.new(a,player); player:StartAction(a.action)").unwrap();
    let input = Keys {
        aim: true,
        forward: false,
    };
    let first = f.advance(PlayerUpdatePhase::Start, &input);
    let phase = f.cancel(first, "Aiming");
    assert!(matches!(f.advance(phase, &input), PlayerUpdateStep::Done));
    assert_eq!(
        (f.chr.borrow().vars.forward_x, f.chr.borrow().vars.forward_y),
        (0.0, 1.0)
    );
}
