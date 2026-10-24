//! Components with small saved forms: `EntityScriptInfo`, `UiConfig`,
//! `MetaTagComponent`, `CraftBench`, `SpriteConfig`, `SpriteOverlayConfig`
//! and `Durability`.

use pz_scripts::objects::entity::components::combat::MaterialType;

use crate::context::{EntityContext, Lookup, ScriptRegister};
use crate::io::{err, read_header, HeaderSize, HeaderWrite, Reader, Result, Writer};

/// `zombie.entity.components.script.EntityScriptInfo`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EntityScriptInfo {
    /// `originalIsItem`: the script is an item script.
    pub original_is_item: bool,
    /// `originalScript`: the script's full type.
    pub original_script: Option<String>,
}

impl EntityScriptInfo {
    pub fn load(&mut self, r: &mut Reader<'_>) -> Result<()> {
        self.original_is_item = r.bool()?;
        self.original_script = if r.bool()? { Some(r.string()?) } else { None };
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        w.bool(self.original_is_item);
        w.bool(self.original_script.is_some());
        if let Some(s) = &self.original_script {
            w.string(s);
        }
    }
}

/// `zombie.entity.components.ui.UiConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiConfig {
    pub xui_skin_name: Option<String>,
    pub entity_style_name: Option<String>,
    pub ui_enabled: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            xui_skin_name: None,
            entity_style_name: None,
            ui_enabled: true,
        }
    }
}

impl UiConfig {
    pub fn load(&mut self, r: &mut Reader<'_>) -> Result<()> {
        self.xui_skin_name = if r.bool()? { Some(r.string()?) } else { None };
        self.entity_style_name = if r.bool()? { Some(r.string()?) } else { None };
        self.ui_enabled = r.bool()?;
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        for s in [&self.xui_skin_name, &self.entity_style_name] {
            w.bool(s.is_some());
            if let Some(s) = s {
                w.string(s);
            }
        }
        w.bool(self.ui_enabled);
    }
}

/// `zombie.entity.meta.MetaTagComponent`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MetaTag {
    pub stored_id: i64,
}

/// `zombie.entity.components.crafting.CraftBench`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CraftBench {
    /// `recipeTagQuery`; `null` saves (and loads back) as `""`.
    pub recipe_tag_query: String,
    /// `fluidInputChannels` (`EnumBitStore<ResourceChannel>` bits).
    pub fluid_input_channels: i32,
    /// `energyInputChannels` bits.
    pub energy_input_channels: i32,
}

impl CraftBench {
    pub fn load(&mut self, r: &mut Reader<'_>) -> Result<()> {
        self.recipe_tag_query = r.string()?;
        self.fluid_input_channels = r.i32()?;
        self.energy_input_channels = r.i32()?;
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        w.string(&self.recipe_tag_query);
        w.i32(self.fluid_input_channels);
        w.i32(self.energy_input_channels);
    }
}

/// A script referenced through a `ScriptsDictionary` register
/// (`ScriptRegister.saveScript` / `loadScript`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictScriptRef {
    /// `registryId`, or `None` when the script was not in the dictionary
    /// (saved as a zero flag).
    pub id: Option<i16>,
    /// The script's full type, when the context resolved it.
    pub name: Option<String>,
    /// `getScriptVersion()` written after the reference.
    pub script_version: i64,
}

/// Reads `loadScript` + the version long. `Ok(None)` when the dictionary
/// lookup fails: the original keeps its previous script then.
fn load_dict_script(
    r: &mut Reader<'_>,
    ctx: &dyn EntityContext,
    reg: ScriptRegister,
) -> Result<Option<DictScriptRef>> {
    let id = if r.bool()? { Some(r.i16()?) } else { None };
    let version = r.i64()?;
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(match ctx.dictionary_script(reg, id) {
        Lookup::Missing => None,
        Lookup::Unknown => Some(DictScriptRef {
            id: Some(id),
            name: None,
            script_version: version,
        }),
        // Saving writes the script's own version.
        Lookup::Found(s) => Some(DictScriptRef {
            id: Some(id),
            name: Some(s.name),
            script_version: s.script_version,
        }),
    })
}

