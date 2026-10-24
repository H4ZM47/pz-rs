//! `VehiclePart`, `VehicleDoor`, `VehicleWindow` and `VehicleLight`.
//!
//! A part is shared ([`PartRef`]) and keeps its state behind a `RefCell`
//! that is only borrowed for the duration of one field access, so Lua
//! callbacks may re-enter the part and its vehicle at any point (as in the
//! original, where parts are plain Java objects).

use crate::engine::VehicleEngine;
use crate::env;
use crate::items::{self, ItemRef};
use crate::jmath::{lerp, Vector3f};
use crate::rand;
use crate::vehicle::BaseVehicle;
use pz_inventory::device::DeviceData;
use pz_inventory::{ComponentBlob, ItemContainer, SavedItem};
use pz_kahlua::{TableRef, Value};
use pz_scripts::objects::vehicles::{Model, Part as ScriptPart};
use std::cell::{Ref, RefCell, RefMut};
use std::rc::{Rc, Weak};

pub type PartRef = Rc<VehiclePart>;

/// `VehicleLight`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleLight {
    pub active: bool,
    pub offset: Vector3f,
    pub dist: f32,
    pub intensity: f32,
    pub dot: f32,
    pub focusing: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Default for VehicleLight {
    fn default() -> Self {
        VehicleLight {
            active: false,
            offset: Vector3f::default(),
            dist: 16.0,
            intensity: 1.0,
            dot: 0.96,
            focusing: 0,
            r: 1.0,
            g: 0.823_529_4,
            b: 0.705_882_4,
        }
    }
}

/// `VehicleDoor`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleDoor {
    pub open: bool,
    pub locked: bool,
    pub lock_broken: bool,
}

/// `VehicleWindow` (its health is the part's condition).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleWindow {
    pub openable: bool,
    pub open: bool,
    pub open_delta: f32,
}

/// The item of a part: decoded, or a record the item codec keeps as
/// bytes (it is there, but its values cannot be read).
#[derive(Clone, Debug)]
pub enum PartItem {
    Item(ItemRef),
    Raw(SavedItem),
}

/// `VehiclePart` state.
#[derive(Debug, Default)]
pub struct PartState {
    pub created: bool,
    pub part_id: Option<String>,
    pub script_part: Option<Rc<ScriptPart>>,
    pub container: Option<ItemContainer>,
    pub item: Option<PartItem>,
    pub mod_data: Option<TableRef>,
    pub last_updated: f32,
    pub update_flags: i16,
    pub parent: Option<Weak<VehiclePart>>,
    pub children: Vec<Weak<VehiclePart>>,
    pub door: Option<VehicleDoor>,
    pub engine: Option<VehicleEngine>,
    pub window: Option<VehicleWindow>,
    pub category: Option<String>,
    pub condition: i32,
    pub specific_item: bool,
    pub wheel_friction: f32,
    pub mechanic_skill_installer: i32,
    pub suspension_damping: f32,
    pub suspension_compression: f32,
    pub engine_loudness: f32,
    pub durability: f32,
    pub light: Option<VehicleLight>,
    pub device_data: Option<DeviceData>,
    pub has_player_in_range: bool,
    /// ECS components as saved (`GameEntity.saveEntity`).
    pub components: Vec<ComponentBlob>,
}

/// `VehiclePart`.
#[derive(Debug)]
pub struct VehiclePart {
    vehicle: Weak<BaseVehicle>,
    st: RefCell<PartState>,
    pub(crate) lua: RefCell<Option<Weak<dyn pz_kahlua::value::LuaObject>>>,
}

/// `VehiclePart.getNumberByCondition`.
pub fn number_by_condition(number: f32, cond: f32, min: f32) -> f32 {
    let cond = cond + 20.0 * (100.0 - cond) / 100.0;
    let cond_delta = cond / 100.0;
    crate::jmath::java_round_f(min.max(number * cond_delta) * 100.0) as f32 / 100.0
}

