//! `zombie.entity.components.combat.DurabilityScript` and the
//! `zombie.iso.enums.MaterialType` it stores.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::component_script::cast;
use crate::objects::entity::{array_get, parse_float, ComponentBase, ComponentScript, ComponentType};
use crate::parser::{Block, BlockElement};

entity_enum!(
    /// `zombie.iso.enums.MaterialType`.
    #[allow(non_camel_case_types)]
    MaterialType,
    "zombie.iso.enums.MaterialType",
    [
        Default, Flesh, Flesh_Hollow, Concrete, Plaster, Stone, Wood, Wood_Solid, Brick, Metal, Metal_Large,
        Metal_Light, Metal_Solid, Glass, Glass_Light, Glass_Solid, Cinderblock, Plastic, Ceramic, Rubber, Fabric,
        Carpet, Dirt, Grass, Gravel, Sand, Snow,
    ]
);

#[derive(Debug)]
pub struct DurabilityScript {
    pub component: ComponentBase,
    max_hit_points: f32,
    current_hit_points: f32,
    material: Option<MaterialType>,
}

impl Default for DurabilityScript {
    fn default() -> Self {
        Self::new()
    }
}

impl DurabilityScript {
    pub fn new() -> Self {
        DurabilityScript {
            component: ComponentBase::new(ComponentType::Durability),
            max_hit_points: 0.0,
            current_hit_points: 0.0,
            material: None,
        }
    }

    /// `getMaxHitPoints`.
    pub fn max_hit_points(&self) -> f32 {
        self.max_hit_points
    }

    /// `getCurrentHitPoints`.
    pub fn current_hit_points(&self) -> f32 {
        self.current_hit_points
    }

    /// `getMaterial`.
    pub fn material(&self) -> Option<MaterialType> {
        self.material
    }
}

impl ComponentScript for DurabilityScript {
    component_accessors!("zombie.entity.components.combat.DurabilityScript");

    fn copy_from(&mut self, other: &dyn ComponentScript) -> ScriptResult {
        let other = cast::<DurabilityScript>(other, self)?;
        self.current_hit_points = other.current_hit_points;
        self.max_hit_points = other.max_hit_points;
        self.material = other.material;
        Ok(())
    }

    fn load(&mut self, _sm: &ScriptManager, block: &Block) -> ScriptResult {
        for element in &block.elements {
            let BlockElement::Value(i) = element else {
                continue;
            };
            let s = &block.values[*i].string;
            if java::trim(s).is_empty() || !s.contains('=') {
                continue;
            }
            let split = java::split_char(s, '=');
            let k = java::trim(array_get(&split, 0)?);
            let v = java::trim(array_get(&split, 1)?);
            if java::eq_ignore_case(k, "MaxHitPoints") {
                self.max_hit_points = parse_float(v)?;
                self.current_hit_points = self.max_hit_points;
            } else if java::eq_ignore_case(k, "Material") {
                self.material = Some(MaterialType::value_of(v)?);
            }
        }
        Ok(())
    }
}
