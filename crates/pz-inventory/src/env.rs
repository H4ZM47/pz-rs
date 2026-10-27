//! What item loading and saving need from the rest of the game.

use std::rc::Rc;

use crate::component::{ComponentCodec, OpaqueComponents};
use crate::item::InventoryItem;
use crate::template::{ItemClass, ItemTemplate};

/// A moveable sprite's properties (`IsoSprite.getProperties()`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldSpriteInfo {
    /// `CustomItem`.
    pub custom_item: Option<String>,
    /// `PickUpWeight` as a number.
    pub pick_up_weight: Option<f32>,
    /// `CustomName` and `GroupName`.
    pub custom_name: Option<String>,
    pub group_name: Option<String>,
    /// The sprite's index in its `IsoSpriteGrid` and the grid's count.
    pub sprite_grid: Option<(i32, i32)>,
    /// `ForceSingleItem`.
    pub force_single_item: bool,
}

/// Game state and lookups used by item `load`/`save`.
///
/// The defaults describe a game where every item id is creatable (with
/// an unknown script, so items stay raw records) and no load-time lookup
/// changes anything.
pub trait ItemEnv {
    /// `InventoryItemFactory.CreateItem(registryID)`'s script, or `None`
    /// when the item cannot be created (the original then skips it).
    fn template(&self, _registry_id: i16) -> Option<Rc<ItemTemplate>> {
        None
    }

    /// Whether `CreateItem(registryID)` returns an item. True by default
    /// so unknown items are kept as raw records.
    fn exists(&self, registry_id: i16) -> bool {
        let _ = registry_id;
        true
    }

    /// `WorldDictionary.getItemTypeFromID`.
    fn full_type(&self, _registry_id: i16) -> Option<String> {
        None
    }

    /// DictionaryData.getItemTypeDebugString: the actual registered type,
    /// or the native literal when getItemTypeFromID returns null.
    fn item_type_debug_string(&self, registry_id: i16) -> String {
        self.full_type(registry_id)
            .unwrap_or_else(|| "Unknown".into())
    }

    /// `WorldDictionary.getItemRegistryID`: -1 for an unknown type.
    fn registry_id(&self, _full_type: &str) -> i16 {
        -1
    }

    /// `ScriptManager.FindItem(fullType).getObsolete()`.
    fn obsolete(&self, _full_type: &str) -> bool {
        false
    }

    /// `GameTime.getWorldAgeHours()`.
    fn world_age_hours(&self) -> f64 {
        0.0
    }

    /// `BloodBodyPartType.MAX.index()` (length of `ItemVisual` arrays).
    fn blood_body_parts(&self) -> usize {
        // LOADER STUB: runtime value from the game (see pz-world SaveTables).
        0
    }

    /// The component codec (see [`crate::component`]).
    fn components(&self) -> &dyn ComponentCodec {
        &OpaqueComponents
    }

    /// Whether items of this script are decoded into fields; others stay
    /// raw records. `AnimalInventoryItem` (a whole `IsoAnimal`) is not
    /// ported.
    fn decodes(&self, t: &ItemTemplate) -> bool {
        t.class != ItemClass::Animal && self.components().knows_factory_components(t)
    }

    /// `extraItems` load and save: id to full type and back through the
    /// `WorldDictionary` (unchanged for known items).
    fn extra_item_round_trip(&self, id: i16) -> i16 {
        id
    }

    /// `setTexture(Texture.getSharedTexture(name))` followed by the save
    /// test: the name saved again, or `None` when the texture is missing
    /// or matches the script icon.
    fn texture_round_trip(&self, _t: &ItemTemplate, name: &str) -> Option<String> {
        Some(name.to_string())
    }

    /// `RecordedMedia.getMediaDataFromIndex(index)`: `None` when unknown
    /// (the index is reset to -1), `Some(name)` with the translated
    /// display name the item takes (`Some(None)` keeps the name).
    fn recorded_media_name(&self, _index: i16) -> Option<Option<String>> {
        Some(None)
    }

    /// The properties `Moveable.ReadFromWorldSprite` reads from a sprite
    /// that has `IsMoveAble` (`None` for other or unknown sprites).
    fn world_sprite(&self, _name: &str) -> Option<WorldSpriteInfo> {
        None
    }

