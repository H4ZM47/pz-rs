//! ScriptsDictionary registration runs after the component scripts finish
//! loading. Keep saved ids and assign new ids in the native loadList order.
use pz_inventory::dictionary::{Register, RegisterEntry, WorldDictionary};
use pz_scripts::java::JHashMap;
use pz_scripts::objects::entity::components::spriteconfig::{
    SpriteConfigScript, SpriteOverlayConfigScript,
};
use pz_scripts::objects::entity::{ComponentType, GameEntityScript};
use pz_scripts::{ScriptManager, ScriptType};
use sha2::{Digest, Sha256};

fn version(text: &str) -> i64 {
    // Guava HashCode.asLong reads the first eight digest bytes little endian.
    let hash = Sha256::digest(text.as_bytes());
    i64::from_le_bytes(hash[..8].try_into().unwrap())
}

#[derive(Default)]
struct Loaded {
    sprites: JHashMap<i64>,
    overlays: JHashMap<i64>,
}

impl Loaded {
    fn record(&mut self, entity: &GameEntityScript) -> Result<(), String> {
        let name = entity.base.script_object_name();
        if let Some(config) = entity.component::<SpriteConfigScript>(ComponentType::SpriteConfig) {
            let full = entity.full_name().map_err(|e| e.to_string())?;
            self.sprites
                .insert(full, version(&config.dictionary_version_text(name)));
        }
        if let Some(overlay) =
            entity.component::<SpriteOverlayConfigScript>(ComponentType::SpriteOverlayConfig)
        {
            let full = entity.full_name().map_err(|e| e.to_string())?;
            let text = overlay
                .dictionary_version_text(name)
                .ok_or_else(|| format!("null overlay style in {full}"))?;
            self.overlays.insert(full, version(&text));
        }
        Ok(())
    }
}

pub(super) fn register(dictionary: &mut WorldDictionary, sm: &ScriptManager) -> Result<(), String> {
    let mut loaded = Loaded::default();
    for collection in sm.collections() {
        match collection.script_type() {
            ScriptType::Entity => {
                for script in sm.get_all::<GameEntityScript>(ScriptType::Entity) {
                    loaded.record(&script.borrow())?;
                }
            }
            ScriptType::Item => {
                for script in sm.get_all::<pz_scripts::objects::items::Item>(ScriptType::Item) {
                    loaded.record(&script.borrow().entity)?;
                }
            }
            _ => {}
        }
    }
    let empty = JHashMap::default();
    for (name, scripts) in [
        ("SpriteConfigs", &loaded.sprites),
        ("SpriteOverlayConfigs", &loaded.overlays),
        ("ContextMenuConfigs", &empty),
    ] {
        let register = if let Some(i) = dictionary
            .script_registers
            .iter()
            .position(|r| r.name == name)
        {
            &mut dictionary.script_registers[i]
        } else {
            dictionary.script_registers.push(Register {
                name: name.into(),
                ..Default::default()
            });
            dictionary.script_registers.last_mut().unwrap()
        };
        reconcile(register, scripts)?;
    }
    Ok(())
}

fn reconcile(register: &mut Register, scripts: &JHashMap<i64>) -> Result<(), String> {
    let mut named = JHashMap::default();
    for entry in register.entries.drain(..) {
        named.insert(entry.name.clone(), entry);
    }
    for (name, &version) in scripts.iter() {
        if let Some(stored) = named.get_mut(name) {
            stored.version = Some(version);
            stored.is_loaded = true;
        } else {
            let id = register.next_id;
            register.next_id = id
                .checked_add(1)
                .ok_or_else(|| format!("{} script id limit reached", register.name))?;
            named.insert(
                name.to_string(),
                RegisterEntry {
                    registry_id: id,
                    name: name.to_string(),
                    version: Some(version),
                    is_loaded: true,
                },
            );
        }
    }
    register.entries = named.values().cloned().collect();
    Ok(())
}
