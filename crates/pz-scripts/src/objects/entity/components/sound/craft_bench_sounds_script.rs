//! `zombie.scripting.entity.components.sound.CraftBenchSoundsScript` and
//! the `zombie.entity.components.sounds.CraftBenchSound` entries it holds.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;

/// `CraftBenchSound`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CraftBenchSound {
    pub id: Option<String>,
    pub game_sound: Option<String>,
    pub param1: Option<String>,
    pub param2: Option<String>,
    pub param3: Option<String>,
}

#[derive(Debug)]
pub struct CraftBenchSoundsScript {
    pub component: ComponentBase,
    sounds: Vec<CraftBenchSound>,
}

impl Default for CraftBenchSoundsScript {
    fn default() -> Self {
        Self::new()
    }
}

impl CraftBenchSoundsScript {
    pub fn new() -> Self {
        CraftBenchSoundsScript { component: ComponentBase::new(ComponentType::CraftBenchSounds), sounds: Vec::new() }
    }

    /// `getSounds`.
    pub fn sounds(&self) -> &[CraftBenchSound] {
        &self.sounds
    }
}

impl ComponentScript for CraftBenchSoundsScript {
    component_accessors!("zombie.scripting.entity.components.sound.CraftBenchSoundsScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    /// `load`: `id = gameSound [param1 [param2 [param3]]]`. The original
    /// stores the second parameter again as `param3`.
    fn load(&mut self, _sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            let ss = java::split_ws(val);
            if ss.is_empty() {
                continue;
            }
            self.sounds.push(CraftBenchSound {
                id: Some(key.to_string()),
                game_sound: Some(ss[0].to_string()),
                param1: ss.get(1).map(|s| s.to_string()),
                param2: ss.get(2).map(|s| s.to_string()),
                param3: if ss.len() > 3 { Some(ss[2].to_string()) } else { None },
            });
        }
        Ok(())
    }
}
