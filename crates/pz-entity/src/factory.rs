//! `GameEntityFactory`'s component creation: components built from a
//! `GameEntityScript`'s component scripts (`instanceComponents` +
//! `CreateComponentFromScript`), and the defaults object constructors add
//! (`IsoFeedingTrough.createFluidContainer`).

use pz_scripts::objects::definitions::{FilterType, FluidFilterScript, FluidType};
use pz_scripts::objects::entity::components::attributes::AttributesScript;
use pz_scripts::objects::entity::components::combat::{DurabilityScript, MaterialType};
use pz_scripts::objects::entity::components::crafting::{CraftBenchScript, CraftLogicScript};
use pz_scripts::objects::entity::components::fluids::FluidContainerScript;
use pz_scripts::objects::entity::components::ui::UiConfigScript;
use pz_scripts::objects::entity::{ComponentScript, ComponentType, GameEntityScript};

use crate::components::*;
use crate::context::{EntityContext, FluidInfo, Lookup, ScriptRegister};
use crate::entity::{Component, Components};
use crate::fluids::{FluidContainer, FluidFilter, FluidInstance, FluidRef};

/// The `Rand` calls component creation makes.
pub trait FactoryRng {
    /// `Rand.Next(int max)`: `[0, max)`.
    fn next_int(&mut self, max: i32) -> i32;
    /// `Rand.Next(float min, float max)`.
    fn next_float(&mut self, min: f32, max: f32) -> f32;
}

/// What the entity being built is, as `instanceComponents` and
/// `EntityScriptInfo.setOriginalScript` ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    /// An `IsoObject`; `multi_square_master` is `SpriteConfig`'s
    /// `isMultiSquareMaster()` for the object's tile.
    IsoObject {
        multi_square_master: bool,
    },
    InventoryItem,
    Other,
}

/// `ResourceChannel` set bits (`EnumBitStore.copyFrom`).
fn channel_bits<'a>(
    it: impl IntoIterator<
        Item = &'a pz_scripts::objects::entity::components::resources::ResourceChannel,
    >,
) -> i32 {
    it.into_iter().fold(0, |b, c| b | c.bits())
}

/// The fluid a name resolves to (`Fluid.Get(name)`), as a saved reference.
fn fluid_ref(ctx: &dyn EntityContext, name: &str) -> Option<(FluidRef, Option<FluidInfo>)> {
    let info = match ctx.fluid_by_name(name) {
        Lookup::Missing => return None,
        Lookup::Found(f) => Some(f),
        Lookup::Unknown => None,
    };
    let r = match FluidType::value_of(name) {
        Ok(t) if t != FluidType::Modded && t != FluidType::None => FluidRef::Type(t.id()),
        _ => FluidRef::Modded(name.to_string()),
    };
    Some((r, info))
}

/// `FluidFilterScript.createFilter()` (fluids that do not resolve are
/// skipped; the original throws).
pub fn filter_from_script(script: &FluidFilterScript, ctx: &dyn EntityContext) -> FluidFilter {
    let mut f = FluidFilter::new(script.is_whitelist());
    if let Some(made) = script.filter() {
        f.whitelist = made.filter_type == FilterType::Whitelist;
    }
    for c in script.categories() {
        if let Ok(c) = pz_scripts::objects::definitions::FluidCategory::value_of(c) {
            f.add_category(c);
        }
    }
    for s in script.fluids() {
        match fluid_ref(ctx, s) {
            Some((FluidRef::Type(id), _)) => {
                if let Some(t) = crate::fluids::fluid_type_from_id(id) {
                    f.add_fluid(t);
                }
            }
            Some((FluidRef::Modded(n), _)) => f.add_fluid_string(&n),
            None => {}
        }
    }
    f
}

impl FluidFilter {
    /// `allows(Fluid)`.
    pub fn allows(&self, fluid: &FluidRef, info: Option<&FluidInfo>) -> bool {
        let mut type_match = self.fluids.is_empty() && self.fluid_strings.is_empty();
        match fluid {
            FluidRef::Type(id) => {
                if self.fluids.iter().any(|t| t.id() == *id) {
                    type_match = true;
                }
            }
            FluidRef::Modded(s) => {
                if self.fluid_strings.contains(s) {
                    type_match = true;
                }
            }
        }
        let cats = info.map(|i| i.categories.as_slice()).unwrap_or(&[]);
        let category_match =
            self.categories.is_empty() || self.categories.iter().any(|c| cats.contains(c));
        let m = type_match && category_match;
        if self.whitelist {
            m
        } else {
            !m
        }
    }
}

