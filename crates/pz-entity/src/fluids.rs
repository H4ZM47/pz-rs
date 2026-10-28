//! `zombie.entity.components.fluids`: the `Fluid` registry and the saved
//! forms of `FluidInstance`, `FluidFilter`, `FluidContainer`, `FluidSample`,
//! `FluidConsume` and `SealedFluidProperties`.

use std::collections::HashMap;

use pz_classfile::Jar;
use pz_scripts::objects::definitions::{FluidCategory, FluidDefinitionScript, FluidType};
use pz_scripts::{ScriptManager, ScriptType};

use crate::context::{EntityContext, FluidInfo, Lookup};
use crate::io::{
    pack_unit, read_header, unpack_unit, HeaderSize, HeaderWrite, Reader, Result, Writer,
};
use crate::java;

/// `FluidContainer.DEF_CONTAINER_NAME`.
pub const DEF_CONTAINER_NAME: &str = "FluidContainer";

/// `FluidType.FromId`.
pub fn fluid_type_from_id(id: i8) -> Option<FluidType> {
    FluidType::ALL.iter().copied().find(|t| t.id() == id)
}

/// `FluidCategory.FromId`.
pub fn fluid_category_from_id(id: i8) -> Option<FluidCategory> {
    FluidCategory::ALL.iter().copied().find(|t| t.id() == id)
}

/// `Fluid`'s registries after `Fluid.Init`: `fluidEnumMap` (by type id) and
/// `fluidStringMap` (by name, enum fluids included).
#[derive(Clone, Debug, Default)]
pub struct FluidRegistry {
    by_id: HashMap<i8, FluidInfo>,
    by_name: HashMap<String, FluidInfo>,
    // Absent metadata means unknown, distinct from a known native null.
    properties_by_name: HashMap<String, Option<SealedFluidProperties>>,
}

impl FluidRegistry {
    /// `Fluid.Init` over the fluid definition scripts. `enum_fluids` are the
    /// `FluidType` names `Fluid.<clinit>` creates fluids for
    /// ([`crate::tables::enum_fluid_names`]); `defs` are
    /// `(fluidType, fluidTypeString, color)` per definition script, in load
    /// order. An enum fluid without a script keeps its white default.
    pub fn from_definitions<'a>(
        enum_fluids: &[String],
        defs: impl IntoIterator<Item = (FluidType, Option<&'a str>, [f32; 4], Vec<FluidCategory>)>,
    ) -> FluidRegistry {
        let mut reg = FluidRegistry::default();
        let mut enum_info: HashMap<String, ([f32; 4], Vec<FluidCategory>)> = enum_fluids
            .iter()
            .map(|n| (n.clone(), ([1.0, 1.0, 1.0, 1.0], Vec::new())))
            .collect();
        for (ty, type_string, color, categories) in defs {
            if ty == FluidType::Modded {
                if let Some(s) = type_string {
                    reg.by_name.insert(
                        s.to_string(),
                        FluidInfo {
                            name: s.to_string(),
                            color,
                            categories,
                            ..Default::default()
                        },
                    );
                }
            } else if let Some(c) = enum_info.get_mut(ty.name()) {
                *c = (color, categories);
            }
        }
        for name in enum_fluids {
            let (color, categories) = enum_info[name].clone();
            let info = FluidInfo {
                name: name.clone(),
                color,
                categories,
                ..Default::default()
            };
            if let Ok(t) = FluidType::value_of(name) {
                reg.by_id.insert(t.id(), info.clone());
            }
            reg.by_name.insert(name.clone(), info);
        }
        reg
    }