impl VehiclePart {
    /// `new VehiclePart(vehicle)`.
    pub fn new(vehicle: &Rc<BaseVehicle>) -> PartRef {
        Rc::new(VehiclePart {
            vehicle: Rc::downgrade(vehicle),
            st: RefCell::new(PartState {
                last_updated: -1.0,
                condition: -1,
                specific_item: true,
                ..Default::default()
            }),
            lua: RefCell::new(None),
        })
    }

    pub(crate) fn new_weak(vehicle: Weak<BaseVehicle>) -> PartRef {
        Rc::new(VehiclePart {
            vehicle,
            st: RefCell::new(PartState {
                last_updated: -1.0,
                condition: -1,
                specific_item: true,
                ..Default::default()
            }),
            lua: RefCell::new(None),
        })
    }

    pub fn st(&self) -> Ref<'_, PartState> {
        self.st.borrow()
    }

    pub fn st_mut(&self) -> RefMut<'_, PartState> {
        self.st.borrow_mut()
    }

    /// `getVehicle()` / `getOwner()`.
    pub fn vehicle(&self) -> Rc<BaseVehicle> {
        self.vehicle
            .upgrade()
            .expect("vehicle part outlived its vehicle")
    }

    pub fn vehicle_weak(&self) -> &Weak<BaseVehicle> {
        &self.vehicle
    }

    /// `getId()`.
    pub fn id(&self) -> Option<String> {
        let st = self.st();
        match &st.script_part {
            Some(sp) => sp.id.clone(),
            None => st.part_id.clone(),
        }
    }

    /// `getId()` compared to a string.
    pub fn id_is(&self, id: &str) -> bool {
        let st = self.st();
        match &st.script_part {
            Some(sp) => sp.id.as_deref() == Some(id),
            None => st.part_id.as_deref() == Some(id),
        }
    }

    /// `getScriptPart()`.
    pub fn script_part(&self) -> Option<Rc<ScriptPart>> {
        self.st().script_part.clone()
    }

    /// `getArea()`.
    pub fn area(&self) -> Option<String> {
        self.st().script_part.as_ref().and_then(|p| p.area.clone())
    }

    /// `getItemType()`.
    pub fn item_type(&self) -> Option<Vec<String>> {
        self.st()
            .script_part
            .as_ref()
            .and_then(|p| p.item_type.clone())
    }

    /// `getLuaFunction(name)`.
    pub fn lua_function(&self, name: &str) -> Option<String> {
        let st = self.st();
        let sp = st.script_part.as_ref()?;
        sp.lua_functions
            .as_ref()?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    /// `getTable(id)`: a copy of the script table.
    pub fn table(&self, id: &str) -> Option<TableRef> {
        let st = self.st();
        let sp = st.script_part.as_ref()?;
        let t = sp
            .tables
            .as_ref()?
            .iter()
            .find(|(k, _)| k.as_deref() == Some(id))
            .map(|(_, t)| t.clone())?;
        Some(crate::lua::script_table_to_lua(&t))
    }

    /// `hasModData()`.
    pub fn has_mod_data(&self) -> bool {
        self.st().mod_data.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// `getModData()` (created on first use).
    pub fn mod_data(&self) -> TableRef {
        let mut st = self.st_mut();
        st.mod_data.get_or_insert_with(TableRef::new).clone()
    }

    /// `getInventoryItem()` when decoded.
    pub fn item(&self) -> Option<ItemRef> {
        match &self.st().item {
            Some(PartItem::Item(i)) => Some(i.clone()),
            _ => None,
        }
    }

    /// `getInventoryItem() != null`.
    pub fn has_item(&self) -> bool {
        self.st().item.is_some()
    }

    /// `getCondition()`.
    pub fn condition(&self) -> i32 {
        self.st().condition
    }

    /// `isContainer()`.
    pub fn is_container(&self) -> bool {
        self.st()
            .script_part
            .as_ref()
            .is_some_and(|p| p.container.is_some())
    }

    /// `getContainerSeatNumber()`.
    pub fn container_seat_number(&self) -> i32 {
        self.st()
            .script_part
            .as_ref()
            .and_then(|p| p.container.as_ref())
            .map_or(-1, |c| c.seat)
    }

    /// `getContainerContentType()`.
    pub fn container_content_type(&self) -> Option<String> {
        self.st()
            .script_part
            .as_ref()
            .and_then(|p| p.container.as_ref())
            .and_then(|c| c.content_type.clone())
    }

    /// `isSeat()`.
    pub fn is_seat(&self) -> bool {
        let st = self.st();
        let Some(c) = st.script_part.as_ref().and_then(|p| p.container.as_ref()) else {
            return false;
        };
        match &c.seat_id {
            Some(s) if !s.is_empty() => c.seat > -1,
            _ => false,
        }
    }

    /// `isInventoryItemUninstalled()`.
    pub fn is_inventory_item_uninstalled(&self) -> bool {
        self.item_type().is_some_and(|t| !t.is_empty()) && !self.has_item()
    }

    /// `getContainerContentAmount()`.
    pub fn container_content_amount(&self) -> f32 {
        if !self.is_container() {
            return 0.0;
        }
        if self.has_mod_data() {
            if let Value::Num(d) = self.mod_data().get_str("contentAmount") {
                return d as f32;
            }
        }
        0.0
    }

    /// `getContainerCapacity()` (no character).
    pub fn container_capacity(&self) -> i32 {
        if !self.is_container() {
            return 0;
        }
        let occupied = self.is_occupied_seat();
        let st = self.st();
        if let Some(c) = &st.container {
            return items::container_capacity(c, occupied);
        }
        let sp = st.script_part.as_ref().unwrap();
        let sc = sp.container.as_ref().unwrap();
        if let Some(PartItem::Item(item)) = &st.item {
            let item = item.borrow();
            if sc.condition_affects_capacity {
                return number_by_condition(item.max_capacity as f32, st.condition as f32, 5.0)
                    as i32;
            }
            return item.max_capacity;
        }
        sc.capacity
    }

    fn is_occupied_seat(&self) -> bool {
        let seat = self.container_seat_number();
        if seat < 0 || !self.is_seat() {
            return false;
        }
        self.vehicle
            .upgrade()
            .is_some_and(|v| v.character(seat).is_some())
    }

    /// `setContainerCapacity(cap)`.
    pub fn set_container_capacity(&self, cap: i32) {
        if !self.is_container() {
            return;
        }
        if let Some(c) = self.st_mut().container.as_mut() {
            c.capacity = cap;
        }
    }

    /// `setContainerContentAmount(amount, force, noUpdateMass)`.
    pub fn set_container_content_amount(&self, amount: f32, force: bool, no_update_mass: bool) {
        if !self.is_container() {
            return;
        }
        let mut cap = self
            .st()
            .script_part
            .as_ref()
            .unwrap()
            .container
            .as_ref()
            .unwrap()
            .capacity;
        let item = self.item();
        if let Some(i) = &item {
            cap = i.borrow().max_capacity;
        }
        let mut amount = amount;
        if !force {
            amount = amount.min(cap as f32);
        }
        amount = amount.max(0.0);
        self.mod_data()
            .set_str("contentAmount", Value::Num(amount as f64));
        if let Some(i) = &item {
            i.borrow_mut().item_capacity = amount;
        }
        if !no_update_mass {
            self.vehicle().update_total_mass();
        }
    }

    /// `getWheelIndex()`.
    pub fn wheel_index(&self) -> i32 {
        let wheel = match self.st().script_part.as_ref().and_then(|p| p.wheel.clone()) {
            Some(w) => w,
            None => return -1,
        };
        let v = self.vehicle();
        let script = match v.script() {
            Some(s) => s,
            None => return -1,
        };
        let s = script.borrow();
        for i in 0..s.wheel_count() {
            if s.wheel(i).id.as_deref() == Some(wheel.as_str()) {
                return i as i32;
            }
        }
        -1
    }

    /// `getParent()`.
    pub fn parent(&self) -> Option<PartRef> {
        self.st().parent.as_ref().and_then(Weak::upgrade)
    }

    /// `getChildCount()`.
    pub fn child_count(&self) -> usize {
        self.st().children.len()
    }

    /// `getChild(i)`.
    pub fn child(&self, i: usize) -> Option<PartRef> {
        self.st().children.get(i).and_then(Weak::upgrade)
    }

    /// `getChildWindow()`.
    pub fn child_window(&self) -> Option<PartRef> {
        (0..self.child_count())
            .filter_map(|i| self.child(i))
            .find(|c| c.st().window.is_some())
    }

    /// `getEnclosingDoor()` (whether there is one).
    pub fn has_enclosing_door(&self) -> bool {
        if self.st().door.is_some() {
            return true;
        }
        self.parent().is_some_and(|p| p.has_enclosing_door())
    }

    /// `createSpotLight(...)`.
    pub fn create_spot_light(
        &self,
        x_offset: f32,
        y_offset: f32,
        dist: f32,
        intensity: f32,
        dot: f32,
        focusing: i32,
    ) {
        let mut st = self.st_mut();
        let l = st.light.get_or_insert_with(VehicleLight::default);
        l.offset = Vector3f::new(x_offset, y_offset, 0.0);
        l.dist = dist;
        l.intensity = intensity;
        l.dot = dot;
        l.focusing = focusing;
    }

    /// `createSpotLightColor(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_spot_light_color(
        &self,
        x_offset: f32,
        y_offset: f32,
        dist: f32,
        intensity: f32,
        dot: f32,
        focusing: i32,
        r: f32,
        g: f32,
        b: f32,
    ) {
        self.create_spot_light(x_offset, y_offset, dist, intensity, dot, focusing);
        let mut st = self.st_mut();
        let l = st.light.as_mut().unwrap();
        l.r = r;
        l.g = g;
        l.b = b;
    }

    /// `getLightDistance()`.
    pub fn light_distance(&self) -> f32 {
        let st = self.st();
        st.light.as_ref().map_or(0.0, |l| {
            lerp(l.dist * 0.5, l.dist, st.condition as f32 / 100.0)
        })
    }

    /// `getLightIntensity()`.
    pub fn light_intensity(&self) -> f32 {
        let st = self.st();
        st.light.as_ref().map_or(0.0, |l| {
            lerp(l.intensity * 0.5, l.intensity, st.condition as f32 / 100.0)
        })
    }

    /// `getLightFocusing()`.
    pub fn light_focusing(&self) -> f32 {
        let st = self.st();
        if st.light.is_none() {
            return 0.0;
        }
        (20 + (80.0 * (1.0 - st.condition as f32 / 100.0)) as i32) as f32
    }

    /// `setLightActive(active)`.
    pub fn set_light_active(&self, active: bool) {
        let mut st = self.st_mut();
        if let Some(l) = st.light.as_mut() {
            if l.active != active {
                l.active = active;
            }
        }
    }

    /// `createSignalDevice()`.
    pub fn create_signal_device(&self) {
        let mut st = self.st_mut();
        if st.device_data.is_none() {
            st.device_data = Some(DeviceData::default());
        }
    }

    /// `setCondition(condition)`.
    pub fn set_condition(&self, condition: i32) {
        let condition = condition.min(100).max(0);
        let v = self.vehicle();
        if v.driver_regardless_of_tow().is_some() {
            let old = self.condition();
            if (old > 60 && condition < 60 && condition > 40) || (old > 40 && condition < 40) {
                env::events().event("OnVehicleDamageTexture");
            }
        }
        self.st_mut().condition = condition;
        if let Some(item) = self.item() {
            items::set_condition(&mut item.borrow_mut(), condition);
        }
        v.update_damage_overlay_later();
        if condition <= 0 && self.id_is("lightbar") {
            v.set_lightbar_siren_mode(0);
        }
        let trailer = self
            .st()
            .script_part
            .as_ref()
            .is_some_and(|p| p.id.as_deref() == Some("TrailerTrunk"));
        if trailer {
            if let Some(c) = self.st_mut().container.as_mut() {
                c.capacity = 80.max(condition);
            }
        }
    }

    /// `damage(amount)`.
    pub fn damage(&self, amount: i32) {
        if self.st().window.is_some() {
            self.window_damage(amount);
            return;
        }
        self.set_condition(self.condition() - amount);
    }

    /// `VehicleWindow.isHittable()`.
    pub fn window_is_hittable(&self) -> bool {
        let st = self.st();
        let Some(w) = st.window else { return false };
        if st.condition == 0 || w.open {
            return false;
        }
        drop(st);
        self.item_type().is_none() || self.has_item()
    }

    /// `VehicleWindow.damage(amount)`.
    pub fn window_damage(&self, amount: i32) {
        if amount <= 0 || !self.window_is_hittable() {
            return;
        }
        let v = self.vehicle();
        if v.is_alarmed() {
            v.trigger_alarm();
        }
        self.set_condition(self.condition() - amount);
        if self.condition() == 0 {
            if self.has_item() {
                self.set_inventory_item(None, 0);
            }
            env::events().event("VehicleWindowSmash");
        }
    }

    /// `VehicleWindow.setOpen(open)`.
    pub fn set_window_open(&self, open: bool) {
        if let Some(w) = self.st_mut().window.as_mut() {
            w.open = open;
        }
        self.vehicle().update_damage_overlay_later();
    }

    /// `setInventoryItem(item, mechanicSkill)`.
    pub fn set_inventory_item(&self, item: Option<PartItem>, mechanic_skill: i32) {
        self.st_mut().item = item.clone();
        self.do_inventory_item_stats(item.as_ref(), mechanic_skill);
        let v = self.vehicle();
        v.update_total_mass();
        v.update_damage_overlay_later();
        if self.is_set_all_models_visible() {
            self.set_all_models_visible(item.is_some());
        }
        v.update_part_stats();
        v.update_bullet_stats();
    }

    /// `isSetAllModelsVisible()`.
    pub fn is_set_all_models_visible(&self) -> bool {
        self.st()
            .script_part
            .as_ref()
            .is_some_and(|p| p.set_all_models_visible)
    }

    /// `setAllModelsVisible(visible)`.
    pub fn set_all_models_visible(&self, visible: bool) {
        let models: Vec<Model> = match self
            .st()
            .script_part
            .as_ref()
            .and_then(|p| p.models.clone())
        {
            Some(m) if !m.is_empty() => m,
            _ => return,
        };
        let me = self.vehicle().part_ref(self);
        if let Some(me) = me {
            for (i, m) in models.iter().enumerate() {
                self.vehicle().set_model_visible(&me, i, m, visible);
            }
        }
    }

    /// `setModelVisible(id, visible)`.
    pub fn set_model_visible(&self, id: &str, visible: bool) {
        let models = match self
            .st()
            .script_part
            .as_ref()
            .and_then(|p| p.models.clone())
        {
            Some(m) => m,
            None => return,
        };
        let Some((i, m)) = models
            .iter()
            .enumerate()
            .find(|(_, m)| m.id.as_deref() == Some(id))
        else {
            return;
        };
        if let Some(me) = self.vehicle().part_ref(self) {
            self.vehicle().set_model_visible(&me, i, m, visible);
        }
    }

    /// `doInventoryItemStats(item, mechanicSkill)`.
    pub fn do_inventory_item_stats(&self, new_item: Option<&PartItem>, mechanic_skill: i32) {
        match new_item {
            Some(PartItem::Item(item)) => {
                let (max_capacity, condition, item_capacity) = {
                    let i = item.borrow();
                    (i.max_capacity, i.condition, i.item_capacity)
                };
                let s = items::stats(&item.borrow());
                if self.is_container() {
                    let affects = self
                        .st()
                        .script_part
                        .as_ref()
                        .unwrap()
                        .container
                        .as_ref()
                        .unwrap()
                        .condition_affects_capacity;
                    if max_capacity > 0 && affects {
                        self.set_container_capacity(number_by_condition(
                            max_capacity as f32,
                            condition as f32,
                            5.0,
                        ) as i32);
                    } else if max_capacity > 0 {
                        self.set_container_capacity(max_capacity);
                    }
                    self.set_container_content_amount(item_capacity, false, false);
                }
                {
                    let mut st = self.st_mut();
                    st.suspension_compression =
                        number_by_condition(s.suspension_compression, condition as f32, 0.6);
                    st.suspension_damping =
                        number_by_condition(s.suspension_damping, condition as f32, 0.6);
                    if s.engine_loudness > 0.0 {
                        st.engine_loudness =
                            number_by_condition(s.engine_loudness, condition as f32, 10.0);
                    }
                }
                self.set_condition(condition);
                let mut st = self.st_mut();
                st.mechanic_skill_installer = mechanic_skill;
                if s.durability > 0.0 {
                    st.durability = s.durability;
                }
            }
            Some(PartItem::Raw(_)) => {}
            None => {
                let cap = self
                    .st()
                    .script_part
                    .as_ref()
                    .and_then(|p| p.container.as_ref())
                    .map(|c| c.capacity);
                if let Some(cap) = cap {
                    self.set_container_capacity(if cap > 0 { cap } else { 0 });
                }
                self.st_mut().mechanic_skill_installer = 0;
                self.set_container_content_amount(0.0, false, false);
                let mut st = self.st_mut();
                st.suspension_compression = 0.0;
                st.suspension_damping = 0.0;
                st.wheel_friction = 0.0;
                st.engine_loudness = 0.0;
            }
        }
    }

    /// `setRandomCondition(item)`.
    pub fn set_random_condition(&self, item: Option<&ItemRef>) {
        let v = self.vehicle();
        let vtype = crate::vehicle_type::type_from_name(&v.vehicle_type());
        if v.is_good_car() {
            let max = item.map_or(100, |i| i.borrow().condition_max);
            self.set_condition(rand::next_range(max - max / 3, max));
            if let Some(i) = item {
                items::set_condition(&mut i.borrow_mut(), self.condition());
            }
            return;
        }
        let mut max = item.map_or(100, |i| i.borrow().condition_max);
        if let Some(t) = &vtype {
            max = crate::jmath::fastfloor(max as f32 * t.random_base_vehicle_quality());
        }
        let mut cond = 100.0f32;
        if let Some(i) = item {
            let mut chance = items::stats(&i.borrow()).chance_to_spawn_damaged;
            if let Some(t) = &vtype {
                chance += t.chance_to_part_damage;
            }
            if chance > 0 && rand::next(100) < chance {
                cond = rand::next_range(max - max / 2, max) as f32;
            }
        } else {
            let mut chance = 30;
            if let Some(t) = &vtype {
                chance += t.chance_to_part_damage;
            }
            if rand::next(100) < chance {
                cond = rand::next_float(max as f32 * 0.5, max as f32);
            }
        }
        cond = apply_general_condition(cond);
        self.set_condition(cond as i32);
        if let Some(i) = item {
            items::set_condition(&mut i.borrow_mut(), self.condition());
        }
    }

    /// `setGeneralCondition(item, baseQuality, chanceToSpawnDamaged)`.
    pub fn set_general_condition(
        &self,
        item: Option<&ItemRef>,
        base_quality: f32,
        chance_to_spawn_damaged: f32,
    ) {
        let max = (100.0 * base_quality) as i32;
        let mut cond = 100.0f32;
        if let Some(i) = item {
            let mut tot = items::stats(&i.borrow()).chance_to_spawn_damaged;
            tot += chance_to_spawn_damaged as i32;
            if tot > 0 && rand::next(100) < tot {
                cond = rand::next_range(max - max / 2, max) as f32;
            }
        } else {
            let tot = 30 + chance_to_spawn_damaged as i32;
            if rand::next(100) < tot {
                cond = rand::next_float(max as f32 * 0.5, max as f32);
            }
        }
        cond = apply_general_condition(cond);
        self.set_condition(cond as i32);
        if let Some(i) = item {
            items::set_condition(&mut i.borrow_mut(), self.condition());
        }
    }

    /// `repair()`.
    pub fn repair(self: &Rc<Self>) {
        let v = self.vehicle();
        let content_amount = self.container_content_amount();
        if self.is_inventory_item_uninstalled() {
            let types = self.item_type().unwrap_or_default();
            let t = types[rand::next(types.len() as i32) as usize].clone();
            if !t.is_empty() {
                if let Some(item) = env::items().create_item(&t) {
                    let item = Rc::new(RefCell::new(item));
                    self.set_inventory_item(Some(PartItem::Item(item.clone())), 0);
                    let mc = item.borrow().max_capacity;
                    if mc > 0 {
                        item.borrow_mut().item_capacity = mc as f32;
                    }
                    if let Some(f) = self.lua_function("init") {
                        crate::lua::call_void(
                            &f,
                            &[
                                crate::lua::LuaArg::Vehicle(v.clone()),
                                crate::lua::LuaArg::Part(self.clone()),
                            ],
                        );
                    }
                }
            }
        }
        if let Some(d) = self.st_mut().door.as_mut() {
            if d.lock_broken {
                d.lock_broken = false;
            }
        }
        if self.condition() != 100 {
            self.set_condition(100);
            let item = self.st().item.clone();
            if item.is_some() {
                let skill = self.st().mechanic_skill_installer;
                self.do_inventory_item_stats(item.as_ref(), skill);
            }
        }
        if self.is_container()
            && self.st().container.is_none()
            && content_amount != self.container_capacity() as f32
        {
            self.set_container_content_amount(self.container_capacity() as f32, false, false);
        }
        if let Some(item) = self.item() {
            let drainable = items::is_drainable(&item.borrow());
            if drainable && items::current_uses_float(&item.borrow()) < 1.0 {
                let mut i = item.borrow_mut();
                i.uses = items::max_uses(&i);
                items::update_weight(&mut i);
            }
        }
        if self.id().is_some_and(|i| i.eq_ignore_ascii_case("Engine")) {
            let (loudness, power) = match v.script() {
                Some(s) => {
                    let s = s.borrow();
                    (
                        (s.engine_loudness() as f64 * env::sandbox().zombie_attraction_multiplier)
                            as i32,
                        s.engine_force() as i32,
                    )
                }
                None => (0, 0),
            };
            v.set_engine_feature(100, loudness, power);
        }
        v.update_part_stats();
        v.update_bullet_stats();
    }
}

/// The `carGeneralCondition` adjustment and clamp shared by
/// `setRandomCondition` and `setGeneralCondition`.
fn apply_general_condition(mut cond: f32) -> f32 {
    match env::sandbox().car_general_condition {
        1 => {
            let hi = rand::next_float(cond * 0.3, cond * 0.9);
            cond -= rand::next_float(cond * 0.3, hi);
        }
        2 => cond -= rand::next_float(cond * 0.1, cond * 0.3),
        4 => cond += rand::next_float(cond * 0.2, cond * 0.4),
        5 => cond += rand::next_float(cond * 0.5, cond * 0.9),
        _ => {}
    }
    cond = cond.max(0.0);
    cond.min(100.0)
}