impl FluidContainer {
    /// A fluid instance's colour (`FluidInstance.getColor()`): its own, or
    /// its fluid's; white when the fluid is not registered.
    pub fn instance_color(fi: &FluidInstance, ctx: &dyn EntityContext) -> [f32; 3] {
        if let Some(c) = fi.custom_color {
            return c;
        }
        let info = match &fi.fluid {
            Some(FluidRef::Type(id)) => ctx.fluid_by_id(*id),
            Some(FluidRef::Modded(n)) => ctx.fluid_by_name(n),
            None => Lookup::Missing,
        };
        match info {
            Lookup::Found(f) => [f.color[0], f.color[1], f.color[2]],
            _ => [1.0, 1.0, 1.0],
        }
    }

    /// `getColor()`: the fluid colours weighted by their share of the
    /// summed amount, in list order; white when empty.
    pub fn get_color(&self, ctx: &dyn EntityContext) -> [f32; 3] {
        let sum = self.amount();
        if sum <= 0.0 {
            return [1.0, 1.0, 1.0];
        }
        let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
        for f in &self.fluids {
            let pct = f.amount / sum;
            let c = Self::instance_color(f, ctx);
            r += c[0] * pct;
            g += c[1] * pct;
            b += c[2] * pct;
        }
        [r, g, b]
    }

    /// `Empty()`.
    pub fn empty(&mut self) {
        self.fluids.clear();
        self.amount_cache = 0.0;
        self.recalculate_caches(true);
    }

    /// `removeFluidInstanceIfEmpty` for the instance at `i`.
    pub(crate) fn remove_if_empty(&mut self, i: usize) -> bool {
        if self.fluids[i].amount.abs() < 1.0e-4 {
            self.fluids.remove(i);
            return true;
        }
        false
    }

    /// `addFluid(Fluid, amount)` (and the `FluidInstance` form with a
    /// colour): clamps to the free capacity, merges into an instance of the
    /// same fluid (mixing a `Colors` fluid's colour in LCH) or appends one.
    pub fn add_fluid(
        &mut self,
        fluid: FluidRef,
        custom_color: Option<[f32; 3]>,
        add: f32,
        ctx: &dyn EntityContext,
    ) {
        let add = add.max(0.0);
        if add <= 0.0 {
            return;
        }
        let full = (self.capacity - self.amount_mut()).abs() < 1.0e-4;
        if full || !self.can_add_fluid(&fluid, ctx) {
            return;
        }
        let available = self.capacity - self.amount_mut();
        let add = add.min(available);
        let info = lookup_ref(ctx, &fluid);
        let fluid_color = info.as_ref().map(|i| [i.color[0], i.color[1], i.color[2]]);
        let add_color = custom_color.or(fluid_color).unwrap_or([1.0, 1.0, 1.0]);
        match self
            .fluids
            .iter_mut()
            .find(|f| f.fluid.as_ref() == Some(&fluid))
        {
            None => {
                let mut fi = FluidInstance::new(fluid, add);
                fi.custom_color = Some(add_color).filter(|c| Some(*c) != fluid_color);
                self.fluids.push(fi);
            }
            Some(fi) => {
                let colors = info.as_ref().is_some_and(|i| {
                    i.categories
                        .contains(&pz_scripts::objects::definitions::FluidCategory::Colors)
                });
                let cur = fi.custom_color.or(fluid_color).unwrap_or([1.0, 1.0, 1.0]);
                if colors && add_color != cur {
                    // `mixColor(color, add)`.
                    let mixed = if fi.amount == 0.0 {
                        add_color
                    } else {
                        let delta = add / (add + fi.amount);
                        let m = crate::color::lerp_lch(
                            [cur[0], cur[1], cur[2], 1.0],
                            [add_color[0], add_color[1], add_color[2], 1.0],
                            delta,
                        );
                        [m[0], m[1], m[2]]
                    };
                    fi.custom_color = Some(mixed).filter(|c| Some(*c) != fluid_color);
                }
                fi.amount += add;
            }
        }
        self.cache_invalidated = true;
    }

