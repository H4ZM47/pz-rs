//! `InventoryItemFactory.CreateItem(registryID)` from a save's
//! `WorldDictionary` and the loaded item scripts, as an [`ItemEnv`].

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use pz_scripts::objects::entity::ComponentType;
use pz_scripts::objects::items::Item;
use pz_scripts::ScriptManager;

use crate::component::{ComponentCodec, OpaqueComponents};
use crate::dictionary::WorldDictionary;
use crate::env::{ItemEnv, WorldSpriteInfo};
use crate::template::ItemTemplate;

/// Items of a save: the dictionary's ids mapped to script templates.
pub struct ScriptItems {
    pub dictionary: WorldDictionary,
    templates: HashMap<i16, Rc<ItemTemplate>>,
    /// `GameTime.getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `BloodBodyPartType.MAX.index()` (runtime value from the game).
    pub blood_body_parts: usize,
    pub components: Box<dyn ComponentCodec>,
    /// Items kept raw, by reason.
    pub raw_counts: RefCell<BTreeMap<String, usize>>,
    /// `Texture.getSharedTexture` finds item textures (false when no
    /// textures are loaded, as on a server: saved texture names drop).
    pub textures_loaded: bool,
    /// `ZomboidRadio.getRecordedMedia()`: index to translated item name.
    /// `None` when recorded media are not loaded (indices reset to -1).
    pub recorded_media: Option<HashMap<i16, String>>,
    /// Moveable sprite properties (from the tile definitions), by name.
    pub world_sprites: HashMap<String, WorldSpriteInfo>,
    /// The `ClothingItem` assets; with them loads run `synchWithVisual`.
    pub assets: Option<crate::assets::ItemAssets>,
    /// `actualWeight` of every script item, by full type.
    script_weights: HashMap<String, f32>,
    /// Scripts marked `Obsolete`, by full type.
    obsolete: HashMap<String, bool>,
    /// The game's `Rand` (`None`: every draw is 0, or `min` for floats).
    pub rand: RefCell<Option<Box<dyn crate::component::FactoryRng>>>,
    /// `ZomboidRadio`'s channel lists (runtime input; `None`: no radio
    /// data, as when `ZomboidRadio` is not loaded).
    pub channels: Option<Box<dyn crate::device::ChannelLists>>,
    /// `MapDefinitions.definitions` (runtime input from Lua `LootMaps`).
    pub map_definitions: Vec<String>,
}

/// `ScriptManager.FindItem(type, false)` with `createItemInternal`'s
/// `Empty` fallback (a script whose name drops the suffix and that has a
/// `FluidContainer` or `Durability` component). The flag is set when the
/// fallback found a `FluidContainer` script (the new item is emptied).
fn find_script(sm: &ScriptManager, item_type: &str) -> Option<(Rc<RefCell<Item>>, bool)> {
    let find = |n: &str| sm.find_item(n, false)?.downcast::<Item>();
    if let Some(i) = find(item_type) {
        return Some((i, false));
    }
    let base = item_type.strip_suffix("Empty")?;
    let i = find(base)?;
    let (durability, fluid) = {
        let b = i.borrow();
        (
            b.entity.contains_component(ComponentType::Durability),
            b.entity.contains_component(ComponentType::FluidContainer),
        )
    };
    if durability {
        return Some((i, false));
    }
    fluid.then_some((i, true))
}

impl ScriptItems {
    /// Builds the templates for every dictionary item that
    /// `CreateItem(registryID)` can create.
    ///
    /// `item_names` is the `ItemName` translation table
    /// (`Translator.getItemNameFromFullType`), runtime input from the
    /// game's `Translate/<lang>/ItemName.json` (see [`parse_item_names`]).
    pub fn new(
        sm: &ScriptManager,
        dictionary: WorldDictionary,
        item_names: &HashMap<String, String>,
    ) -> Self {
        let mut templates = HashMap::new();
        let script_weights = pz_scripts::objects::items::get_all_items(sm)
            .iter()
            .map(|i| {
                let i = i.borrow();
                (i.full_name(), i.actual_weight)
            })
            .collect();
        let mut obsolete = HashMap::new();
        for (&id, info) in &dictionary.items {
            let mut item_type = info.full_type.clone();
            let mut moveable = None;
            if let Some(name) = item_type.strip_prefix("Moveables.") {
                if !item_type.eq_ignore_ascii_case("Moveables.Moveable") {
                    moveable = Some(name.to_string());
                    item_type = "Moveables.Moveable".into();
                }
            }
            let Some((script, empty)) = find_script(sm, &item_type) else {
                continue;
            };
            let s = script.borrow();
            obsolete.insert(info.full_type.clone(), s.obsolete);
            // isValid(): !obsolete && !removed && isLoaded.
            if info.removed || info.obsolete || s.obsolete {
                continue;
            }
            drop(s);
            let Some(t) = ItemTemplate::from_script_ref(sm, &script) else {
                continue;
            };
            let s = script.borrow();
            let t = if empty {
                let mut t = (*t).clone();
                t.empty_fluid_container = true;
                Rc::new(t)
            } else {
                t
            };
            let t = match item_names.get(&s.full_name()) {
                Some(n) if *n != t.display_name => {
                    let mut t = (*t).clone();
                    t.display_name = n.clone();
                    Rc::new(t)
                }
                _ => t,
            };
            let t = match moveable {
                Some(m) => {
                    let mut t = (*t).clone();
                    t.type_ = m.clone();
                    t.moveable_sprite = Some(m);
                    Rc::new(t)
                }
                None => t,
            };
            templates.insert(id, t);
        }
        ScriptItems {
            dictionary,
            templates,
            world_age_hours: 0.0,
            blood_body_parts: 0,
            components: Box::new(OpaqueComponents),
            raw_counts: RefCell::new(BTreeMap::new()),
            textures_loaded: false,
            world_sprites: HashMap::new(),
            script_weights,
            recorded_media: None,
            obsolete,
            assets: None,
            rand: RefCell::new(None),
            channels: None,
            map_definitions: Vec::new(),
        }
    }
}

