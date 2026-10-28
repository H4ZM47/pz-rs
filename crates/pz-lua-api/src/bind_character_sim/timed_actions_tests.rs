use super::tests::{run, setup_with};
use super::*;

fn setup() -> (Thread, LuaGameCharacterRef) {
    let (mut thread, chr, _) = setup_with(ScriptManager::new());
    crate::timed_actions::register(&mut thread);
    crate::bind_states::register(&mut thread, &shared_script_manager());
    run(
        &mut thread,
        r#"
        events = {}
        function note(s) events[#events+1] = s end
        T = {}; T.__index = T
        function T:isValid() return true end
        function T:waitToStart() return false end
        function T:start() note(self.name .. ':start') end
        function T:update()
            -- These re-enter the real character/stack during a callback.
            assert(self.character:getCharacterActions():contains(self.action))
            assert(self.character:getInventory() ~= nil)
            note(self.name .. ':update')
        end
        function T:perform() note(self.name .. ':perform') end
        function T:complete() note(self.name .. ':complete') end
        function T:stop() note(self.name .. ':stop') end
        function T:interruptWaitToStart() note(self.name .. ':interrupt') end
        Derived = {}; Derived.__index = Derived; setmetatable(Derived, T)
        function make(name, duration)
            local t = setmetatable({name=name, maxTime=duration, character=player}, Derived)
            t.action = LuaTimedActionNew.new(t, player)
            return t
        end
    "#,
    )
    .unwrap();
    (thread, chr)
}

#[test]
fn timed_actions_inherited_callbacks_modify_real_deltas_and_receive_animation_events() {
    let (mut thread, chr) = setup();
    run(
        &mut thread,
        r#"
        function T:getDeltaModifiers(d)
            assert(player:getCharacterActions():contains(self.action))
            if savedDelta then assert(savedDelta == d) end
            savedDelta = d
            d:setTurnDelta(2.5)
            d:setMaxMoveDelta(0.75)
            d:setTwistDelta(1.25)
        end
        function T:animEvent(name, parameter)
            assert(player:getInventory() ~= nil)
            note(name .. ':' .. parameter)
        end
        a = make('derived', 20)
        player:StartAction(a.action)
    "#,
    )
    .unwrap();
    for _ in 0..2 {
        let deltas = crate::timed_actions::movement_modifiers(
            &mut thread,
            &chr,
            pz_characters::character::state::MoveDeltaModifiers::default(),
        );
        assert_eq!(
            (deltas.turn_delta, deltas.move_delta, deltas.twist_delta),
            (2.5, 0.75, 1.25)
        );
    }
    crate::timed_actions::anim_event(&mut thread, &chr, "AuthoredEvent", Some("value"));
    run(&mut thread, "assert(events[2] == 'AuthoredEvent:value')").unwrap();
}

#[test]
fn timed_actions_keep_lua_pending_queue_separate_and_advance_during_perform() {
    let (mut thread, chr) = setup();
    run(
        &mut thread,
        r#"
        pending = {}
        function schedule(t)
            pending[#pending+1] = t
            if #pending == 1 then player:StartAction(t.action) end
        end
        function T:perform()
            note(self.name .. ':perform')
            assert(pending[1] == self)
            table.remove(pending, 1)
            if pending[1] then player:StartAction(pending[1].action) end
        end
        first = make('queuedA', 1)
        second = make('queuedB', 1)
        schedule(first); schedule(second)
        assert(player:getCharacterActions():size() == 1 and #pending == 2)
        assert(not second.action:isStarted())
    "#,
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut thread, &chr, 1.0);
    run(&mut thread,"assert(#pending == 1 and pending[1] == second) assert(player:getCharacterActions():get(0) == second.action)").unwrap();
    crate::timed_actions::update_with_multiplier(&mut thread, &chr, 1.0);
    run(
        &mut thread,
        "assert(#pending == 0 and player:getCharacterActions():isEmpty())",
    )
    .unwrap();
}

#[test]
fn timed_actions_preserve_nan_time_and_exact_integer_duration() {
    let (mut thread, chr) = setup();
    run(
        &mut thread,
        r#"
        a = make('numeric', 1)
        a.action:setTime(16777217)
        assert(a.action:getTime() == 16777217)
        player:StartAction(a.action)
        a.action:setCurrentTime(0/0)
        assert(a.action:getCurrentTime() ~= a.action:getCurrentTime())
    "#,
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut thread, &chr, 1.0);
    run(
        &mut thread,
        r#"
        assert(a.action:getCurrentTime() ~= a.action:getCurrentTime())
        assert(a.action:getJobDelta() ~= a.action:getJobDelta())
        assert(not a.action:finished())
        a.action:setTime(-1)
        a.action:setCurrentTime(-2)
        assert(a.action:getCurrentTime() == 0)
        a.action:setCurrentTime(2)
        assert(a.action:getCurrentTime() == -1)
    "#,
    )
    .unwrap();
}

#[test]
fn timed_actions_validity_and_wait_callbacks_preserve_distinct_protected_call_rules() {
    let (mut thread, chr) = setup();
    run(
        &mut thread,
        r#"
        a = make('booleanError', 5)
        function a:isValid() error(true) end
        player:StartAction(a.action)
        assert(a.action:isStarted())
        b = make('waitError', 5)
        function b:waitToStart() error(false) end
        player:StartAction(b.action)
        assert(not b.action:isStarted())
    "#,
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut thread, &chr, 1.0);
    run(
        &mut thread,
        "assert(not b.action:isStarted() and events[#events] == 'booleanError:start')",
    )
    .unwrap();
}

#[test]
fn timed_actions_use_typed_action_enums_and_preserve_enum_identity_across_lua_reset() {
    let (mut thread, _) = setup();
    let class = "zombie.characters.CharacterActionAnims";
    crate::expose::expose_enum(&mut thread, class, &["AuthoredTask"]);
    crate::expose::expose_enum(&mut thread, "test.OtherChoice", &["AuthoredTask"]);
    run(
        &mut thread,
        r#"
        a = make('enum', 10)
        a.action:setActionAnim(CharacterActionAnims.AuthoredTask)
        assert(player:getVariableString('PerformingAction') == 'AuthoredTask')
        assert(not pcall(a.action.setActionAnim, a.action, OtherChoice.AuthoredTask))
        assert(not pcall(a.action.setActionAnim, a.action, {}))
    "#,
    )
    .unwrap();
    let old = crate::expose::enum_value(class, "AuthoredTask");
    let mut fresh = Thread::new(pz_kahlua::TableRef::new());
    crate::expose::expose_enum(&mut fresh, class, &["AuthoredTask"]);
    let new = crate::expose::enum_value(class, "AuthoredTask");
    assert!(
        old.raw_equals(&new),
        "Lua reset replaced a static enum constant"
    );
}

#[test]
fn timed_actions_read_live_clock_after_validity_callback() {
    struct RestoreClock(Rc<RefCell<pz_core::game_time::GameTime>>);
    impl Drop for RestoreClock {
        fn drop(&mut self) {
            crate::bind_climate::install_game_time(self.0.clone());
        }
    }
    let (mut thread, chr) = setup();
    let _restore = RestoreClock(crate::bind_climate::shared_game_time());
    let clock = Rc::new(RefCell::new(pz_core::game_time::GameTime::default()));
    crate::bind_climate::install_game_time(clock.clone());
    let callback_clock = clock.clone();
    thread.globals().set_str(
        "changeClock",
        Value::Native(pz_kahlua::NativeFunction::new("changeClock", move |_| {
            callback_clock.borrow_mut().multiplier = 3.0;
            Ok(0)
        })),
    );
    run(
        &mut thread,
        r#"
        a=make('liveClock',100)
        function a:isValid() if mutateClock then changeClock() end return true end
        player:StartAction(a.action)
        mutateClock=true
    "#,
    )
    .unwrap();
    let before = clock
        .borrow()
        .get_multiplier(&crate::bind_climate::LuaClimateContext::new());
    crate::timed_actions::update(&mut thread, &chr);
    let after = clock
        .borrow()
        .get_multiplier(&crate::bind_climate::LuaClimateContext::new());
    assert_ne!(before, after, "fixture did not change the current clock");
    let action = chr.borrow().character_actions.borrow()[0].clone();
    assert_eq!(
        action.borrow().current_time,
        after,
        "action used the pre-callback frame snapshot"
    );
}

#[test]
fn timed_actions_player_state_voice_latches_before_action_callback() {
    struct Recorder(RefCell<Vec<String>>);
    impl pz_characters::character::SimContext for Recorder {
        fn event(&self, event: pz_characters::character::SimEvent) {
            self.0.borrow_mut().push(format!("{event:?}"));
        }
    }
    let (mut thread, chr) = setup();
    run(
        &mut thread,
        r#"
        a=make('voice',100)
        function a:animEvent(name, parameter)
            assert(player:getVariableBoolean('PlayerVoiceSound'))
            note(parameter)
        end
        player:StartAction(a.action)
    "#,
    )
    .unwrap();
    let sm = shared_script_manager();
    let chars = shared_characters();
    let mut world = pz_characters::character::state::NullWorld;
    let recorder = Recorder(RefCell::new(Vec::new()));
    let chars = chars.borrow();
    let mut cx = pz_characters::character::state::StateCx::new(&sm, &chars, &recorder, &mut world);
    let event = pz_anim::AnimEventRecord {
        state_name: None,
        layer_depth: -1,
        track: None,
        event: Rc::new(pz_anim::AnimEvent {
            event_name: "playervoicesound".into(),
            parameter_value: Some("AuthoredVoice".into()),
            ..Default::default()
        }),
    };
    let state = pz_characters::character::states::player::actions::<LuaItem>();
    state.anim_event(&mut chr.borrow_mut(), &mut cx, &event);
    crate::timed_actions::anim_event(&mut thread, &chr, "playervoicesound", Some("AuthoredVoice"));
    state.anim_event(&mut chr.borrow_mut(), &mut cx, &event);
    assert_eq!(recorder.0.borrow().len(), 1);
    state.exit(&mut chr.borrow_mut(), &mut cx);
    assert!(!chr.borrow().variable_boolean("PlayerVoiceSound"));
}

#[test]
fn timed_actions_remove_only_first_duplicate_identity_like_stack_remove_element() {
    let (mut thread, chr) = setup();
    run(&mut thread,"a=make('duplicate',1) player:StartAction(a.action) player:getCharacterActions():add(a.action)").unwrap();
    crate::timed_actions::update_with_multiplier(&mut thread, &chr, 1.0);
    run(&mut thread,"assert(player:getCharacterActions():size()==1 and player:getCharacterActions():get(0)==a.action)").unwrap();
}

#[test]
fn timed_actions_cancellation_callbacks_run_before_action_tick_and_character_resume() {
    struct NoStats;
    impl pz_characters::character::SimContext for NoStats {
        fn do_character_stats(&self) -> bool {
            false
        }
    }
    let (mut thread, chr) = setup();
    run(
        &mut thread,
        r#"
        a=make('cancelled',10)
        function a:stop()
            note('cancelled:stop')
            self.character:setIsFarming(false)
            self.character:setAsleep(true)
        end
        player:setIsFarming(true)
        assert(player:isFarming())
        player:StartAction(a.action)
    "#,
    )
    .unwrap();
    chr.borrow_mut().vars.last_x = 19.0;
    chr.borrow_mut().vars.x = 5.0;
    // This is the host order between the two player-update phases.
    crate::timed_actions::cancel(&mut thread, &chr, "Walking");
    crate::timed_actions::update_with_multiplier(&mut thread, &chr, 1.0);
    let sm = shared_script_manager();
    let chars = shared_characters();
    chars.borrow_mut().sim =
        pz_characters::SimStatics::from_tables(&sm.builtin_tables.borrow()).unwrap();
    let mut world = pz_characters::character::state::NullWorld;
    let chars = chars.borrow();
    let mut cx = pz_characters::character::state::StateCx::new(&sm, &chars, &NoStats, &mut world);
    chr.borrow_mut()
        .update_player_after_actions(&mut Default::default(), &mut cx)
        .unwrap();
    assert!(chr.borrow().asleep);
    assert_eq!(
        chr.borrow().vars.last_x,
        19.0,
        "resume ignored Lua's asleep change"
    );
    run(
        &mut thread,
        "assert(not player:isFarming()) assert(events[2]=='cancelled:stop' and #events==2)",
    )
    .unwrap();
}

#[test]
fn timed_actions_live_stack_identity_and_reentrant_perform_preserve_next_action() {
    let (mut t, chr) = setup();
    run(
        &mut t,
        r#"
        first = make('first', 1)
        next = make('next', 20)
        function first:perform()
            note('first:perform')
            player:StartAction(next.action)
        end
        list = player:getCharacterActions()
        player:StartAction(first.action)
        assert(list == player:getCharacterActions() and list:get(0) == first.action)
        assert(first.action:getTable() == first and first.action:isStarted())
    "#,
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut t, &chr, 1.0);
    run(
        &mut t,
        r#"
        assert(list:size() == 1 and list:get(0) == next.action)
        assert(not list:contains(first.action) and next.action:isStarted())
        assert(events[1] == 'first:start' and events[2] == 'first:update')
        assert(events[3] == 'first:perform' and events[4] == 'next:start')
        assert(events[5] == 'first:complete')
        list:clear()
        assert(player:getCharacterActions():isEmpty())
    "#,
    )
    .unwrap();
}

#[test]
fn timed_actions_wait_cancel_and_flags_use_real_character_owner() {
    let (mut t, chr) = setup();
    run(
        &mut t,
        r#"
        a = make('wait', 10)
        function a:waitToStart() return true end
        player:StartAction(a.action)
        assert(not a.action:isStarted())
        a.action:setBlockMovementEtc(true)
        a.action:setOverrideAnimation(true)
    "#,
    )
    .unwrap();
    assert!(chr.borrow().player.action_blocks_movement);
    assert!(chr.borrow().player.action_overrides_animation);
    assert_eq!(
        chr.borrow().environment.first_action_calories_modifier,
        Some(1.0)
    );
    crate::timed_actions::cancel(&mut t, &chr, "Aiming");
    assert_eq!(chr.borrow().character_actions.borrow().len(), 1);
    crate::timed_actions::cancel(&mut t, &chr, "Running");
    run(
        &mut t,
        "assert(events[1] == 'wait:interrupt' and player:getCharacterActions():isEmpty())",
    )
    .unwrap();
    assert!(!chr.borrow().player.action_blocks_movement);
    assert!(!chr.borrow().vars.has_timed_actions);
}

#[test]
fn timed_actions_multiplier_progress_infinite_force_completion_and_animation_cleanup() {
    let (mut t, chr) = setup();
    run(
        &mut t,
        r#"
        a = make('finite', 4)
        player:StartAction(a.action)
        a.action:setActionAnim('AuthoredWork')
        a.action:setAnimVariable('AuthoredFlag', true)
    "#,
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut t, &chr, 0.5);
    run(
        &mut t,
        "assert(a.action:getCurrentTime() == 0.5 and a.action:getJobDelta() == 0.125)",
    )
    .unwrap();
    run(
        &mut t,
        "a.action:setWaitForFinished(true) a.action:setCurrentTime(4)",
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut t, &chr, 0.0);
    assert_eq!(chr.borrow().character_actions.borrow().len(), 1);
    run(&mut t, "a.action:forceComplete()").unwrap();
    crate::timed_actions::update_with_multiplier(&mut t, &chr, 0.0);
    assert!(chr.borrow().character_actions.borrow().is_empty());
    assert!(!chr.borrow().variable_boolean("IsPerformingAnAction"));
    assert_eq!(chr.borrow().variable_string("AuthoredFlag"), "");
    run(
        &mut t,
        "b=make('infinite', -1) player:StartAction(b.action)",
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut t, &chr, 100.0);
    assert_eq!(chr.borrow().character_actions.borrow().len(), 1);
    run(
        &mut t,
        "assert(not b.action:finished()) b.action:forceStop()",
    )
    .unwrap();
    crate::timed_actions::update_with_multiplier(&mut t, &chr, 0.0);
    run(
        &mut t,
        "assert(player:getCharacterActions():isEmpty()) assert(events[#events] == 'infinite:stop')",
    )
    .unwrap();
}
