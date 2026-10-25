//! `IsoPlayer`'s local-player statics (`players`, `numPlayers`,
//! `getInstance`) over the bound characters, and the Lua globals that read
//! them (`getPlayer`, `getSpecificPlayer`, `getNumActivePlayers`).
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
}