    /// `canAddFluid(Fluid)`.
    pub fn can_add_fluid(&self, fluid: &FluidRef, ctx: &dyn EntityContext) -> bool {
        let contains = self.fluids.iter().any(|f| f.fluid.as_ref() == Some(fluid));
        if self.fluids.len() >= 8 && !contains || self.input_locked || !self.can_player_empty {
            return false;
        }
        let info = lookup_ref(ctx, fluid);
        for f in &self.fluids {
            let Some(other) = &f.fluid else { continue };
            let other_info = lookup_ref(ctx, other);
            if !(can_blend_with(other, other_info.as_ref(), fluid, info.as_ref())
                && can_blend_with(fluid, info.as_ref(), other, other_info.as_ref()))
            {
                return false;
            }
        }
        !(self
            .whitelist
            .as_ref()
            .is_some_and(|w| !w.allows(fluid, info.as_ref()))
            || self
                .blacklist
                .as_ref()
                .is_some_and(|b| !b.allows(fluid, info.as_ref())))
    }

    /// `readFromScript(FluidContainerScript)` on a fresh container,
    /// including the initial fluids (`getInitialAmount()` is called twice,
    /// as the original does: each call of a ranged amount draws).
    pub fn from_script(
        script: &FluidContainerScript,
        ctx: &dyn EntityContext,
        rng: &mut dyn FactoryRng,
    ) -> Self {
        let mut fc = FluidContainer {
            whitelist: script.whitelist().map(|f| filter_from_script(f, ctx)),
            blacklist: script.blacklist().map(|f| filter_from_script(f, ctx)),
            container_name: script.container_name().to_string(),
            custom_drink_sound: Some(script.custom_drink_sound().to_string()),
            rain_catcher: script.rain_catcher(),
            fills_with_clean_water: script.is_filled_with_clean_water(),
            input_locked: false,
            can_player_empty: true,
            ..Default::default()
        };
        fc.set_capacity(script.capacity());
        let initial_amount = |rng: &mut dyn FactoryRng| {
            if !script.initial_amount_set() {
                script.capacity()
            } else {
                let (min, max) = script.initial_amount_range();
                if min == max {
                    min
                } else {
                    rng.next_float(min, max)
                }
            }
        };
        if let Some(initial) = script.initial_fluids().filter(|f| !f.is_empty()) {
            if initial_amount(rng) > 0.0 {
                let amount = initial_amount(rng);
                let picks: Vec<_> = if initial.len() == 1 {
                    vec![&initial[0]]
                } else if script.is_initial_fluids_is_random() {
                    vec![&initial[rng.next_int(initial.len() as i32) as usize]]
                } else {
                    initial.iter().collect()
                };
                for fs in picks {
                    // `addInitialFluid`: a fluid that does not resolve is
                    // logged and skipped.
                    if let Some((r, _)) = fluid_ref(ctx, fs.fluid_type()) {
                        let color = fs.custom_color().map(|c| [c.r, c.g, c.b]);
                        fc.add_fluid(r, color, amount * fs.percentage(), ctx);
                    }
                }
            }
        }
        fc.input_locked = script.input_locked();
        fc.can_player_empty = script.can_empty();
        fc.hidden_amount = script.is_hidden_amount();
        fc
    }
}

/// The registered fluid a saved reference names (`None` when unknown or
/// missing).
fn lookup_ref(ctx: &dyn EntityContext, r: &FluidRef) -> Option<FluidInfo> {
    let l = match r {
        FluidRef::Type(id) => ctx.fluid_by_id(*id),
        FluidRef::Modded(n) => ctx.fluid_by_name(n),
    };
    match l {
        Lookup::Found(f) => Some(f),
        _ => None,
    }
}

/// `Fluid.canBlendWith(other)`.
fn can_blend_with(
    this: &FluidRef,
    this_info: Option<&FluidInfo>,
    other: &FluidRef,
    other_info: Option<&FluidInfo>,
) -> bool {
    if this == other {
        return true;
    }
    let Some(i) = this_info else { return true };
    !(i.blend_whitelist
        .as_ref()
        .is_some_and(|w| !w.allows(other, other_info))
        || i.blend_blacklist
            .as_ref()
            .is_some_and(|b| !b.allows(other, other_info)))
}

/// `Float.parseFloat`: surrounding whitespace and a type suffix allowed.
fn java_parse_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    t.parse().ok()
}

