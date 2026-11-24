//! Build 42.22 `LuaManager$GlobalObject` singleplayer-faithful sync, access,
//! game-speed/pause, hutch lookup, sound-bank, and debug-primitive globals:
//! - Container/character/item sync helpers (`sendAddItemToContainer`,
//!   `sendRemoveItemFromContainer`, `sendAddItemsToContainer`,
//!   `sendRemoveItemsFromContainer`, `sendReplaceItemInContainer`,
//!   `sendItemStats`, `syncVisuals`, `sendHumanVisual`, `sendEquip`,
//!   `syncItemActivated`, `sendPlayerStatsChange`, `sendSyncPlayerFields`,
//!   `syncClothingFields`, `sendDamage`, `sendClothing`, `sendSwitchSeat`,
//!   `SendCommandToServer`, `emulateAnimEvent`, `emulateAnimEventOnce`)
//! - Game-state/access/player queries (`isGamePaused`, `getGameSpeed`,
//!   `isAdmin`, `getAccessLevel`, `checkPermissions`, `getPlayerByOnlineID`,
//!   `getPlayerFromUsername`, `getHutch`, `showDebugInfoInChat`)
//! - Sound bank (`getFMODSoundBank`, `zombie.audio.BaseSoundBank`)
//! - Debug world primitives (`renderLine`, `renderIsoCircle`, `renderIsoRect`)

