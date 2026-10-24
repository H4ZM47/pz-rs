//! `zombie.scripting.entity.components.fluids.FluidContainerScript` and its
//! nested `FluidScript`.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::definitions::{Color, FluidFilterScript};
use crate::objects::entity::component_script::cast;
use crate::objects::entity::{
    array_get, contains_whitespace, log_error, normalize, parse_float, remove_whitespace, ComponentBase,
    ComponentScript, ComponentType,
};
use crate::parser::{Block, BlockElement};
use crate::script_type::ScriptLoadMode;

const TAG_OVERRIDE: &str = "override";

/// `FluidUtil.getMinContainerCapacity`.
const MIN_CONTAINER_CAPACITY: f32 = 0.05;

/// `FluidContainerScript.FluidScript`: one initial fluid.
#[derive(Clone, Debug, PartialEq)]
pub struct FluidScript {
    fluid_type: String,
    percentage: f32,
    custom_color: Option<Color>,
}

impl FluidScript {
    fn new(fluid_type: &str) -> Self {
        FluidScript { fluid_type: fluid_type.to_string(), percentage: 1.0, custom_color: None }
    }

    /// `getFluidType`.
    pub fn fluid_type(&self) -> &str {
        &self.fluid_type
    }

    /// `getPercentage`.
    pub fn percentage(&self) -> f32 {
        self.percentage
    }

    /// `getCustomColor`.
    pub fn custom_color(&self) -> Option<Color> {
        self.custom_color
    }
    // TODO(Fluid): `getFluid` resolves `Fluid.Get(fluidType)` lazily.
}

#[derive(Debug)]
pub struct FluidContainerScript {
    pub component: ComponentBase,
    whitelist: Option<FluidFilterScript>,
    blacklist: Option<FluidFilterScript>,
    container_name: String,
    capacity: f32,
    initial_amount_set: bool,
    initial_amount_min: f32,
    initial_amount_max: f32,
    initial_fluids: Option<Vec<FluidScript>>,
    initial_fluids_is_random: bool,
    input_locked: bool,
    can_empty: bool,
    hidden_amount: bool,
    rain_catcher: f32,
    fills_with_clean_water: bool,
    custom_drink_sound: String,
    transfer_rate: f32,
}

impl Default for FluidContainerScript {
    fn default() -> Self {
        Self::new()
    }
}

impl FluidContainerScript {
    pub fn new() -> Self {
        FluidContainerScript {
            component: ComponentBase::new(ComponentType::FluidContainer),
            whitelist: None,
            blacklist: None,
            container_name: "FluidContainer".to_string(),
            capacity: 1.0,
            initial_amount_set: false,
            initial_amount_min: 0.0,
            initial_amount_max: 1.0,
            initial_fluids: None,
            initial_fluids_is_random: false,
            input_locked: false,
            can_empty: true,
            hidden_amount: false,
            rain_catcher: 0.0,
            fills_with_clean_water: false,
            custom_drink_sound: "DrinkingFromGeneric".to_string(),
            transfer_rate: 1.0,
        }
    }

