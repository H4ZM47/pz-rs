//! `zombie.core.stash.StashSystem`, `Stash`, and `StashBuilding` — annotated
//! stash maps, building stash queues, and `map_meta.bin` stash persistence.
//!
//! `StashDescriptions` is populated by Build 42.22 Lua scripts under
//! `media/lua/shared/StashDescriptions/*.lua` (via `StashUtil.newStash`).
//! `StashSystem.init()` reads that global table into `allStashes` and seeds
//! `possibleStashes`; `IsoWorld::load_meta_grid_files` restores saved
//! `StashState` from `map_meta.bin`, and `game_window_save` snapshots the live
//! state back into `MetaWorld::stash`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::meta_save::{StashBuilding, StashState};

use crate::bind_items::{ItemRef, LuaInventoryItem};
use crate::bind_java_util::JavaList;
use crate::expose::{expose_class, str_arg, with_this};

pub const STASH_SYSTEM_CLASS: &str = "zombie.core.stash.StashSystem";
pub const STASH_CLASS: &str = "zombie.core.stash.Stash";
pub const STASH_BUILDING_CLASS: &str = "zombie.core.stash.StashBuilding";

/// One symbol or text annotation on a stash map (`zombie.core.stash.StashAnnotation`).
#[derive(Clone, Debug, PartialEq)]
pub struct StashAnnotation {
    pub symbol: Option<String>,
    pub text: Option<String>,
    pub x: f32,
    pub y: f32,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub rotation: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// One container entry defined on a stash (`zombie.core.stash.StashContainer`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashContainer {
    pub room: Option<String>,
    pub container_sprite: Option<String>,
    pub container_type: Option<String>,
    pub cont_x: i32,
    pub cont_y: i32,
    pub cont_z: i32,
    pub container_item: Option<String>,
}

/// Parsed stash definition (`zombie.core.stash.Stash`).
#[derive(Clone, Debug, PartialEq)]
pub struct Stash {
    pub name: String,
    pub kind: String,
    pub item: String,
    pub custom_name: Option<String>,
    pub building_x: i32,
    pub building_y: i32,
    pub spawn_table: Option<String>,
    pub annotations: Vec<StashAnnotation>,
    pub containers: Vec<StashContainer>,
    pub barricades: i32,
    pub zombies: i32,
    pub spawn_only_on_zed: bool,
    pub min_day_to_spawn: i32,
    pub max_day_to_spawn: i32,
    pub min_trap_to_spawn: i32,
    pub max_trap_to_spawn: i32,
}