    /// [`from_definitions`](Self::from_definitions) with the enum fluids from
    /// the jar and the definitions a loaded [`ScriptManager`] holds.
    pub fn from_scripts(jar: &Jar, sm: &ScriptManager) -> pz_classfile::Result<FluidRegistry> {
        let enum_fluids = crate::tables::enum_fluid_names(jar)?;
        let colors = crate::tables::named_colors(jar)?;
        let defs: Vec<_> = sm
            .get_all::<FluidDefinitionScript>(ScriptType::FluidDefinition)
            .into_iter()
            .map(|d| {
                let d = d.borrow();
                // `getColor`: a colour reference replaces the r/g/b values.
                let c = d.color();
                let c = match d
                    .color_reference()
                    .and_then(|n| colors.get(&n.to_lowercase()))
                {
                    Some(&[r, g, b, a]) => pz_scripts::objects::definitions::Color { r, g, b, a },
                    None => c,
                };
                let cats: Vec<FluidCategory> = d.categories().iter().copied().collect();
                (
                    d.fluid_type(),
                    d.fluid_type_string().map(str::to_string),
                    [c.r, c.g, c.b, c.a],
                    cats,
                )
            })
            .collect();
        let mut reg = Self::from_definitions(
            &enum_fluids,
            defs.iter()
                .map(|(t, s, c, k)| (*t, s.as_deref(), *c, k.clone())),
        );
        // Both native Fluid constructors leave properties null. The public
        // colour-only from_definitions helper deliberately does not invent
        // this metadata; this actual script/constructor path knows it.
        reg.properties_by_name = reg.by_name.keys().map(|name| (name.clone(), None)).collect();
        // The tail of `Fluid.Init`: blend filters, created once every fluid
        // is registered.
        let ctx = crate::context::BasicContext {
            fluids: Some(reg.clone()),
            ..Default::default()
        };
        for d in sm.get_all::<FluidDefinitionScript>(ScriptType::FluidDefinition) {
            let d = d.borrow();
            let name = match d.fluid_type() {
                FluidType::Modded => match d.fluid_type_string() {
                    Some(s) => s.to_string(),
                    None => continue,
                },
                t => t.name().to_string(),
            };
            // Fluid.setScript allocates properties only when the definition
            // has an explicitly non-default property. Otherwise the actual
            // native field is null, rather than an unknown zero value.
            let properties = d.has_properties_set().then(|| SealedFluidProperties {
                fatigue_change: d.fatigue_change(),
                hunger_change: d.hunger_change(),
                stress_change: d.stress_change(),
                thirst_change: d.thirst_change(),
                unhappy_change: d.unhappy_change(),
                calories: d.calories(),
                carbohydrates: d.carbohydrates(),
                lipids: d.lipids(),
                proteins: d.proteins(),
                alcohol: d.alcohol(),
                flu_reduction: d.flu_reduction(),
                pain_reduction: d.pain_reduction(),
                endurance_change: d.endurance_change(),
                food_sickness_change: d.food_sickness_change(),
                poison: d.poison_max_effect().player_effect() as f32,
            });
            reg.set_properties(&name, properties);
            let white = d
                .blend_whitelist()
                .map(|f| crate::factory::filter_from_script(f, &ctx));
            let black = d
                .blend_blacklist()
                .map(|f| crate::factory::filter_from_script(f, &ctx));
            if white.is_none() && black.is_none() {
                continue;
            }
            reg.set_blend_filters(&name, white, black);
        }
        Ok(reg)
    }

    /// Sets a registered fluid's `blendWhitelist` / `blendBlacklist`.
    pub fn set_blend_filters(
        &mut self,
        name: &str,
        whitelist: Option<FluidFilter>,
        blacklist: Option<FluidFilter>,
    ) {
        let Some(info) = self.by_name.get_mut(name) else {
            return;
        };
        if whitelist.is_some() {
            info.blend_whitelist = whitelist;
        }
        if blacklist.is_some() {
            info.blend_blacklist = blacklist;
        }
        let info = info.clone();
        if let Ok(t) = FluidType::value_of(name) {
            if let Some(e) = self.by_id.get_mut(&t.id()) {
                *e = info;
            }
        }
    }