impl ItemEnv for ScriptItems {
    fn template(&self, registry_id: i16) -> Option<Rc<ItemTemplate>> {
        self.templates.get(&registry_id).cloned()
    }

    fn exists(&self, registry_id: i16) -> bool {
        self.templates.contains_key(&registry_id)
    }

    fn full_type(&self, registry_id: i16) -> Option<String> {
        self.dictionary.item_type(registry_id).map(str::to_string)
    }

    fn registry_id(&self, full_type: &str) -> i16 {
        self.dictionary.item_id(full_type).unwrap_or(-1)
    }

    fn obsolete(&self, full_type: &str) -> bool {
        self.obsolete.get(full_type).copied().unwrap_or(false)
    }

    fn world_age_hours(&self) -> f64 {
        self.world_age_hours
    }

    fn blood_body_parts(&self) -> usize {
        self.blood_body_parts
    }

    fn components(&self) -> &dyn ComponentCodec {
        self.components.as_ref()
    }

    fn world_sprite(&self, name: &str) -> Option<WorldSpriteInfo> {
        self.world_sprites.get(name).cloned()
    }

    fn script_actual_weight(&self, name: &str) -> Option<f32> {
        let full = if name.contains('.') {
            name.to_string()
        } else {
            format!("Base.{name}")
        };
        self.script_weights.get(&full).copied()
    }

    fn texture_round_trip(&self, _t: &ItemTemplate, name: &str) -> Option<String> {
        // LOADER STUB: with textures loaded the name is kept unless it is
        // the script icon's; the icon comparison needs the texture names.
        self.textures_loaded.then(|| name.to_string())
    }

    fn recorded_media_name(&self, index: i16) -> Option<Option<String>> {
        match &self.recorded_media {
            None => None,
            Some(m) => m.get(&index).map(|n| Some(n.clone())),
        }
    }

    fn assets(&self) -> Option<&crate::assets::ItemAssets> {
        self.assets.as_ref()
    }

    fn rand_next(&self, bound: i32) -> i32 {
        match self.rand.borrow_mut().as_mut() {
            Some(r) if bound > 0 => r.next_int(bound),
            _ => 0,
        }
    }

    fn rand_float(&self, min: f32, max: f32) -> f32 {
        match self.rand.borrow_mut().as_mut() {
            Some(r) => r.next_float(min, max),
            None => min,
        }
    }

    fn channel_lists(&self) -> &dyn crate::device::ChannelLists {
        match &self.channels {
            Some(c) => c.as_ref(),
            None => &crate::device::NoChannels,
        }
    }

    fn map_definitions(&self) -> Vec<String> {
        self.map_definitions.clone()
    }

    fn note_raw(&self, _registry_id: i16, reason: &str) {
        *self
            .raw_counts
            .borrow_mut()
            .entry(reason.to_string())
            .or_default() += 1;
    }
}

/// Reads a flat `{"key": "value", ...}` translation file (the format of
/// `Translate/<lang>/ItemName.json`). Later keys replace earlier ones.
pub fn parse_item_names(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut it = text.chars().peekable();
    let mut strings = Vec::new();
    while let Some(c) = it.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = it.next() {
            match c {
                '"' => break,
                '\\' => match it.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let hex: String = it.by_ref().take(4).collect();
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            s.push(ch);
                        }
                    }
                    Some(o) => s.push(o),
                    None => {}
                },
                o => s.push(o),
            }
        }
        strings.push(s);
        if strings.len() == 2 {
            let v = strings.pop().unwrap();
            let k = strings.pop().unwrap();
            out.insert(k, v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_names() {
        let m = parse_item_names("{\n \"Base.A\": \"An \\\"A\\\"\",\n \"Base.B\": \"B\\u00e9\"\n}");
        assert_eq!(m["Base.A"], "An \"A\"");
        assert_eq!(m["Base.B"], "B\u{e9}");
    }
}