    /// `ScriptManager.FindItem(name).actualWeight`.
    fn script_actual_weight(&self, _name: &str) -> Option<f32> {
        None
    }

    /// The script `ScriptsDictionary.<register>.getInfoForID(id)` names
    /// (its full type): `None` when there is no dictionary, `Some(None)`
    /// when the id is not registered.
    fn script_dictionary_name(&self, _register: &str, _id: i16) -> Option<Option<String>> {
        None
    }

    /// `Rand.Next(bound)`. The default returns 0; the game's generator is
    /// not seeded here.
    fn rand_next(&self, _bound: i32) -> i32 {
        0
    }

    /// `Rand.Next(float min, float max)`. The default returns `min`.
    fn rand_float(&self, min: f32, _max: f32) -> f32 {
        min
    }

    /// The `ClothingItem` assets. With them, `load` and [`create_item`]
    /// run `synchWithVisual` (visuals from the assets, blood and dirt
    /// levels, colour); without them (the default) visuals stay as saved.
    fn assets(&self) -> Option<&crate::assets::ItemAssets> {
        None
    }

    /// `OutfitRNG` for new visuals.
    fn visual_rng(&self) -> &dyn crate::visual::VisualRng {
        &crate::visual::NoVisualRng
    }

    /// `ZomboidRadio.GetChannelList` for new radios' presets. The default
    /// has no radio data (every list is `null`).
    fn channel_lists(&self) -> &dyn crate::device::ChannelLists {
        &crate::device::NoChannels
    }

    /// `MapDefinitions.definitions` (the keys of the Lua `LootMaps.Init`
    /// table, in iteration order) for a new map item without a `Map`
    /// value. Empty (the default) gives `"Default"`.
    fn map_definitions(&self) -> Vec<String> {
        Vec::new()
    }

    /// Called when an item is kept as raw bytes, with the reason.
    fn note_raw(&self, _registry_id: i16, _reason: &str) {}
}

/// `InventoryItemFactory.CreateItem(registryID)`.
pub fn create_item(env: &dyn ItemEnv, registry_id: i16) -> Option<InventoryItem> {
    let t = env.template(registry_id)?;
    Some(create_from_template(env, t, registry_id))
}

/// `Item.InstanceItem` and `createItemInternal` for a script template.
///
/// `Rand` draws (through [`ItemEnv::rand_next`] / [`ItemEnv::rand_float`]),
/// in the original's order: the components' (`GameEntityFactory`), the
/// `OutfitRNG` seed, the model index (`synchWithVisual`), then the item id.
/// Before those, the subclass draws of `InstanceItem` and the item
/// constructors (see [`instance_draws`]).
pub fn create_from_template(
    env: &dyn ItemEnv,
    template: Rc<ItemTemplate>,
    registry_id: i16,
) -> InventoryItem {
    let mut item = create_template_prefix(env, template.clone(), registry_id);
    let codec = env.components();
    match codec.factory_entity(&template, &mut crate::component::EnvRng(env)) {
        Some(values) => drop(
            item.set_components(values)
                .expect("unowned component factory values"),
        ),
        None => {
            item.components = crate::item_components::ItemComponents::from_encoded_blobs(
                codec.factory_components(&template, &mut crate::component::EnvRng(env)),
                crate::component::COMPONENT_WORLD_VERSION,
            )
            .expect("explicit component archive import")
        }
    }
    finish_template_before_empty(env, &mut item);
    if template.empty_fluid_container {
        if let Some(context) = codec.entity_context() {
            if let Some(mut fluid) = item.get_fluid_container(context) {
                fluid.empty();
                drop(
                    item.set_fluid_container(fluid)
                        .expect("unowned empty-fluid factory value"),
                );
            }
        }
    }
    finish_template_after_empty(env, &mut item);
    item
}

/// An actual factory receiver and completed native prefix on failure.
#[derive(Debug)]
#[must_use]
pub struct FactoryItemFailure {
    pub error: pz_entity::runtime_load::RuntimeHostError,
    pub item: Option<crate::shared::ItemRef>,
    pub retired: pz_entity::RetiredComponents,
}