impl Stash {
    fn from_lua_table(tbl: &TableRef) -> Option<Self> {
        let name = tbl_str(tbl, "name")?;
        let kind = tbl_str(tbl, "type").unwrap_or_default();
        let item = tbl_str(tbl, "item").unwrap_or_default();
        let custom_name = tbl_str(tbl, "customName");
        let building_x = tbl_i32(tbl, "buildingX").unwrap_or(0);
        let building_y = tbl_i32(tbl, "buildingY").unwrap_or(0);
        let spawn_table = tbl_str(tbl, "spawnTable");
        let barricades = tbl_i32(tbl, "barricades").unwrap_or(0);
        let zombies = tbl_i32(tbl, "zombies").unwrap_or(0);
        let spawn_only_on_zed = tbl_bool(tbl, "spawnOnlyOnZed").unwrap_or(false);
        let (min_day_to_spawn, max_day_to_spawn) = parse_range(tbl_str(tbl, "daysToSpawn"));
        let (min_trap_to_spawn, max_trap_to_spawn) = parse_range(tbl_str(tbl, "traps"));

        let mut annotations = Vec::new();
        if let Value::Table(ann_tbl) = tbl.rawget(&Value::str("annotations")) {
            let len = ann_tbl.len();
            for i in 1..=len {
                if let Value::Table(a) = ann_tbl.rawget(&Value::from(i as f64)) {
                    annotations.push(StashAnnotation {
                        symbol: tbl_str(&a, "symbol"),
                        text: tbl_str(&a, "text"),
                        x: tbl_f32(&a, "x").unwrap_or(0.0),
                        y: tbl_f32(&a, "y").unwrap_or(0.0),
                        anchor_x: tbl_f32(&a, "anchorX").unwrap_or(0.5),
                        anchor_y: tbl_f32(&a, "anchorY").unwrap_or(0.5),
                        rotation: tbl_f32(&a, "rotation").unwrap_or(0.0),
                        r: tbl_f32(&a, "r").unwrap_or(0.0),
                        g: tbl_f32(&a, "g").unwrap_or(0.0),
                        b: tbl_f32(&a, "b").unwrap_or(0.0),
                    });
                }
            }
        }

        let mut containers = Vec::new();
        if let Value::Table(cont_tbl) = tbl.rawget(&Value::str("containers")) {
            let len = cont_tbl.len();
            for i in 1..=len {
                if let Value::Table(c) = cont_tbl.rawget(&Value::from(i as f64)) {
                    containers.push(StashContainer {
                        room: tbl_str(&c, "room"),
                        container_sprite: tbl_str(&c, "containerSprite"),
                        container_type: tbl_str(&c, "containerType"),
                        cont_x: tbl_i32(&c, "contX").unwrap_or(-1),
                        cont_y: tbl_i32(&c, "contY").unwrap_or(-1),
                        cont_z: tbl_i32(&c, "contZ").unwrap_or(-1),
                        container_item: tbl_str(&c, "containerItem"),
                    });
                }
            }
        }

        Some(Self {
            name,
            kind,
            item,
            custom_name,
            building_x,
            building_y,
            spawn_table,
            annotations,
            containers,
            barricades,
            zombies,
            spawn_only_on_zed,
            min_day_to_spawn,
            max_day_to_spawn,
            min_trap_to_spawn,
            max_trap_to_spawn,
        })
    }

    fn as_building(&self) -> StashBuilding {
        StashBuilding {
            stash_name: self.name.clone(),
            building_x: self.building_x,
            building_y: self.building_y,
        }
    }
}

fn parse_range(raw: Option<String>) -> (i32, i32) {
    let Some(s) = raw else { return (-1, -1) };
    let s = s.trim();
    if let Some((a, b)) = s.split_once('-') {
        let min = a.trim().parse::<i32>().unwrap_or(-1);
        let max = b.trim().parse::<i32>().unwrap_or(-1);
        (min, max)
    } else if let Ok(v) = s.parse::<i32>() {
        (v, v)
    } else {
        (-1, -1)
    }
}

fn tbl_str(tbl: &TableRef, key: &str) -> Option<String> {
    match tbl.rawget(&Value::str(key)) {
        Value::Str(s) => Some(s.as_ref().to_owned()),
        _ => None,
    }
}

fn tbl_i32(tbl: &TableRef, key: &str) -> Option<i32> {
    tbl.rawget(&Value::str(key)).as_num().map(|n| n as i32)
}

fn tbl_f32(tbl: &TableRef, key: &str) -> Option<f32> {
    tbl.rawget(&Value::str(key)).as_num().map(|n| n as f32)
}

