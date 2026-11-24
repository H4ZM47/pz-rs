//! Build 42.22 Lua bindings for:
//! - `zombie.iso.areas.NonPvpZone`
//! - `zombie.characters.Faction`
//! - `zombie.core.random.Rand`

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;

const NON_PVP_ZONE_CLASS: &str = "zombie.iso.areas.NonPvpZone";
const FACTION_CLASS: &str = "zombie.characters.Faction";
const RAND_CLASS: &str = "zombie.core.random.Rand";

#[derive(Clone, Debug, Default)]
struct NonPvpZoneData {
    title: String,
    x: i32,
    y: i32,
    x2: i32,
    y2: i32,
    size: i32,
}

impl NonPvpZoneData {
    fn new(title: String, x: i32, y: i32, x2: i32, y2: i32) -> Self {
        let (min_x, max_x) = if x <= x2 { (x, x2) } else { (x2, x) };
        let (min_y, max_y) = if y <= y2 { (y, y2) } else { (y2, y) };
        let size = ((max_x - min_x).abs() * (max_y - min_y).abs()).max(0);
        Self {
            title,
            x: min_x,
            y: min_y,
            x2: max_x,
            y2: max_y,
            size,
        }
    }

    fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x2 && py >= self.y && py < self.y2
    }
}

#[derive(Clone, Debug)]
struct FactionData {
    name: String,
    owner: String,
    tag: Option<String>,
    players: Vec<String>,
    tag_color: Option<Value>,
}

impl FactionData {
    fn new(name: String, owner: String) -> Self {
        Self {
            name,
            owner,
            tag: None,
            players: Vec::new(),
            tag_color: None,
        }
    }

    fn is_owner(&self, user: &str) -> bool {
        self.owner == user
    }

    fn is_member(&self, user: &str) -> bool {
        self.players.iter().any(|p| p == user)
    }

    fn has_user(&self, user: &str) -> bool {
        self.is_owner(user) || self.is_member(user)
    }
}

#[derive(Clone, Debug)]
struct RandState(u64);

impl Default for RandState {
    fn default() -> Self {
        Self(0x2545_F491_4F6C_DD1D)
    }
}

impl RandState {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_int(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        (self.next_u64() % (max as u64)) as i32
    }

    fn next_int_range(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        let span = (max as i64 - min as i64) as u64;
        min + (self.next_u64() % span) as i32
    }

    fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        if max <= min {
            return min;
        }
        let unit = ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32);
        min + unit * (max - min)
    }
}

thread_local! {
    static NON_PVP_ZONES: RefCell<Vec<Rc<RefCell<NonPvpZoneData>>>> = const { RefCell::new(Vec::new()) };
    static FACTIONS: RefCell<Vec<Rc<RefCell<FactionData>>>> = const { RefCell::new(Vec::new()) };
    static RAND: RefCell<RandState> = RefCell::new(RandState::default());
}

/// Reset NonPvpZone & Faction session state on world teardown.
pub fn reset() {
    NON_PVP_ZONES.with(|z| z.borrow_mut().clear());
    FACTIONS.with(|f| f.borrow_mut().clear());
}

/// Snapshot of all NonPvpZone rectangles `(x, y, x2, y2)` for highlight rendering.
pub fn non_pvp_zone_bounds() -> Vec<(i32, i32, i32, i32)> {
    NON_PVP_ZONES.with(|z| {
        z.borrow()
            .iter()
            .map(|zone| {
                let b = zone.borrow();
                (b.x, b.y, b.x2, b.y2)
            })
            .collect()
    })
}

#[derive(Debug)]
struct NonPvpZoneObj(Rc<RefCell<NonPvpZoneData>>);

