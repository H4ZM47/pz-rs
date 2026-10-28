//! `zombie.scripting.objects.FluidDefinitionScript` with the enums it loads:
//! `FluidType`, `FluidCategory` and `PoisonEffect`.

use std::collections::BTreeSet;

use super::{exception, parse_float, Color, FluidFilterScript};
use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::{ScriptLoadMode, ScriptType};

java_enum!(
    /// `zombie.entity.components.fluids.FluidType`.
    FluidType,
    "zombie.entity.components.fluids.FluidType",
    [
        Water, Petrol, RubbingAlcohol, TaintedWater, Beer, Whiskey, SodaPop, Coffee, Tea, Wine, Bleach, Blood,
        Honey, Mead, Acid, SpiffoJuice, SecretFlavoring, CarbonatedWater, CowMilk, SheepMilk, CleaningLiquid,
        AnimalBlood, AnimalGrease, Dye, HairDye, Paint, PoisonWeak, PoisonNormal, PoisonStrong, PoisonPotent,
        AnimalMilk, Modded, None,
    ]
);

impl FluidType {
    /// `getId`.
    pub fn id(self) -> i8 {
        use FluidType::*;
        match self {
            Water => 1,
            Petrol => 2,
            RubbingAlcohol => 3,
            TaintedWater => 4,
            Beer => 5,
            Whiskey => 6,
            SodaPop => 7,
            Coffee => 8,
            Tea => 9,
            Wine => 10,
            Bleach => 11,
            Blood => 12,
            Honey => 14,
            Mead => 15,
            Acid => 16,
            SpiffoJuice => 17,
            SecretFlavoring => 18,
            CarbonatedWater => 19,
            CowMilk => 20,
            SheepMilk => 21,
            CleaningLiquid => 22,
            AnimalBlood => 23,
            AnimalGrease => 24,
            Dye => 64,
            HairDye => 65,
            Paint => 66,
            PoisonWeak => 70,
            PoisonNormal => 71,
            PoisonStrong => 72,
            PoisonPotent => 73,
            AnimalMilk => 74,
            Modded => 127,
            None => -1,
        }
    }

    /// `FromNameLower`: `None` stands for Java's `null`.
    pub fn from_name_lower(name: &str) -> Option<FluidType> {
        let lower = java::to_lower(name);
        Self::ALL.iter().copied().find(|t| java::to_lower(t.name()) == lower)
    }

    /// `containsNameLowercase`.
    pub fn contains_name_lowercase(name: &str) -> bool {
        Self::from_name_lower(name).is_some()
    }
}

java_enum!(
    /// `zombie.entity.components.fluids.FluidCategory`.
    FluidCategory,
    "zombie.entity.components.fluids.FluidCategory",
    [Beverage, Alcoholic, Hazardous, Medical, Industrial, Colors, Dyes, HairDyes, Paint, Fuel, Poisons, Water]
);

impl FluidCategory {
    /// `getId`: ordinal + 1.
    pub fn id(self) -> i8 {
        self as i8 + 1
    }
}

java_enum!(
    /// `zombie.entity.components.fluids.PoisonEffect`.
    PoisonEffect,
    "zombie.entity.components.fluids.PoisonEffect",
    [None, Mild, Medium, Severe, Extreme, Deadly]
);

impl PoisonEffect {
    /// `getLevel`.
    pub fn level(self) -> i32 {
        match self {
            PoisonEffect::None => 0,
            PoisonEffect::Mild => 1,
            PoisonEffect::Medium => 2,
            PoisonEffect::Severe => 3,
            PoisonEffect::Extreme => 4,
            PoisonEffect::Deadly => 100,
        }
    }

    /// `getPlayerEffect`.
    pub fn player_effect(self) -> i32 {
        self.level() * 7
    }
}

/// `FluidDefinitionScript.PropertyValue`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PropertyValue {
    default_value: f32,
    value: f32,
}

impl PropertyValue {
    /// `isSet`.
    fn is_set(&self) -> bool {
        self.value != self.default_value
    }

    /// `reset`.
    fn reset(&mut self) {
        self.value = self.default_value;
    }
}

