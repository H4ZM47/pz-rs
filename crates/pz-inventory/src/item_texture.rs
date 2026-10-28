//! Exclusive actual item asset slots and explicitly separate archive profiles.
//!
//! Assets retain the canonical Rc<LuaTexture>. Rust archive item copies share
//! those assets by identity; they do not copy mutable texture bodies. Pointer
//! getters never borrow a texture body or synthesize a texture from its name.
use std::rc::Rc;

use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::RetiredComponents;
use pz_kahlua::Value;
use pz_render::{TextureHostError, TextureRef};

use crate::shared::{self, ItemRef};
use crate::template::ScriptItemRef;
use crate::{InventoryItem, ItemKind};

pub const QUESTION_TEXTURE: &str = "media/inventory/Question_On.png";

pub fn texture_host_error(error: TextureHostError) -> RuntimeHostError {
    match error {
        TextureHostError::Unavailable(operation) => RuntimeHostError::Unsupported {
            operation,
            kind: None,
        },
        TextureHostError::Borrowed(operation) => RuntimeHostError::Borrowed(operation),
    }
}
fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

/// Genuine script-definition capture input, separate from an item instance.
/// Construct explicitly only when these actual nullable slots are known. None
/// on ItemTemplate.texture_capture instead means uncaptured/archive data.
#[derive(Clone, Debug)]
pub struct CapturedScriptTextures {
    pub normal: Option<TextureRef>,
    pub special: Vec<Option<TextureRef>>,
    pub world_name: Option<String>,
    pub special_world_names: Vec<String>,
    pub color_mask_name: Option<String>,
    pub fluid_mask_name: Option<String>,
}
impl CapturedScriptTextures {
    pub fn new(normal: Option<TextureRef>, special: Vec<Option<TextureRef>>) -> Self {
        Self {
            normal,
            special,
            world_name: None,
            special_world_names: Vec::new(),
            color_mask_name: None,
            fluid_mask_name: None,
        }
    }
}
fn same(a: &Option<TextureRef>, b: &Option<TextureRef>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}
impl PartialEq for CapturedScriptTextures {
    fn eq(&self, other: &Self) -> bool {
        same(&self.normal, &other.normal)
            && self.special.len() == other.special.len()
            && self
                .special
                .iter()
                .zip(&other.special)
                .all(|(a, b)| same(a, b))
            && self.world_name == other.world_name
            && self.special_world_names == other.special_world_names
            && self.color_mask_name == other.color_mask_name
            && self.fluid_mask_name == other.fluid_mask_name
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArchiveTextureProfile {
    pub saved_name: Option<String>,
}

/// Physical order: base, color mask, fluid mask, rotten, cooked, burnt.
/// Food slots are still distinct nullable owning fields even when aliased.
#[derive(Clone, Debug)]
pub struct LiveItemTextures {
    slots: [Option<TextureRef>; 6],
    pub world_name: Option<String>,
    pub food_world_names: [Option<String>; 3],
    pub script_item: Option<ScriptItemRef>,
}
impl LiveItemTextures {
    pub fn slots(&self) -> impl Iterator<Item = &TextureRef> {
        self.slots.iter().filter_map(Option::as_ref)
    }
    pub fn base_slot(&self) -> Option<&TextureRef> {
        self.slots[0].as_ref()
    }
}
impl PartialEq for LiveItemTextures {
    fn eq(&self, other: &Self) -> bool {
        self.slots.iter().zip(&other.slots).all(|(a, b)| same(a, b))
            && self.world_name == other.world_name
            && self.food_world_names == other.food_world_names
            && self.script_item == other.script_item
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ItemTextures {
    Archive(ArchiveTextureProfile),
    Live(LiveItemTextures),
}
impl Default for ItemTextures {
    fn default() -> Self {
        Self::Archive(ArchiveTextureProfile::default())
    }
}
impl ItemTextures {
    pub fn live(&self) -> HostResult<&LiveItemTextures> {
        match self {
            Self::Live(v) => Ok(v),
            Self::Archive(_) => Err(unsupported(
                "actual item textures are uncaptured archive data",
            )),
        }
    }
    pub fn live_slot(&self) -> Option<&LiveItemTextures> {
        match self {
            Self::Live(v) => Some(v),
            Self::Archive(_) => None,
        }
    }
    pub fn archive_saved_name(&self) -> HostResult<Option<&str>> {
        match self {
            Self::Archive(v) => Ok(v.saved_name.as_deref()),
            Self::Live(_) => Err(unsupported("live texture requires actual asset save")),
        }
    }
}
fn read<T>(item: &ItemRef, f: impl FnOnce(&InventoryItem) -> HostResult<T>) -> HostResult<T> {
    let body = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual item texture field"))?;
    f(&body)
}
fn slot(item: &ItemRef, index: usize) -> HostResult<Option<TextureRef>> {
    read(item, |body| Ok(body.textures.live()?.slots[index].clone()))
}
pub fn get_texture_color_mask_ref(item: &ItemRef) -> HostResult<Option<TextureRef>> {
    slot(item, 1)
}
pub fn get_texture_fluid_mask_ref(item: &ItemRef) -> HostResult<Option<TextureRef>> {
    slot(item, 2)
}
pub fn get_texture_ref(item: &ItemRef) -> HostResult<Option<TextureRef>> {
    read(item, |body| body.textures.live().map(|_| ()))?;
    if crate::moddata::has_mod_data_ref(item).map_err(crate::ModDataError::into_host)? {
        let table =
            crate::moddata::get_mod_data_ref(item).map_err(crate::ModDataError::into_host)?;
        let value = table
            .try_rawget(&Value::str("customInventoryIcon"))
            .map_err(crate::ModDataError::from)
            .map_err(crate::ModDataError::into_host)?;
        if let Value::Str(name) = value {
            if let Some(texture) =
                pz_render::get_shared_texture_ref(&name).map_err(texture_host_error)?
            {
                return Ok(Some(texture));
            }
        }
    }
    slot(item, 0)
}
pub fn get_icon_ref(item: &ItemRef) -> HostResult<Option<TextureRef>> {
    get_texture_ref(item)
}
pub fn get_tex_ref(item: &ItemRef) -> HostResult<Option<TextureRef>> {
    let index = read(item, |body| {
        Ok(match &body.kind {
            ItemKind::Food(food) if food.burnt => 5,
            ItemKind::Food(food) if food.age >= food.off_age_max as f32 => 3,
            ItemKind::Food(food) if food.cooked => 4,
            _ => 0,
        })
    })?;
    if index == 0 {
        get_texture_ref(item)
    } else {
        slot(item, index)
    }
}
pub fn get_world_texture_ref(item: &ItemRef) -> HostResult<Option<String>> {
    read(item, |body| {
        let textures = body.textures.live()?;
        let index = match &body.kind {
            ItemKind::Food(food) if food.burnt => Some(2),
            ItemKind::Food(food) if food.age >= food.off_age_max as f32 => Some(0),
            ItemKind::Food(food) if food.cooked => Some(1),
            _ => None,
        };
        Ok(index.map_or_else(
            || textures.world_name.clone(),
            |i| textures.food_world_names[i].clone(),
        ))
    })
}
pub fn item_tint_ref(item: &ItemRef) -> HostResult<[f32; 4]> {
    read(item, |body| Ok(body.col))
}
pub fn get_script_item_ref(item: &ItemRef) -> HostResult<Option<ScriptItemRef>> {
    read(item, |body| Ok(body.textures.live()?.script_item.clone()))
}
/// InventoryItem.hasTag reads its actual mutable scriptItem field. A null
/// script is a native null receiver boundary, never a template fallback.
pub fn has_tag_ref(item: &ItemRef, tag: Option<&Rc<pz_scripts::registry::RegistryKey>>) -> HostResult<bool> {
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(Rc::clone(item)));
    let result = (|| {
        let script = get_script_item_ref(item)?.ok_or_else(|| unsupported("InventoryItem.hasTag null scriptItem"))?;
        retired.push_payload(Box::new(script.clone()));
        let body = script.0.try_borrow().map_err(|_| RuntimeHostError::Borrowed("InventoryItem.hasTag script"))?;
        Ok(body.has_tag(tag))
    })();
    crate::component_host::retire_components(retired);
    result
}

pub fn script_scale_world_icon(script: &ScriptItemRef) -> HostResult<f32> {
    let script = script
        .0
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual item script scaleWorldIcon"))?;
    Ok(script.scale_world_icon)
}

fn empty_live(script: Option<ScriptItemRef>) -> LiveItemTextures {
    LiveItemTextures {
        slots: std::array::from_fn(|_| None),
        world_name: None,
        food_world_names: std::array::from_fn(|_| None),
        script_item: script,
    }
}
/// Publish known Java constructor-null fields before the first script read.
/// An unavailable capture then preserves that actual partial receiver.
pub(crate) fn begin_native_constructor(
    item: &ItemRef,
    script: Option<ScriptItemRef>,
) -> HostResult<()> {
    let mut retired = RetiredComponents::new();
    let result = (|| {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("item texture constructor"))?;
        retired.push_payload(Box::new(std::mem::replace(
            &mut body.textures,
            ItemTextures::Live(empty_live(script)),
        )));
        Ok(())
    })();
    crate::component_host::retire_components(retired);
    shared::observe_item(item);
    result
}
fn replace_slot_ref(item: &ItemRef, index: usize, new: Option<&TextureRef>) -> HostResult<()> {
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(new.cloned()));
    let result = (|| {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("actual item texture setter"))?;
        let ItemTextures::Live(live) = &mut body.textures else {
            return Err(unsupported("actual item texture setter on archive profile"));
        };
        retired.push_payload(Box::new(std::mem::replace(
            &mut live.slots[index],
            new.cloned(),
        )));
        Ok(())
    })();
    crate::component_host::retire_components(retired);
    shared::observe_item(item);
    result
}
pub fn set_texture_ref(item: &ItemRef, new: Option<&TextureRef>) -> HostResult<()> {
    replace_slot_ref(item, 0, new)
}
fn mask_ref(item: &ItemRef, index: usize, name: &str) -> HostResult<()> {
    // Native first publishes the try lookup, even null, before fallback.
    let first = pz_render::try_get_texture_ref(name).map_err(texture_host_error)?;
    replace_slot_ref(item, index, first.as_ref())?;
    if first.is_none() {
        let fallback =
            pz_render::get_shared_texture_ref(QUESTION_TEXTURE).map_err(texture_host_error)?;
        replace_slot_ref(item, index, fallback.as_ref())?;
    }
    Ok(())
}
pub fn set_texture_color_mask_ref(item: &ItemRef, name: &str) -> HostResult<()> {
    mask_ref(item, 1, name)
}
pub fn set_texture_fluid_mask_ref(item: &ItemRef, name: &str) -> HostResult<()> {
    mask_ref(item, 2, name)
}

