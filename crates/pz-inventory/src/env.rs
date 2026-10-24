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
    t: Rc<ItemTemplate>,
    registry_id: i16,
) -> InventoryItem {
    let mut item = InventoryItem::create(t.clone(), registry_id);
    instance_draws(env, &mut item);
    // Item.InstanceItem: moveables and radios read their world sprite;
    // createItemInternal then reads the `Moveables.<sprite>` name.
    if let Some(ws) = t
        .world_object_sprite
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        item.read_from_world_sprite(ws, env);
    }
    let codec = env.components();
    match codec.factory_entity(&t, &mut crate::component::EnvRng(env)) {
        Some(c) => item.set_components(c),
        None => {
            item.components = codec.factory_components(&t, &mut crate::component::EnvRng(env));
        }
    }
    // `OutfitRNG.setSeed(Rand.Next(Integer.MAX_VALUE))` around
    // `synchWithVisual` (the visual draws come from `visual_rng`).
    let _ = env.rand_next(i32::MAX);
    item.pick_model_index(&mut |m| env.rand_next(m));
    if let Some(a) = env.assets() {
        item.synch_with_visual(Some(a), &crate::context::Loose, env.visual_rng());
    }
    // createItemInternal: a `<type>Empty` fluid container starts empty.
    if t.empty_fluid_container {
        let ctx = env.components().entity_context();
        if let Some(mut fc) = ctx.and_then(|c| item.get_fluid_container(c)) {
            fc.empty();
            item.set_fluid_container(fc);
        }
    }
    item.id = env.rand_next(2146250223).wrapping_add(1233423);
    if let Some(ms) = &t.moveable_sprite {
        item.read_from_world_sprite(ms, env);
    }
    // createItemInternal: a `Drainable` starts full.
    if matches!(t.class, ItemClass::Drainable | ItemClass::WeaponPart) {
        item.uses = item.get_max_uses();
        item.update_weight_by(|n| env.script_actual_weight(n));
    }
    item
}

/// The `Rand` draws `Item.InstanceItem` and the item constructors make
/// before the components: a padlock's key id; a clothing item's palette
/// pick and its random `col`; an alarm clock's random alarm.
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
        _ => {}
    }
}

/// [`ItemEnv`] with every default.
pub struct NoItemEnv;
impl ItemEnv for NoItemEnv {}