    /// Adds or replaces one fluid (`Modded` fluids go in by name only).
    pub fn insert(&mut self, type_id: Option<i8>, info: FluidInfo) {
        self.properties_by_name.remove(&info.name);
        if let Some(id) = type_id {
            self.by_id.insert(id, info.clone());
        }
        self.by_name.insert(info.name.clone(), info);
    }

    /// Set authenticated properties metadata for an existing fluid. A null
    /// value is explicit; callers with only colour/name data leave it unknown.
    pub fn set_properties(&mut self, name: &str, properties: Option<SealedFluidProperties>) -> bool {
        if !self.by_name.contains_key(name) {
            return false;
        }
        self.properties_by_name.insert(name.to_owned(), properties);
        true
    }

    pub fn properties(&self, fluid: &FluidRef) -> Lookup<Option<SealedFluidProperties>> {
        let info = match fluid {
            FluidRef::Type(id) => self.by_id(*id),
            FluidRef::Modded(name) => self.by_name(name),
        };
        let Some(info) = info else { return Lookup::Missing; };
        self.properties_by_name.get(&info.name).cloned().map_or(Lookup::Unknown, Lookup::Found)
    }

    /// `Fluid.Get(FluidType.FromId(id))`.
    pub fn by_id(&self, id: i8) -> Option<&FluidInfo> {
        self.by_id.get(&id)
    }

    /// `Fluid.Get(name)`.
    pub fn by_name(&self, name: &str) -> Option<&FluidInfo> {
        self.by_name.get(name)
    }
}

/// Which fluid a `FluidInstance` holds, as saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FluidRef {
    /// A built-in fluid by `FluidType` id.
    Type(i8),
    /// A `FluidType.Modded` fluid by type string.
    Modded(String),
}

/// `FluidInstance`'s saved state.
#[derive(Clone, Debug, PartialEq)]
pub struct FluidInstance {
    /// `None` when the save names no fluid (the instance's fluid is null).
    pub fluid: Option<FluidRef>,
    /// The colour when it differs from the fluid's (`r, g, b`); `None`
    /// means the fluid's own colour.
    pub custom_color: Option<[f32; 3]>,
    pub amount: f32,
}

impl FluidInstance {
    pub fn new(fluid: FluidRef, amount: f32) -> Self {
        FluidInstance {
            fluid: Some(fluid),
            custom_color: None,
            amount,
        }
    }

    fn lookup(&self, ctx: &dyn EntityContext) -> Lookup<FluidInfo> {
        match &self.fluid {
            None => Lookup::Missing,
            Some(FluidRef::Type(id)) => ctx.fluid_by_id(*id),
            Some(FluidRef::Modded(s)) => ctx.fluid_by_name(s),
        }
    }

    /// `FluidInstance.load`. A custom colour equal to the registered fluid's
    /// colour is folded back (the original only saves a differing colour).
    pub fn load(r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<FluidInstance> {
        let h = read_header(r, HeaderSize::Byte)?;
        let fluid = if h & 1 != 0 {
            Some(FluidRef::Type(r.i8()?))
        } else if h & 2 != 0 {
            Some(FluidRef::Modded(r.string()?))
        } else {
            None
        };
        let mut fi = FluidInstance {
            fluid,
            custom_color: None,
            amount: 0.0,
        };
        if h & 4 != 0 {
            let c = [
                unpack_unit(r.i8()?),
                unpack_unit(r.i8()?),
                unpack_unit(r.i8()?),
            ];
            let same = matches!(fi.lookup(ctx), Lookup::Found(f) if f.color[..3] == c);
            if !same {
                fi.custom_color = Some(c);
            }
        }
        fi.amount = r.f32()?;
        Ok(fi)
    }

    /// `FluidInstance.save`.
    pub fn save(&self, w: &mut Writer) {
        let mut h = HeaderWrite::start(w, HeaderSize::Byte);
        match &self.fluid {
            Some(FluidRef::Type(id)) => {
                h.add(1);
                w.i8(*id);
            }
            Some(FluidRef::Modded(s)) => {
                h.add(2);
                w.string(s);
            }
            None => {}
        }
        if let Some(c) = self.custom_color {
            h.add(4);
            for v in c {
                w.i8(pack_unit(v));
            }
        }
        w.f32(self.amount);
        h.write(w);
    }
}

/// `FluidFilter`'s saved state.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FluidFilter {
    /// `FilterType.Whitelist` (else `Blacklist`).
    pub whitelist: bool,
    /// `fluidEnums` (an `EnumSet`: ordinal order, no duplicates).
    pub fluids: Vec<FluidType>,
    /// `fluidStrings` (a `HashSet`: see [`java::hash_set_order`]).
    pub fluid_strings: Vec<String>,
    /// `categories` (an `EnumSet`).
    pub categories: Vec<FluidCategory>,
}