fn current_script_capture(template: &crate::ItemTemplate) -> HostResult<CapturedScriptTextures> {
    if let Some(script) = &template.script {
        let script = script
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual item constructor script"))?;
        if !script.texture_capture_complete() {
            return Err(unsupported("actual script texture capture incomplete"));
        }
        let mut capture = CapturedScriptTextures::new(
            script.normal_texture_slot().cloned(),
            script.special_texture_slots().to_vec(),
        );
        capture.world_name = script.world_texture_name.clone();
        capture.special_world_names = script.special_world_texture_names.clone();
        capture.color_mask_name = script.icon_color_mask.clone();
        capture.fluid_mask_name = script.icon_fluid_mask.clone();
        Ok(capture)
    } else {
        template.texture_capture.clone().ok_or_else(|| {
            unsupported("actual constructor requires captured script texture profile")
        })
    }
}
fn initialize_food_java_fields(item: &ItemRef) -> HostResult<()> {
    let mut item = item.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("Food allocation fields"))?;
    if let ItemKind::Food(food) = &mut item.kind {
        food.poison = false;
        food.poison_power = 0;
        food.poison_detection_level = 0;
        food.poison_level_for_recipe = 0;
        food.herbalist_type = None;
    }
    Ok(())
}
fn finish_food_super_fields(item: &ItemRef) -> HostResult<()> {
    let mut item = item.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("Food constructor fields"))?;
    if let ItemKind::Food(food) = &mut item.kind { food.poison_detection_level = -1; }
    Ok(())
}

