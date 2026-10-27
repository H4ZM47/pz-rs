//! Data and inventory phases of `IsoMannequin.addToWorld`.
//!
//! The engine enters here with a retained actual receiver after releasing its
//! world and sprite guards. Outfit selection is independent of rendering.
//! Model/atlas allocation remains the renderer's responsibility.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use pz_characters::worn_items::{
    add_items_to_item_container, set_from_item_visuals, WornItems, WornItemsInventoryHost,
};
use pz_inventory::shared::{ContainerRef, ItemRef, SharedContainer};
use pz_inventory::visual::human::HumanVisualOwner;
use pz_inventory::visual::ItemVisual;
use pz_kahlua::Thread;
use pz_scripts::objects::models::MannequinScript;
use pz_scripts::registry::RegistryKey;
use pz_scripts::{ScriptError, ScriptManager, ScriptResult, ScriptType};
use pz_world::objects::{IsoMannequin, ObjectKind};
use pz_world::zones::MannequinZoneData;
use pz_world::{IsoDirections, IsoObjectRef};

#[cfg(test)]
mod tests;

fn borrowed(what: &str) -> ScriptError {
    ScriptError::new(format!("host borrow conflict: {what}"))
}

fn properties_from_sprite(m: &mut IsoMannequin, name: Option<&str>) {
    let Some(number) = name
        .and_then(|s| s.strip_prefix("location_shop_mall_01_"))
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return;
    };
    let (prefix, pose) = match number {
        65..=67 => ("FemaleWhite", number - 64),
        68..=70 => ("MaleWhite", number - 67),
        73..=75 => ("FemaleBlack", number - 72),
        76..=78 => ("MaleBlack", number - 75),
        _ => return,
    };
    // The native switch matches exact names, not numeric suffix aliases.
    if name != Some(format!("location_shop_mall_01_{number}").as_str()) {
        return;
    }
    m.mannequin_script_name = Some(format!("{prefix}{pose:02}"));
    m.dir = if pose == 2 {
        IsoDirections::S
    } else {
        IsoDirections::SE
    } as i8;
}

fn properties_from_zone(m: &mut IsoMannequin, zone: &MannequinZoneData) {
    if zone.female != -1 {
        m.female = zone.female == 1;
    }
    if let Some(dir) = zone.dir {
        m.dir = dir as i8;
    }
    if let Some(v) = &zone.mannequin_script {
        m.mannequin_script_name = Some(v.clone());
    }
    if let Some(v) = &zone.skin {
        m.texture_name = Some(v.clone());
    }
    if let Some(v) = &zone.pose {
        m.pose = Some(v.clone());
    }
    if let Some(v) = &zone.outfit {
        m.outfit = Some(v.clone());
    }
}

fn init_script(m: &mut IsoMannequin, scripts: &ScriptManager) {
    let script = m
        .mannequin_script_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .and_then(|name| scripts.get::<MannequinScript>(ScriptType::Mannequin, name));
    if let Some(script) = script {
        let script = script.borrow();
        m.female = script.is_female();
        m.model_script_name = script.model_script_name().map(str::to_owned);
        if m.texture_name.is_none() {
            m.texture_name = script.texture().map(str::to_owned);
        }
        m.anim_set = script.anim_set().map(str::to_owned);
        m.anim_state = script.anim_state().map(str::to_owned);
        if m.pose.is_none() {
            m.pose = script.pose().map(str::to_owned);
        }
        if m.outfit.is_none() {
            m.outfit = script.outfit().map(str::to_owned);
        }
    } else {
        m.model_script_name = Some(if m.female { "FemaleBody" } else { "MaleBody" }.into());
        m.texture_name = Some(
            if m.female {
                "F_Mannequin_White"
            } else {
                "M_Mannequin_White"
            }
            .into(),
        );
        m.anim_set = Some("mannequin".into());
        m.anim_state = Some(if m.female { "female" } else { "male" }.into());
        m.outfit = None;
    }
}