impl LuaObject for NonPvpZoneObj {
    fn class_name(&self) -> &'static str {
        NON_PVP_ZONE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct FactionObj(Rc<RefCell<FactionData>>);

impl LuaObject for FactionObj {
    fn class_name(&self) -> &'static str {
        FACTION_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn str_arg(c: &NativeCall<'_>, idx: usize) -> Option<String> {
    c.arg(idx).as_str().map(str::to_string)
}

fn i32_arg(c: &NativeCall<'_>, idx: usize) -> i32 {
    c.arg(idx).as_num().unwrap_or(0.0) as i32
}

fn call_zero(thread: &mut Thread, obj: &Value, method: &str) -> Option<Value> {
    let f = thread.table_get(obj, &Value::str(method)).ok()?;
    if f.is_nil() {
        return None;
    }
    thread
        .pcall(&f, std::slice::from_ref(obj))
        .ok()?
        .into_iter()
        .next()
}

fn player_username(thread: &mut Thread, val: &Value) -> Option<String> {
    match val {
        Value::Str(s) => Some(s.as_ref().to_string()),
        v if !v.is_nil() => match call_zero(thread, v, "getUsername") {
            Some(Value::Str(s)) => Some(s.as_ref().to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn player_xy(thread: &mut Thread, val: &Value) -> Option<(i32, i32)> {
    if val.is_nil() {
        return None;
    }
    let x = call_zero(thread, val, "getX")?.as_num()? as i32;
    let y = call_zero(thread, val, "getY")?.as_num()? as i32;
    Some((x, y))
}

fn with_zone<R>(c: &NativeCall<'_>, f: impl FnOnce(&NonPvpZoneData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let z = o.as_any().downcast_ref::<NonPvpZoneObj>()?;
    let res = Some(f(&z.0.borrow()));
    res
}

fn with_zone_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut NonPvpZoneData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let z = o.as_any().downcast_ref::<NonPvpZoneObj>()?;
    let res = Some(f(&mut z.0.borrow_mut()));
    res
}

fn with_faction<R>(c: &NativeCall<'_>, f: impl FnOnce(&FactionData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let fac = o.as_any().downcast_ref::<FactionObj>()?;
    let res = Some(f(&fac.0.borrow()));
    res
}

fn with_faction_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut FactionData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let fac = o.as_any().downcast_ref::<FactionObj>()?;
    let res = Some(f(&mut fac.0.borrow_mut()));
    res
}

/// Register `NonPvpZone`, `Faction`, and `Rand`.
pub fn register(thread: &mut Thread) {
    let z = expose_class(thread, NON_PVP_ZONE_CLASS, Some("java.lang.Object"));
    z.ctor(|c| {
        let data = if c.nargs() >= 5 {
            NonPvpZoneData::new(
                str_arg(c, 0).unwrap_or_default(),
                i32_arg(c, 1),
                i32_arg(c, 2),
                i32_arg(c, 3),
                i32_arg(c, 4),
            )
        } else {
            NonPvpZoneData::default()
        };
        Ok(c.push(Value::Object(Rc::new(NonPvpZoneObj(Rc::new(
            RefCell::new(data),
        ))))))
    });
    z.method("getX", |c| Ok(c.push(with_zone(c, |z| z.x).unwrap_or(0) as f64)));
    z.method("setX", |c| {
        let v = i32_arg(c, 1);
        with_zone_mut(c, |z| z.x = v);
        Ok(0)
    });
    z.method("getY", |c| Ok(c.push(with_zone(c, |z| z.y).unwrap_or(0) as f64)));
    z.method("setY", |c| {
        let v = i32_arg(c, 1);
        with_zone_mut(c, |z| z.y = v);
        Ok(0)
    });
    z.method("getX2", |c| Ok(c.push(with_zone(c, |z| z.x2).unwrap_or(0) as f64)));
    z.method("setX2", |c| {
        let v = i32_arg(c, 1);
        with_zone_mut(c, |z| z.x2 = v);
        Ok(0)
    });
    z.method("getY2", |c| Ok(c.push(with_zone(c, |z| z.y2).unwrap_or(0) as f64)));
    z.method("setY2", |c| {
        let v = i32_arg(c, 1);
        with_zone_mut(c, |z| z.y2 = v);
        Ok(0)
    });
    z.method("getTitle", |c| {
        let s = with_zone(c, |z| z.title.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    z.method("setTitle", |c| {
        if let Some(s) = str_arg(c, 1) {
            with_zone_mut(c, |z| z.title = s);
        }
        Ok(0)
    });
    z.method("getSize", |c| Ok(c.push(with_zone(c, |z| z.size).unwrap_or(0) as f64)));
    z.method("setSize", |c| {
        let v = i32_arg(c, 1);
        with_zone_mut(c, |z| z.size = v);
        Ok(0)
    });
    z.method("syncNonPvpZone", |c| {
        if matches!(c.arg(1), Value::Bool(true)) {
            if let Some(title) = with_zone(c, |z| z.title.clone()) {
                NON_PVP_ZONES.with(|list| list.borrow_mut().retain(|z| z.borrow().title != title));
            }
        }
        Ok(0)
    });
    z.static_fn("addNonPvpZone", |c| {
        let title = str_arg(c, 0).unwrap_or_default();
        let zone = Rc::new(RefCell::new(NonPvpZoneData::new(
            title,
            i32_arg(c, 1),
            i32_arg(c, 2),
            i32_arg(c, 3),
            i32_arg(c, 4),
        )));
        NON_PVP_ZONES.with(|list| list.borrow_mut().push(zone.clone()));
        Ok(c.push(Value::Object(Rc::new(NonPvpZoneObj(zone)))))
    });
    z.static_fn("removeNonPvpZone", |c| {
        if let Some(title) = str_arg(c, 0) {
            NON_PVP_ZONES.with(|list| list.borrow_mut().retain(|z| z.borrow().title != title));
        }
        Ok(0)
    });
    z.static_fn("getZoneByTitle", |c| {
        let found = str_arg(c, 0).and_then(|title| {
            NON_PVP_ZONES.with(|list| {
                list.borrow()
                    .iter()
                    .find(|z| z.borrow().title == title)
                    .cloned()
                    .map(|rc| Value::Object(Rc::new(NonPvpZoneObj(rc))))
            })
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });
    z.static_fn("getNonPvpZone", |c| {
        let (x, y) = (i32_arg(c, 0), i32_arg(c, 1));
        let found = NON_PVP_ZONES.with(|list| {
            list.borrow()
                .iter()
                .find(|z| z.borrow().contains(x, y))
                .cloned()
                .map(|rc| Value::Object(Rc::new(NonPvpZoneObj(rc))))
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });
    z.static_fn("getAllZones", |c| {
        let items = NON_PVP_ZONES.with(|list| {
            list.borrow()
                .iter()
                .map(|rc| Value::Object(Rc::new(NonPvpZoneObj(rc.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    z.static_fn("isInNonPvpZone", |c| {
        let arg0 = c.arg(0);
        let inside = player_xy(c.thread, &arg0).is_some_and(|(x, y)| {
            NON_PVP_ZONES.with(|list| list.borrow().iter().any(|z| z.borrow().contains(x, y)))
        });
        Ok(c.push(inside))
    });

    let f = expose_class(thread, FACTION_CLASS, Some("java.lang.Object"));
    f.ctor(|c| {
        let name = str_arg(c, 0).unwrap_or_default();
        let owner = str_arg(c, 1).unwrap_or_default();
        let fac = Rc::new(RefCell::new(FactionData::new(name, owner)));
        Ok(c.push(Value::Object(Rc::new(FactionObj(fac)))))
    });
    f.method("canCreateTag", |c| {
        Ok(c.push(with_faction(c, |f| !f.players.is_empty()).unwrap_or(false)))
    });
    f.method("addPlayer", |c| {
        if let Some(user) = str_arg(c, 1) {
            with_faction_mut(c, |f| {
                if !f.players.iter().any(|p| p == &user) && f.owner != user {
                    f.players.push(user);
                }
            });
        }
        Ok(0)
    });
    f.method("removePlayer", |c| {
        if let Some(user) = str_arg(c, 1) {
            with_faction_mut(c, |f| f.players.retain(|p| p != &user));
        }
        Ok(0)
    });
    f.method("isOwner", |c| {
        let ok = str_arg(c, 1)
            .is_some_and(|u| with_faction(c, |f| f.is_owner(&u)).unwrap_or(false));
        Ok(c.push(ok))
    });
    f.method("isMember", |c| {
        let ok = str_arg(c, 1)
            .is_some_and(|u| with_faction(c, |f| f.is_member(&u)).unwrap_or(false));
        Ok(c.push(ok))
    });
    f.method("getPlayers", |c| {
        let items = with_faction(c, |f| {
            f.players.iter().map(|s| Value::str(s)).collect::<Vec<_>>()
        })
        .unwrap_or_default();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    f.method("getTag", |c| {
        let tag = with_faction(c, |f| f.tag.clone()).flatten();
        Ok(c.push(tag.map_or(Value::Nil, |s| Value::str(&s))))
    });
    f.method("setTag", |c| {
        let tag = str_arg(c, 1);
        with_faction_mut(c, |f| f.tag = tag);
        Ok(0)
    });
    f.method("getTagColor", |c| {
        let col = with_faction(c, |f| f.tag_color.clone()).flatten();
        Ok(c.push(col.unwrap_or(Value::Nil)))
    });
    f.method("setTagColor", |c| {
        let col = c.arg(1);
        with_faction_mut(c, |f| f.tag_color = (!col.is_nil()).then_some(col));
        Ok(0)
    });
    f.method("getName", |c| {
        let s = with_faction(c, |f| f.name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    f.method("setName", |c| {
        if let Some(s) = str_arg(c, 1) {
            with_faction_mut(c, |f| f.name = s);
        }
        Ok(0)
    });
    f.method("getOwner", |c| {
        let s = with_faction(c, |f| f.owner.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    f.method("setOwner", |c| {
        if let Some(s) = str_arg(c, 1) {
            with_faction_mut(c, |f| f.owner = s);
        }
        Ok(0)
    });
    f.method("removeFaction", |c| {
        if let Some(name) = with_faction(c, |f| f.name.clone()) {
            FACTIONS.with(|list| list.borrow_mut().retain(|f| f.borrow().name != name));
        }
        Ok(0)
    });
    f.method("syncFaction", |_| Ok(0));
    f.static_fn("getFactions", |c| {
        let items = FACTIONS.with(|list| {
            list.borrow()
                .iter()
                .map(|rc| Value::Object(Rc::new(FactionObj(rc.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    f.static_fn("createFaction", |c| {
        let name = str_arg(c, 0).unwrap_or_default();
        let owner = str_arg(c, 1).unwrap_or_default();
        let fac = Rc::new(RefCell::new(FactionData::new(name, owner)));
        FACTIONS.with(|list| list.borrow_mut().push(fac.clone()));
        Ok(c.push(Value::Object(Rc::new(FactionObj(fac)))))
    });
    f.static_fn("canCreateFaction", |c| {
        let arg0 = c.arg(0);
        let ok = player_username(c.thread, &arg0).is_some_and(|u| {
            !FACTIONS.with(|list| list.borrow().iter().any(|f| f.borrow().has_user(&u)))
        });
        Ok(c.push(ok))
    });
    f.static_fn("isAlreadyInFaction", |c| {
        let arg0 = c.arg(0);
        let ok = player_username(c.thread, &arg0).is_some_and(|u| {
            FACTIONS.with(|list| list.borrow().iter().any(|f| f.borrow().has_user(&u)))
        });
        Ok(c.push(ok))
    });
    f.static_fn("isInSameFaction", |c| {
        let arg0 = c.arg(0);
        let arg1 = c.arg(1);
        let u1 = player_username(c.thread, &arg0);
        let u2 = player_username(c.thread, &arg1);
        let ok = match (u1, u2) {
            (Some(a), Some(b)) => FACTIONS.with(|list| {
                list.borrow()
                    .iter()
                    .any(|f| f.borrow().has_user(&a) && f.borrow().has_user(&b))
            }),
            _ => false,
        };
        Ok(c.push(ok))
    });
    f.static_fn("factionExist", |c| {
        let ok = str_arg(c, 0).is_some_and(|name| {
            FACTIONS.with(|list| list.borrow().iter().any(|f| f.borrow().name == name))
        });
        Ok(c.push(ok))
    });
    f.static_fn("tagExist", |c| {
        let ok = str_arg(c, 0).is_some_and(|tag| {
            FACTIONS.with(|list| {
                list.borrow()
                    .iter()
                    .any(|f| f.borrow().tag.as_deref() == Some(tag.as_str()))
            })
        });
        Ok(c.push(ok))
    });
    f.static_fn("getPlayerFaction", |c| {
        let arg0 = c.arg(0);
        let found = player_username(c.thread, &arg0).and_then(|u| {
            FACTIONS.with(|list| {
                list.borrow()
                    .iter()
                    .find(|f| f.borrow().has_user(&u))
                    .cloned()
                    .map(|rc| Value::Object(Rc::new(FactionObj(rc))))
            })
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });
    f.static_fn("getFaction", |c| {
        let found = str_arg(c, 0).and_then(|name| {
            FACTIONS.with(|list| {
                list.borrow()
                    .iter()
                    .find(|f| f.borrow().name == name)
                    .cloned()
                    .map(|rc| Value::Object(Rc::new(FactionObj(rc))))
            })
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });

    let r = expose_class(thread, RAND_CLASS, Some("java.lang.Object"));
    r.static_fn("Next", |c| {
        if let Value::Object(obj) = c.arg(0) {
            if let Some(list) = obj.as_any().downcast_ref::<JavaList>() {
                let items = list.items.borrow();
                if items.is_empty() {
                    return Ok(c.push(Value::Nil));
                }
                let idx = RAND.with(|r| r.borrow_mut().next_int(items.len() as i32)) as usize;
                return Ok(c.push(items[idx].clone()));
            }
        }
        let a = c.arg(0).as_num();
        let b = c.arg(1).as_num();
        let out = RAND.with(|r| {
            let mut r = r.borrow_mut();
            match (a, b) {
                (Some(a), Some(b)) => {
                    if a.fract() == 0.0 && b.fract() == 0.0 {
                        r.next_int_range(a as i32, b as i32) as f64
                    } else {
                        r.next_float_range(a as f32, b as f32) as f64
                    }
                }
                (Some(a), None) => r.next_int(a as i32) as f64,
                _ => 0.0,
            }
        });
        Ok(c.push(out))
    });
    r.static_fn("NextInclusive", |c| {
        let min = i32_arg(c, 0);
        let max = i32_arg(c, 1);
        let v = RAND.with(|r| r.borrow_mut().next_int_range(min, max.saturating_add(1)));
        Ok(c.push(v as f64))
    });
    r.static_fn("NextBool", |c| {
        let ok = c.arg(0).as_num().is_some_and(|n| {
            let inv = n as i32;
            inv > 0 && RAND.with(|r| r.borrow_mut().next_int(inv) == 0)
        });
        Ok(c.push(ok))
    });
    r.static_fn("AdjustForFramerate", |c| {
        let chance = i32_arg(c, 0);
        Ok(c.push(chance.max(1) as f64))
    });
    r.static_fn("PerThirtiethOfASecond", |c| {
        let chance = i32_arg(c, 0);
        let ok = chance > 0 && RAND.with(|r| r.borrow_mut().next_int(chance) == 0);
        Ok(c.push(ok))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn nonpvp_faction_and_rand_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local z = NonPvpZone.addNonPvpZone("SafeMall", 100, 200, 150, 250)
            assert(z:getTitle() == "SafeMall" and z:getSize() == 2500)
            assert(NonPvpZone.getZoneByTitle("SafeMall") ~= nil)
            assert(NonPvpZone.getNonPvpZone(120, 220) ~= nil)
            assert(NonPvpZone.getNonPvpZone(99, 220) == nil)
            assert(NonPvpZone.getAllZones():size() == 1)
            local fake_p = { getX = function() return 125 end, getY = function() return 225 end, getUsername = function() return "alice" end }
            assert(NonPvpZone.isInNonPvpZone(fake_p) == true)
            NonPvpZone.removeNonPvpZone("SafeMall")
            assert(NonPvpZone.getAllZones():size() == 0)

            assert(Faction.canCreateFaction(fake_p) == true)
            local fac = Faction.createFaction("Rangers", "alice")
            assert(Faction.factionExist("Rangers") == true)
            assert(Faction.isAlreadyInFaction("alice") == true)
            fac:addPlayer("bob")
            assert(fac:isOwner("alice") and fac:isMember("bob") and fac:canCreateTag())
            assert(Faction.isInSameFaction(fake_p, "bob") == true)
            fac:setTag("RNG")
            assert(Faction.tagExist("RNG") == true)
            assert(Faction.getPlayerFaction("bob"):getName() == "Rangers")
            fac:removePlayer("bob")
            assert(fac:getPlayers():size() == 0)
            fac:removeFaction()
            assert(Faction.getFactions():size() == 0)

            local r = Rand.Next(5, 10)
            assert(r >= 5 and r < 10)
            local ri = Rand.NextInclusive(3, 3)
            assert(ri == 3)
            assert(Rand.NextBool(1) == true)
            local list = ArrayList.new()
            list:add("only")
            assert(Rand.Next(list) == "only")
            "#,
            Some("nonpvp_faction_rand_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
