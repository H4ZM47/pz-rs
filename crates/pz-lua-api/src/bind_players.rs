//! `IsoPlayer`'s local-player statics (`players`, `numPlayers`,
//! `getInstance`) over the bound characters, and the Lua globals that read
//! them (`getPlayer`, `getSpecificPlayer`, `getNumActivePlayers`) and the
//! player-screen geometry used by Lua inventory and HUD layouts.
//!
//! The engine fills the slots as the original's `IsoPlayer` constructor
//! and `GameLoadingState` do ([`set_player`], [`set_player_instance`],
//! [`set_num_players`]); the HUD reads them through
//! [`crate::hud::characters`].

use crate::bind_character_sim::{character_value, LuaGameCharacterRef};
use crate::expose::{global, int_arg};
use pz_kahlua::{LuaError, Thread, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// `IsoPlayer.MAX`.
pub const MAX_PLAYERS: usize = 4;

struct IsoPlayerStatics {
    /// `IsoPlayer.players`.
    players: [Option<LuaGameCharacterRef>; MAX_PLAYERS],
    /// `IsoPlayer.numPlayers` (1 from the static initializer).
    num_players: i32,
    /// `IsoPlayer.instance`.
    instance: Option<LuaGameCharacterRef>,
    /// `IsoPlayer.assumedPlayer`.
    assumed_player: i32,
}

impl Default for IsoPlayerStatics {
    fn default() -> Self {
        IsoPlayerStatics { players: Default::default(), num_players: 1, instance: None, assumed_player: 0 }
    }
}

thread_local! {
    static STATICS: RefCell<IsoPlayerStatics> = RefCell::default();
}

fn in_slots(s: &IsoPlayerStatics, chr: &LuaGameCharacterRef) -> bool {
    s.players.iter().flatten().any(|p| Rc::ptr_eq(p, chr))
}

/// `IsoPlayer.players[i] = chr`. Keeps each character's
/// [`in_players`](pz_characters::character::GameCharacter::in_players)
/// (`isLocalPlayer`) in step with the slots. Panics for `i >= 4`, as the
/// original's array store throws.
pub fn set_player(i: usize, chr: Option<LuaGameCharacterRef>) {
    STATICS.with(|s| {
        let mut s = s.borrow_mut();
        let old = std::mem::replace(&mut s.players[i], chr.clone());
        if let Some(old) = old {
            if !in_slots(&s, &old) {
                old.borrow_mut().in_players = false;
            }
        }
        if let Some(c) = &chr {
            c.borrow_mut().in_players = true;
        }
    });
    if let Some(c) = &chr {
        crate::chat::characters::attach_chat_element(c);
    }
}

/// `IsoPlayer.players[i]`; `None` for an empty slot or `i >= 4`.
pub fn player(i: usize) -> Option<LuaGameCharacterRef> {
    STATICS.with(|s| s.borrow().players.get(i).cloned().flatten())
}

/// `IsoPlayer.numPlayers = n`.
pub fn set_num_players(n: i32) {
    STATICS.with(|s| s.borrow_mut().num_players = n);
}

/// `IsoPlayer.numPlayers`.
pub fn num_players() -> i32 {
    STATICS.with(|s| s.borrow().num_players)
}

/// `IsoPlayer.setInstance(chr)`; keeps
/// [`player_instance`](pz_characters::character::GameCharacter::player_instance)
/// in step.
pub fn set_player_instance(chr: Option<LuaGameCharacterRef>) {
    let old = STATICS.with(|s| std::mem::replace(&mut s.borrow_mut().instance, chr.clone()));
    if let Some(old) = old {
        old.borrow_mut().player_instance = false;
    }
    if let Some(c) = chr {
        c.borrow_mut().player_instance = true;
    }
}

/// `IsoPlayer.getInstance()`.
pub fn player_instance() -> Option<LuaGameCharacterRef> {
    STATICS.with(|s| s.borrow().instance.clone())
}

/// `IsoPlayer.assumedPlayer = i`.
pub fn set_assumed_player(i: i32) {
    STATICS.with(|s| s.borrow_mut().assumed_player = i);
}

/// `IsoPlayer.getPlayerIndex()`: the instance's `playerIndex`, else
/// `assumedPlayer`.
pub fn player_index() -> i32 {
    STATICS.with(|s| {
        let s = s.borrow();
        s.instance.as_ref().map_or(s.assumed_player, |p| p.borrow().player.player_index)
    })
}

/// `IsoPlayer.allPlayersDead()`: no living player among the first
/// `numPlayers` slots (`IsoWorld.addCoopPlayers` is not ported, so always
/// empty).
pub fn all_players_dead() -> bool {
    let n = num_players().clamp(0, MAX_PLAYERS as i32) as usize;
    !(0..n).any(|i| player(i).is_some_and(|p| !p.borrow().is_dead()))
}

/// Clears every slot, the instance and `numPlayers` (back to 1), e.g. when
/// leaving a game.
pub fn reset() {
    for i in 0..MAX_PLAYERS {
        set_player(i, None);
    }
    set_player_instance(None);
    set_num_players(1);
}

fn player_value(p: Option<LuaGameCharacterRef>) -> Value {
    p.map(|p| character_value(&p)).unwrap_or_default()
}

/// `IsoCamera.getScreenLeft/Top/Width/Height(player)`. The offsets depend
/// on the requested slot, while dimensions depend on the live player
/// count. The original does not validate the slot or require an occupant.
fn player_screen_rect(player: i32) -> [i32; 4] {
    let core = crate::bind_core::shared_core();
    let core = core.borrow();
    let (width, height) = (core.get_screen_width(), core.get_screen_height());
    let count = num_players();
    [
        if player == 1 || player == 3 { width / 2 } else { 0 },
        if player == 2 || player == 3 { height / 2 } else { 0 },
        if count > 1 { width / 2 } else { width },
        if count > 2 { height / 2 } else { height },
    ]
}

/// The `LuaManager.GlobalObject` player getters.
pub fn register(thread: &mut Thread) {
    global(thread, "getPlayer", |c| Ok(c.push(player_value(player_instance()))));
    global(thread, "getSpecificPlayer", |c| {
        let i = int_arg(c, 0);
        if !(0..MAX_PLAYERS as i32).contains(&i) {
            return Err(LuaError::Runtime {
                message: format!("Index {i} out of bounds for length {MAX_PLAYERS}"),
                class: "java.lang.ArrayIndexOutOfBoundsException",
            });
        }
        Ok(c.push(player_value(player(i as usize))))
    });
    global(thread, "getNumActivePlayers", |c| Ok(c.push(num_players())));
    for (component, name) in [
        "getPlayerScreenLeft",
        "getPlayerScreenTop",
        "getPlayerScreenWidth",
        "getPlayerScreenHeight",
    ].into_iter().enumerate() {
        global(thread, name, move |c| {
            let rect = player_screen_rect(int_arg(c, 0));
            Ok(c.push(rect[component]))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_core::core::Core;
    use pz_kahlua::TableRef;

    fn read_screen(thread: &mut Thread, player: i32) -> Vec<i32> {
        let source = format!(
            "return getPlayerScreenLeft({player}), getPlayerScreenTop({player}), \
             getPlayerScreenWidth({player}), getPlayerScreenHeight({player})"
        );
        let code = pz_kahlua::compiler::compile(&source, Some("screen"), Some("screen"), None).unwrap();
        let closure = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(code), thread.globals().clone()));
        thread.pcall(&closure, &[])
            .unwrap_or_else(|(error, trace)| panic!("{error:?}\n{trace}"))
            .into_iter().map(|value| value.as_num().expect("screen coordinate must be numeric") as i32).collect()
    }

    #[test]
    fn player_screen_globals_follow_live_layout_without_requiring_player_slots() {
        reset();
        let core = Rc::new(RefCell::new(Core::new()));
        core.borrow_mut().set_screen_size(1281, 721);
        crate::bind_core::install_core(core.clone());
        let mut thread = Thread::new(TableRef::new());
        register(&mut thread);
        for (count, width, height) in [(1, 1281, 721), (2, 640, 721), (3, 640, 360), (4, 640, 360)] {
            set_num_players(count);
            for (slot, left, top) in [(0, 0, 0), (1, 640, 0), (2, 0, 360), (3, 640, 360), (-1, 0, 0), (9, 0, 0)] {
                assert_eq!(read_screen(&mut thread, slot), [left, top, width, height]);
            }
        }
        core.borrow_mut().set_screen_size(1920, 1080);
        assert_eq!(read_screen(&mut thread, 3), [960, 540, 960, 540]);
        reset();
        assert_eq!(read_screen(&mut thread, 0), [0, 0, 1920, 1080]);
        // Registered globals read the current Core rather than capturing
        // the previous session's instance.
        let next = Rc::new(RefCell::new(Core::new()));
        next.borrow_mut().set_screen_size(800, 600);
        crate::bind_core::install_core(next);
        assert_eq!(read_screen(&mut thread, 0), [0, 0, 800, 600]);
    }
}