use crate::bind_character_sim::character_value;
use crate::bind_players::{num_players, player};
use crate::bind_world_object::object_value;
use crate::expose::{expose_class, global, int_arg, num_arg, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const BASE_SOUND_BANK_CLASS: &str = "zombie.audio.BaseSoundBank";

#[derive(Clone, Debug, PartialEq)]
pub struct DebugRenderLine {
    pub x1: f32,
    pub y1: f32,
    pub z1: f32,
    pub x2: f32,
    pub y2: f32,
    pub z2: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DebugRenderIsoCircle {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
    pub segments: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DebugRenderIsoRect {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub thickness: i32,
}

#[derive(Default)]
struct SoundBankState {
    voices: HashMap<String, (String, f32)>,
    footsteps: HashMap<String, [String; 4]>,
}

struct LuaSoundBank;

impl LuaObject for LuaSoundBank {
    fn class_name(&self) -> &str {
        BASE_SOUND_BANK_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static SOUND_BANK: RefCell<SoundBankState> = RefCell::new(SoundBankState::default());
    static DEBUG_LINES: RefCell<Vec<DebugRenderLine>> = const { RefCell::new(Vec::new()) };
    static DEBUG_CIRCLES: RefCell<Vec<DebugRenderIsoCircle>> = const { RefCell::new(Vec::new()) };
    static DEBUG_RECTS: RefCell<Vec<DebugRenderIsoRect>> = const { RefCell::new(Vec::new()) };
}

/// Drain queued debug world lines submitted via `renderLine(...)`.
pub fn take_debug_render_lines() -> Vec<DebugRenderLine> {
    DEBUG_LINES.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// Drain queued debug world iso circles submitted via `renderIsoCircle(...)`.
pub fn take_debug_render_circles() -> Vec<DebugRenderIsoCircle> {
    DEBUG_CIRCLES.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Drain queued debug world iso rects submitted via `renderIsoRect(...)`.
pub fn take_debug_render_rects() -> Vec<DebugRenderIsoRect> {
    DEBUG_RECTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Clear per-session debug primitive queues.
pub(crate) fn reset_session_handles() {
    DEBUG_LINES.with(|l| l.borrow_mut().clear());
    DEBUG_CIRCLES.with(|c| c.borrow_mut().clear());
    DEBUG_RECTS.with(|r| r.borrow_mut().clear());
}

pub fn register(thread: &mut Thread) {
    // Sound bank (`zombie.audio.BaseSoundBank` / `getFMODSoundBank()`).
    let sb = expose_class(thread, BASE_SOUND_BANK_CLASS, Some("java.lang.Object"));
    sb.method("addVoice", |c| {
        let alias = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let sound = str_arg(c, 2).map(|s| s.to_string()).unwrap_or_default();
        let priority = num_arg(c, 3) as f32;
        if !alias.is_empty() {
            SOUND_BANK.with(|b| b.borrow_mut().voices.insert(alias, (sound, priority)));
        }
        Ok(0)
    });
    sb.method("addFootstep", |c| {
        let alias = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let grass = str_arg(c, 2).map(|s| s.to_string()).unwrap_or_default();
        let wood = str_arg(c, 3).map(|s| s.to_string()).unwrap_or_default();
        let concrete = str_arg(c, 4).map(|s| s.to_string()).unwrap_or_default();
        let upstairs = str_arg(c, 5).map(|s| s.to_string()).unwrap_or_default();
        if !alias.is_empty() {
            SOUND_BANK.with(|b| {
                b.borrow_mut()
                    .footsteps
                    .insert(alias, [grass, wood, concrete, upstairs])
            });
        }
        Ok(0)
    });

    global(thread, "getFMODSoundBank", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaSoundBank))))
    });

    // Game-speed, pause, admin/access, and player/hutch lookup globals.
    global(thread, "isGamePaused", |c| {
        Ok(c.push(crate::bind_climate::is_game_paused()))
    });
    global(thread, "getGameSpeed", |c| {
        let speed = pz_ui::with_ui(|ui| ui.speed_controls.clone())
            .and_then(|element| {
                element
                    .ext::<crate::hud::speed_controls::SpeedControls>()
                    .map(|s| s.get_current_game_speed())
            })
            .unwrap_or_else(|| crate::bind_climate::climate_settings().game_speed);
        Ok(c.push(speed))
    });
    global(thread, "isAdmin", |c| Ok(c.push(false)));
    global(thread, "getAccessLevel", |c| Ok(c.push("None")));
    global(thread, "checkPermissions", |c| {
        let is_client_fn = c.thread.globals().get_str("isClient");
        let is_client = if is_client_fn.is_nil() {
            false
        } else {
            matches!(c.thread.call1(&is_client_fn, &[])?, Value::Bool(true))
        };
        Ok(c.push(!is_client))
    });
    global(thread, "showDebugInfoInChat", |c| {
        if let Some(msg) = str_arg(c, 0) {
            pz_core::debug::log(&msg);
        }
        Ok(0)
    });
    global(thread, "getPlayerByOnlineID", |c| {
        let id = int_arg(c, 0);
        let n = num_players().clamp(0, 4) as usize;
        for i in 0..n {
            if let Some(p) = player(i) {
                let matches = {
                    let ch = p.borrow();
                    ch.id == id || (i as i32) == id
                };
                if matches {
                    return Ok(c.push(character_value(&p)));
                }
            }
        }
        Ok(c.push(Value::Nil))
    });
    global(thread, "getPlayerFromUsername", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let n = num_players().clamp(0, 4) as usize;
        for i in 0..n {
            if let Some(p) = player(i) {
                let matches = {
                    let ch = p.borrow();
                    ch.chat.username.as_deref() == Some(name.as_ref())
                };
                if matches {
                    return Ok(c.push(character_value(&p)));
                }
            }
        }
        Ok(c.push(Value::Nil))
    });
    global(thread, "getHutch", |c| {
        let x = int_arg(c, 0);
        let y = int_arg(c, 1);
        let z = int_arg(c, 2);
        let found = crate::bind_world::with_world(|w| {
            w.hutches
                .iter()
                .find(|h| {
                    h.try_borrow().ok().is_some_and(|o| {
                        o.square_slot().is_some_and(|sq| {
                            let sq = sq.borrow();
                            sq.x == x && sq.y == y && sq.z == z
                        })
                    })
                })
                .cloned()
        });
        let val = match found {
            Some(obj) => object_value(&obj)?,
            None => Value::Nil,
        };
        Ok(c.push(val))
    });

    // Singleplayer-faithful sync helpers: in Build 42.22 Java these return
    // immediately when `!GameClient.bClient && !GameServer.bServer`.
    for name in [
        "sendAddItemToContainer",
        "sendRemoveItemFromContainer",
        "sendAddItemsToContainer",
        "sendRemoveItemsFromContainer",
        "sendReplaceItemInContainer",
        "sendItemStats",
        "syncVisuals",
        "sendHumanVisual",
        "sendEquip",
        "syncItemActivated",
        "sendPlayerStatsChange",
        "sendSyncPlayerFields",
        "syncClothingFields",
        "sendDamage",
        "sendClothing",
        "sendSwitchSeat",
        "SendCommandToServer",
        "emulateAnimEvent",
        "emulateAnimEventOnce",
    ] {
        global(thread, name, |_c| Ok(0));
    }

    // Debug world primitives (`renderLine` / `renderIsoCircle` / `renderIsoRect`).
    global(thread, "renderLine", |c| {
        let line = DebugRenderLine {
            x1: num_arg(c, 0) as f32,
            y1: num_arg(c, 1) as f32,
            z1: num_arg(c, 2) as f32,
            x2: num_arg(c, 3) as f32,
            y2: num_arg(c, 4) as f32,
            z2: num_arg(c, 5) as f32,
            r: num_arg(c, 6) as f32,
            g: num_arg(c, 7) as f32,
            b: num_arg(c, 8) as f32,
            a: if c.nargs() >= 10 {
                num_arg(c, 9) as f32
            } else {
                1.0
            },
        };
        DEBUG_LINES.with(|l| l.borrow_mut().push(line));
        Ok(0)
    });
    global(thread, "renderIsoCircle", |c| {
        let circle = DebugRenderIsoCircle {
            x: num_arg(c, 0) as f32,
            y: num_arg(c, 1) as f32,
            z: num_arg(c, 2) as f32,
            radius: num_arg(c, 3) as f32,
            segments: int_arg(c, 4).max(3),
            r: num_arg(c, 5) as f32,
            g: num_arg(c, 6) as f32,
            b: num_arg(c, 7) as f32,
            a: if c.nargs() >= 9 {
                num_arg(c, 8) as f32
            } else {
                1.0
            },
        };
        DEBUG_CIRCLES.with(|cl| cl.borrow_mut().push(circle));
        Ok(0)
    });
    global(thread, "renderIsoRect", |c| {
        let rect = DebugRenderIsoRect {
            x: num_arg(c, 0) as f32,
            y: num_arg(c, 1) as f32,
            z: num_arg(c, 2) as f32,
            w: num_arg(c, 3) as f32,
            h: num_arg(c, 4) as f32,
            r: num_arg(c, 5) as f32,
            g: num_arg(c, 6) as f32,
            b: num_arg(c, 7) as f32,
            a: if c.nargs() >= 9 {
                num_arg(c, 8) as f32
            } else {
                1.0
            },
            thickness: if c.nargs() >= 10 {
                int_arg(c, 9).max(1)
            } else {
                1
            },
        };
        DEBUG_RECTS.with(|rc| rc.borrow_mut().push(rect));
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn global_sp_sync_and_debug_primitives_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            assert(isGamePaused() == false)
            assert(getGameSpeed() == 1)
            assert(isAdmin() == false)
            assert(getAccessLevel() == "None")
            assert(checkPermissions(nil, nil) == true)
            assert(getPlayerByOnlineID(999) == nil)
            assert(getPlayerFromUsername("missing") == nil)
            assert(getHutch(0, 0, 0) == nil)
            showDebugInfoInChat("debug smoke")

            local sb = getFMODSoundBank()
            assert(sb ~= nil)
            sb:addVoice("testVoice", "event:/voice", 1.0)
            sb:addFootstep("testStep", "grass", "wood", "concrete", "upstairs")

            sendAddItemToContainer(nil, nil)
            sendRemoveItemFromContainer(nil, nil)
            sendAddItemsToContainer(nil, nil)
            sendRemoveItemsFromContainer(nil, nil)
            sendReplaceItemInContainer(nil, nil, nil)
            sendItemStats(nil)
            syncVisuals(nil)
            sendHumanVisual(nil)
            sendEquip(nil)
            syncItemActivated(nil, nil)
            sendPlayerStatsChange(nil)
            sendSyncPlayerFields(nil, 0)
            syncClothingFields(nil)
            sendDamage(nil)
            sendClothing(nil, nil, nil)
            sendSwitchSeat(nil, nil, 0, 1)
            SendCommandToServer("/help")
            emulateAnimEvent(nil, 100, "Event", "param")
            emulateAnimEventOnce(nil, 100, "Event", "param")

            renderLine(1, 2, 0, 3, 4, 0, 1, 0.5, 0.25, 0.8)
            renderIsoCircle(5, 6, 0, 2.5, 16, 0, 1, 0, 0.9)
            renderIsoRect(10, 11, 0, 2, 3, 1, 1, 0, 0.7, 2)
        "#;
        let f = stdlib::load_string(src, Some("global_sp_sync"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();

        let lines = take_debug_render_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].x1, 1.0);
        assert_eq!(lines[0].a, 0.8);
        let circles = take_debug_render_circles();
        assert_eq!(circles.len(), 1);
        assert_eq!(circles[0].radius, 2.5);
        assert_eq!(circles[0].segments, 16);
        let rects = take_debug_render_rects();
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].w, 2.0);
        assert_eq!(rects[0].thickness, 2);
    }
}