impl FluidFilter {
    pub fn new(whitelist: bool) -> Self {
        FluidFilter {
            whitelist,
            ..Default::default()
        }
    }

    /// `add(FluidType)`: the type and its name (`Modded` is refused).
    pub fn add_fluid(&mut self, t: FluidType) {
        if t == FluidType::Modded || self.fluids.contains(&t) {
            return;
        }
        self.fluids.push(t);
        self.fluids.sort();
        self.add_fluid_string(t.name());
    }

    /// `add(String)` of a fluid name (the caller checks it is registered).
    pub fn add_fluid_string(&mut self, s: &str) {
        if self.fluid_strings.iter().any(|x| x == s) {
            return;
        }
        let mut v = std::mem::take(&mut self.fluid_strings);
        v.push(s.to_string());
        self.fluid_strings = java::hash_set_order(v);
    }

    /// `add(FluidCategory)`.
    pub fn add_category(&mut self, c: FluidCategory) {
        if !self.categories.contains(&c) {
            self.categories.push(c);
            self.categories.sort();
        }
    }

    /// `FluidFilter.load`: unknown type ids, categories and fluid names are
    /// dropped.
    pub fn load(r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<FluidFilter> {
        let mut f = FluidFilter::new(r.bool()?);
        for _ in 0..r.i8()?.max(0) {
            if let Some(t) = fluid_type_from_id(r.i8()?) {
                if !f.fluids.contains(&t) {
                    f.fluids.push(t);
                }
            }
        }
        f.fluids.sort();
        let mut strings = Vec::new();
        for _ in 0..r.i8()?.max(0) {
            let s = r.string()?;
            if !ctx.fluid_by_name(&s).is_missing() {
                strings.push(s);
            }
        }
        f.fluid_strings = java::hash_set_order(strings);
        for _ in 0..r.i8()?.max(0) {
            if let Some(c) = fluid_category_from_id(r.i8()?) {
                f.add_category(c);
            }
        }
        Ok(f)
    }

    /// `FluidFilter.save`.
    pub fn save(&self, w: &mut Writer) {
        w.bool(self.whitelist);
        w.i8(self.fluids.len() as i8);
        for t in &self.fluids {
            w.i8(t.id());
        }
        w.i8(self.fluid_strings.len() as i8);
        for s in &self.fluid_strings {
            w.string(s);
        }
        w.i8(self.categories.len() as i8);
        for c in &self.categories {
            w.i8(c.id());
        }
    }
}

/// `FluidContainer`'s state: the saved fields, plus the script-only fields
/// a load leaves in place and the amount cache (`amountCache`,
/// `cacheInvalidated`), which equality ignores.
#[derive(Clone, Debug)]
pub struct FluidContainer {
    pub capacity: f32,
    pub fluids: Vec<FluidInstance>,
    pub whitelist: Option<FluidFilter>,
    pub blacklist: Option<FluidFilter>,
    pub input_locked: bool,
    pub can_player_empty: bool,
    pub container_name: String,
    pub hidden_amount: bool,
    /// Not reset by `load`: a script's value survives a save without one.
    pub rain_catcher: f32,
    /// Script-only (`customDrinkSound`), not saved.
    pub custom_drink_sound: Option<String>,
    /// Script-only (`fillsWithCleanWater`), not saved.
    pub fills_with_clean_water: bool,
    /// `amountCache`: the clamped amount as of the last recalculation.
    pub amount_cache: f32,
    /// `cacheInvalidated`.
    pub cache_invalidated: bool,
}

impl PartialEq for FluidContainer {
    fn eq(&self, o: &Self) -> bool {
        self.capacity == o.capacity
            && self.fluids == o.fluids
            && self.whitelist == o.whitelist
            && self.blacklist == o.blacklist
            && self.input_locked == o.input_locked
            && self.can_player_empty == o.can_player_empty
            && self.container_name == o.container_name
            && self.hidden_amount == o.hidden_amount
            && self.rain_catcher == o.rain_catcher
            && self.custom_drink_sound == o.custom_drink_sound
            && self.fills_with_clean_water == o.fills_with_clean_water
    }
}

impl Default for FluidContainer {
    /// A pooled container after `reset()`.
    fn default() -> Self {
        FluidContainer {
            capacity: 1.0,
            fluids: Vec::new(),
            whitelist: None,
            blacklist: None,
            input_locked: false,
            can_player_empty: true,
            container_name: DEF_CONTAINER_NAME.to_string(),
            hidden_amount: false,
            rain_catcher: 0.0,
            custom_drink_sound: None,
            fills_with_clean_water: false,
            amount_cache: 0.0,
            cache_invalidated: false,
        }
    }
}

impl FluidContainer {
    /// The sum of the fluid amounts (`recalculateCaches` adds them last to
    /// first), before the capacity clamp.
    pub fn amount(&self) -> f32 {
        self.fluids.iter().rev().fold(0.0, |a, f| a + f.amount)
    }