pub(crate) fn constructor_script_ref(
    item: &ItemRef,
    template: &crate::ItemTemplate,
) -> HostResult<()> {
    // The script constructor only reads its argument. The retained scriptItem
    // field stays null until Item.InstanceItem.setScriptItem at PC3398.
    initialize_food_java_fields(item)?;
    crate::literature::initialize_java_fields(item)?;
    begin_native_constructor(item, None)?;
    let capture = current_script_capture(template)?;
    set_texture_ref(item, capture.normal.as_ref())?;
    {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("actual constructor world texture"))?;
        let ItemTextures::Live(live) = &mut body.textures else {
            unreachable!()
        };
        live.world_name = capture.world_name.clone();
    }
    crate::literature::finish_super_fields(item)?;
    if template.class == crate::ItemClass::Food {
        finish_food_super_fields(item)?;
        pz_render::set_warn_fail_find_texture(false);
        for (n, index) in [3, 4, 5].into_iter().enumerate() {
            if let Some(texture) = capture.special.get(n) {
                replace_slot_ref(item, index, texture.as_ref())?;
            }
        }
        pz_render::set_warn_fail_find_texture(true);
        for index in [3, 4, 5] {
            if slot(item, index)?.is_none() {
                let base = slot(item, 0)?;
                replace_slot_ref(item, index, base.as_ref())?;
            }
        }
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("actual Food constructor world textures"))?;
        let ItemTextures::Live(live) = &mut body.textures else {
            unreachable!()
        };
        for n in 0..3 {
            live.food_world_names[n] = capture.special_world_names.get(n).cloned();
        }
    }
    shared::observe_item(item);
    Ok(())
}
/// Item.InstanceItem.setScriptItem publication, separate from constructor reads.
pub(crate) fn set_script_item_ref(
    item: &ItemRef,
    script: Option<&ScriptItemRef>,
) -> HostResult<()> {
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(script.cloned()));
    let result = (|| {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("actual setScriptItem"))?;
        let ItemTextures::Live(live) = &mut body.textures else {
            return Err(unsupported("actual setScriptItem on archive profile"));
        };
        retired.push_payload(Box::new(std::mem::replace(
            &mut live.script_item,
            script.cloned(),
        )));
        Ok(())
    })();
    crate::component_host::retire_components(retired);
    shared::observe_item(item);
    result
}