/// `AttributeInstance.setValueFromScriptString`: the parsed value, or the
/// type's initial value when parsing fails. Bounds validation is not
/// ported. `None` for the enum kinds (not ported).
fn attribute_from_string(
    info: &crate::tables::AttributeTypeInfo,
    v: &str,
) -> Option<AttributeValue> {
    use crate::context::AttributeKind as K;
    use crate::tables::InitialValue as I;
    let init_i = || match &info.initial {
        Some(I::Int(i)) => *i,
        Some(I::Float(f)) => *f as i64,
        _ => 0,
    };
    let init_f = || match &info.initial {
        Some(I::Float(f)) => *f,
        Some(I::Int(i)) => *i as f64,
        _ => 0.0,
    };
    Some(match info.kind {
        K::Long => AttributeValue::Long(v.parse().unwrap_or_else(|_| init_i())),
        K::Int => AttributeValue::Int(v.parse().unwrap_or_else(|_| init_i() as i32)),
        K::Short => AttributeValue::Short(v.parse().unwrap_or_else(|_| init_i() as i16)),
        K::Byte => AttributeValue::Byte(v.parse().unwrap_or_else(|_| init_i() as i8)),
        K::Float => AttributeValue::Float(java_parse_f64(v).map_or(init_f() as f32, |f| f as f32)),
        K::Double => AttributeValue::Double(java_parse_f64(v).unwrap_or_else(init_f)),
        K::String => AttributeValue::String(v.to_string()),
        // `Boolean.parseBoolean`.
        K::Bool => AttributeValue::Bool(v.eq_ignore_ascii_case("true")),
        K::Enum | K::EnumSet | K::EnumStringSet => return None,
    })
}

/// `AttributesScript.getTemplateContainer()` copied
/// (`AttributeContainer.readFromScript`): `putFromScript` per key/value
/// pair in the script's `HashMap` order.
pub fn attributes_from_script(
    script: &AttributesScript,
    ctx: &dyn EntityContext,
) -> AttributeContainer {
    let mut c = AttributeContainer::default();
    for (k, v) in script.kv_pairs().iter() {
        let Some(info) = ctx.attribute_type_by_name(k) else {
            continue;
        };
        if let Some(val) = attribute_from_string(&info, v) {
            c.set(info.id, val);
        }
    }
    c
}

/// `IsoFeedingTrough.createFluidContainer`: capacity `maxWater`, rain
/// catcher 0.55, a Water/TaintedWater whitelist.
pub fn feeding_trough_fluid_container(max_water: f32) -> FluidContainer {
    let mut fc = FluidContainer::default();
    fc.set_capacity(max_water);
    fc.rain_catcher = 0.55;
    let mut ff = FluidFilter::new(true);
    ff.add_fluid(FluidType::Water);
    ff.add_fluid(FluidType::TaintedWater);
    fc.whitelist = Some(ff);
    fc
}

/// Looks up a dictionary script reference for a script-made sprite config.
fn dict_ref(
    lookup: &DictionaryLookup<'_>,
    reg: ScriptRegister,
    name: Option<String>,
) -> Option<DictScriptRef> {
    let name = name?;
    let (id, version) = match lookup(reg, &name) {
        Some((id, v)) => (Some(id), v),
        None => (None, 0),
    };
    Some(DictScriptRef {
        id,
        name: Some(name),
        script_version: version,
    })
}

/// `ScriptsDictionary.<register>` lookup: a script name's registry id and
/// its `getScriptVersion()`.
pub type DictionaryLookup<'a> = dyn Fn(ScriptRegister, &str) -> Option<(i16, i64)> + 'a;

/// Script-side inputs `create_from_script` needs besides the scripts.
pub struct FactoryInput<'a> {
    pub ctx: &'a dyn EntityContext,
    pub rng: &'a mut dyn FactoryRng,
    /// `ScriptsDictionary.<register>`: a script name's registry id and its
    /// `getScriptVersion()`.
    pub dictionary: &'a DictionaryLookup<'a>,
}