    /// `recalculateCaches`' amount: the sum, or the capacity when it is
    /// within 0.001 of it (or above).
    fn clamped_amount(&self) -> f32 {
        let a = self.amount();
        if self.capacity - a < 0.001 {
            self.capacity
        } else {
            a
        }
    }

    /// `recalculateCaches(force)`'s amount part (the colour is computed on
    /// demand by [`get_color`](Self::get_color)).
    pub fn recalculate_caches(&mut self, force: bool) {
        if self.cache_invalidated || force {
            self.amount_cache = self.clamped_amount();
            self.cache_invalidated = false;
        }
    }

    /// `getAmount()` without updating the cache.
    pub fn get_amount(&self) -> f32 {
        if self.cache_invalidated {
            self.clamped_amount()
        } else {
            self.amount_cache
        }
    }

    /// `getAmount()` as the original calls it internally (recalculating).
    pub(crate) fn amount_mut(&mut self) -> f32 {
        self.recalculate_caches(false);
        self.amount_cache
    }

    /// `getCapacity()`.
    pub fn get_capacity(&self) -> f32 {
        self.capacity
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.get_amount() == 0.0
    }

    /// `isFull()`: `PZMath.equal(getAmount(), capacity, 1.0E-4f)`.
    pub fn is_full(&self) -> bool {
        (self.capacity - self.get_amount()).abs() < 1.0e-4
    }

    /// `getFreeCapacity()`.
    pub fn get_free_capacity(&self) -> f32 {
        let free = self.capacity - self.get_amount();
        if free.abs() < 1.0e-4 {
            0.0
        } else {
            free
        }
    }

    /// `getFilledRatio()`.
    pub fn get_filled_ratio(&self) -> f32 {
        if self.is_full() {
            return 1.0;
        }
        if self.is_empty() {
            return 0.0;
        }
        if self.capacity != 0.0 {
            self.get_amount() / self.capacity
        } else {
            0.0
        }
    }

