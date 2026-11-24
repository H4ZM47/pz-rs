//! `zombie.iso.areas.NonPvpZone` and `zombie.characters.Faction`, synced with
//! `map_meta.bin` (`pz_world::meta_save::{NonPvpZone, Faction}`).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_core::core::ColorInfo;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, NativeCall, Thread, Value};
use pz_world::meta_save::{Faction, NonPvpZone};

use crate::bind_character_sim::CharacterObject;
use crate::bind_core::LuaColorInfo;
use crate::bind_java_util::JavaList;
use crate::expose::{bool_arg, expose_class, int_arg, str_arg, with_this};

pub const NON_PVP_ZONE_CLASS: &str = "zombie.iso.areas.NonPvpZone";
pub const FACTION_CLASS: &str = "zombie.characters.Faction";

/// Shared mutable handle to one `NonPvpZone`.
pub type NonPvpZoneRef = Rc<RefCell<NonPvpZone>>;
/// Shared mutable handle to one `Faction`.
pub type FactionRef = Rc<RefCell<Faction>>;

/// Lua wrapper for `zombie.iso.areas.NonPvpZone`.
pub struct LuaNonPvpZone(pub NonPvpZoneRef);

impl LuaObject for LuaNonPvpZone {
    fn class_name(&self) -> &str {
        NON_PVP_ZONE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Lua wrapper for `zombie.characters.Faction`.
pub struct LuaFaction(pub FactionRef);

impl LuaObject for LuaFaction {
    fn class_name(&self) -> &str {
        FACTION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static NON_PVP_ZONES: RefCell<Vec<NonPvpZoneRef>> = const { RefCell::new(Vec::new()) };
    static NON_PVP_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static FACTIONS: RefCell<Vec<FactionRef>> = const { RefCell::new(Vec::new()) };
    static FACTIONS_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Replace live `NonPvpZone` list from `map_meta.bin`.
pub fn install_non_pvp_zones(zones: Vec<NonPvpZone>) {
    NON_PVP_ZONES.with(|list| {
        let mut list = list.borrow_mut();
        list.clear();
        list.extend(zones.into_iter().map(|z| Rc::new(RefCell::new(z))));
    });
    NON_PVP_ACTIVE.with(|a| a.set(true));
}

/// Replace live `Faction` list from `map_meta.bin`.
pub fn install_factions(factions: Vec<Faction>) {
    FACTIONS.with(|list| {
        let mut list = list.borrow_mut();
        list.clear();
        list.extend(factions.into_iter().map(|f| Rc::new(RefCell::new(f))));
    });
    FACTIONS_ACTIVE.with(|a| a.set(true));
}

/// Snapshot live `NonPvpZone` list for `map_meta.bin` (`None` when inactive).
pub fn snapshot_non_pvp_zones() -> Option<Vec<NonPvpZone>> {
    if !NON_PVP_ACTIVE.with(Cell::get) {
        return None;
    }
    Some(NON_PVP_ZONES.with(|list| list.borrow().iter().map(|z| z.borrow().clone()).collect()))
}

/// Snapshot live `Faction` list for `map_meta.bin` (`None` when inactive).
pub fn snapshot_factions() -> Option<Vec<Faction>> {
    if !FACTIONS_ACTIVE.with(Cell::get) {
        return None;
    }
    Some(FACTIONS.with(|list| list.borrow().iter().map(|f| f.borrow().clone()).collect()))
}

/// Reset both lists on world exit.
pub fn reset() {
    NON_PVP_ZONES.with(|list| list.borrow_mut().clear());
    NON_PVP_ACTIVE.with(|a| a.set(false));
    FACTIONS.with(|list| list.borrow_mut().clear());
    FACTIONS_ACTIVE.with(|a| a.set(false));
}

fn username_arg(c: &NativeCall<'_>, idx: usize) -> Option<String> {
    match c.arg(idx) {
        Value::Str(s) => Some(s.as_ref().to_owned()),
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<CharacterObject>()
            .and_then(|ch| ch.0.borrow().username().map(str::to_string)),
        _ => None,
    }
}

fn find_player_faction(username: &str) -> Option<FactionRef> {
    FACTIONS.with(|list| {
        list.borrow()
            .iter()
            .find(|f| {
                let f = f.borrow();
                f.owner == username || f.players.iter().any(|p| p == username)
            })
            .cloned()
    })
}

/// Expose `zombie.iso.areas.NonPvpZone` and `zombie.characters.Faction`.
pub fn register(thread: &mut Thread) {
    register_non_pvp_zone(thread);
    register_faction(thread);
}

fn register_non_pvp_zone(thread: &mut Thread) {
    let b = expose_class(thread, NON_PVP_ZONE_CLASS, Some("java.lang.Object"));
    b.static_fn("getAllZones", |c| {
        let items = NON_PVP_ZONES.with(|list| {
            list.borrow()
                .iter()
                .map(|z| Value::Object(Rc::new(LuaNonPvpZone(z.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    b.static_fn("getNonPvpZone", |c| {
        let x = int_arg(c, 0);
        let y = int_arg(c, 1);
        let found = NON_PVP_ZONES.with(|list| {
            list.borrow()
                .iter()
                .find(|z| {
                    let z = z.borrow();
                    x >= z.x && x < z.x2 && y >= z.y && y < z.y2
                })
                .cloned()
        });
        Ok(c.push(
            found
                .map(|z| Value::Object(Rc::new(LuaNonPvpZone(z))))
                .unwrap_or(Value::Nil),
        ))
    });
    b.static_fn("getZoneByTitle", |c| {
        let Some(title) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let found = NON_PVP_ZONES.with(|list| {
            list.borrow()
                .iter()
                .find(|z| z.borrow().title == title.as_ref())
                .cloned()
        });
        Ok(c.push(
            found
                .map(|z| Value::Object(Rc::new(LuaNonPvpZone(z))))
                .unwrap_or(Value::Nil),
        ))
    });
    b.static_fn("addNonPvpZone", |c| {
        let title = str_arg(c, 0).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        let (ax, ay, bx, by) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let (x, x2) = if ax <= bx { (ax, bx) } else { (bx, ax) };
        let (y, y2) = if ay <= by { (ay, by) } else { (by, ay) };
        let size = (x2 - x).abs().saturating_mul((y2 - y).abs());
        let zone = Rc::new(RefCell::new(NonPvpZone {
            x,
            y,
            x2,
            y2,
            size,
            title,
        }));
        NON_PVP_ZONES.with(|list| list.borrow_mut().push(zone.clone()));
        NON_PVP_ACTIVE.with(|a| a.set(true));
        Ok(c.push(Value::Object(Rc::new(LuaNonPvpZone(zone)))))
    });
    b.static_fn("removeNonPvpZone", |c| {
        if let Some(title) = str_arg(c, 0) {
            NON_PVP_ZONES.with(|list| {
                list.borrow_mut().retain(|z| z.borrow().title != title.as_ref());
            });
            NON_PVP_ACTIVE.with(|a| a.set(true));
        }
        Ok(0)
    });

    b.method("getTitle", |c| {
        let v = with_this::<LuaNonPvpZone, _>(c, |z| z.0.borrow().title.clone())?;
        Ok(c.push(Value::str(&v)))
    });
    b.method("setTitle", |c| {
        let s = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        with_this::<LuaNonPvpZone, _>(c, |z| z.0.borrow_mut().title = s)?;
        Ok(0)
    });
    for (name, field) in [
        ("getX", 0),
        ("getY", 1),
        ("getX2", 2),
        ("getY2", 3),
        ("getSize", 4),
    ] {
        b.method(name, move |c| {
            let v = with_this::<LuaNonPvpZone, _>(c, |z| {
                let z = z.0.borrow();
                match field {
                    0 => z.x,
                    1 => z.y,
                    2 => z.x2,
                    3 => z.y2,
                    _ => z.size,
                }
            })?;
            Ok(c.push(v))
        });
    }
    for (name, field) in [
        ("setX", 0),
        ("setY", 1),
        ("setX2", 2),
        ("setY2", 3),
        ("setSize", 4),
    ] {
        b.method(name, move |c| {
            let v = int_arg(c, 1);
            with_this::<LuaNonPvpZone, _>(c, |z| {
                let mut z = z.0.borrow_mut();
                match field {
                    0 => z.x = v,
                    1 => z.y = v,
                    2 => z.x2 = v,
                    3 => z.y2 = v,
                    _ => z.size = v,
                }
            })?;
            Ok(0)
        });
    }
    b.method("syncNonPvpZone", |c| {
        let remove = bool_arg(c, 1);
        let title = with_this::<LuaNonPvpZone, _>(c, |z| z.0.borrow().title.clone())?;
        if remove {
            NON_PVP_ZONES.with(|list| list.borrow_mut().retain(|z| z.borrow().title != title));
        }
        Ok(0)
    });
}

fn register_faction(thread: &mut Thread) {
    let b = expose_class(thread, FACTION_CLASS, Some("java.lang.Object"));
    b.static_fn("getFactions", |c| {
        let items = FACTIONS.with(|list| {
            list.borrow()
                .iter()
                .map(|f| Value::Object(Rc::new(LuaFaction(f.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    b.static_fn("getFaction", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let found = FACTIONS.with(|list| {
            list.borrow()
                .iter()
                .find(|f| f.borrow().name == name.as_ref())
                .cloned()
        });
        Ok(c.push(
            found
                .map(|f| Value::Object(Rc::new(LuaFaction(f))))
                .unwrap_or(Value::Nil),
        ))
    });
    b.static_fn("factionExist", |c| {
        let exists = str_arg(c, 0).is_some_and(|name| {
            FACTIONS.with(|list| list.borrow().iter().any(|f| f.borrow().name == name.as_ref()))
        });
        Ok(c.push(exists))
    });
    b.static_fn("tagExist", |c| {
        let exists = str_arg(c, 0).is_some_and(|tag| {
            FACTIONS.with(|list| {
                list.borrow().iter().any(|f| {
                    f.borrow()
                        .tag
                        .as_ref()
                        .is_some_and(|(t, _)| t == tag.as_ref())
                })
            })
        });
        Ok(c.push(exists))
    });
    b.static_fn("createFaction", |c| {
        let name = str_arg(c, 0).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        let owner = username_arg(c, 1).unwrap_or_default();
        let faction = Rc::new(RefCell::new(Faction {
            name,
            owner,
            tag: None,
            players: Vec::new(),
        }));
        FACTIONS.with(|list| list.borrow_mut().push(faction.clone()));
        FACTIONS_ACTIVE.with(|a| a.set(true));
        Ok(c.push(Value::Object(Rc::new(LuaFaction(faction)))))
    });
    b.static_fn("getPlayerFaction", |c| {
        let found = username_arg(c, 0).and_then(|u| find_player_faction(&u));
        Ok(c.push(
            found
                .map(|f| Value::Object(Rc::new(LuaFaction(f))))
                .unwrap_or(Value::Nil),
        ))
    });
    b.static_fn("isAlreadyInFaction", |c| {
        let in_fac = username_arg(c, 0).is_some_and(|u| find_player_faction(&u).is_some());
        Ok(c.push(in_fac))
    });
    b.static_fn("canCreateFaction", |c| {
        let can = username_arg(c, 0).is_some_and(|u| find_player_faction(&u).is_none());
        Ok(c.push(can))
    });

    b.method("getName", |c| {
        let v = with_this::<LuaFaction, _>(c, |f| f.0.borrow().name.clone())?;
        Ok(c.push(Value::str(&v)))
    });
    b.method("setName", |c| {
        let s = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        with_this::<LuaFaction, _>(c, |f| f.0.borrow_mut().name = s)?;
        Ok(0)
    });
    b.method("getOwner", |c| {
        let v = with_this::<LuaFaction, _>(c, |f| f.0.borrow().owner.clone())?;
        Ok(c.push(Value::str(&v)))
    });
    b.method("setOwner", |c| {
        let s = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        with_this::<LuaFaction, _>(c, |f| f.0.borrow_mut().owner = s)?;
        Ok(0)
    });
    b.method("getTag", |c| {
        let v = with_this::<LuaFaction, _>(c, |f| {
            f.0.borrow()
                .tag
                .as_ref()
                .map(|(t, _)| Value::str(t))
                .unwrap_or(Value::Nil)
        })?;
        Ok(c.push(v))
    });
    b.method("setTag", |c| {
        let tag = str_arg(c, 1).map(|s| s.as_ref().to_owned());
        with_this::<LuaFaction, _>(c, |f| {
            let mut f = f.0.borrow_mut();
            match tag {
                Some(t) if !t.is_empty() => {
                    let rgb = f.tag.as_ref().map(|(_, c)| *c).unwrap_or([1.0, 1.0, 1.0]);
                    f.tag = Some((t, rgb));
                }
                _ => f.tag = None,
            }
        })?;
        Ok(0)
    });
    b.method("getTagColor", |c| {
        let rgb = with_this::<LuaFaction, _>(c, |f| {
            f.0.borrow()
                .tag
                .as_ref()
                .map(|(_, rgb)| *rgb)
                .unwrap_or([1.0, 1.0, 1.0])
        })?;
        let ci = ColorInfo::new(rgb[0], rgb[1], rgb[2], 1.0);
        Ok(c.push(Value::Object(LuaColorInfo::new(ci))))
    });
    b.method("setTagColor", |c| {
        let rgb = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaColorInfo>()
                .map(|ci| {
                    let k = ci.c.get();
                    [k.r, k.g, k.b]
                })
                .ok_or_else(|| LuaError::runtime("expected ColorInfo"))?,
            _ => return Err(LuaError::runtime("expected ColorInfo")),
        };
        with_this::<LuaFaction, _>(c, |f| {
            let mut f = f.0.borrow_mut();
            let t = f.tag.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
            f.tag = Some((t, rgb));
        })?;
        Ok(0)
    });
    b.method("getPlayers", |c| {
        let items = with_this::<LuaFaction, _>(c, |f| {
            f.0.borrow().players.iter().map(|p| Value::str(p)).collect()
        })?;
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    b.method("addPlayer", |c| {
        if let Some(p) = username_arg(c, 1) {
            with_this::<LuaFaction, _>(c, |f| {
                let mut f = f.0.borrow_mut();
                if f.owner != p && !f.players.iter().any(|x| x == &p) {
                    f.players.push(p);
                }
            })?;
        }
        Ok(0)
    });
    b.method("removePlayer", |c| {
        if let Some(p) = username_arg(c, 1) {
            with_this::<LuaFaction, _>(c, |f| {
                f.0.borrow_mut().players.retain(|x| x != &p);
            })?;
        }
        Ok(0)
    });
    b.method("isOwner", |c| {
        let res = username_arg(c, 1).is_some_and(|u| {
            with_this::<LuaFaction, _>(c, |f| f.0.borrow().owner == u).unwrap_or(false)
        });
        Ok(c.push(res))
    });
    b.method("isMember", |c| {
        let res = username_arg(c, 1).is_some_and(|u| {
            with_this::<LuaFaction, _>(c, |f| {
                let f = f.0.borrow();
                f.owner == u || f.players.iter().any(|p| p == &u)
            })
            .unwrap_or(false)
        });
        Ok(c.push(res))
    });
    b.method("canCreateTag", |c| {
        let _ = with_this::<LuaFaction, _>(c, |_| ())?;
        Ok(c.push(true))
    });
    b.method("removeFaction", |c| {
        let target = with_this::<LuaFaction, _>(c, |f| f.0.clone())?;
        FACTIONS.with(|list| list.borrow_mut().retain(|f| !Rc::ptr_eq(f, &target)));
        FACTIONS_ACTIVE.with(|a| a.set(true));
        Ok(0)
    });
    b.method("syncFaction", |c| {
        let _ = with_this::<LuaFaction, _>(c, |_| ())?;
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("nonpvp_faction_test.lua"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap()
    }

    #[test]
    fn non_pvp_zone_and_faction_lua_api_and_snapshot_roundtrip() {
        reset();
        assert!(snapshot_non_pvp_zones().is_none());
        assert!(snapshot_factions().is_none());

        let mut t = Thread::new(TableRef::new());
        crate::bind_java_util::register(&mut t);
        crate::bind_core::register(&mut t, None, crate::bind_core::shared_core());
        register(&mut t);

        let res = run(
            &mut t,
            r#"
            local z = NonPvpZone.addNonPvpZone("SafeTown", 100, 200, 110, 220)
            local hit = NonPvpZone.getNonPvpZone(105, 210)
            local miss = NonPvpZone.getNonPvpZone(50, 50)
            local count0 = NonPvpZone.getAllZones():size()
            local size = z:getSize()

            local f = Faction.createFaction("Survivors", "Alice")
            f:addPlayer("Bob")
            f:setTag("SRV")
            f:setTagColor(ColorInfo.new(0.25, 0.5, 0.75, 1.0))
            local exists = Faction.factionExist("Survivors")
            local tagExists = Faction.tagExist("SRV")
            local bobIn = Faction.isAlreadyInFaction("Bob")
            local bobFac = Faction.getPlayerFaction("Bob"):getName()
            local r = f:getTagColor():getR()
            return count0, hit ~= nil, miss == nil, size, exists, tagExists, bobIn, bobFac, r
            "#,
        );
        assert_eq!(res[0].as_num(), Some(1.0));
        assert!(res[1].truthy());
        assert!(res[2].truthy());
        assert_eq!(res[3].as_num(), Some(200.0));
        assert!(res[4].truthy());
        assert!(res[5].truthy());
        assert!(res[6].truthy());
        assert_eq!(res[7].as_str(), Some("Survivors"));
        assert!((res[8].as_num().unwrap() - 0.25).abs() < 1e-4);

        let zsnap = snapshot_non_pvp_zones().expect("zones active");
        let fsnap = snapshot_factions().expect("factions active");
        assert_eq!(zsnap.len(), 1);
        assert_eq!(zsnap[0].title, "SafeTown");
        assert_eq!(fsnap.len(), 1);
        assert_eq!(fsnap[0].name, "Survivors");
        assert_eq!(fsnap[0].players, vec!["Bob".to_owned()]);

        reset();
        assert!(snapshot_non_pvp_zones().is_none());
        install_non_pvp_zones(zsnap.clone());
        install_factions(fsnap.clone());
        assert_eq!(snapshot_non_pvp_zones(), Some(zsnap));
        assert_eq!(snapshot_factions(), Some(fsnap));

        reset();
    }
}
