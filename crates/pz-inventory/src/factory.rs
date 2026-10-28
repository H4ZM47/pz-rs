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
    find_script_with(sm, item_type, false)
}

/// [`find_script`] with `FindItem`'s `moduleDefaultsToBase`.
fn find_script_with(
    sm: &ScriptManager,
    item_type: &str,
    module_defaults_to_base: bool,
) -> Option<(Rc<RefCell<Item>>, bool)> {
    let find = |n: &str| sm.find_item(n, module_defaults_to_base)?.downcast::<Item>();
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

    fn script_dictionary_name(&self, register: &str, id: i16) -> Option<Option<String>> {
        Some(
            self.dictionary
                .script_register(register)
                .and_then(|r| r.name_of(id))
                .map(str::to_string),
        )
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

/// `InventoryItemFactory.CreateItem(itemType, useDelta, moduleDefaultsToBase)`
/// (`createItemInternal` for a first-time item): the script found by full
/// type or by name (`Moveables.<sprite>` through `Moveables.Moveable`, a
/// `<type>Empty` through its fluid-container or durability script), then
/// `Item.InstanceItem` with its `Rand` draws in order and the item id
/// (see [`crate::env::create_from_template`]); a drainable gets
/// `(int)(maxUses * useDelta)` uses. `None` when no script is found.
///
/// `display_name` is `Translator.getItemNameFromFullType` of the script's
/// full type (`Item.OnScriptsLoaded` sets the display name from it);
/// `None` keeps the script's. Not modelled: the poisonous berry and
/// mushroom of `Core` (unset until a game starts).
pub fn create_item_by_type(
    env: &dyn ItemEnv,
    sm: &ScriptManager,
    item_type: &str,
    use_delta: f32,
    module_defaults_to_base: bool,
    display_name: &dyn Fn(&str) -> Option<String>,
) -> Option<crate::item::InventoryItem> {
    let (template, registry_id) =
        runtime_item_template(sm, item_type, module_defaults_to_base, display_name)?;
    let mut item = crate::env::create_from_template(env, template, registry_id);
    if use_delta != 1.0
        && matches!(
            item.template.class,
            crate::template::ItemClass::Drainable | crate::template::ItemClass::WeaponPart
        )
    {
        item.uses = (item.get_max_uses() as f32 * use_delta) as i32;
        item.update_weight_by(|n| env.script_actual_weight(n));
    }
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_factory_keeps_the_loaded_script_dictionary_id() {
        let sm = ScriptManager::new();
        sm.registries
            .get("item_type")
            .unwrap()
            .borrow_mut()
            .register_id("test:normal", true)
            .unwrap();
        sm.statics
            .items
            .borrow_mut()
            .constants
            .insert("item_type", "NORMAL", "test:normal");
        sm.parse_script(
            pz_scripts::ScriptLoadMode::Init,
            "module Base { item TestMarble { ItemType = test:normal, Weight = 0.1, } }",
        )
        .unwrap();
        sm.get_module("Base")
            .unwrap()
            .bucket(pz_scripts::ScriptType::Item)
            .load_scripts(&sm, pz_scripts::ScriptLoadMode::Init);
        let script = sm
            .find_item("Base.TestMarble", false)
            .unwrap()
            .downcast::<pz_scripts::objects::items::Item>()
            .unwrap();
        script.borrow_mut().entity.set_registry_id(73).unwrap();
        struct Env;
        impl ItemEnv for Env {}
        for ty in ["Base.TestMarble", "TestMarble"] {
            let item = create_item_by_type(&Env, &sm, ty, 1.0, true, &|_| None).unwrap();
            assert_eq!(
                item.registry_id, 73,
                "string factory discarded WorldDictionary registration"
            );
            assert_eq!(item.get_full_type(), "Base.TestMarble");
            assert!(
                item.item_type.is_none(),
                "ComboItem constructor leaves itemType null"
            );
        }
    }

    #[test]
    fn created_item_types_follow_subclass_constructor_references() {
        use pz_scripts::objects::items::ItemType;
        let sm = ScriptManager::new();
        let cases = [
            ("NORMAL", None),
            ("KEY", Some("KEY")),
            ("WEAPON_PART", Some("WEAPON")),
            ("CONTAINER", Some("CONTAINER")),
            ("FOOD", Some("FOOD")),
            ("LITERATURE", Some("LITERATURE")),
            ("ALARM_CLOCK", Some("ALARM_CLOCK")),
            ("ALARM_CLOCK_CLOTHING", Some("ALARM_CLOCK_CLOTHING")),
            ("WEAPON", Some("WEAPON")),
            ("CLOTHING", Some("CLOTHING")),
            ("DRAINABLE", None),
            ("RADIO", Some("RADIO")),
            ("MOVEABLE", Some("MOVEABLE")),
            ("MAP", None),
            ("ANIMAL", Some("ANIMAL")),
        ];
        for (field, _) in cases {
            let id = format!("test:{}", field.to_lowercase());
            sm.registries
                .get("item_type")
                .unwrap()
                .borrow_mut()
                .register_id(&id, false)
                .unwrap();
            sm.statics
                .items
                .borrow_mut()
                .constants
                .insert("item_type", field, &id);
            sm.parse_script(
                pz_scripts::ScriptLoadMode::Init,
                &format!("module Base {{ item Test{field} {{ ItemType = {id}, Weight = 0.1, }} }}"),
            )
            .unwrap();
        }
        sm.get_module("Base")
            .unwrap()
            .bucket(pz_scripts::ScriptType::Item)
            .load_scripts(&sm, pz_scripts::ScriptLoadMode::Init);
        struct Env;
        impl ItemEnv for Env {}
        for (field, expected) in cases {
            let item =
                create_item_by_type(&Env, &sm, &format!("Base.Test{field}"), 1.0, true, &|_| {
                    None
                })
                .unwrap();
            match expected {
                Some(expected) => assert!(
                    Rc::ptr_eq(
                        item.item_type.as_ref().unwrap(),
                        &ItemType::constant(&sm, expected).unwrap()
                    ),
                    "wrong constructor type for {field}"
                ),
                None => assert!(
                    item.item_type.is_none(),
                    "constructor unexpectedly assigned {field}"
                ),
            }
        }
    }

    #[test]
    fn item_names() {
        let m = parse_item_names("{\n \"Base.A\": \"An \\\"A\\\"\",\n \"Base.B\": \"B\\u00e9\"\n}");
        assert_eq!(m["Base.A"], "An \"A\"");
        assert_eq!(m["Base.B"], "B\u{e9}");
    }
}

/// Actual-owner counterpart of create_item_by_type. The entire constructor
/// sequence uses the same actual receiver and returns ownership on failures.
pub fn create_item_by_type_ref(
    env: &dyn ItemEnv,
    sm: &ScriptManager,
    item_type: &str,
    use_delta: f32,
    module_defaults_to_base: bool,
    display_name: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<crate::shared::ItemRef>, crate::env::FactoryItemFailure> {
    let Some((template, registry_id)) =
        runtime_item_template(sm, item_type, module_defaults_to_base, display_name)
    else {
        return Ok(None);
    };
    let item = crate::env::create_from_template_ref(env, template, registry_id)?;
    {
        let mut body = item.borrow_mut();
        if use_delta != 1.0
            && matches!(
                body.template.class,
                crate::template::ItemClass::Drainable | crate::template::ItemClass::WeaponPart
            )
        {
            body.uses = (body.get_max_uses() as f32 * use_delta) as i32;
            body.update_weight_by(|name| env.script_actual_weight(name));
        }
    }
    Ok(Some(item))
}

fn runtime_item_template(
    sm: &ScriptManager,
    item_type: &str,
    module_defaults_to_base: bool,
    display_name: &dyn Fn(&str) -> Option<String>,
) -> Option<(Rc<ItemTemplate>, i16)> {
    let mut item_type = item_type.to_string();
    let mut moveable = None;
    if let Some(name) = item_type.strip_prefix("Moveables.") {
        if !item_type.eq_ignore_ascii_case("Moveables.Moveable") {
            // `itemType.split("\\.")[1]`.
            moveable = Some(name.split('.').next().unwrap_or("").to_string());
            item_type = "Moveables.Moveable".into();
        }
    }
    let (script, empty) = find_script_with(sm, &item_type, module_defaults_to_base)?;
    let mut t = ItemTemplate::from_script_ref(sm, &script)?;
    let full = script.borrow().full_name();
    if empty || moveable.is_some() {
        let mut c = (*t).clone();
        c.empty_fluid_container = empty;
        if let Some(m) = moveable {
            c.type_ = m.clone();
            c.moveable_sprite = Some(m);
        }
        t = Rc::new(c);
    }
    if let Some(n) = display_name(&full).filter(|n| *n != t.display_name) {
        let mut c = (*t).clone();
        c.display_name = n;
        t = Rc::new(c);
    }
    let registry_id = script.borrow().entity.registry_id();
    Some((t, registry_id))
}