    /// `setCapacity`: at least `FluidUtil.getMinContainerCapacity()`; when
    /// `getAmount()` exceeds it the fluids shrink by the capacity ratio
    /// (empty instances are removed).
    pub fn set_capacity(&mut self, capacity: f32) {
        let old = self.capacity;
        self.capacity = capacity.max(0.05);
        if self.amount_mut() > self.capacity {
            for i in (0..self.fluids.len()).rev() {
                let f = &mut self.fluids[i];
                let ratio = f.amount / old;
                f.amount = self.capacity * ratio;
                self.remove_if_empty(i);
            }
        }
        self.cache_invalidated = true;
    }

    /// `FluidContainer.load` into this container (loading in place keeps
    /// `rainCatcher` when the save has none). Instances whose fluid does not
    /// resolve are dropped.
    pub fn load(&mut self, r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<()> {
        self.load_data_without_cache(r, ctx)?;
        self.recalculate_caches(true);
        Ok(())
    }

    /// Field decoding for an actual runtime component. Its host completes
    /// native load with the real colour/properties cache data phase followed
    /// by owner notifications outside every component/entity guard. The DTO
    /// archive loader above retains its separate value-cache contract.
    pub(crate) fn load_data_without_cache(&mut self, r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<()> {
        self.amount_cache = 0.0;
        self.capacity = 1.0;
        self.whitelist = None;
        self.blacklist = None;
        self.input_locked = false;
        self.can_player_empty = false;
        self.hidden_amount = false;
        self.fluids.clear();
        let h = read_header(r, HeaderSize::Short)?;
        if h & 1 != 0 {
            self.capacity = r.f32()?;
        }
        if h & 2 != 0 {
            let n = if h & 4 != 0 { 1 } else { r.i8()? as i32 };
            for _ in 0..n {
                let fi = FluidInstance::load(r, ctx)?;
                if !fi.lookup(ctx).is_missing() {
                    self.fluids.push(fi);
                }
            }
        }
        if h & 8 != 0 {
            self.whitelist = Some(FluidFilter::load(r, ctx)?);
        }
        if h & 16 != 0 {
            self.blacklist = Some(FluidFilter::load(r, ctx)?);
        }
        self.input_locked = h & 32 != 0;
        self.can_player_empty = h & 64 != 0;
        self.container_name = if h & 128 != 0 {
            r.string()?
        } else {
            DEF_CONTAINER_NAME.to_string()
        };
        self.hidden_amount = h & 256 != 0;
        if h & 512 != 0 {
            self.rain_catcher = r.f32()?;
        }
        Ok(())
    }

    /// `FluidContainer.save`.
    pub fn save(&self, w: &mut Writer) {
        let mut h = HeaderWrite::start(w, HeaderSize::Short);
        if self.capacity != 1.0 {
            h.add(1);
            w.f32(self.capacity);
        }
        if !self.fluids.is_empty() {
            h.add(2);
            if self.fluids.len() == 1 {
                h.add(4);
            } else {
                w.i8(self.fluids.len() as i8);
            }
            for f in &self.fluids {
                f.save(w);
            }
        }
        if let Some(f) = &self.whitelist {
            h.add(8);
            f.save(w);
        }
        if let Some(f) = &self.blacklist {
            h.add(16);
            f.save(w);
        }
        if self.input_locked {
            h.add(32);
        }
        if self.can_player_empty {
            h.add(64);
        }
        if self.container_name != DEF_CONTAINER_NAME {
            h.add(128);
            w.string(&self.container_name);
        }
        if self.hidden_amount {
            h.add(256);
        }
        if self.rain_catcher > 0.0 {
            h.add(512);
            w.f32(self.rain_catcher);
        }
        h.write(w);
    }
}

/// `FluidSample`'s saved state.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FluidSample {
    pub sealed: bool,
    pub amount: f32,
    pub fluids: Vec<FluidInstance>,
}

impl FluidSample {
    /// `FluidSample.Load`: the amount is corrected to the instances' sum.
    pub fn load(r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<FluidSample> {
        let sealed = r.bool()?;
        let mut amount = r.f32()?;
        let n = r.i32()?;
        let mut fluids = Vec::new();
        let mut sum = 0.0f32;
        for _ in 0..n.max(0) {
            let fi = FluidInstance::load(r, ctx)?;
            sum += fi.amount;
            fluids.push(fi);
        }
        if sum != amount {
            amount = sum;
        }
        Ok(FluidSample {
            sealed,
            amount,
            fluids,
        })
    }

    /// `FluidSample.Save`.
    pub fn save(&self, w: &mut Writer) {
        w.bool(self.sealed);
        w.f32(self.amount);
        w.i32(self.fluids.len() as i32);
        for f in &self.fluids {
            f.save(w);
        }
    }
}

/// `SealedFluidProperties`' saved state, in field order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SealedFluidProperties {
    pub fatigue_change: f32,
    pub hunger_change: f32,
    pub stress_change: f32,
    pub thirst_change: f32,
    pub unhappy_change: f32,
    pub calories: f32,
    pub carbohydrates: f32,
    pub lipids: f32,
    pub proteins: f32,
    pub alcohol: f32,
    pub flu_reduction: f32,
    pub pain_reduction: f32,
    pub endurance_change: f32,
    pub food_sickness_change: i32,
    pub poison: f32,
}

impl SealedFluidProperties {
    fn floats_mut(&mut self) -> [&mut f32; 13] {
        [
            &mut self.fatigue_change,
            &mut self.hunger_change,
            &mut self.stress_change,
            &mut self.thirst_change,
            &mut self.unhappy_change,
            &mut self.calories,
            &mut self.carbohydrates,
            &mut self.lipids,
            &mut self.proteins,
            &mut self.alcohol,
            &mut self.flu_reduction,
            &mut self.pain_reduction,
            &mut self.endurance_change,
        ]
    }

