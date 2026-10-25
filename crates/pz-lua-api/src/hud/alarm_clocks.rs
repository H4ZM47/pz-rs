//! [`HudAlarmClocks`] over the local players' items: the `AlarmClock` /
//! `AlarmClockClothing` items among `players[i].getWornItems()` and
//! `getInventory().getItems()`, their state ([`pz_inventory::alarm`]) and
//! the clock's `stopRinging` / `setAlarmSet`; and
//! [`update_player_alarm_clocks`], the `update()` the cell runs on the
//! players' set alarms.

use super::host::{with_host, AlarmClockItem, AlarmClockRef, HudAlarmClocks};
use crate::bind_items::ItemRef;
use crate::bind_players;
use pz_inventory::alarm::{alarm, alarm_mut, is_digital, AlarmEffects, AlarmTime};
use pz_inventory::item::ItemKind;
use pz_inventory::InventoryItem;

fn is_alarm_clock(it: &InventoryItem) -> bool {
    matches!(
        it.kind,
        ItemKind::AlarmClock(_) | ItemKind::AlarmClockClothing(..)
    )
}

/// The display name the item's constructor got (`Item.displayName`, the
/// translated item name).
fn constructor_name(it: &InventoryItem) -> String {
    let full = it.get_full_type();
    crate::bind_text::with_translator_mut(|t| {
        t.get_item_name_from_full_type(&full, |_| Some(it.template.display_name.clone()))
    })
    .unwrap_or_else(|| it.template.display_name.clone())
}

/// A player's worn alarm clocks and the alarm clocks in their inventory,
/// each with `isWorn()` / `isEquipped()`.
fn clocks(player: usize) -> (Vec<ItemRef>, Vec<(ItemRef, bool, bool)>) {
    let Some(p) = bind_players::player(player) else {
        return (Vec::new(), Vec::new());
    };
    let p = p.borrow();
    let worn = p
        .items
        .worn()
        .filter(|i| is_alarm_clock(&i.borrow()))
        .collect();
    let inv = p
        .items
        .inventory_items()
        .into_iter()
        .filter(|i| is_alarm_clock(&i.borrow()))
        .map(|i| {
            let id = i.borrow().id;
            let w = p.items.is_worn(id);
            let e = p.items.is_equipped(id);
            (i, w, e)
        })
        .collect();
    (worn, inv)
}

/// `isDigital()` of an alarm clock item.
pub fn alarm_clock_is_digital(it: &InventoryItem) -> bool {
    is_digital(it, &constructor_name(it))
}

fn view(it: &InventoryItem, worn: bool, equipped: bool) -> AlarmClockItem {
    let a = alarm(it).cloned().unwrap_or_default();
    AlarmClockItem {
        clothing: matches!(it.kind, ItemKind::AlarmClockClothing(..)),
        digital_tag: it.has_tag("base:digital"),
        is_digital: alarm_clock_is_digital(it),
        alarm_set: a.alarm_set,
        ringing: a.is_ringing(),
        worn,
        equipped,
    }
}

/// `GameTime.getMinutes()` of the shared `GameTime`.
fn game_minutes() -> i32 {
    let tod = crate::bind_climate::shared_game_time().borrow().time_of_day;
    AlarmTime::from_time_of_day(tod, 0.0).minutes
}

fn find(player: usize, r: AlarmClockRef) -> Option<ItemRef> {
    let (worn, inv) = clocks(player);
    match r {
        AlarmClockRef::Worn(j) => worn.get(j).cloned(),
        AlarmClockRef::Inventory(j) => inv.get(j).map(|(i, ..)| i.clone()),
    }
}

/// The local players' alarm clocks.
pub struct PlayerAlarmClocks;

impl HudAlarmClocks for PlayerAlarmClocks {
    fn worn_clocks(&self, player: usize) -> Vec<AlarmClockItem> {
        clocks(player)
            .0
            .iter()
            .map(|i| view(&i.borrow(), true, true))
            .collect()
    }
    fn inventory_clocks(&self, player: usize) -> Vec<AlarmClockItem> {
        clocks(player)
            .1
            .iter()
            .map(|(i, w, e)| view(&i.borrow(), *w, *e))
            .collect()
    }
    fn stop_ringing(&mut self, player: usize, r: AlarmClockRef) {
        if let Some(i) = find(player, r) {
            let m = game_minutes();
            if let Some(a) = alarm_mut(&mut i.borrow_mut()) {
                a.stop_ringing(m);
            }
        }
    }
    fn set_alarm_set(&mut self, player: usize, r: AlarmClockRef, set: bool) {
        // `syncAlarmClock()` sends packets in multiplayer only.
        // TODO(network): SyncPlayerAlarmClock.
        if let Some(i) = find(player, r) {
            let m = game_minutes();
            if let Some(a) = alarm_mut(&mut i.borrow_mut()) {
                a.set_alarm_set(set, m);
            }
        }
    }
}

