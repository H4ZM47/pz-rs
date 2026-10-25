//! Opt-in proof using the installed Lua queue, real player and normal frames.

use super::{lua_check, run_frames};
use crate::game_window::App;
use pz_anim::AnimationVariableSource;
use pz_lua_api::bind_players;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug)]
struct IdleSnapshot {
    position: (f32, f32, f32),
    action: String,
    animation: Option<String>,
    player_moving: bool,
    pressed_movement: bool,
    is_moving: bool,
    root: (f32, f32),
    weighted_motion: bool,
}

fn idle_snapshot(app: &mut App) -> IdleSnapshot {
    let player = bind_players::player(0).expect("idle check has a real local player");
    let player = player.borrow();
    let mut snapshot = IdleSnapshot {
        position: (player.vars.x, player.vars.y, player.vars.z),
        action: player.state.action_context.current_state_name().to_string(),
        animation: None,
        player_moving: player.player_moving,
        pressed_movement: player.get_variable_boolean("pressedMovement"),
        is_moving: player.get_variable_boolean("isMoving"),
        root: (f32::NAN, f32::NAN),
        weighted_motion: true,
    };
    let id = player.id;
    drop(player);
    if let Some((animation, animator)) = app
        .world
        .as_mut()
        .and_then(|world| world.models.as_mut())
        .and_then(|models| models.animation_mut(id))
    {
        let root = animation.deferred_movement(false);
        snapshot.root = (root.x, root.y);
        snapshot.animation = animator.current_state_name().map(str::to_string);
        snapshot.weighted_motion = animation.tracks().iter().any(|track| {
            track.blend_weight() > 0.001
                && track.use_deferred_movement()
                && track.deferred_movement_diff().length_squared() > 0.0
        });
    }
    snapshot
}

/// Releasing input does not instantly remove the previous walk clip's
/// deferred movement. Advance the real simulation until that movement has
/// settled, before the parent captures the position used by queue and save.
pub(super) fn wait_for_idle(app: &mut App) {
    assert!(
        !pz_lua_api::bind_input::is_key_down_raw(17),
        "W is still held"
    );
    lua_check(
        app.lua.as_mut().unwrap(),
        r#"
        assert(UIManager.getSpeedControls():getCurrentGameSpeed() == 1, "idle check is paused")
        if SurvivalGuide and SurvivalGuide.instance then
            assert(not SurvivalGuide.instance:isVisible(), "survival guide is still open")
        end
    "#,
    );
    let mut previous = idle_snapshot(app).position;
    let mut stationary_frames = 0;
    for frame in 1..=120 {
        assert!(
            !pz_lua_api::bind_climate::is_game_paused(),
            "simulation paused while settling"
        );
        run_frames(app, 1);
        assert!(
            app.error.is_none(),
            "simulation failed while settling: {:?}",
            app.error
        );
        let snapshot = idle_snapshot(app);
        let idle = snapshot.action.eq_ignore_ascii_case("idle")
            && snapshot
                .animation
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("idle"))
            && !snapshot.player_moving
            && !snapshot.pressed_movement
            && !snapshot.is_moving
            && snapshot.root == (0.0, 0.0)
            && !snapshot.weighted_motion;
        stationary_frames = if idle && snapshot.position == previous {
            stationary_frames + 1
        } else {
            0
        };
        if frame <= 3 || stationary_frames == 3 || frame == 120 {
            eprintln!("session-check idle settling frame {frame}: {snapshot:?}, stationary={stationary_frames}");
        }
        if stationary_frames == 3 {
            return;
        }
        previous = snapshot.position;
    }
    panic!("player never reached real idle with zero deferred motion within 120 ordinary frames");
}