/// The names passed to `addProperty`, in field order (all default to 0).
const PROPERTY_NAMES: [&str; 14] = [
    "fatigueChange",
    "hungerChange",
    "stressChange",
    "thirstChange",
    "unhappyChange",
    "calories",
    "carbohydrates",
    "lipids",
    "proteins",
    "alcohol",
    "fluReduction",
    "painReduction",
    "enduranceChange",
    "foodSicknessChange",
];

const FATIGUE_CHANGE: usize = 0;
const HUNGER_CHANGE: usize = 1;
const STRESS_CHANGE: usize = 2;
const THIRST_CHANGE: usize = 3;
const UNHAPPY_CHANGE: usize = 4;
const CALORIES: usize = 5;
const CARBOHYDRATES: usize = 6;
const LIPIDS: usize = 7;
const PROTEINS: usize = 8;
const ALCOHOL: usize = 9;
const FLU_REDUCTION: usize = 10;
const PAIN_REDUCTION: usize = 11;
const ENDURANCE_CHANGE: usize = 12;
const FOOD_SICKNESS_CHANGE: usize = 13;

const DEFAULT_COLOR: Color = Color::new(0.0, 0.0, 1.0, 1.0);

#[derive(Debug)]
pub struct FluidDefinitionScript {
    pub base: BaseScriptObject,
    properties: [PropertyValue; 14],
    exists_as_vanilla: bool,
    mod_id: Option<String>,
    fluid_type: FluidType,
    fluid_type_string: Option<String>,
    color_reference: Option<String>,
    color: Color,
    categories: BTreeSet<FluidCategory>,
    display_name: String,
    blend_whitelist: Option<Box<FluidFilterScript>>,
    blend_blacklist: Option<Box<FluidFilterScript>>,
    has_poison: bool,
    poison_max_effect: PoisonEffect,
    poison_min_amount: f32,
    poison_dilute_ratio: f32,
}

impl FluidDefinitionScript {
    pub fn new() -> Self {
        FluidDefinitionScript {
            base: BaseScriptObject::new(ScriptType::FluidDefinition),
            properties: [PropertyValue { default_value: 0.0, value: 0.0 }; 14],
            exists_as_vanilla: false,
            mod_id: None,
            fluid_type: FluidType::None,
            fluid_type_string: None,
            color_reference: None,
            color: DEFAULT_COLOR,
            categories: BTreeSet::new(),
            display_name: "Fluid".to_string(),
            blend_whitelist: None,
            blend_blacklist: None,
            has_poison: false,
            poison_max_effect: PoisonEffect::None,
            poison_min_amount: 1.0,
            poison_dilute_ratio: 0.0,
        }
    }

    fn prop(&self, i: usize) -> f32 {
        self.properties[i].value
    }

    /// `getExistsAsVanilla`.
    pub fn exists_as_vanilla(&self) -> bool {
        self.exists_as_vanilla
    }

    /// `isVanilla`.
    pub fn is_vanilla(&self) -> bool {
        self.mod_id.as_deref() == Some("pz-vanilla")
    }

    /// `getModID`.
    pub fn mod_id(&self) -> Option<&str> {
        self.mod_id.as_deref()
    }

    /// `getFluidType`.
    pub fn fluid_type(&self) -> FluidType {
        self.fluid_type
    }

    /// `getFluidTypeString`.
    pub fn fluid_type_string(&self) -> Option<&str> {
        self.fluid_type_string.as_deref()
    }

    /// `getDisplayName`.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The `colorReference` key, which `getColor` resolves through
    /// `Colors.GetColorByName` (throwing when unknown). TODO(Colors)
    pub fn color_reference(&self) -> Option<&str> {
        self.color_reference.as_deref()
    }

    /// `getColor` without the `colorReference` lookup.
    pub fn color(&self) -> Color {
        self.color
    }

    /// `getCategories`, in `EnumSet` (ordinal) order.
    pub fn categories(&self) -> &BTreeSet<FluidCategory> {
        &self.categories
    }

    /// `getBlendWhitelist`.
    pub fn blend_whitelist(&self) -> Option<&FluidFilterScript> {
        self.blend_whitelist.as_deref()
    }

    /// `getBlendBlackList`.
    pub fn blend_blacklist(&self) -> Option<&FluidFilterScript> {
        self.blend_blacklist.as_deref()
    }