impl FactoryItemFailure {
    pub fn into_error_and_retire(mut self) -> pz_entity::runtime_load::RuntimeHostError {
        if let Some(item) = self.item.take() {
            self.retired
                .push_owner(pz_entity::EntityOwner::from_ref(&item));
        }
        crate::component_host::retire_components(self.retired);
        self.error
    }
}

pub fn create_item_ref(
    env: &dyn ItemEnv,
    registry_id: i16,
) -> Result<Option<crate::shared::ItemRef>, FactoryItemFailure> {
    let Some(template) = env.template(registry_id) else {
        return Ok(None);
    };
    create_from_template_ref(env, template, registry_id).map(Some)
}

/// Production actual-owner factory. Native component construction and pool
/// reuse happen while this exact receiver already exists, before subsequent
/// visual/model/id draws and before any Lua initialisation exposure.
pub fn create_from_template_ref(
    env: &dyn ItemEnv,
    template: Rc<ItemTemplate>,
    registry_id: i16,
) -> Result<crate::shared::ItemRef, FactoryItemFailure> {
    // Publish the same actual receiver before any captured constructor read.
    // Archive creation remains the explicitly separate value API above.
    let value = InventoryItem::create(template.clone(), registry_id);
    let item = crate::shared::try_new_item(value).map_err(|failure| FactoryItemFailure {
        error: failure.error,
        item: Some(failure.item),
        retired: failure.retired,
    })?;
    let mut retired = pz_entity::RetiredComponents::new();
    crate::item_texture::constructor_script_ref(&item, &template).map_err(|error| {
        FactoryItemFailure {
            error,
            item: Some(item.clone()),
            retired: pz_entity::RetiredComponents::new(),
        }
    })?;
    // Native Food construction (including all texture capture) has returned
    // before Item.InstanceItem PC632 reads this current argument script field.
    // Any earlier constructor failure retains the raw 60-minute default.
    copy_food_minutes_to_cook_ref(&item, &template).map_err(|error| FactoryItemFailure {
        error,
        item: Some(item.clone()),
        retired: std::mem::take(&mut retired),
    })?;
    finish_constructor_prefix(env, &mut item.borrow_mut());
    // Item.InstanceItem PC3336/3345: common condition setters follow the
    // subclass constructor/draw/world-sprite phases, before mask setters.
    let condition_callback = {
        let mut body = item.borrow_mut();
        body.condition = body.condition_max.max(0).min(body.condition_max);
        body.broken = body.condition <= 0;
        body.broken && body.template.on_break.is_some()
    };
    crate::shared::observe_item(&item);
    if condition_callback {
        return Err(FactoryItemFailure {
            error: pz_entity::runtime_load::RuntimeHostError::Unsupported {
                operation: "Item.InstanceItem setCondition OnBreak callback",
                kind: None,
            },
            item: Some(item),
            retired,
        });
    }
    crate::item_texture::set_script_item_ref(&item, template.script.as_ref()).map_err(|error| {
        FactoryItemFailure {
            error,
            item: Some(item.clone()),
            retired: pz_entity::RetiredComponents::new(),
        }
    })?;
    crate::item_texture::constructor_masks_ref(&item, &template).map_err(|error| {
        FactoryItemFailure {
            error,
            item: Some(item.clone()),
            retired: pz_entity::RetiredComponents::new(),
        }
    })?;
    let mut native_notices = Vec::new();
    let built = env.components().factory_runtime(
        &item,
        &template,
        &mut crate::component::EnvRng(env),
        &mut retired,
        &mut native_notices,
    );
    // factory_runtime has ended every body/script guard, including Err.
    // A later unported lookup must not erase earlier native warning output.
    let emitted = crate::component_host::emit_native_notices(&native_notices);
    if let Err(error) = emitted {
        return Err(FactoryItemFailure {
            error,
            item: Some(item),
            retired,
        });
    }
    if let Err(error) = built {
        return Err(FactoryItemFailure {
            error,
            item: Some(item),
            retired,
        });
    }
    // Native InstanceItem has no finally around synchWithVisual: failure
    // retains the newly published OutfitRNG seed and earlier texture writes.
    let rng = env.visual_rng();
    let saved_seed = rng.get_seed();
    rng.set_seed(env.rand_next(i32::MAX) as i64);
    crate::item_texture::synch_with_visual_ref(
        &item,
        env.assets(),
        &crate::context::Loose,
        rng,
        &mut |m| env.rand_next(m),
    )
    .map_err(|error| FactoryItemFailure {
        error,
        item: Some(item.clone()),
        retired: std::mem::take(&mut retired),
    })?;
    rng.set_seed(saved_seed);
    if template.empty_fluid_container {
        let component = item
            .borrow()
            .get_fluid_ref()
            .map_err(|error| FactoryItemFailure {
                error: pz_entity::runtime_load::RuntimeHostError::Storage(error),
                item: Some(item.clone()),
                retired: pz_entity::RetiredComponents::new(),
            });
        match component {
            Ok(Some(component)) => {
                let result = {
                    let Some(context) = env.components().entity_context() else {
                        retired.push_component(component);
                        return Err(FactoryItemFailure {
                            error: pz_entity::runtime_load::RuntimeHostError::Unsupported {
                                operation: "actual factory fluid context",
                                kind: Some(pz_entity::ComponentType::FluidContainer),
                            },
                            item: Some(item),
                            retired,
                        });
                    };
                    component
                        .try_borrow_mut()
                        .map_err(|_| {
                            pz_entity::runtime_load::RuntimeHostError::Borrowed(
                                "empty factory fluid",
                            )
                        })
                        .and_then(|mut body| {
                            body.fluid_empty_data(context)
                                .map_err(pz_entity::runtime_load::RuntimeHostError::Storage)
                        })
                };
                retired.push_component(component);
                match result {
                    Ok(outcome) => {
                        if let Err(error) = crate::component_host::notify_fluid_update(&outcome) {
                            return Err(FactoryItemFailure {
                                error,
                                item: Some(item),
                                retired,
                            });
                        }
                    }
                    Err(error) => {
                        return Err(FactoryItemFailure {
                            error,
                            item: Some(item),
                            retired,
                        })
                    }
                }
            }
            Ok(None) => {}
            Err(mut failure) => {
                failure.retired.append(retired);
                return Err(failure);
            }
        }
    }
    finish_template_after_empty(env, &mut item.borrow_mut());
    // Pool reuse can replace the last actual cached-script owner. Successful
    // construction is also callable under outer WORLD/sprite scopes.
    crate::component_host::retire_components(retired);
    Ok(item)
}