fn promote_loaded_inventory(object: &IsoObjectRef, scripts: &ScriptManager) -> ScriptResult {
    let registry = scripts
        .registries
        .get("item_body_location")
        .ok_or_else(|| ScriptError::new("missing item_body_location registry"))?;
    let group = crate::bind_characters::shared_characters()
        .borrow_mut()
        .body_locations
        .get_group("Human")?;
    let saved = {
        let mut object = object
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoMannequin"))?;
        let ObjectKind::Mannequin(m) = &mut object.kind else {
            return Ok(());
        };
        if m.runtime_container.is_some() {
            return Ok(());
        }
        m.container
            .take()
            .map(|c| (c, std::mem::take(&mut m.worn_items)))
    };
    let Some((saved, worn)) = saved else {
        return Ok(());
    };
    let mut decoded = 0;
    let input_to_decoded: Vec<_> = saved
        .items
        .iter()
        .map(|item| {
            if matches!(item, pz_inventory::SavedItem::Item(_)) {
                let index = decoded;
                decoded += 1;
                Some(index)
            } else {
                None
            }
        })
        .collect();
    let container = SharedContainer::from_container(saved);
    let members = Rc::new(RefCell::new(WornItems::new(group.clone())));
    let retired = {
        let mut object = object
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoMannequin"))?;
        let ObjectKind::Mannequin(m) = &mut object.kind else {
            return Ok(());
        };
        m.install_runtime_inventory(container.clone(), members.clone())
    };
    drop(retired);
    // Native load catches a worn-data error while keeping the attached
    // container and completed membership prefix. Unknown Human locations skip.
    let restored = (|| -> ScriptResult {
        for (location, index) in worn {
            let key = registry.borrow().get_id(&location)?;
            let item = usize::try_from(index)
                .ok()
                .and_then(|i| input_to_decoded.get(i).copied().flatten())
                .and_then(|i| container.borrow().items.get(i).cloned());
            let Some(item) = item else { continue };
            if group.borrow().location(key.as_ref())?.is_none() {
                continue;
            }
            if let Some(key) = key {
                members.borrow_mut().set_item(Some(key), Some(item))?;
            }
        }
        Ok(())
    })();
    if let Err(error) = restored {
        eprintln!("IsoMannequin.load worn items: {error}");
    }
    Ok(())
}

struct VisualOwner {
    female: bool,
    zombie: bool,
    skeleton: bool,
}
impl HumanVisualOwner for VisualOwner {
    fn is_female(&self) -> bool {
        self.female
    }
    fn is_zombie(&self) -> bool {
        self.zombie
    }
    fn is_skeleton(&self) -> bool {
        self.skeleton
    }
}

struct InventoryHost<'a>(&'a mut Thread);
impl WornItemsInventoryHost for InventoryHost<'_> {
    fn create_item(&mut self, item_type: &str) -> ScriptResult<Option<ItemRef>> {
        let item = crate::bind_items::try_create_item(item_type, 1.0)
            .map_err(|error| pz_scripts::ScriptError::new(format!("{error:?}")))?;
        if let Some(item) = &item {
            crate::bind_items::initialise_item(self.0, item);
        }
        Ok(item)
    }
    fn copy_visual_and_sync(&mut self, item: &ItemRef, from: &ItemVisual) -> ScriptResult {
        let env = crate::bind_items::item_env();
        let owner = env.owner(item);
        if env.context().assets().is_some() {
            pz_inventory::moddata::prepare_clothing_mod_data_ref(item)
                .map_err(|error| ScriptError::new(error.to_string()))?;
        }
        let mut item = item
            .try_borrow_mut()
            .map_err(|_| borrowed("InventoryItem"))?;
        if let Some(visual) = item
            .try_get_visual(env.context().assets(), owner.as_ref(), env.visual_rng())
            .map_err(|error| ScriptError::new(error.to_string()))?
        {
            *visual = from.clone();
            item.try_synch_with_visual(env.context().assets(), owner.as_ref(), env.visual_rng())
                .map_err(|error| ScriptError::new(error.to_string()))?;
        }
        Ok(())
    }
    fn body_location(
        &mut self,
        item: &ItemRef,
        clothing: bool,
    ) -> ScriptResult<Option<Rc<RegistryKey>>> {
        let item = item.try_borrow().map_err(|_| borrowed("InventoryItem"))?;
        if !clothing {
            return Ok(item.can_be_equipped());
        }
        let script = item
            .template
            .script
            .as_ref()
            .ok_or_else(|| ScriptError::new("java.lang.NullPointerException: script item"))?;
        let location = script.0.borrow().body_location.clone();
        Ok(location)
    }
    fn get_holes_number(&mut self, item: &ItemRef) -> ScriptResult<i32> {
        let env = crate::bind_items::item_env();
        let owner = env.owner(item);
        if env.context().assets().is_some() {
            pz_inventory::moddata::prepare_clothing_mod_data_ref(item)
                .map_err(|error| ScriptError::new(error.to_string()))?;
        }
        let mut item = item
            .try_borrow_mut()
            .map_err(|_| borrowed("InventoryItem"))?;
        item.try_get_visual(env.context().assets(), owner.as_ref(), env.visual_rng())
            .map_err(|error| ScriptError::new(error.to_string()))?
            .map(|v| v.get_holes_number())
            .ok_or_else(|| ScriptError::new("java.lang.NullPointerException: item visual"))
    }
    fn set_condition_no_sound(&mut self, item: &ItemRef, condition: i32) -> ScriptResult {
        let env = crate::bind_items::item_env();
        let owner = env.owner(item);
        let out = item
            .try_borrow_mut()
            .map_err(|_| borrowed("InventoryItem"))?
            .set_condition_no_sound(condition, owner.as_ref(), env.context());
        env.outcome(item, crate::bind_items::ItemOutcome::Condition(out));
        Ok(())
    }
    fn add_item(&mut self, container: &ContainerRef, item: &ItemRef) -> ScriptResult {
        crate::bind_item_container::add_item(self.0, container, Some(item.clone()))
            .map(|_| ())
            .map_err(|e| ScriptError::new(e.to_string()))
    }
}