    pub fn has_poison(&self) -> bool {
        self.has_poison
    }

    /// `getPoisonMaxEffect`.
    pub fn poison_max_effect(&self) -> PoisonEffect {
        self.poison_max_effect
    }

    /// `getPoisonMinAmount`.
    pub fn poison_min_amount(&self) -> f32 {
        self.poison_min_amount
    }

    /// `getPoisonDiluteRatio`.
    pub fn poison_dilute_ratio(&self) -> f32 {
        self.poison_dilute_ratio
    }

    /// `getFatigueChange`.
    pub fn fatigue_change(&self) -> f32 {
        self.prop(FATIGUE_CHANGE) / 100.0
    }

    /// `getHungerChange`.
    pub fn hunger_change(&self) -> f32 {
        self.prop(HUNGER_CHANGE) / 100.0
    }

    /// `getStressChange`.
    pub fn stress_change(&self) -> f32 {
        self.prop(STRESS_CHANGE) / 100.0
    }

    /// `getThirstChange`.
    pub fn thirst_change(&self) -> f32 {
        self.prop(THIRST_CHANGE) / 100.0
    }

    /// `getUnhappyChange`.
    pub fn unhappy_change(&self) -> f32 {
        self.prop(UNHAPPY_CHANGE)
    }

    /// `getCalories`.
    pub fn calories(&self) -> f32 {
        self.prop(CALORIES)
    }

    /// `getCarbohydrates`.
    pub fn carbohydrates(&self) -> f32 {
        self.prop(CARBOHYDRATES)
    }

    /// `getLipids`.
    pub fn lipids(&self) -> f32 {
        self.prop(LIPIDS)
    }

    /// `getProteins`.
    pub fn proteins(&self) -> f32 {
        self.prop(PROTEINS)
    }

    /// `getAlcohol`.
    pub fn alcohol(&self) -> f32 {
        self.prop(ALCOHOL)
    }

    /// `getFluReduction`.
    pub fn flu_reduction(&self) -> f32 {
        self.prop(FLU_REDUCTION)
    }

    /// `getPainReduction`.
    pub fn pain_reduction(&self) -> f32 {
        self.prop(PAIN_REDUCTION)
    }

    /// `getEnduranceChange`.
    pub fn endurance_change(&self) -> f32 {
        self.prop(ENDURANCE_CHANGE)
    }

    /// `getFoodSicknessChange`: `(int)` cast of the stored float.
    pub fn food_sickness_change(&self) -> i32 {
        self.prop(FOOD_SICKNESS_CHANGE) as i32
    }

    /// `hasPropertiesSet`.
    pub fn has_properties_set(&self) -> bool {
        self.properties.iter().any(PropertyValue::is_set)
    }

    /// Raw `(name, value)` of every property, in declaration order.
    pub fn properties(&self) -> impl Iterator<Item = (&'static str, f32)> + '_ {
        PROPERTY_NAMES.iter().zip(&self.properties).map(|(n, p)| (*n, p.value))
    }

    /// `LoadCategories`.
    fn load_categories(&mut self, block: &Block) -> ScriptResult {
        for value in &block.values {
            if java::is_null_or_whitespace(Some(&value.string)) {
                continue;
            }
            self.categories.insert(FluidCategory::value_of(java::trim(&value.string))?);
        }
        Ok(())
    }

    /// `LoadPoison`.
    fn load_poison(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            self.has_poison = true;
            if java::eq_ignore_case("maxEffect", key) {
                self.poison_max_effect = PoisonEffect::value_of(val)?;
            } else if java::eq_ignore_case("minAmount", key) {
                self.poison_min_amount = parse_float(val)?;
            } else if java::eq_ignore_case("diluteRatio", key) {
                self.poison_dilute_ratio = parse_float(val)?;
            } else {
                sm.script_error(&format!(
                    "Unknown key '{key}' val({val}) in fluid poison definition: {}",
                    self.base.full_type()?
                ));
                if sm.debug {
                    return Err(exception("FluidDefinition error."));
                }
            }
        }
        Ok(())
    }