    /// `SealedFluidProperties.load` (flag 8192 reads an int the save wrote
    /// as a float).
    pub fn load(r: &mut Reader<'_>) -> Result<SealedFluidProperties> {
        let mut p = SealedFluidProperties::default();
        let h = read_header(r, HeaderSize::Integer)?;
        for (i, f) in p.floats_mut().into_iter().enumerate() {
            if h & (1 << i) != 0 {
                *f = r.f32()?;
            }
        }
        if h & 8192 != 0 {
            p.food_sickness_change = r.i32()?;
        }
        if h & 16384 != 0 {
            p.poison = r.f32()?;
        }
        Ok(p)
    }

    /// `SealedFluidProperties.save`: positive values only.
    pub fn save(&self, w: &mut Writer) {
        let mut h = HeaderWrite::start(w, HeaderSize::Integer);
        let mut copy = self.clone();
        for (i, f) in copy.floats_mut().into_iter().enumerate() {
            if *f > 0.0 {
                h.add(1 << i);
                w.f32(*f);
            }
        }
        if self.food_sickness_change != 0 {
            h.add(8192);
            w.f32(self.food_sickness_change as f32);
        }
        if self.poison > 0.0 {
            h.add(16384);
            w.f32(self.poison);
        }
        h.write(w);
    }
}

/// `FluidConsume`'s saved state.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FluidConsume {
    pub amount: f32,
    /// `PoisonEffect.getLevel()`.
    pub poison_level: i32,
    pub properties: SealedFluidProperties,
}

impl FluidConsume {
    /// `FluidConsume.Load`.
    pub fn load(r: &mut Reader<'_>) -> Result<FluidConsume> {
        Ok(FluidConsume {
            amount: r.f32()?,
            poison_level: r.i32()?,
            properties: SealedFluidProperties::load(r)?,
        })
    }

    /// `FluidConsume.Save`.
    pub fn save(&self, w: &mut Writer) {
        w.f32(self.amount);
        w.i32(self.poison_level);
        self.properties.save(w);
    }
}