fn save_dict_script(w: &mut Writer, s: &DictScriptRef) {
    w.bool(s.id.is_some());
    if let Some(id) = s.id {
        w.i16(id);
    }
    w.i64(s.script_version);
}

/// `zombie.entity.components.spriteconfig.SpriteConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SpriteConfig {
    /// `configScript`.
    pub config_script: Option<DictScriptRef>,
    /// What `save` writes for `isMultiSquareMaster()` (the tile info the
    /// world derives from the owner's sprite); loads set it to the saved
    /// value (`wasLoadedAsMaster`).
    pub multi_square_master: bool,
}

impl SpriteConfig {
    pub fn load(&mut self, r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<()> {
        if !r.bool()? {
            // "Sprite config has no script saved."
            return Ok(());
        }
        let script = load_dict_script(r, ctx, ScriptRegister::SpriteConfigs)?;
        self.multi_square_master = r.bool()?;
        if script.is_some() {
            self.config_script = script;
        }
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        w.bool(self.config_script.is_some());
        if let Some(s) = &self.config_script {
            save_dict_script(w, s);
            w.bool(self.multi_square_master);
        }
    }
}

/// `zombie.entity.components.spriteconfig.SpriteOverlayConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SpriteOverlayConfig {
    pub config_script: Option<DictScriptRef>,
    pub applied_style: Option<String>,
}

impl SpriteOverlayConfig {
    pub fn load(&mut self, r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<()> {
        if !r.bool()? {
            return Ok(());
        }
        if let Some(s) = load_dict_script(r, ctx, ScriptRegister::SpriteOverlayConfigs)? {
            self.config_script = Some(s);
        }
        self.applied_style = None;
        if r.bool()? {
            let style = r.string()?;
            let script = self.config_script.as_ref().and_then(|s| s.name.as_deref());
            if ctx.overlay_style_exists(script, &style) {
                self.applied_style = Some(style);
            }
        }
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        w.bool(self.config_script.is_some());
        if let Some(s) = &self.config_script {
            save_dict_script(w, s);
        }
        w.bool(self.applied_style.is_some());
        if let Some(s) = &self.applied_style {
            w.string(s);
        }
    }
}

/// `zombie.entity.components.combat.Durability`.
#[derive(Clone, Debug, PartialEq)]
pub struct Durability {
    pub current_hit_points: f32,
    pub max_hit_points: f32,
    pub material: MaterialType,
}

impl Default for Durability {
    fn default() -> Self {
        Durability {
            current_hit_points: 1.0,
            max_hit_points: 1.0,
            material: MaterialType::Default,
        }
    }
}

impl Durability {
    pub fn load(&mut self, r: &mut Reader<'_>) -> Result<()> {
        *self = Durability::default();
        let h = read_header(r, HeaderSize::Short)?;
        if h & 1 != 0 {
            self.current_hit_points = r.f32()?;
        }
        if h & 2 != 0 {
            self.max_hit_points = r.f32()?;
        }
        if h & 4 != 0 {
            let s = r.string()?;
            // `MaterialType.valueOf` throws on an unknown name.
            match MaterialType::value_of(&s) {
                Ok(m) => self.material = m,
                Err(_) => {
                    return err(format!(
                        "No enum constant zombie.iso.enums.MaterialType.{s}"
                    ))
                }
            }
        }
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        let mut h = HeaderWrite::start(w, HeaderSize::Short);
        if self.current_hit_points != 1.0 {
            h.add(1);
            w.f32(self.current_hit_points);
        }
        if self.max_hit_points != 1.0 {
            h.add(2);
            w.f32(self.max_hit_points);
        }
        if self.material != MaterialType::Default {
            h.add(4);
            w.string(self.material.name());
        }
        h.write(w);
    }
}