    /// `loadBlockFluids`.
    fn load_block_fluids(&mut self, sm: &ScriptManager, block: &Block, is_override: bool) -> ScriptResult {
        if is_override {
            if let Some(f) = &mut self.initial_fluids {
                f.clear();
            }
        }
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if !java::eq_ignore_case(k, "Fluid") {
                continue;
            }
            let fs = Self::read_fluid(v)?;
            self.initial_fluids.get_or_insert_with(Vec::new).push(fs);
        }
        for child in &block.children {
            if !java::eq_ignore_case(child.type_str(), "Fluid") {
                continue;
            }
            match Self::read_fluid_as_block(child)? {
                Some(fs) => self.initial_fluids.get_or_insert_with(Vec::new).push(fs),
                None => sm.warn("Unable to read fluid block."),
            }
        }
        Ok(())
    }

    /// `readFluidAsBlock`: `type` (last one wins), then `percentage`, `r`, `g`, `b`.
    fn read_fluid_as_block(block: &Block) -> ScriptResult<Option<FluidScript>> {
        let mut fs = None;
        for value in &block.values {
            if java::eq_ignore_case("type", java::trim(value.get_key())) {
                fs = Some(FluidScript::new(java::trim(value.get_value())));
            }
        }
        let Some(mut fs) = fs else { return Ok(None) };
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if java::eq_ignore_case("percentage", k) {
                fs.percentage = parse_float(v)?;
                continue;
            }
            let color = || Color::new(1.0, 1.0, 1.0, 1.0);
            if java::eq_ignore_case("r", k) {
                fs.custom_color.get_or_insert_with(color).r = parse_float(v)?;
            } else if java::eq_ignore_case("g", k) {
                fs.custom_color.get_or_insert_with(color).g = parse_float(v)?;
            } else if java::eq_ignore_case("b", k) {
                fs.custom_color.get_or_insert_with(color).b = parse_float(v)?;
            }
        }
        Ok(Some(fs))
    }

    /// `readFluid`: `type[:percentage[:r:g:b]]`.
    fn read_fluid(v: &str) -> ScriptResult<FluidScript> {
        let opt = java::split_char(v, ':');
        let mut fs = FluidScript::new(array_get(&opt, 0)?);
        if opt.len() > 1 {
            fs.percentage = parse_float(opt[1])?;
        }
        if opt.len() == 5 {
            let r = parse_float(opt[2])?;
            let g = parse_float(opt[3])?;
            let b = parse_float(opt[4])?;
            fs.custom_color = Some(Color::new(r, g, b, 1.0));
        }
        Ok(fs)
    }

    /// The whitelist filter script. TODO(Fluid): `getWhitelistCopy` copies
    /// the runtime `FluidFilter`.
    pub fn whitelist(&self) -> Option<&FluidFilterScript> {
        self.whitelist.as_ref()
    }

    /// The blacklist filter script (see [`whitelist`](Self::whitelist)).
    pub fn blacklist(&self) -> Option<&FluidFilterScript> {
        self.blacklist.as_ref()
    }

    /// `getContainerName`.
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// `getCustomDrinkSound`.
    pub fn custom_drink_sound(&self) -> &str {
        &self.custom_drink_sound
    }

    /// `getCapacity`.
    pub fn capacity(&self) -> f32 {
        self.capacity
    }

    /// Whether `InitialPercent*` was set (`getInitialAmount` returns the
    /// capacity otherwise).
    pub fn initial_amount_set(&self) -> bool {
        self.initial_amount_set
    }

    /// The `[min, max]` range `getInitialAmount` rolls in.
    /// TODO(Rand): the roll itself.
    pub fn initial_amount_range(&self) -> (f32, f32) {
        (self.initial_amount_min, self.initial_amount_max)
    }

    /// `getInitialFluids`.
    pub fn initial_fluids(&self) -> Option<&[FluidScript]> {
        self.initial_fluids.as_deref()
    }

    /// `isInitialFluidsIsRandom`.
    pub fn is_initial_fluids_is_random(&self) -> bool {
        self.initial_fluids_is_random
    }

    /// `getInputLocked`.
    pub fn input_locked(&self) -> bool {
        self.input_locked
    }

    /// `getCanEmpty`.
    pub fn can_empty(&self) -> bool {
        self.can_empty
    }

    /// `isHiddenAmount`.
    pub fn is_hidden_amount(&self) -> bool {
        self.hidden_amount
    }

    /// `getRainCatcher`.
    pub fn rain_catcher(&self) -> f32 {
        self.rain_catcher
    }

    /// `isFilledWithCleanWater`.
    pub fn is_filled_with_clean_water(&self) -> bool {
        self.fills_with_clean_water
    }

    /// `getTransferRate`.
    pub fn transfer_rate(&self) -> f32 {
        self.transfer_rate
    }
}

impl ComponentScript for FluidContainerScript {
    component_accessors!("zombie.scripting.entity.components.fluids.FluidContainerScript");