    /// `LoadProperties`.
    fn load_properties(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            match PROPERTY_NAMES.iter().position(|n| java::eq_ignore_case(n, key)) {
                Some(i) => self.properties[i].value = parse_float(val)?,
                None => {
                    sm.script_error(&format!(
                        "Unknown key '{key}' val({val}) in fluid properties definition: {}",
                        self.base.full_type()?
                    ));
                    if sm.debug {
                        return Err(exception("FluidDefinition error."));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Default for FluidDefinitionScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for FluidDefinitionScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.FluidDefinitionScript"
    }

    /// `InitLoadPP`.
    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        self.mod_id = sm.current_load_file_mod();
        if self.mod_id.as_deref() == Some("pz-vanilla") {
            self.exists_as_vanilla = true;
        }
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        if let Some(t) = FluidType::from_name_lower(name) {
            self.fluid_type = t;
        } else {
            self.fluid_type = FluidType::Modded;
            self.fluid_type_string = Some(name.to_string());
        }
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "displayName") {
                // TODO(Translator): Translator.getFluidText(val).
                self.display_name = val.to_string();
            } else if java::eq_ignore_case(key, "colorReference") {
                self.color_reference = Some(val.to_string());
            } else if java::eq_ignore_case("r", key) {
                self.color.r = parse_float(val)?;
            } else if java::eq_ignore_case("g", key) {
                self.color.g = parse_float(val)?;
            } else if java::eq_ignore_case("b", key) {
                self.color.b = parse_float(val)?;
            } else if java::eq_ignore_case(key, "color") {
                let opt = java::split_char(val, ':');
                if opt.len() != 3 {
                    continue;
                }
                self.color.r = parse_float(opt[0])?;
                self.color.g = parse_float(opt[1])?;
                self.color.b = parse_float(opt[2])?;
            } else {
                sm.script_error(&format!(
                    "Unknown key '{key}' val({val}) in fluid definition: {}",
                    self.base.full_type()?
                ));
                if sm.debug {
                    return Err(exception("FluidDefinition error."));
                }
            }
        }
        for child in &block.children {
            let t = child.type_str();
            if java::eq_ignore_case("blendWhiteList", t) {
                let mut f = FluidFilterScript::get_anonymous_with(true);
                f.load_anonymous_from_block(child)?;
                self.blend_whitelist = Some(Box::new(f));
            } else if java::eq_ignore_case("blendBlackList", t) {
                let mut f = FluidFilterScript::get_anonymous_with(false);
                f.load_anonymous_from_block(child)?;
                self.blend_blacklist = Some(Box::new(f));
            } else if java::eq_ignore_case("categories", t) {
                self.load_categories(child)?;
            } else if java::eq_ignore_case("properties", t) {
                self.load_properties(sm, child)?;
            } else if java::eq_ignore_case("poison", t) {
                self.load_poison(sm, child)?;
            } else {
                // The original reports the outer block's type and id.
                sm.script_error(&format!(
                    "Unknown block '{}' val({}) in fluid definition: {}",
                    block.type_.as_deref().unwrap_or("null"),
                    block.id.as_deref().unwrap_or("null"),
                    self.base.full_type()?
                ));
                if sm.debug {
                    return Err(exception("FluidDefinition error."));
                }
            }
        }
        Ok(())
    }

    /// `PreReload`.
    fn pre_reload(&mut self) {
        self.exists_as_vanilla = false;
        self.mod_id = None;
        self.fluid_type = FluidType::None;
        self.fluid_type_string = None;
        self.color_reference = None;
        self.color = DEFAULT_COLOR;
        self.categories.clear();
        self.display_name = "Fluid".to_string();
        self.blend_whitelist = None;
        self.blend_blacklist = None;
        self.has_poison = false;
        self.poison_max_effect = PoisonEffect::None;
        self.poison_min_amount = 1.0;
        self.poison_dilute_ratio = 0.0;
        for p in &mut self.properties {
            p.reset();
        }
    }

    /// `OnScriptsLoaded`.
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        if self.fluid_type == FluidType::None && self.fluid_type_string.is_none() {
            return Err(exception("No fluid type set."));
        }
        if self.has_poison && self.poison_max_effect == PoisonEffect::None {
            return Err(exception("Poison block defined but poison effect is 'None'"));
        }
        Ok(())
    }
}