pub(crate) fn constructor_masks_ref(
    item: &ItemRef,
    template: &crate::ItemTemplate,
) -> HostResult<()> {
    let capture = current_script_capture(template)?;
    if let Some(name) = capture.color_mask_name {
        set_texture_color_mask_ref(item, &format!("{name}.png"))?;
    }
    if let Some(name) = capture.fluid_mask_name {
        set_texture_fluid_mask_ref(item, &format!("{name}.png"))?;
    }
    Ok(())
}

/// Native directly dereferences the actual script item; no world-name proxy.
pub fn is_use_world_item_ref(item: &ItemRef) -> HostResult<bool> {
    let script = get_script_item_ref(item)?
        .ok_or_else(|| unsupported("InventoryItem.isUseWorldItem null script diagnostic"))?;
    let script = script
        .0
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual item script useWorldItem"))?;
    Ok(script.use_world_item)
}

/// Preserves a new host/provider error separately from legacy scalar codecs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemPersistenceError {
    Host(RuntimeHostError),
    Codec(crate::io::SaveError),
}
impl std::fmt::Display for ItemPersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Host(error) => write!(f, "{error:?}"),
            Self::Codec(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for ItemPersistenceError {}
impl From<RuntimeHostError> for ItemPersistenceError {
    fn from(value: RuntimeHostError) -> Self {
        Self::Host(value)
    }
}
impl From<crate::io::SaveError> for ItemPersistenceError {
    fn from(value: crate::io::SaveError) -> Self {
        Self::Codec(value)
    }
}

impl From<crate::ModDataError> for ItemPersistenceError {
    fn from(error: crate::ModDataError) -> Self {
        Self::Host(error.into_host())
    }
}
impl From<ItemPersistenceError> for crate::io::SaveError {
    /// Explicit legacy value/archive edge. Actual callers retain the enum.
    fn from(error: ItemPersistenceError) -> Self {
        match error {
            ItemPersistenceError::Codec(error) => error,
            ItemPersistenceError::Host(error) => Self(format!("{error:?}")),
        }
    }
}

fn texture_name(texture: &TextureRef) -> HostResult<String> {
    Ok(pz_render::try_texture_descriptor(texture)
        .map_err(texture_host_error)?
        .name
        .to_string())
}
fn native_saved_name(item: &ItemRef) -> HostResult<Option<String>> {
    let base = slot(item, 0)?;
    let Some(texture) = base else { return Ok(None) };
    // Native checks actual getName before Question identity and rereads it for
    // normalization and output. Descriptor copies are only momentary reads.
    let _first_name = texture_name(&texture)?;
    // PC1092 captures this current base before getSharedTexture executes.
    let identity_texture =
        slot(item, 0)?.ok_or_else(|| unsupported("InventoryItem.save null texture diagnostic"))?;
    let question =
        pz_render::get_shared_texture_ref(QUESTION_TEXTURE).map_err(texture_host_error)?;
    if question
        .as_ref()
        .is_some_and(|q| Rc::ptr_eq(q, &identity_texture))
    {
        return Ok(None);
    }
    let script = get_script_item_ref(item)?
        .ok_or_else(|| unsupported("InventoryItem.save null script icon diagnostic"))?;
    let icon = {
        let script = script
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual save script icon"))?;
        script.icon.clone()
    };
    let Some(icon) = icon else { return Ok(None) };
    if icon == "None" || icon == "default" {
        return Ok(None);
    }
    let current = slot(item, 0)?
        .ok_or_else(|| unsupported("InventoryItem.save null texture name diagnostic"))?;
    let name = texture_name(&current)?;
    let normalized = if let Some(index) = name.rfind(std::path::MAIN_SEPARATOR) {
        name[index + 1..].replace(".png", "")
    } else {
        name
    };
    if normalized == format!("Item_{icon}") {
        return Ok(None);
    }
    let current = slot(item, 0)?
        .ok_or_else(|| unsupported("InventoryItem.save null output texture diagnostic"))?;
    Ok(Some(texture_name(&current)?))
}
/// Write directly into the caller's existing writer; later failure retains
/// completed scalar/component/table prefix and unpatched native header slots.
pub fn save_item_ref(
    item: &ItemRef,
    w: &mut crate::io::Writer,
    env: &dyn crate::ItemEnv,
) -> Result<(), ItemPersistenceError> {
    crate::shared_save::save_item_ref(item, w, env)
}

pub(crate) fn save_item_ref_in_context(
    item: &ItemRef,
    w: &mut crate::io::Writer,
    env: &dyn crate::ItemEnv,
    context: &mut crate::shared_save::SaveContext,
) -> Result<(), ItemPersistenceError> {
    let continuation = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual item save prefix"))?;
        body.save_native_prefix(w, env)?
    };
    let name = native_saved_name(item)?;
    let container = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual item save suffix"))?;
        body.save_native_suffix_base(w, continuation, name.as_deref())?;
        match &body.kind {
            ItemKind::Container(_) => true,
            ItemKind::Weapon(weapon) => {
                weapon.save_native(w, env, &body.template)?;
                false
            }
            ItemKind::Map { map_id, .. } => {
                w.string(map_id);
                return Err(unsupported("actual MapItem symbols save authority").into());
            }
            _ => {
                body.save_kind(w, env)?;
                false
            }
        }
    };
    if container {
        crate::shared_save::save_bag_tail(item, w, env, context)?;
    }
    Ok(())
}