    fn pre_reload(&mut self) {
        let component = std::mem::replace(&mut self.component, ComponentBase::new(ComponentType::FluidContainer));
        *self = FluidContainerScript { component, ..FluidContainerScript::new() };
    }

    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        self.capacity = self.capacity.max(MIN_CONTAINER_CAPACITY);
        if self.initial_amount_min > self.initial_amount_max {
            std::mem::swap(&mut self.initial_amount_min, &mut self.initial_amount_max);
        }
        self.initial_amount_min = self.initial_amount_min.clamp(0.0, 1.0);
        self.initial_amount_max = self.initial_amount_max.clamp(0.0, 1.0);
        self.initial_amount_min *= self.capacity;
        self.initial_amount_max *= self.capacity;
        if !self.initial_fluids_is_random {
            if let Some(fluids) = self.initial_fluids.as_mut().filter(|f| !f.is_empty()) {
                let mut values: Vec<f32> = fluids.iter().map(|f| f.percentage).collect();
                normalize(&mut values);
                for (f, v) in fluids.iter_mut().zip(values) {
                    f.percentage = v;
                }
            }
        }
        // TODO(Fluid): the filters are also sealed.
        if let Some(w) = &mut self.whitelist {
            w.create_filter()?;
        }
        if let Some(b) = &mut self.blacklist {
            b.create_filter()?;
        }
        Ok(())
    }

    /// `copyFrom`. The original builds a new (empty) `initialFluids` list
    /// and never adds the copied fluids to it.
    fn copy_from(&mut self, other: &dyn ComponentScript) -> ScriptResult {
        let other = cast::<FluidContainerScript>(other, self)?;
        self.container_name = other.container_name.clone();
        self.capacity = other.capacity;
        self.initial_fluids_is_random = other.initial_fluids_is_random;
        self.initial_amount_set = other.initial_amount_set;
        self.initial_amount_min = other.initial_amount_min;
        self.initial_amount_max = other.initial_amount_max;
        self.input_locked = other.input_locked;
        self.can_empty = other.can_empty;
        self.rain_catcher = other.rain_catcher;
        self.fills_with_clean_water = other.fills_with_clean_water;
        self.hidden_amount = other.hidden_amount;
        self.custom_drink_sound = other.custom_drink_sound.clone();
        self.transfer_rate = other.transfer_rate;
        if other.initial_fluids.is_some() {
            self.initial_fluids = Some(Vec::new());
        }
        if let Some(w) = &other.whitelist {
            self.whitelist = Some(w.copy());
        }
        if let Some(b) = &other.blacklist {
            self.blacklist = Some(b.copy());
        }
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for element in &block.elements {
            let child = match element {
                BlockElement::Value(i) => {
                    let s = &block.values[*i].string;
                    if java::trim(s).is_empty() || !s.contains('=') {
                        continue;
                    }
                    let split = java::split_char(s, '=');
                    let k = java::trim(array_get(&split, 0)?);
                    let v = java::trim(array_get(&split, 1)?);
                    self.load_value(sm, k, v)?;
                    continue;
                }
                BlockElement::Block(i) => &block.children[*i],
            };
            // The original tests the component block's id, not the child's.
            let is_override = block.id.as_deref().is_some_and(|id| java::eq_ignore_case(id, TAG_OVERRIDE));
            if java::eq_ignore_case(child.type_str(), "fluids") {
                self.load_block_fluids(sm, child, is_override)?;
                continue;
            }
            let is_white_list = java::eq_ignore_case(child.type_str(), "whitelist");
            if !is_white_list && !java::eq_ignore_case(child.type_str(), "blacklist") {
                continue;
            }
            let existing = if is_white_list { &mut self.whitelist } else { &mut self.blacklist };
            let mut filter = match existing.take() {
                Some(f) if !is_override => f,
                _ => FluidFilterScript::get_anonymous_with(is_white_list),
            };
            let r = filter.load_anonymous_from_block(child);
            *existing = Some(filter);
            r?;
        }
        Ok(())
    }
}

impl FluidContainerScript {
    fn load_value(&mut self, sm: &ScriptManager, k: &str, v: &str) -> ScriptResult {
        if java::eq_ignore_case(k, "Capacity") {
            self.capacity = parse_float(v)?;
        } else if java::eq_ignore_case(k, "ContainerName") {
            let mut v = v.to_string();
            if contains_whitespace(&v) {
                log_error(sm, &format!("Sanitizing container name '{v}', name may not contain whitespaces."));
                v = remove_whitespace(&v);
            }
            self.container_name = v;
        } else if java::eq_ignore_case(k, "InitialPercent") {
            self.initial_amount_min = parse_float(v)?;
            self.initial_amount_max = self.initial_amount_min;
            self.initial_amount_set = true;
        } else if java::eq_ignore_case(k, "InitialPercentMin") {
            self.initial_amount_min = parse_float(v)?;
            self.initial_amount_set = true;
        } else if java::eq_ignore_case(k, "InitialPercentMax") {
            self.initial_amount_max = parse_float(v)?;
            self.initial_amount_set = true;
        } else if java::eq_ignore_case(k, "PickRandomFluid") {
            self.initial_fluids_is_random = java::eq_ignore_case(v, "true");
        } else if java::eq_ignore_case(k, "InputLocked") {
            self.input_locked = java::eq_ignore_case(v, "true");
        } else if java::eq_ignore_case(k, "Opened") {
            self.can_empty = java::eq_ignore_case(v, "true");
        } else if java::eq_ignore_case(k, "HiddenAmount") {
            self.hidden_amount = java::eq_ignore_case(v, "true");
        } else if java::eq_ignore_case(k, "RainFactor") {
            self.rain_catcher = parse_float(v)?;
        } else if java::eq_ignore_case(k, "FillsWithCleanWater") {
            self.fills_with_clean_water = java::eq_ignore_case(v, "true");
        } else if java::eq_ignore_case(k, "CustomDrinkSound") {
            self.custom_drink_sound = v.to_string();
        } else if java::eq_ignore_case(k, "TransferRate") {
            self.transfer_rate = parse_float(v)?;
        }
        Ok(())
    }
}
