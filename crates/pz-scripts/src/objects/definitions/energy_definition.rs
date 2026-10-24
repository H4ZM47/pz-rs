//! `zombie.scripting.objects.EnergyDefinitionScript` and
//! `zombie.entity.energy.EnergyType`.

use super::{exception, parse_float, Color};
use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::{ScriptLoadMode, ScriptType};

java_enum!(
    /// `zombie.entity.energy.EnergyType`.
    EnergyType,
    "zombie.entity.energy.EnergyType",
    [None, Electric, Mechanical, Thermal, Steam, VoidEnergy, Modded]
);

impl EnergyType {
    /// `getId`.
    pub fn id(self) -> i8 {
        match self {
            EnergyType::None => -1,
            EnergyType::Electric => 1,
            EnergyType::Mechanical => 2,
            EnergyType::Thermal => 3,
            EnergyType::Steam => 4,
            EnergyType::VoidEnergy => 126,
            EnergyType::Modded => 127,
        }
    }

    /// `FromNameLower`: `None` stands for Java's `null`.
    pub fn from_name_lower(name: &str) -> Option<EnergyType> {
        let lower = java::to_lower(name);
        Self::ALL.iter().copied().find(|t| java::to_lower(t.name()) == lower)
    }

    /// `containsNameLowercase`.
    pub fn contains_name_lowercase(name: &str) -> bool {
        Self::from_name_lower(name).is_some()
    }
}

const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);

#[derive(Debug)]
pub struct EnergyDefinitionScript {
    pub base: BaseScriptObject,
    exists_as_vanilla: bool,
    mod_id: Option<String>,
    energy_type: EnergyType,
    energy_type_string: Option<String>,
    display_name: Option<String>,
    color: Color,
    icon_texture_name: Option<String>,
    horizontal_bar_texture_name: Option<String>,
    vertical_bar_texture_name: Option<String>,
}

impl EnergyDefinitionScript {
    pub fn new() -> Self {
        EnergyDefinitionScript {
            base: BaseScriptObject::new(ScriptType::EnergyDefinition),
            exists_as_vanilla: false,
            mod_id: None,
            energy_type: EnergyType::None,
            energy_type_string: None,
            display_name: None,
            color: WHITE,
            icon_texture_name: None,
            horizontal_bar_texture_name: None,
            vertical_bar_texture_name: None,
        }
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

    /// `getEnergyType`.
    pub fn energy_type(&self) -> EnergyType {
        self.energy_type
    }

    /// `getEnergyTypeString`.
    pub fn energy_type_string(&self) -> Option<&str> {
        self.energy_type_string.as_deref()
    }

    /// `getDisplayName`.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// `getColor`.
    pub fn color(&self) -> Color {
        self.color
    }

    /// The `iconTexture` key; `getIconTexture` resolves it. TODO(Texture)
    pub fn icon_texture_name(&self) -> Option<&str> {
        self.icon_texture_name.as_deref()
    }

    /// The `horizontalBarTexture` key. TODO(Texture)
    pub fn horizontal_bar_texture_name(&self) -> Option<&str> {
        self.horizontal_bar_texture_name.as_deref()
    }

    /// The `verticalBarTexture` key. TODO(Texture)
    pub fn vertical_bar_texture_name(&self) -> Option<&str> {
        self.vertical_bar_texture_name.as_deref()
    }
}

impl Default for EnergyDefinitionScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for EnergyDefinitionScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.EnergyDefinitionScript"
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
        if let Some(t) = EnergyType::from_name_lower(name) {
            self.energy_type = t;
        } else {
            self.energy_type = EnergyType::Modded;
            self.energy_type_string = Some(name.to_string());
        }
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "displayName") {
                // TODO(Translator): Translator.getEntityText(val).
                self.display_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "r") {
                self.color.r = parse_float(val)?;
            } else if java::eq_ignore_case(key, "g") {
                self.color.g = parse_float(val)?;
            } else if java::eq_ignore_case(key, "b") {
                self.color.b = parse_float(val)?;
            } else if java::eq_ignore_case(key, "color") {
                let opt = java::split_char(val, ':');
                if opt.len() != 3 {
                    continue;
                }
                self.color.r = parse_float(opt[0])?;
                self.color.g = parse_float(opt[1])?;
                self.color.b = parse_float(opt[2])?;
            } else if java::eq_ignore_case(key, "iconTexture") {
                self.icon_texture_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "horizontalBarTexture") {
                self.horizontal_bar_texture_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "verticalBarTexture") {
                self.vertical_bar_texture_name = Some(val.to_string());
            } else {
                sm.script_error(&format!("Unknown key '{key}' val({val}) in energy definition: {name}"));
                if sm.debug {
                    return Err(exception("EnergyDefinition error."));
                }
            }
        }
        Ok(())
    }

    /// `PreReload`.
    fn pre_reload(&mut self) {
        self.base.enabled = true;
        self.exists_as_vanilla = false;
        self.mod_id = None;
        self.energy_type = EnergyType::None;
        self.energy_type_string = None;
        self.display_name = None;
        self.color = WHITE;
        self.icon_texture_name = None;
        self.horizontal_bar_texture_name = None;
        self.vertical_bar_texture_name = None;
    }

    /// `OnScriptsLoaded`. Resolving the three textures (falling back to the
    /// static defaults) is left to the texture system. TODO(Texture)
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        if self.energy_type == EnergyType::None && self.energy_type_string.is_none() {
            return Err(exception("No energy type set."));
        }
        Ok(())
    }
}