fn create_template_prefix(
    env: &dyn ItemEnv,
    t: Rc<ItemTemplate>,
    registry_id: i16,
) -> InventoryItem {
    let mut item = InventoryItem::create(t, registry_id);
    // Explicit archive/value factory uses its template's captured scalar. It
    // is separate from the actual factory's current script-argument read.
    if let crate::ItemKind::Food(food) = &mut item.kind {
        food.minutes_to_cook = item.template.minutes_to_cook as f32;
    }
    finish_constructor_prefix(env, &mut item);
    item.condition = item.condition_max.max(0).min(item.condition_max);
    item.broken = item.condition <= 0;
    item
}

fn copy_food_minutes_to_cook_ref(
    item: &crate::shared::ItemRef,
    template: &ItemTemplate,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    if template.class != ItemClass::Food {
        return Ok(());
    }
    let minutes = if let Some(script) = template.script.as_ref() {
        script
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("InstanceItem Food minutesToCook script"))?
            .minutes_to_cook
    } else {
        // Explicit captured fixture input, rather than a live-script proxy.
        template.minutes_to_cook
    };
    let mut body = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("InstanceItem Food minutesToCook receiver"))?;
    let crate::ItemKind::Food(food) = &mut body.kind else {
        return Err(RuntimeHostError::InvalidHostState(
            "InstanceItem Food receiver class",
        ));
    };
    food.minutes_to_cook = minutes as f32;
    Ok(())
}

fn finish_constructor_prefix(env: &dyn ItemEnv, item: &mut InventoryItem) {
    let t = item.template.clone();
    instance_draws(env, item);
    // Item.InstanceItem: moveables and radios read their world sprite;
    // createItemInternal then reads the `Moveables.<sprite>` name.
    if t.class == ItemClass::Moveable {
        // `new Moveable(...)` starts at the default weight; InstanceItem
        // then takes the moveable's weight after `ReadFromWorldSprite`
        // (`this.actualWeight = moveable.getActualWeight()`), so a sprite
        // that is not found leaves 1.0 rather than the script weight.
        item.weight = 1.0;
        item.actual_weight = 1.0;
        if let Some(ws) = t.world_object_sprite.as_deref() {
            item.read_from_world_sprite(ws, env);
        }
        let w = if item.name == item.get_full_type() {
            0.0
        } else {
            item.actual_weight.max(0.0)
        };
        item.weight = w;
        item.actual_weight = w;
    } else if let Some(ws) = t
        .world_object_sprite
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        item.read_from_world_sprite(ws, env);
    }
}