/// First/repeated `initOutfit`, pose validation and persistent `syncModel`
/// fields. `zone` must be the first matching actual meta-cell mannequin zone,
/// and absent for a detached receiver. SoftReset callers skip this phase.
pub fn activate(
    thread: &mut Thread,
    object: &IsoObjectRef,
    sprite_name: Option<&str>,
    zone: Option<&MannequinZoneData>,
    game_dir: &Path,
    no_black_clothes: bool,
) -> ScriptResult {
    let scripts = crate::bind_registry::shared_script_manager();
    promote_loaded_inventory(object, &scripts)?;
    let group = crate::bind_characters::shared_characters()
        .borrow_mut()
        .body_locations
        .get_group("Human")?;
    let first = {
        let mut object = object
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoMannequin"))?;
        let ObjectKind::Mannequin(m) = &mut object.kind else {
            return Ok(());
        };
        if m.runtime_worn_items.is_none() {
            m.runtime_worn_items = Some(Rc::new(RefCell::new(WornItems::new(group))));
        }
        if m.pending_loaded_skin_resolution {
            let owner = VisualOwner {
                female: m.female,
                zombie: m.zombie,
                skeleton: m.skeleton,
            };
            m.texture_name = crate::bind_appearance::with_styles(thread, |styles| {
                m.human_visual.get_skin_texture(&owner, styles)
            });
            m.pending_loaded_skin_resolution = false;
        }
        let first = !m.init;
        if first {
            m.init = true;
            if sprite_name.is_none() {
                return Err(ScriptError::new(
                    "java.lang.NullPointerException: mannequin sprite",
                ));
            }
            properties_from_sprite(m, sprite_name);
            if let Some(zone) = zone {
                properties_from_zone(m, zone);
            }
        }
        init_script(m, &scripts);
        first
    };
    if first {
        let result =
            crate::bind_appearance::with_dress_context_options(thread, no_black_clothes, |ctx| {
                let mut object = object
                    .try_borrow_mut()
                    .map_err(|_| borrowed("IsoMannequin"))?;
                let ObjectKind::Mannequin(m) = &mut object.kind else {
                    return Ok(());
                };
                let name = match m.outfit.as_deref() {
                    None => Some(
                        ctx.outfits
                            .get_random_non_professional_outfit(m.female)
                            .ok_or_else(|| {
                                ScriptError::new("java.lang.NullPointerException: mannequin outfit")
                            })?
                            .name
                            .clone(),
                    ),
                    Some(s) if s.eq_ignore_ascii_case("none") => None,
                    Some(s) => Some(s.to_owned()),
                };
                if let Some(name) = name {
                    pz_characters::population::dress_in_named_outfit(
                        &mut m.human_visual,
                        m.female,
                        Some(&name),
                        &mut m.item_visuals,
                        ctx,
                    )
                    .map_err(ScriptError::new)?;
                }
                m.human_visual.set_hair_model(Some(""));
                m.human_visual.set_beard_model(Some(""));
                Ok(())
            })
            .ok_or_else(|| ScriptError::new("mannequin dressing owners are unavailable"))?;
        result?;
        let (container, worn, visuals, retired) = {
            let mut object = object
                .try_borrow_mut()
                .map_err(|_| borrowed("IsoMannequin"))?;
            let ObjectKind::Mannequin(m) = &mut object.kind else {
                return Ok(());
            };
            let container = match &m.runtime_container {
                Some(c) => c.clone(),
                None => {
                    let mut c = pz_inventory::ItemContainer::new(Some("mannequin"));
                    c.explored = true;
                    SharedContainer::from_container(c)
                }
            };
            let worn = m.runtime_worn_items.as_ref().unwrap().clone();
            let retired = m.install_runtime_inventory(container.clone(), worn.clone());
            (container, worn, m.item_visuals.clone(), retired)
        };
        drop(retired);
        // Detach prior contents before invoking any item callbacks.
        let old = {
            let mut container = container.borrow_mut();
            (
                std::mem::take(&mut container.items),
                std::mem::take(&mut container.raw_items),
            )
        };
        for item in &old.0 {
            pz_inventory::shared::set_container(item, None);
        }
        drop(old);
        let mut host = InventoryHost(thread);
        let result = set_from_item_visuals(&worn, &visuals, &mut host)
            .and_then(|_| add_items_to_item_container(&worn, &container, &mut host));
        result?;
    }
    let mut receiver = object
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoMannequin"))?;
    let ObjectKind::Mannequin(m) = &mut receiver.kind else {
        return Ok(());
    };
    let set = m
        .anim_set
        .as_deref()
        .map(|n| pz_anim::AnimationSet::load(n, &game_dir.join("media/AnimSets").join(n)));
    let state = set.as_ref().and_then(|s| {
        m.anim_state
            .as_deref()
            .and_then(|n| s.states.get(&n.to_lowercase()))
    });
    match state {
        None => m.pose = Some("Invalid".into()),
        Some(state)
            if !state.nodes.iter().any(|n| {
                m.pose
                    .as_deref()
                    .is_some_and(|p| pz_scripts::java::eq_ignore_case(&n.name, p))
            }) =>
        {
            let index = pz_core::random::next(state.nodes.len() as i32) as usize;
            m.pose = Some(
                state
                    .nodes
                    .get(index)
                    .ok_or_else(|| {
                        ScriptError::new("java.lang.NullPointerException: mannequin pose")
                    })?
                    .name
                    .clone(),
            );
        }
        _ => {}
    }
    m.human_visual.force_model_script = m.model_script_name.clone();
    m.human_visual
        .set_skin_texture_name(m.texture_name.as_deref());
    let items: Vec<_> = m
        .runtime_worn_items
        .as_ref()
        .map(|worn| {
            worn.borrow()
                .items()
                .iter()
                .map(|entry| entry.item.clone())
                .collect()
        })
        .unwrap_or_default();
    drop(receiver);
    let env = crate::bind_items::item_env();
    let mut visuals = Vec::new();
    for item in items {
        let owner = env.owner(&item);
        if env.context().assets().is_some() {
            pz_inventory::moddata::prepare_clothing_mod_data_ref(&item)
                .map_err(|error| ScriptError::new(error.to_string()))?;
        }
        let mut item = item
            .try_borrow_mut()
            .map_err(|_| borrowed("InventoryItem"))?;
        if let Some(visual) = item
            .try_get_visual(env.context().assets(), owner.as_ref(), env.visual_rng())
            .map_err(|error| ScriptError::new(error.to_string()))?
        {
            visuals.push(visual.clone());
        }
    }
    let mut receiver = object
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoMannequin"))?;
    if let ObjectKind::Mannequin(m) = &mut receiver.kind {
        m.item_visuals = visuals;
    }
    Ok(())
}