/// Native load setTexture(shared(name)) publishes true null and has no fallback.
pub fn load_texture_ref(item: &ItemRef, name: &str) -> HostResult<()> {
    let texture = pz_render::get_shared_texture_ref(name).map_err(texture_host_error)?;
    set_texture_ref(item, texture.as_ref())
}
fn sync_icon_ref(item: &ItemRef, icon: &str) -> HostResult<()> {
    let texture =
        pz_render::try_get_texture_ref(&format!("Item_{icon}")).map_err(texture_host_error)?;
    set_texture_ref(item, texture.as_ref())?;
    if texture.is_none() {
        let question =
            pz_render::get_shared_texture_ref(QUESTION_TEXTURE).map_err(texture_host_error)?;
        set_texture_ref(item, question.as_ref())?;
    }
    Ok(())
}
/// Native model draw, first texture write, clothing color/visual work, then
/// second texture write. No texture-provider callback spans an item guard.
pub fn synch_with_visual_ref(
    item: &ItemRef,
    assets: Option<&crate::ItemAssets>,
    owner: &dyn crate::ItemOwner,
    rng: &dyn crate::visual::VisualRng,
    model_rand: &mut dyn FnMut(i32) -> i32,
) -> HostResult<()> {
    let first: HostResult<Option<String>> = (|| {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("item synch model index"))?;
        // Model/scalar branches do not read a texture field. The strict actual
        // factory establishes Live before entry; an explicit archive receiver
        // can still run a branch which never requests an actual asset.
        body.pick_model_index(model_rand);
        Ok(
            match (body.model_index, body.template.icons_for_texture.as_ref()) {
                (-1, _) | (_, None) => None,
                (index, Some(icons)) => {
                    let index = usize::try_from(index).map_err(|_| {
                        unsupported("native negative IconsForTexture index diagnostic")
                    })?;
                    Some(
                        icons
                            .get(index)
                            .ok_or_else(|| unsupported("native IconsForTexture index diagnostic"))?
                            .clone(),
                    )
                }
            },
        )
    })();
    shared::observe_item(item);
    if let Some(icon) = first?.filter(|s| !s.chars().all(char::is_whitespace)) {
        sync_icon_ref(item, &icon)?;
    }
    if assets.is_some() {
        crate::moddata::prepare_clothing_mod_data_ref(item)
            .map_err(crate::ModDataError::into_host)?;
    }
    let second: HostResult<Option<String>> = (|| {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("item synch clothing"))?;
        body.try_synch_with_visual(assets, owner, rng)
            .map_err(crate::ModDataError::into_host)?;
        if !matches!(
            body.kind,
            ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..) | ItemKind::Container(_)
        ) {
            return Ok(None);
        }
        let Some(visual) = body.visual.as_ref() else {
            return Ok(None);
        };
        let Some(clothing) = body
            .try_get_clothing_item(assets, owner)
            .map_err(crate::ModDataError::into_host)?
        else {
            return Ok(None);
        };
        if clothing.base_textures.len() <= 1 && visual.texture_choice <= -1 {
            return Ok(None);
        }
        let Some(icons) = body.template.icons_for_texture.as_ref() else {
            return Ok(None);
        };
        let pick = |index: i8| {
            usize::try_from(index)
                .ok()
                .and_then(|n| icons.get(n))
                .cloned()
        };
        Ok(pick(visual.base_texture).or_else(|| pick(visual.texture_choice)))
    })();
    shared::observe_item(item);
    if let Some(icon) = second?.filter(|s| !s.chars().all(char::is_whitespace)) {
        sync_icon_ref(item, &icon)?;
    }
    Ok(())
}