fn tbl_bool(tbl: &TableRef, key: &str) -> Option<bool> {
    match tbl.rawget(&Value::str(key)) {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

/// Lua wrapper for `zombie.core.stash.Stash`.
pub struct LuaStash(pub Rc<Stash>);

impl LuaObject for LuaStash {
    fn class_name(&self) -> &str {
        STASH_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Lua wrapper for `zombie.core.stash.StashBuilding`.
pub struct LuaStashBuilding(pub StashBuilding);

impl LuaObject for LuaStashBuilding {
    fn class_name(&self) -> &str {
        STASH_BUILDING_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct StashRegistry {
    /// `None` before `init()` or after `Reset()` (matches `MetaWorld::stash`).
    all_stashes: Option<Vec<Rc<Stash>>>,
    possible_stashes: Vec<StashBuilding>,
    buildings_to_do: Vec<StashBuilding>,
    already_read_map: Vec<String>,
}

impl Default for StashRegistry {
    fn default() -> Self {
        Self {
            all_stashes: None,
            possible_stashes: Vec::new(),
            buildings_to_do: Vec::new(),
            already_read_map: Vec::new(),
        }
    }
}

thread_local! {
    static STASH_STATE: RefCell<StashRegistry> = RefCell::new(StashRegistry::default());
}

/// `StashSystem.initAllStashes()`: parse the Lua global `StashDescriptions`
/// array into `all_stashes`.
pub fn init_all_stashes(thread: &mut Thread) {
    let mut parsed = Vec::new();
    let desc = thread.globals().rawget(&Value::str("StashDescriptions"));
    if let Value::Table(tbl) = desc {
        let len = tbl.len();
        for i in 1..=len {
            if let Value::Table(entry) = tbl.rawget(&Value::from(i as f64)) {
                if let Some(stash) = Stash::from_lua_table(&entry) {
                    parsed.push(Rc::new(stash));
                }
            }
        }
    }
    STASH_STATE.with(|s| {
        s.borrow_mut().all_stashes = Some(parsed);
    });
}

/// `StashSystem.init()`: on the first call of a session, load
/// `StashDescriptions`; every described stash starts as possible (its
/// constructor registers it), whatever its building coordinates. Later calls
/// keep the current lists.
pub fn init(thread: &mut Thread) {
    if STASH_STATE.with(|s| s.borrow().all_stashes.is_some()) {
        return;
    }
    init_all_stashes(thread);
    STASH_STATE.with(|s| {
        let mut reg = s.borrow_mut();
        let possible = reg
            .all_stashes
            .as_ref()
            .map(|all| all.iter().map(|st| st.as_building()).collect())
            .unwrap_or_default();
        reg.possible_stashes = possible;
    });
}

/// `StashSystem.reinit()`: reset all lists and re-initialize from `StashDescriptions`.
pub fn reinit(thread: &mut Thread) {
    reset();
    init(thread);
}

/// `StashSystem.Reset()`: release all stash state on world exit.
pub fn reset() {
    STASH_STATE.with(|s| {
        *s.borrow_mut() = StashRegistry::default();
    });
}

/// Restore saved `StashState` loaded from `map_meta.bin`.
pub fn install_state(state: StashState) {
    STASH_STATE.with(|s| {
        let mut reg = s.borrow_mut();
        if reg.all_stashes.is_none() {
            reg.all_stashes = Some(Vec::new());
        }
        reg.possible_stashes = state.possible_stashes;
        reg.buildings_to_do = state.buildings_to_do;
        reg.already_read_map = state.already_read_map;
    });
}

/// Snapshot live state for `map_meta.bin` (`None` while uninitialized/reset).
pub fn snapshot_state() -> Option<StashState> {
    STASH_STATE.with(|s| {
        let reg = s.borrow();
        reg.all_stashes.as_ref()?;
        Some(StashState {
            possible_stashes: reg.possible_stashes.clone(),
            buildings_to_do: reg.buildings_to_do.clone(),
            already_read_map: reg.already_read_map.clone(),
        })
    })
}

/// `StashSystem.getStash(stashName)`.
pub fn get_stash(name: &str) -> Option<Rc<Stash>> {
    STASH_STATE.with(|s| {
        let reg = s.borrow();
        reg.all_stashes
            .as_ref()?
            .iter()
            .find(|st| st.name == name)
            .cloned()
    })
}

/// `StashSystem.doStashItem(stash, item)`: the item takes the stash's
/// custom name; a `Map` stash also becomes the item's stash map and leaves
/// the possible list (every entry at its building coordinates). The building
/// is not queued here: that happens when the map is read
/// ([`prepare_building_stash`]).
pub fn do_stash_item(stash: &Stash, item: &ItemRef) {
    if let Some(custom) = stash.custom_name.as_deref() {
        item.borrow_mut().set_name(custom);
    }
    if stash.kind != "Map" {
        return;
    }
    remove_possible_at(stash.building_x, stash.building_y);
    item.borrow_mut().stash_map = Some(stash.name.clone());
}

/// Drops every possible stash whose building is at `(x, y)`.
fn remove_possible_at(x: i32, y: i32) {
    STASH_STATE.with(|s| {
        s.borrow_mut()
            .possible_stashes
            .retain(|b| b.building_x != x || b.building_y != y)
    });
}

/// `StashSystem.prepareBuildingStash(stashName)` (`MapItem.doBuildingStash`
/// when a stash map is read): the first read of a known stash records the
/// name and queues its building. Unknown names and repeated reads change
/// nothing; the possible list is left alone.
///
/// TODO(port): `doBuildingStash` for a building that is already fully
/// streamed in (loot, zombies, barricades, traps).
pub fn prepare_building_stash(stash_name: &str) {
    let Some(stash) = get_stash(stash_name) else {
        return;
    };
    STASH_STATE.with(|s| {
        let mut reg = s.borrow_mut();
        if reg.already_read_map.iter().any(|n| n == stash_name) {
            return;
        }
        reg.already_read_map.push(stash_name.to_owned());
        reg.buildings_to_do.push(stash.as_building());
    });
}

/// Whether `(x, y)` lies strictly inside the building rectangle.
fn strictly_inside(b: &StashBuilding, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
    b.building_x > x1 && b.building_x < x2 && b.building_y > y1 && b.building_y < y2
}

/// `StashSystem.visitedBuilding(def)`: a visited building (bounds
/// `x1..x2`, `y1..y2`) can no longer receive a stash; possible stashes
/// strictly inside it are dropped.
pub fn visited_building(x1: i32, y1: i32, x2: i32, y2: i32) {
    STASH_STATE.with(|s| {
        s.borrow_mut()
            .possible_stashes
            .retain(|b| !strictly_inside(b, x1, y1, x2, y2))
    });
}

/// `StashSystem.isStashBuilding(def)`: a possible or queued stash lies
/// strictly inside the building bounds.
pub fn is_stash_building(x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
    STASH_STATE.with(|s| {
        let reg = s.borrow();
        reg.possible_stashes
            .iter()
            .chain(reg.buildings_to_do.iter())
            .any(|b| strictly_inside(b, x1, y1, x2, y2))
    })
}

/// What [`check_stash_item`] needs from the world.
pub struct StashCheck<'a> {
    /// The item's stash chance (`InventoryItem.stashChance`: 80 unless the
    /// container distribution set it); 0 or less uses 60.
    pub stash_chance: i32,
    /// Sandbox `AnnotatedMapChance` (1 never, 2 extremely rare ... 6).
    pub annotated_map_chance: i32,
    /// `GameTime.getDaysSurvived()`.
    pub days_survived: i32,
    /// The item's container belongs to a corpse.
    pub in_corpse: bool,
    /// For the square `(x, y, 0)`: `None` without a room in a building,
    /// else whether that building has been visited.
    pub building_visited: &'a dyn Fn(i32, i32) -> Option<bool>,
    /// `Rand.Next(n)`.
    pub rand: &'a mut dyn FnMut(i32) -> i32,
}

/// The percentage chance a map item has to become annotated, or `None`
/// when the sandbox turns annotated maps off.
fn annotated_chance(stash_chance: i32, sandbox: i32) -> Option<i32> {
    let base = if stash_chance > 0 { stash_chance } else { 60 };
    let adjust = match sandbox {
        1 => return None,
        2 => 15,
        3 => 10,
        5 => -10,
        6 => -20,
        _ => 0,
    };
    Some(base + adjust)
}

/// `StashSystem.checkStashItem(item)` (spawned loot): after a chance roll,
/// picks one stash for this item type that is still possible, whose spawn
/// rules (corpse-only, day range) allow it and whose building exists and
/// has not been visited, and applies it with [`do_stash_item`]. Returns
/// whether the item became a stash item.
pub fn check_stash_item(item: &ItemRef, env: &mut StashCheck<'_>) -> bool {
    let empty = STASH_STATE.with(|s| s.borrow().possible_stashes.is_empty());
    if empty {
        return false;
    }
    let Some(chance) = annotated_chance(env.stash_chance, env.annotated_map_chance) else {
        return false;
    };
    if (env.rand)(100) > 100 - chance {
        return false;
    }
    let full_type = item.borrow().get_full_type();
    let days = env.days_survived;
    let candidates: Vec<Rc<Stash>> = STASH_STATE.with(|s| {
        let reg = s.borrow();
        let Some(all) = reg.all_stashes.as_ref() else {
            return Vec::new();
        };
        all.iter()
            .filter(|st| {
                st.item == full_type
                    && (!st.spawn_only_on_zed || env.in_corpse)
                    && (st.min_day_to_spawn <= -1 || days >= st.min_day_to_spawn)
                    && (st.max_day_to_spawn <= -1 || days <= st.max_day_to_spawn)
                    && reg.possible_stashes.iter().any(|b| b.stash_name == st.name)
            })
            .cloned()
            .collect()
    });
    let usable: Vec<Rc<Stash>> = candidates
        .into_iter()
        .filter(|st| (env.building_visited)(st.building_x, st.building_y) == Some(false))
        .collect();
    if usable.is_empty() {
        return false;
    }
    let pick = (env.rand)(usable.len() as i32).clamp(0, usable.len() as i32 - 1) as usize;
    do_stash_item(&usable[pick], item);
    true
}

/// `GameTime.getDaysSurvived()`: the longest survival of the local players
/// in whole days, wrapped at 30 as the original does.
pub fn days_survived() -> i32 {
    let ctx = crate::bind_character_sim::current_sim_context();
    let hours = (0..crate::bind_players::num_players().max(0) as usize)
        .filter_map(crate::bind_players::player)
        .map(|p| p.borrow().hours_survived(&*ctx) as f32)
        .fold(0.0f32, f32::max);
    (hours as i32 / 24) % 30
}

/// The world side of [`StashCheck`] for a Lua call: the live meta grid,
/// sandbox, players and `Rand`. The corpse rule needs the item's container
/// owner, which the caller passes.
pub fn check_stash_item_in_world(item: &ItemRef, stash_chance: i32, in_corpse: bool) -> bool {
    let annotated_map_chance = crate::bind_world::power::sandbox_integer("AnnotatedMapChance").unwrap_or(4);
    let building_visited = |x: i32, y: i32| {
        crate::bind_world::with_world(|w| {
            let meta = w.meta_grid.as_ref()?;
            let room = meta.get_room_at(x, y, 0)?;
            let b = room.building?;
            meta.building_defs.get(b.0 as usize).map(|d| d.has_been_visited)
        })
    };
    let mut rand = |n: i32| pz_core::random::next(n);
    check_stash_item(
        item,
        &mut StashCheck {
            stash_chance,
            annotated_map_chance,
            days_survived: days_survived(),
            in_corpse,
            building_visited: &building_visited,
            rand: &mut rand,
        },
    )
}

/// The item's direct container belongs to a corpse
/// (`item.getContainer().getParent() instanceof IsoDeadBody`).
pub fn item_in_corpse(item: &ItemRef) -> bool {
    let Ok(Some(container)) = pz_inventory::shared::try_container_of(item) else {
        return false;
    };
    let Ok(body) = container.try_borrow() else {
        return false;
    };
    let parent = crate::bind_item_container::parent_of(&body);
    crate::bind_globals::instance_of(&parent, crate::bind_dead_body::ISO_DEAD_BODY)
}

fn item_arg(c: &NativeCall<'_>, idx: usize) -> LuaResult<ItemRef> {
    match c.arg(idx) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaInventoryItem>()
            .map(|l| l.item.clone())
            .ok_or_else(|| LuaError::runtime("expected InventoryItem")),
        _ => Err(LuaError::runtime("expected InventoryItem")),
    }
}

fn stash_arg(c: &NativeCall<'_>, idx: usize) -> LuaResult<Rc<Stash>> {
    match c.arg(idx) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaStash>()
            .map(|l| l.0.clone())
            .ok_or_else(|| LuaError::runtime("expected Stash")),
        _ => Err(LuaError::runtime("expected Stash")),
    }
}

fn opt_str_val(s: Option<&str>) -> Value {
    s.map(Value::str).unwrap_or(Value::Nil)
}

/// Expose `zombie.core.stash.StashSystem`, `Stash`, and `StashBuilding`.
pub fn register(thread: &mut Thread) {
    let sb = expose_class(thread, STASH_BUILDING_CLASS, Some("java.lang.Object"));
    sb.method("getName", |c| {
        let name = with_this::<LuaStashBuilding, _>(c, |b| b.0.stash_name.clone())?;
        Ok(c.push(Value::str(&name)))
    });
    sb.method("getBuildingX", |c| {
        let x = with_this::<LuaStashBuilding, _>(c, |b| b.0.building_x)?;
        Ok(c.push(x))
    });
    sb.method("getBuildingY", |c| {
        let y = with_this::<LuaStashBuilding, _>(c, |b| b.0.building_y)?;
        Ok(c.push(y))
    });

    let st = expose_class(thread, STASH_CLASS, Some("java.lang.Object"));
    st.method("getName", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.name.clone())?;
        Ok(c.push(Value::str(&v)))
    });
    st.method("getItem", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.item.clone())?;
        Ok(c.push(Value::str(&v)))
    });
    st.method("getType", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.kind.clone())?;
        Ok(c.push(Value::str(&v)))
    });
    st.method("getBuildingX", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.building_x)?;
        Ok(c.push(v))
    });
    st.method("getBuildingY", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.building_y)?;
        Ok(c.push(v))
    });
    st.method("getSpawnTable", |c| {
        let v = with_this::<LuaStash, _>(c, |s| opt_str_val(s.0.spawn_table.as_deref()))?;
        Ok(c.push(v))
    });
    st.method("getCustomName", |c| {
        let v = with_this::<LuaStash, _>(c, |s| opt_str_val(s.0.custom_name.as_deref()))?;
        Ok(c.push(v))
    });
    st.method("getZombies", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.zombies)?;
        Ok(c.push(v))
    });
    st.method("getBarricades", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.barricades)?;
        Ok(c.push(v))
    });
    st.method("isSpawnOnlyOnZed", |c| {
        let v = with_this::<LuaStash, _>(c, |s| s.0.spawn_only_on_zed)?;
        Ok(c.push(v))
    });
    st.method("applyAnnotations", |c| {
        // Verify receiver is a Stash. TODO(port): forward annotations into
        // `UIWorldMap.getSymbolsDirect()` once `WorldMapSymbols` is ported.
        let _ = with_this::<LuaStash, _>(c, |_| ())?;
        Ok(0)
    });

    let sys = expose_class(thread, STASH_SYSTEM_CLASS, Some("java.lang.Object"));
    sys.static_fn("init", |c| {
        init(c.thread);
        Ok(0)
    });
    sys.static_fn("initAllStashes", |c| {
        init_all_stashes(c.thread);
        Ok(0)
    });
    sys.static_fn("reinit", |c| {
        reinit(c.thread);
        Ok(0)
    });
    sys.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
    sys.static_fn("getAllStashes", |c| {
        let items = STASH_STATE.with(|s| {
            s.borrow()
                .all_stashes
                .as_ref()
                .map(|v| {
                    v.iter()
                        .map(|st| Value::Object(Rc::new(LuaStash(st.clone()))))
                        .collect()
                })
                .unwrap_or_default()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    sys.static_fn("getPossibleStashes", |c| {
        let items = STASH_STATE.with(|s| {
            s.borrow()
                .possible_stashes
                .iter()
                .map(|b| Value::Object(Rc::new(LuaStashBuilding(b.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    sys.static_fn("getBuildingsToDo", |c| {
        let items = STASH_STATE.with(|s| {
            s.borrow()
                .buildings_to_do
                .iter()
                .map(|b| Value::Object(Rc::new(LuaStashBuilding(b.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    sys.static_fn("getAlreadyReadMap", |c| {
        let items = STASH_STATE.with(|s| {
            s.borrow()
                .already_read_map
                .iter()
                .map(|n| Value::str(n))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    sys.static_fn("getStash", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let v = get_stash(&name)
            .map(|st| Value::Object(Rc::new(LuaStash(st))))
            .unwrap_or(Value::Nil);
        Ok(c.push(v))
    });
    sys.static_fn("doStashItem", |c| {
        let stash = stash_arg(c, 0)?;
        let item = item_arg(c, 1)?;
        do_stash_item(&stash, &item);
        Ok(0)
    });
    sys.static_fn("prepareBuildingStash", |c| {
        if let Some(name) = str_arg(c, 0) {
            prepare_building_stash(&name);
        }
        Ok(0)
    });
    sys.static_fn("checkStashItem", |c| {
        let item = item_arg(c, 0)?;
        let in_corpse = item_in_corpse(&item);
        let _ = check_stash_item_in_world(&item, 80, in_corpse);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_items::item_value;
    use pz_inventory::item::InventoryItem;
    use pz_inventory::template::{ItemClass, ItemTemplate};
    use pz_kahlua::stdlib::load_string;
    use std::rc::Rc;

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("stash_test.lua"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap()
    }

    fn map_item() -> ItemRef {
        let mut tpl = ItemTemplate::new(ItemClass::Map, "Base", "MuldraughMap");
        tpl.display_name = "Muldraugh Map".into();
        Rc::new(RefCell::new(InventoryItem::create(Rc::new(tpl), 1)))
    }

    #[test]
    fn stash_system_init_do_stash_prepare_and_snapshot_roundtrip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        crate::bind_java_util::register(&mut t);
        crate::bind_items::register(&mut t);
        register(&mut t);

        // Before init, snapshot_state() is None (matching map_meta.bin rule).
        assert!(snapshot_state().is_none());

        run(
            &mut t,
            r#"
            StashDescriptions = {
                {
                    name = "TestStash1",
                    type = "Map",
                    item = "Base.MuldraughMap",
                    customName = "Annotated Map",
                    buildingX = 10660,
                    buildingY = 9758,
                    spawnTable = "GunCache1",
                    zombies = 4,
                    barricades = 50,
                    spawnOnlyOnZed = true,
                    daysToSpawn = "0-30",
                    traps = "1-3",
                    annotations = {
                        { symbol = "X", text = nil, x = 10657, y = 9757, r = 0.1, g = 0.1, b = 0.1 },
                    },
                    containers = {
                        { containerType = "GunBox", room = "livingroom" },
                    },
                },
                {
                    name = "TestStash2",
                    type = "Map",
                    item = "Base.MuldraughMap",
                    buildingX = 10900,
                    buildingY = 9900,
                },
            }
            StashSystem.init()
            "#,
        );

        let item = map_item();
        t.globals()
            .rawset(Value::str("testMap"), item_value(&item));

        let res = run(
            &mut t,
            r#"
            local possible0 = StashSystem.getPossibleStashes():size()
            local allCount = StashSystem.getAllStashes():size()
            local st = StashSystem.getStash("TestStash1")
            local bx = st:getBuildingX()
            local by = st:getBuildingY()
            local zed = st:isSpawnOnlyOnZed()
            StashSystem.doStashItem(st, testMap)
            local possible1 = StashSystem.getPossibleStashes():size()
            local stashNameOnItem = testMap:getStashMap()
            testMap:doBuildingStash()
            local readCount = StashSystem.getAlreadyReadMap():size()
            local read0 = StashSystem.getAlreadyReadMap():get(0)
            local todoCount = StashSystem.getBuildingsToDo():size()
            st:applyAnnotations(nil)
            return possible0, allCount, bx, by, zed, possible1, stashNameOnItem, readCount, read0, todoCount
            "#,
        );
        assert_eq!(res[0].as_num(), Some(2.0));
        assert_eq!(res[1].as_num(), Some(2.0));
        assert_eq!(res[2].as_num(), Some(10660.0));
        assert_eq!(res[3].as_num(), Some(9758.0));
        assert_eq!(res[4].truthy(), true);
        assert_eq!(res[5].as_num(), Some(1.0));
        assert_eq!(format!("{:?}", res[6]), "\"TestStash1\"");
        assert_eq!(res[7].as_num(), Some(1.0));
        assert_eq!(format!("{:?}", res[8]), "\"TestStash1\"");
        assert_eq!(res[9].as_num(), Some(1.0));

        // Snapshot & restore round-trip (as map_meta.bin does).
        let snap = snapshot_state().expect("initialized stash state");
        assert_eq!(snap.possible_stashes.len(), 1);
        assert_eq!(snap.possible_stashes[0].stash_name, "TestStash2");
        assert_eq!(snap.buildings_to_do.len(), 1);
        assert_eq!(snap.buildings_to_do[0].stash_name, "TestStash1");
        assert_eq!(snap.already_read_map, vec!["TestStash1".to_owned()]);

        // Visiting TestStash2's building removes it from possible_stashes.
        visited_building(10890, 9890, 10910, 9910);
        assert!(snapshot_state().unwrap().possible_stashes.is_empty());

        // Reset clears back to None; install_state restores snapshot.
        reset();
        assert!(snapshot_state().is_none());
        install_state(snap.clone());
        assert_eq!(snapshot_state(), Some(snap));

        reset();
    }

    fn two_stashes() -> Vec<Rc<Stash>> {
        let stash = |name: &str, x: i32, zed: bool| {
            Rc::new(Stash {
                name: name.into(),
                kind: "Map".into(),
                item: "Base.MuldraughMap".into(),
                custom_name: None,
                building_x: x,
                building_y: 100,
                spawn_table: None,
                annotations: Vec::new(),
                containers: Vec::new(),
                barricades: 0,
                zombies: 0,
                spawn_only_on_zed: zed,
                min_day_to_spawn: -1,
                max_day_to_spawn: -1,
                min_trap_to_spawn: -1,
                max_trap_to_spawn: -1,
            })
        };
        vec![stash("A", 100, false), stash("B", 200, true)]
    }

    fn install(all: Vec<Rc<Stash>>) {
        reset();
        STASH_STATE.with(|s| {
            let mut reg = s.borrow_mut();
            reg.possible_stashes = all.iter().map(|st| st.as_building()).collect();
            reg.all_stashes = Some(all);
        });
    }

    #[test]
    fn stash_item_queues_its_building_only_when_read() {
        install(two_stashes());
        let item = map_item();
        do_stash_item(&get_stash("A").unwrap(), &item);
        let snap = snapshot_state().unwrap();
        assert!(snap.buildings_to_do.is_empty());
        assert_eq!(snap.possible_stashes.len(), 1);
        assert_eq!(item.borrow().stash_map.as_deref(), Some("A"));

        prepare_building_stash("A");
        prepare_building_stash("A");
        prepare_building_stash("nope");
        let snap = snapshot_state().unwrap();
        assert_eq!(snap.buildings_to_do.len(), 1);
        assert_eq!(snap.already_read_map, vec!["A".to_owned()]);
        // Reading never touches the possible list.
        assert_eq!(snap.possible_stashes.len(), 1);
        reset();
    }

    #[test]
    fn check_stash_item_rolls_and_respects_world_rules() {
        let check = |sandbox: i32, roll: i32, visited: Option<bool>, corpse: bool| {
            install(two_stashes());
            let item = map_item();
            let building = move |_: i32, _: i32| visited;
            let mut draws = vec![roll, 0];
            let mut rand = move |_: i32| draws.remove(0);
            let hit = check_stash_item(
                &item,
                &mut StashCheck {
                    stash_chance: 80,
                    annotated_map_chance: sandbox,
                    days_survived: 0,
                    in_corpse: corpse,
                    building_visited: &building,
                    rand: &mut rand,
                },
            );
            let name = item.borrow().stash_map.clone();
            reset();
            (hit, name)
        };
        // Annotated maps off.
        assert_eq!(check(1, 0, Some(false), false), (false, None));
        // Default chance 80: a roll above 20 fails, 20 or below passes.
        assert_eq!(check(4, 21, Some(false), false), (false, None));
        assert_eq!(check(4, 20, Some(false), false), (true, Some("A".into())));
        // "Extremely rare" (-20) needs a roll of 40 or less.
        assert_eq!(check(6, 41, Some(false), false), (false, None));
        // A visited building or a square without a building rules it out.
        assert_eq!(check(4, 0, Some(true), false), (false, None));
        assert_eq!(check(4, 0, None, false), (false, None));
        // On a corpse both stashes qualify; the second draw picks the first.
        assert_eq!(check(4, 0, Some(false), true), (true, Some("A".into())));
    }

    #[test]
    fn visited_and_stash_building_bounds_are_strict() {
        install(two_stashes());
        assert!(!is_stash_building(100, 100, 110, 110));
        assert!(is_stash_building(99, 99, 101, 101));
        visited_building(100, 100, 110, 110);
        assert_eq!(snapshot_state().unwrap().possible_stashes.len(), 2);
        visited_building(99, 99, 101, 101);
        assert_eq!(snapshot_state().unwrap().possible_stashes.len(), 1);
        reset();
    }
}