/// What [`update_player_alarm_clocks`] did to one clock.
pub struct AlarmUpdate {
    pub item: ItemRef,
    pub effects: AlarmEffects,
}

/// `AlarmClock.update()` / `AlarmClockClothing.update()` on the set alarm
/// clocks the local players carry (in their inventory or its bags), at
/// the shared `GameTime`, with the player's square as the alarm square.
/// The effects (world sound, item sounds) come back for the world and
/// sound threads. (Clocks on the ground are the cell's `processItems`;
/// TODO(world): world items.)
pub fn update_player_alarm_clocks(world_age_hours: f64) -> Vec<AlarmUpdate> {
    let tod = crate::bind_climate::shared_game_time().borrow().time_of_day;
    let (server, client) = {
        let core = crate::bind_core::shared_core();
        let c = core.borrow();
        (c.is_server, c.is_client)
    };
    let mut now = AlarmTime::from_time_of_day(tod, world_age_hours);
    now.server = server;
    now.client = client;
    now.multiplayer = server || client;
    let mut out = Vec::new();
    for i in 0..bind_players::MAX_PLAYERS {
        let Some(p) = bind_players::player(i) else {
            continue;
        };
        let mut clocks = Vec::new();
        collect(&p.borrow().items.inventory_items(), &mut clocks);
        for it in clocks {
            let e = {
                let mut b = it.borrow_mut();
                match alarm_mut(&mut b) {
                    Some(a) if a.should_update() => Some(a.update(&now, true)),
                    _ => None,
                }
            };
            if let Some(effects) = e {
                out.push(AlarmUpdate { item: it, effects });
            }
        }
    }
    out
}

/// The alarm clocks among `items` and in the bags among them.
fn collect(items: &[ItemRef], out: &mut Vec<ItemRef>) {
    for i in items {
        let inner = {
            let b = i.borrow();
            if is_alarm_clock(&b) {
                out.push(i.clone());
            }
            match &b.kind {
                ItemKind::Container(c) => Some(c.container.borrow().items.clone()),
                _ => None,
            }
        };
        if let Some(inner) = inner {
            collect(&inner, out);
        }
    }
}

/// Installs [`PlayerAlarmClocks`] in the HUD host.
pub fn install_alarm_clock_host() {
    with_host(|h| h.alarm_clocks = Box::new(PlayerAlarmClocks));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::LuaGameCharacter;
    use pz_characters::CharacterKind;
    use pz_inventory::{ItemClass, ItemTemplate};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn clock(class: ItemClass, ty: &str, id: i32) -> InventoryItem {
        let mut i = InventoryItem::create(Rc::new(ItemTemplate::new(class, "Base", ty)), 1);
        i.id = id;
        i
    }

    #[test]
    fn players_clocks_set_ring_and_stop() {
        bind_players::reset();
        let sm = pz_scripts::ScriptManager::new();
        let p = Rc::new(RefCell::new(LuaGameCharacter::new(
            &sm,
            CharacterKind::Player,
        )));
        let watch = p.borrow().items.add_item(clock(
            ItemClass::AlarmClockClothing,
            "WristWatch_Classic",
            7,
        ));
        let clock8 = p
            .borrow()
            .items
            .add_item(clock(ItemClass::AlarmClock, "AlarmClock2", 8));
        p.borrow()
            .items
            .add_item(clock(ItemClass::Normal, "Plank", 9));
        p.borrow_mut().items.wear_in_order(&[7]);
        bind_players::set_player(0, Some(p.clone()));
        let mut h = PlayerAlarmClocks;
        let worn = h.worn_clocks(0);
        assert_eq!(worn.len(), 1);
        assert!(worn[0].clothing && !worn[0].is_digital && worn[0].worn);
        let inv = h.inventory_clocks(0);
        assert_eq!(inv.len(), 2);
        assert!(inv[0].worn && !inv[1].worn && !inv[1].equipped && inv[1].is_digital);

        // 7:30, then the clock in the inventory rings at that minute.
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .time_of_day = 7.2;
        {
            let mut a = clock8.borrow_mut();
            let d = alarm_mut(&mut a).unwrap();
            d.set_hour(7);
            d.set_minute(30);
        }
        h.set_alarm_set(0, AlarmClockRef::Inventory(1), true);
        assert!(h.inventory_clocks(0)[1].alarm_set);
        assert_eq!(alarm(&clock8.borrow()).unwrap().force_dont_ring, 10);
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .time_of_day = 7.5;
        let u = update_player_alarm_clocks(48.0);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].item.borrow().id, 8);
        assert!(u[0].effects.started_ringing && u[0].effects.world_sound);
        assert!(h.inventory_clocks(0)[1].ringing);
        h.stop_ringing(0, AlarmClockRef::Inventory(1));
        assert!(!h.inventory_clocks(0)[1].ringing);
        assert_eq!(alarm(&clock8.borrow()).unwrap().force_dont_ring, 30);
        assert!(!alarm(&watch.borrow()).unwrap().alarm_set);
        bind_players::reset();
    }
}