impl InventoryItem {
    /// Collector-only physical detach, preserving profile and scalar/string
    /// state. Caller retires this combined payload outside all foreign guards.
    pub fn take_texture_slots_for_gc(&mut self) -> (Vec<TextureRef>, Option<ScriptItemRef>) {
        match &mut self.textures {
            ItemTextures::Archive(_) => (Vec::new(), None),
            ItemTextures::Live(live) => (
                live.slots.iter_mut().filter_map(Option::take).collect(),
                live.script_item.take(),
            ),
        }
    }
}

/// Texture portion of the genuine nonnull string constructor. This is a
/// distinct native producer, never a fallback for an uncaptured script item.
pub fn constructor_string_ref(item: &ItemRef, name: &str) -> HostResult<()> {
    initialize_food_java_fields(item)?;
    crate::literature::initialize_java_fields(item)?;
    begin_native_constructor(item, None)?;
    let texture = pz_render::try_get_texture_ref(name).map_err(texture_host_error)?;
    set_texture_ref(item, texture.as_ref())?;
    if texture.is_none() {
        let question =
            pz_render::get_shared_texture_ref(QUESTION_TEXTURE).map_err(texture_host_error)?;
        set_texture_ref(item, question.as_ref())?;
    }
    let world_name = format!(
        "{}.png",
        name.replace("Item_", "media/inventory/world/WItem_")
    );
    {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("string constructor world texture"))?;
        let ItemTextures::Live(live) = &mut body.textures else {
            unreachable!()
        };
        live.world_name = Some(world_name.clone());
    }
    crate::literature::finish_super_fields(item)?;
    let food = read(item, |body| Ok(matches!(body.kind, ItemKind::Food(_))))?;
    if food {
        finish_food_super_fields(item)?;
        pz_render::set_warn_fail_find_texture(false);
        let mut world_suffixes = ["Rotten.png", "Cooked.png", "Overdone.png"];
        let variants: [(usize, &[(&str, &str)]); 3] = [
            (
                3,
                &[
                    ("Rotten", "Rotten.png"),
                    ("Spoiled", "Spoiled.png"),
                    ("_Rotten", "_Rotten.png"),
                ],
            ),
            (4, &[("Cooked", "Cooked.png"), ("_Cooked", "_Cooked.png")]),
            (
                5,
                &[
                    ("Overdone", "Overdone.png"),
                    ("Burnt", "Burnt.png"),
                    ("_Burnt", "_Burnt.png"),
                ],
            ),
        ];
        for (n, (index, variants)) in variants.into_iter().enumerate() {
            for &(suffix, world_suffix) in variants {
                let texture = pz_render::try_get_texture_ref(&format!("{name}{suffix}"))
                    .map_err(texture_host_error)?;
                replace_slot_ref(item, index, texture.as_ref())?;
                if texture.is_some() {
                    world_suffixes[n] = world_suffix;
                    break;
                }
            }
        }
        pz_render::set_warn_fail_find_texture(true);
        for index in [3, 4, 5] {
            if slot(item, index)?.is_none() {
                let base = slot(item, 0)?;
                replace_slot_ref(item, index, base.as_ref())?;
            }
        }
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Food string world textures"))?;
        let ItemTextures::Live(live) = &mut body.textures else {
            unreachable!()
        };
        for index in [1, 2, 0] {
            live.food_world_names[index] = Some(world_name.replace(".png", world_suffixes[index]));
        }
    }
    shared::observe_item(item);
    Ok(())
}

#[cfg(test)]
mod tests;