/// `CreateComponentFromScript` for one component script. Components whose
/// script data is not ported (`Attributes`, `Resources`, the furnace,
/// mashing and drying logics) come out in their reset state.
///
/// `entity_full_type` is the owning entity script's full type, which the
/// sprite config scripts are registered under in the dictionary.
pub fn component_from_script(
    script: &dyn ComponentScript,
    entity_full_type: &str,
    input: &mut FactoryInput<'_>,
) -> Option<Component> {
    let kind = script.component_type();
    let any = script.as_any();
    let c = match kind {
        ComponentType::FluidContainer => {
            let s = any.downcast_ref::<FluidContainerScript>()?;
            Component::FluidContainer(FluidContainer::from_script(s, input.ctx, input.rng))
        }
        ComponentType::Attributes => {
            let s = any.downcast_ref::<AttributesScript>()?;
            Component::Attributes(attributes_from_script(s, input.ctx))
        }
        ComponentType::UiConfig => {
            let s = any.downcast_ref::<UiConfigScript>()?;
            Component::UiConfig(UiConfig {
                xui_skin_name: s.xui_skin_name().map(str::to_string),
                entity_style_name: s.entity_style().map(str::to_string),
                ui_enabled: s.is_ui_enabled(),
            })
        }
        ComponentType::CraftBench => {
            let s = any.downcast_ref::<CraftBenchScript>()?;
            Component::CraftBench(CraftBench {
                recipe_tag_query: s.recipe_tag_query().unwrap_or("").to_string(),
                fluid_input_channels: channel_bits(s.fluid_input_channels()),
                energy_input_channels: channel_bits(s.energy_input_channels()),
            })
        }
        ComponentType::CraftLogic | ComponentType::DryingCraftLogic => {
            let s = any.downcast_ref::<CraftLogicScript>()?;
            let logic = CraftLogic {
                recipe_tag_query: s.recipe_tag_query().map(str::to_string),
                start_mode: s.start_mode(),
                inputs_group_name: s.inputs_group_name().map(str::to_string),
                outputs_group_name: s.outputs_group_name().map(str::to_string),
                in_progress: Vec::new(),
                action_anim_override: s.action_anim().map(str::to_string),
            };
            if kind == ComponentType::CraftLogic {
                Component::CraftLogic(logic)
            } else {
                Component::DryingCraftLogic(DryingCraftLogic {
                    logic,
                    wetness: Vec::new(),
                })
            }
        }
        ComponentType::Durability => {
            let s = any.downcast_ref::<DurabilityScript>()?;
            Component::Durability(Durability {
                current_hit_points: s.current_hit_points(),
                max_hit_points: s.max_hit_points(),
                material: s.material().unwrap_or(MaterialType::Default),
            })
        }
        ComponentType::SpriteConfig => {
            let name = Some(entity_full_type.to_string());
            Component::SpriteConfig(SpriteConfig {
                config_script: dict_ref(input.dictionary, ScriptRegister::SpriteConfigs, name),
                multi_square_master: false,
            })
        }
        ComponentType::SpriteOverlayConfig => {
            let name = Some(entity_full_type.to_string());
            Component::SpriteOverlayConfig(SpriteOverlayConfig {
                config_script: dict_ref(
                    input.dictionary,
                    ScriptRegister::SpriteOverlayConfigs,
                    name,
                ),
                applied_style: None,
            })
        }
        k => Component::create(k)?,
    };
    Some(c)
}

/// `GameEntityFactory.createEntity(entity, script, ...)` on an entity with
/// no components: `instanceComponents` (on iso objects that are not the
/// multi-square master, `isoMasterOnly` components are skipped), then the
/// `EntityScriptInfo` naming `script_full_type`.
pub fn create_from_script(
    script: &GameEntityScript,
    script_full_type: &str,
    kind: EntityKind,
    input: &mut FactoryInput<'_>,
) -> Components {
    let mut comps = Components::new();
    let scripts = script.component_scripts();
    let sprite = scripts
        .iter()
        .find(|s| s.component_type() == ComponentType::SpriteConfig);
    match (kind, sprite) {
        (
            EntityKind::IsoObject {
                multi_square_master,
            },
            Some(sprite),
        ) => {
            if let Some(mut c) = component_from_script(sprite.as_ref(), script_full_type, input) {
                if let Component::SpriteConfig(sc) = &mut c {
                    sc.multi_square_master = multi_square_master;
                }
                comps.add(c);
            }
            for s in scripts {
                if s.component_type() == ComponentType::SpriteConfig
                    || !multi_square_master && s.iso_master_only()
                {
                    continue;
                }
                if let Some(c) = component_from_script(s.as_ref(), script_full_type, input) {
                    comps.add(c);
                }
            }
        }
        _ => {
            for s in scripts {
                if let Some(c) = component_from_script(s.as_ref(), script_full_type, input) {
                    comps.add(c);
                }
            }
        }
    }
    comps.add(Component::Script(EntityScriptInfo {
        original_is_item: kind == EntityKind::InventoryItem,
        original_script: Some(script_full_type.to_string()),
    }));
    comps
}
