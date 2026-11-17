use super::super::tests::{run, setup_with};
use super::super::*;

/// A script manager with the traits and moodles the sleep reads.
fn sleep_script_manager() -> ScriptManager {
    let sm = ScriptManager::new();
    {
        let mut st = sm.statics.items.borrow_mut();
        for (reg, field, id) in [
            ("character_trait", "INSOMNIAC", "t:insomniac"),
            ("character_trait", "NIGHT_OWL", "t:nightowl"),
            ("character_trait", "DESENSITIZED", "t:desensitized"),
            ("moodle_type", "PAIN", "t:Pain"),
            ("moodle_type", "STRESS", "t:Stress"),
        ] {
            sm.registries.get(reg).unwrap().borrow_mut().register_id(id, false).unwrap();
            st.constants.insert(reg, field, id);
        }
    }
    sm
}

#[test]
fn falling_asleep_from_lua_then_sleeping_on_in_game_time() {
    let (mut thread, player, _) = setup_with(sleep_script_manager());
    crate::bind_players::set_player(0, Some(player.clone()));
    crate::bind_players::set_num_players(1);
    // Two hours: too short for a nightmare roll.
    let r = run(&mut thread, r#"
        player:setBedType("floor")
        player:setForceWakeUpTime(6)
        player:setAsleep(true)
        getSleepingEvent():setPlayerFallAsleep(player, 2.6)
        assert(IsoPlayer.allPlayersAsleep())
        save(true)
    "#);
    let e = r.err().unwrap_or_default();
    let intruders = run(&mut thread, "getSleepingEvent():setPlayerFallAsleep(player, 2, true, false)");
    super::update_asleep_player(&mut thread, &player);
    crate::bind_players::set_player(0, None);
    assert_eq!(e, "");
    {
        let p = player.borrow();
        let data = p.sleeping_event_data.clone().unwrap();
        assert_eq!((data.sleeping_time, data.nightmare_wake_up), (2.0, -1));
        // timeOfDay + Rand(0, 0.3 * 1.6).
        assert!(p.delay_to_actually_sleep >= p.time_of_sleep && p.delay_to_actually_sleep <= p.time_of_sleep + 0.48);
        // SleepingEvent.update with no zombie close and no nightmare.
        assert!(p.is_asleep());
    }
    let e = intruders.unwrap_err();
    assert!(e.contains("the zombie intruders"), "{e}");
}