fn finish_template_before_empty(env: &dyn ItemEnv, item: &mut InventoryItem) {
    // `OutfitRNG.setSeed(Rand.Next(Integer.MAX_VALUE))` around
    // `synchWithVisual` (the visual draws come from `visual_rng`).
    let rng = env.visual_rng();
    let saved_seed = rng.get_seed();
    rng.set_seed(env.rand_next(i32::MAX) as i64);
    item.pick_model_index(&mut |m| env.rand_next(m));
    if let Some(a) = env.assets() {
        item.synch_with_visual(Some(a), &crate::context::Loose, rng);
    }
    rng.set_seed(saved_seed);
}

fn finish_template_after_empty(env: &dyn ItemEnv, item: &mut InventoryItem) {
    let t = item.template.clone();
    item.id = env.rand_next(2146250223).wrapping_add(1233423);
    if let Some(ms) = &t.moveable_sprite {
        item.read_from_world_sprite(ms, env);
    }
    // createItemInternal: a `Drainable` starts full.
    if matches!(t.class, ItemClass::Drainable | ItemClass::WeaponPart) {
        item.uses = item.get_max_uses();
        item.update_weight_by(|n| env.script_actual_weight(n));
    }
}

/// The `Rand` draws `Item.InstanceItem` and the item constructors make
/// before the components: a padlock's key id; a clothing item's palette
/// pick and its random `col`; an alarm clock's random alarm; a radio's
/// presets and channel (`generatePresets`, `setRandomChannel`); a map
/// item's random map (`MapDefinitions.pickRandom`).
fn instance_draws(env: &dyn ItemEnv, item: &mut InventoryItem) {
    use crate::item::ItemKind;
    let t = item.template.clone();
    let digital = !item.get_full_type().contains("Classic");
    match &mut item.kind {
        ItemKind::Key {
            key_id,
            number_of_key,
        } if t.padlock => {
            *number_of_key = 2;
            *key_id = env.rand_next(10_000_000);
        }
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..) => {
            if t.palette_choices > 0 {
                let _ = env.rand_next(t.palette_choices as i32);
            }
            // `new Color(Rand.Next(255), Rand.Next(255), Rand.Next(255))`.
            let mut c = [0.0f32; 4];
            for v in c.iter_mut().take(3) {
                *v = env.rand_next(255) as f32 / 255.0;
            }
            c[3] = 1.0;
            item.col = c;
            if let ItemKind::AlarmClockClothing(_, a) = &mut item.kind {
                // `randomizeAlarm` (digital clocks only).
                if digital {
                    a.alarm_hour = env.rand_next(23);
                    a.alarm_minutes = env.rand_next(59) / 10 * 10;
                    a.alarm_set = env.rand_next(15) == 1;
                }
            }
        }
        ItemKind::AlarmClock(a) => {
            a.alarm_hour = env.rand_next(23);
            a.alarm_minutes = env.rand_next(59) / 10 * 10;
            a.alarm_set = env.rand_next(6) == 1;
        }
        ItemKind::Radio(_, d) => {
            d.instance_from_script(
                Some(&t.display_name),
                &t.radio,
                env.channel_lists(),
                &mut |b| env.rand_next(b),
            );
        }
        ItemKind::Map { map_id, .. } => {
            *map_id = match t.map.as_deref().filter(|m| !m.trim().is_empty()) {
                Some(m) => m.to_string(),
                None => {
                    let defs = env.map_definitions();
                    if defs.is_empty() {
                        "Default".to_string()
                    } else {
                        defs[env.rand_next(defs.len() as i32) as usize].clone()
                    }
                }
            };
        }
        _ => {}
    }
}

/// [`ItemEnv`] with every default.
pub struct NoItemEnv;
impl ItemEnv for NoItemEnv {}
