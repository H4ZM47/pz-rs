//! Authored fixtures exercise the binding against the player's control
//! loop, animation-variable callback and ordinary player save record.

use super::tests::{run, setup_with};
use super::*;
use pz_characters::character::player::PlayerInput;
use pz_characters::character::save::SaveEnv;
use pz_characters::character::state::{ActionGroups, NullWorld, StateCx, StateRegistry};
use pz_inventory::io::{Reader, Writer};
use pz_scripts::registry::builtin::CharacterStatDecl;
use pz_world::save::SaveTables;

fn setup() -> (Thread, LuaGameCharacterRef) {
    let sm = ScriptManager::new();
    // The synthetic player must be saveable and have the traits queried
    // by its normal input/update and load paths.
    for (reg, field) in [
        ("character_profession", "UNEMPLOYED"),
        ("character_trait", "FAST_HEALER"),
        ("character_trait", "SLOW_HEALER"),
        ("character_trait", "OBESE"),
        ("character_trait", "OVERWEIGHT"),
        ("character_trait", "VERY_UNDERWEIGHT"),
        ("character_trait", "EMACIATED"),
    ] {
        let id = format!("test:{}", field.to_lowercase());
        sm.registries
            .get(reg)
            .unwrap()
            .borrow_mut()
            .register_id(&id, false)
            .unwrap();
        sm.statics
            .items
            .borrow_mut()
            .constants
            .insert(reg, field, &id);
    }
    let (mut thread, player, _) = setup_with(sm);
    let sm = shared_script_manager();
    for field in ["ENDURANCE", "FATIGUE", "PAIN"] {
        sm.builtin_tables
            .borrow_mut()
            .character_stats
            .push(CharacterStatDecl {
                field: field.into(),
                id: field.to_lowercase(),
                minimum: 0.0,
                maximum: 1.0,
                default: if field == "ENDURANCE" { 1.0 } else { 0.0 },
            });
    }
    let chars = shared_characters();
    chars.borrow_mut().sim =
        pz_characters::SimStatics::from_tables(&sm.builtin_tables.borrow()).unwrap();
    let group = chars
        .borrow_mut()
        .body_locations
        .get_group("Human")
        .unwrap();
    player.borrow_mut().init_worn_items(&sm, group);
    crate::bind_states::register(&mut thread, &sm);
    (thread, player)
}

struct CrouchInput {
    down: bool,
    now_ms: i64,
}

impl PlayerInput for CrouchInput {
    fn which_key_down(&self, binding: &str) -> i32 {
        if self.down && binding == "Crouch" {
            46
        } else {
            0
        }
    }

    fn current_time_ms(&self) -> i64 {
        self.now_ms
    }
}

#[test]
fn sneaking_lua_reads_live_player_crouch_input_and_variable_callback() {
    let (mut thread, player) = setup();
    let sm = shared_script_manager();
    let chars = shared_characters();
    let mut world = NullWorld;
    let mut states = StateRegistry::with_ported();
    let mut groups = ActionGroups::default();
    run(&mut thread, "assert(not player:isSneaking())").unwrap();
    for (down, now_ms, expected) in [
        (true, 100, true),
        (true, 200, true),
        (false, 300, true),
        (true, 400, false),
    ] {
        let input = CrouchInput { down, now_ms };
        {
            let chars = chars.borrow();
            let mut cx = StateCx::new(&sm, &chars, &DefaultSimContext, &mut world);
            let mut player = player.borrow_mut();
            player.preupdate_player();
            player.frame_step(&input);
            player
                .update_player(&input, &mut states, &mut groups, &mut cx)
                .unwrap();
        }
        assert_eq!(player.borrow().is_sneaking(), expected);
        thread.globals().set_str("expected", expected);
        run(
            &mut thread,
            r#"
            assert(player:isSneaking() == expected)
            assert(player:getVariableBoolean("sneaking") == expected)
        "#,
        )
        .unwrap();
    }
}

#[test]
fn sneaking_lua_setter_and_variable_share_the_saved_player_field() {
    struct NoItems;
    impl pz_inventory::ItemEnv for NoItems {}

    let (mut thread, player) = setup();
    let sm = shared_script_manager();
    let chars = shared_characters();
    let tables = SaveTables {
        factory: vec![
            (0, "IsoObject".into()),
            (1, "Player".into()),
            (2, "Zombie".into()),
        ],
        blood_body_parts: 18,
        character_stats: Vec::new(),
    };
    let env = SaveEnv {
        items: &NoItems,
        tables: &tables,
    };
    for expected in [true, false] {
        thread.globals().set_str("expected", expected);
        run(
            &mut thread,
            r#"
            player:setSneaking(expected)
            assert(player:isSneaking() == expected)
            assert(player:getVariableBoolean("sneaking") == expected)
            player:setVariable("sneaking", not expected)
            assert(player:isSneaking() == not expected)
            player:setSneaking(expected)
        "#,
        )
        .unwrap();
        assert_eq!(player.borrow().environment.sneaking, expected);

        let mut writer = Writer::new();
        player
            .borrow_mut()
            .save_player(&sm, &chars.borrow(), &DefaultSimContext, &env, &mut writer)
            .unwrap();
        let mut loaded = LuaGameCharacter::new(&sm, CharacterKind::Player);
        let mut reader = Reader::new(&writer.buf);
        loaded
            .load_player(
                &sm,
                &mut chars.borrow_mut(),
                &DefaultSimContext,
                &env,
                &mut reader,
                249,
                &mut |_| panic!("not on fire"),
            )
            .unwrap();
        assert_eq!(reader.remaining(), 0);
        let loaded = Rc::new(RefCell::new(loaded));
        assert!(!Rc::ptr_eq(&player, &loaded));
        thread.globals().set_str("loaded", character_value(&loaded));
        run(
            &mut thread,
            r#"
            assert(loaded:isSneaking() == expected)
            assert(loaded:getVariableBoolean("sneaking") == expected)
        "#,
        )
        .unwrap();
    }
}