pub(super) fn check(app: &mut App) {
    let player = bind_players::player(0).expect("queue check has a real local player");
    let position = {
        let player = player.borrow();
        (player.vars.x, player.vars.y, player.vars.z)
    };
    let messages: Rc<RefCell<Vec<String>>> = Rc::default();
    let previous = Rc::new(RefCell::new(std::mem::replace(
        &mut app.lua.as_mut().unwrap().thread.log,
        Box::new(|_| {}),
    )));
    let prior = previous.clone();
    let captured = messages.clone();
    app.lua.as_mut().unwrap().thread.log = Box::new(move |message| {
        captured.borrow_mut().push(message.to_string());
        prior.borrow_mut()(message);
    });
    lua_check(
        app.lua.as_mut().unwrap(),
        r#"
        require "TimedActions/ISBaseTimedAction"
        require "TimedActions/ISTimedActionQueue"
        local player = getSpecificPlayer(0)
        assert(player and player:isLocalPlayer() and not player:isDraggingCorpse())
        assert(player:getCharacterActions():isEmpty(), "player already has a native action")
        local queue = ISTimedActionQueue.getTimedActionQueue(player)
        assert(#queue.queue == 0 and queue.current == nil, "player already has a Lua action")
        RustSessionQueueProbe = ISBaseTimedAction:derive("RustSessionQueueProbe")
        rustSessionQueueEvents = {}
        rustSessionQueueUpdates = {0, 0}
        function RustSessionQueueProbe:isValid() return true end
        function RustSessionQueueProbe:start()
            rustSessionQueueEvents[#rustSessionQueueEvents+1] = self.probeID .. ':start'
        end
        function RustSessionQueueProbe:update()
            assert(self.character:getCharacterActions():contains(self.action))
            rustSessionQueueUpdates[self.probeID] = rustSessionQueueUpdates[self.probeID] + 1
        end
        -- The installed JAR calls complete unconditionally, while this
        -- installed ISBaseTimedAction has no complete method. An authored
        -- subclass callback is valid and leaves its real perform inherited.
        function RustSessionQueueProbe:complete()
            rustSessionQueueEvents[#rustSessionQueueEvents+1] = self.probeID .. ':complete'
        end
        local function probe(id, duration)
            local action = ISBaseTimedAction.new(RustSessionQueueProbe, player)
            action.probeID = id
            action.maxTime = duration
            return action
        end
        rustSessionQueueFirst, rustSessionQueueSecond = probe(1, 3), probe(2, 4)
        assert(ISTimedActionQueue.add(rustSessionQueueFirst) == queue)
        assert(ISTimedActionQueue.add(rustSessionQueueSecond) == queue)
        assert(#queue.queue == 2 and queue.current == rustSessionQueueFirst)
        assert(rustSessionQueueSecond.action == nil, "pending action prematurely entered native stack")
        assert(player:getCharacterActions():size() == 1)
        assert(player:getCharacterActions():get(0) == rustSessionQueueFirst.action)
    "#,
    );
    for _ in 0..120 {
        run_frames(app, 1);
        if player.borrow().character_actions.borrow().is_empty() {
            break;
        }
    }
    lua_check(
        app.lua.as_mut().unwrap(),
        r#"
        local queue = ISTimedActionQueue.getTimedActionQueue(getSpecificPlayer(0))
        assert(#queue.queue == 0 and queue.current == nil, "installed Lua queue did not finish")
        assert(getSpecificPlayer(0):getCharacterActions():isEmpty(), "native stack did not finish")
        assert(rustSessionQueueUpdates[1] > 0 and rustSessionQueueUpdates[2] > 0)
        local expected = {'1:start', '2:start', '1:complete', '2:complete'}
        assert(#rustSessionQueueEvents == #expected, "wrong callback count")
        for i=1,#expected do assert(rustSessionQueueEvents[i] == expected[i], "wrong callback order") end
        -- The base perform method advanced the actual Lua queue. It was
        -- neither overridden nor manually invoked by this regression.
        assert(RustSessionQueueProbe.perform == ISBaseTimedAction.perform)
    "#,
    );
    let thread = &mut app.lua.as_mut().unwrap().thread;
    thread.log = Box::new(|_| {});
    thread.log = Rc::try_unwrap(previous)
        .ok()
        .expect("queue logger released old owner")
        .into_inner();
    let action_errors: Vec<_> = messages
        .borrow()
        .iter()
        .filter(|message| {
            message.contains("ISBaseTimedAction")
                || message.contains("ISTimedActionQueue")
                || message.contains("RustSessionQueueProbe")
                || message.contains("rust-session-regression")
        })
        .cloned()
        .collect();
    assert!(
        action_errors.is_empty(),
        "installed queue callback errors: {action_errors:?}"
    );
    let player = player.borrow();
    assert_eq!(
        (player.vars.x, player.vars.y, player.vars.z),
        position,
        "idle actions moved player"
    );
    eprintln!("session-check installed queue: two inherited actions admitted, advanced and completed without movement");
}
