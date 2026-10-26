//! What item rendering reads off an instance: the per-instance model lists
//! and the model getters of `InventoryItem`, `HandWeapon` and `Food`
//! (`getStaticModel`, `getWorldStaticItem`/`getWorldStaticModel`,
//! `getStaticModelException`, `getWeaponSprite`, `getModelWeaponPart`),
//! `Food.isFertilized`/`isRotten`, and the item's `FluidContainer`
//! component (`getFluidContainer()`).

use std::rc::Rc;

use pz_entity::{Component, Components, EntityContext, FluidContainer};
use pz_scripts::objects::entity::ComponentType;
use pz_scripts::objects::items::ModelWeaponPart;

use crate::component::{ComponentBlob, COMPONENT_WORLD_VERSION};
use crate::item::{InventoryItem, ItemKind};
use crate::lua_table::{LuaKey, LuaValue};

/// `ItemTag.USE_WORLD_STATIC_MODEL`.
pub const TAG_USE_WORLD_STATIC_MODEL: &str = "base:useworldstaticmodel";

impl InventoryItem {
    fn mod_data_string(&self, key: &str) -> Option<Option<&str>> {
        let v = self.table.as_ref()?.rawget(&LuaKey::Str(key.to_string()))?;
        Some(match v {
            LuaValue::Str(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// `getStaticModelsByIndex()`.
    pub fn get_static_models_by_index(&self) -> Option<&Rc<Vec<String>>> {
        self.static_models_by_index.as_ref()
    }

    /// `getWorldStaticModelsByIndex()`.
    pub fn get_world_static_models_by_index(&self) -> Option<&Rc<Vec<String>>> {
        self.world_static_models_by_index.as_ref()
    }

    /// `HandWeapon.getWeaponSpritesByIndex()` (`None` for other items).
    pub fn get_weapon_sprites_by_index(&self) -> Option<&Rc<Vec<String>>> {
        match &self.kind {
            ItemKind::Weapon(w) => w.weapon_sprites_by_index.as_ref(),
            _ => None,
        }
    }

    /// `HandWeapon.getModelWeaponPart()` (`None` for other items).
    pub fn get_model_weapon_part(&self) -> Option<&Rc<Vec<ModelWeaponPart>>> {
        match &self.kind {
            ItemKind::Weapon(w) => w.model_weapon_part.as_ref(),
            _ => None,
        }
    }

    /// `HandWeapon.getWeaponSprite()`: the indexed sprite when the item has
    /// a model index and the list, else the weapon's own. `None` for other
    /// items. An index past the list's end (the original throws) gives
    /// `None`.
    pub fn get_weapon_sprite(&self) -> Option<String> {
        let ItemKind::Weapon(w) = &self.kind else {
            return None;
        };
        if self.model_index != -1 {
            if let Some(l) = &w.weapon_sprites_by_index {
                return index(l, self.model_index);
            }
        }
        w.weapon_sprite.clone()
    }

    /// `InventoryItem.getStaticModel()` (without the subclass overrides): a
    /// `staticModel` mod data string, the indexed instance model, or the
    /// script's. A non-string mod data value (the original's cast throws)
    /// and an index past the list's end give `None`.
    pub fn get_base_static_model(&self) -> Option<String> {
        if let Some(v) = self.mod_data_string("staticModel") {
            return v.map(str::to_string);
        }
        if self.model_index != -1 {
            if let Some(l) = &self.static_models_by_index {
                return index(l, self.model_index);
            }
        }
        self.template.static_model.clone()
    }

    /// `getStaticModel()` with the `HandWeapon` (indexed weapon sprite, then
    /// the weapon sprite: `staticModel` is only set on `Literature`) and
    /// `Food` (`<model>Burnt` / `Rotten` / `Cooked` model scripts) forms.
    ///
    /// `model_script` is `ScriptManager.getModelScript(name)` answering the
    /// script's `getName()`.
    pub fn get_static_model(
        &self,
        model_script: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        if let ItemKind::Weapon(w) = &self.kind {
            if self.model_index != -1 {
                if let Some(l) = &w.weapon_sprites_by_index {
                    return index(l, self.model_index);
                }
            }
            return w.weapon_sprite.clone();
        }
        if let ItemKind::Food(f) = &self.kind {
            let variant = |suffix: &str| {
                let base = self.get_base_static_model();
                model_script(&format!("{}{suffix}", base.as_deref().unwrap_or("null")))
            };
            if f.burnt {
                if let Some(n) = variant("Burnt") {
                    return Some(n);
                }
            }
            if self.is_rotten() {
                if let Some(n) = variant("Rotten") {
                    return Some(n);
                }
            }
            if f.cooked {
                if let Some(n) = variant("Cooked") {
                    return Some(n);
                }
            }
        }
        self.get_base_static_model()
    }

    /// `tryGetWorldStaticModelByIndex(index)`.
    pub fn try_get_world_static_model_by_index(&self, index: i32) -> Option<String> {
        let l = self.world_static_models_by_index.as_ref()?;
        usize::try_from(index).ok().and_then(|i| l.get(i)).cloned()
    }

    /// `getWorldStaticItem()` / `getWorldStaticModel()`: a
    /// `worldStaticModel` mod data string, the indexed instance model, or
    /// the script's.
    ///
    /// The original first returns `"Flatpack"` when the `Flatpack` mod data
    /// value is (by identity) the literal `"true"`, which only holds for the
    /// value `Moveable` sets at runtime, not for one read from a save. That
    /// runtime path is not ported, so the branch never applies here.
    pub fn get_world_static_model(&self) -> Option<String> {
        if let Some(v) = self.mod_data_string("worldStaticModel") {
            return v.map(str::to_string);
        }
        if let Some(m) = self.try_get_world_static_model_by_index(self.model_index) {
            return Some(m);
        }
        self.template.world_static_model.clone()
    }

    /// `getStaticModelException()`.
    pub fn get_static_model_exception(
        &self,
        model_script: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        if self.has_tag(TAG_USE_WORLD_STATIC_MODEL) {
            return self.get_world_static_model();
        }
        self.get_static_model(model_script)
    }

    /// The model index part of `synchWithVisual`: an item with model lists
    /// and no index picks one with `Rand.Next(size)` (`rand`), the weapon
    /// sprite list first, then the larger of the static model lists.
    pub fn pick_model_index(&mut self, rand: &mut dyn FnMut(i32) -> i32) {
        if self.model_index != -1 {
            return;
        }
        let len = |l: &Option<Rc<Vec<String>>>| l.as_ref().map(|l| l.len() as i32);
        let max = match (&self.kind, self.get_weapon_sprites_by_index()) {
            (ItemKind::Weapon(_), Some(l)) => Some(l.len() as i32),
            _ => match (
                len(&self.static_models_by_index),
                len(&self.world_static_models_by_index),
            ) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            },
        };
        if let Some(max) = max {
            self.model_index = if max <= 0 { 0 } else { rand(max) };
        }
    }

    /// `Food.isFertilized()` (false for other items).
    pub fn is_fertilized(&self) -> bool {
        matches!(&self.kind, ItemKind::Food(f) if f.fertilized)
    }

    /// `Food.setFertilized(b)` (other items ignore it).
    pub fn set_fertilized(&mut self, b: bool) {
        if let ItemKind::Food(f) = &mut self.kind {
            f.fertilized = b;
        }
    }

    /// `Food.isRotten()`: never while fertilized.
    pub fn is_rotten(&self) -> bool {
        match &self.kind {
            ItemKind::Food(f) => !f.fertilized && f.age >= f.off_age_max as f32,
            _ => false,
        }
    }

    /// `Food.isFresh()`: always while fertilized.
    pub fn is_fresh(&self) -> bool {
        match &self.kind {
            ItemKind::Food(f) => f.fertilized || f.age < f.off_age as f32,
            _ => false,
        }
    }

    /// Detached, read-only archive/presentation values. Runtime users obtain
    /// actual component references instead of retaining this snapshot.
    pub fn decode_components(&self, ctx: &dyn EntityContext) -> Option<Components> {
        self.components.values_snapshot(ctx).ok()
    }

    /// Explicit unowned factory/archive replacement. All displaced native
    /// slots are returned for destruction after enclosing guards end.
    pub fn set_components(
        &mut self,
        values: Components,
    ) -> Result<pz_entity::RetiredComponents, pz_entity::runtime_load::RuntimeHostError> {
        use pz_entity::runtime_load::RuntimeHostError;
        let replacement = crate::item_components::ItemComponents::from_values_unowned(values)
            .map_err(|rejected| RuntimeHostError::Storage(rejected.error))?;
        self.components
            .replace_unowned(replacement)
            .map_err(|rejected| rejected.into_error_and_retire())
    }

    /// The actual native allocation; absence and encoded/unavailable state
    /// remain distinct. No body is copied or decoded by this live getter.
    pub fn get_fluid_ref(
        &self,
    ) -> Result<Option<pz_entity::ComponentRef>, pz_entity::RuntimeError> {
        self.components.get_ref(ComponentType::FluidContainer)
    }

    /// The actual Attributes component allocation. An encoded body or a
    /// borrowed store is an error, rather than a missing attribute container.
    pub fn get_attributes_ref(
        &self,
    ) -> Result<Option<pz_entity::ComponentRef>, pz_entity::RuntimeError> {
        self.components.get_ref(ComponentType::Attributes)
    }

    /// Read the actual attribute instance. This propagates nested instance
    /// borrows and encoded storage instead of reporting a false absence.
    pub fn try_get_attribute_by_id(
        &self,
        id: i16,
    ) -> Result<Option<pz_entity::components::attributes::AttributeValue>, pz_entity::RuntimeError>
    {
        let Some(component) = self.get_attributes_ref()? else {
            return Ok(None);
        };
        let body =
            component
                .try_borrow()
                .map_err(|_| pz_entity::RuntimeError::BorrowedComponent {
                    index: None,
                    kind: ComponentType::Attributes,
                })?;
        body.attributes()
            .ok_or(pz_entity::RuntimeError::InvalidBody(
                ComponentType::Attributes,
            ))?
            .get_value(id)
    }

    /// Detached fluid data for existing pure value-presentation consumers.
    /// This is not the Lua/native getFluidContainer implementation.
    pub fn get_fluid_container(&self, ctx: &dyn EntityContext) -> Option<FluidContainer> {
        match self
            .components
            .value_snapshot(ComponentType::FluidContainer, ctx)
            .ok()??
        {
            Component::FluidContainer(fluid) => Some(fluid),
            _ => None,
        }
    }

    /// Detached attributes for value-presentation consumers.
    pub fn get_attributes(
        &self,
        ctx: &dyn EntityContext,
    ) -> Option<pz_entity::components::attributes::AttributeContainer> {
        match self
            .components
            .value_snapshot(ComponentType::Attributes, ctx)
            .ok()??
        {
            Component::Attributes(attributes) => Some(attributes),
            _ => None,
        }
    }

    pub fn get_attribute(
        &self,
        ctx: &dyn EntityContext,
        name: &str,
    ) -> Option<pz_entity::components::attributes::AttributeValue> {
        let id = ctx.attribute_type_by_name(name)?.id;
        self.get_attributes(ctx)?
            .attributes
            .into_iter()
            .find(|attribute| attribute.id == id)
            .map(|attribute| attribute.value)
    }

    /// Legacy data-setting boundary, not a native fluid mutator. Existing
    /// live allocation identity is retained. Adding to an owned entity must
    /// use ItemComponentHost so metadata/owner/lifecycle phases are real.
    pub fn set_fluid_container(
        &mut self,
        fluid: FluidContainer,
    ) -> Result<pz_entity::RetiredComponents, pz_entity::runtime_load::RuntimeHostError> {
        use pz_entity::runtime_load::RuntimeHostError;
        use pz_entity::{RuntimeComponent, RuntimeError};
        match self.get_fluid_ref() {
            Ok(Some(component)) => {
                let mut body = component
                    .try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("set fluid body"))?;
                *body
                    .fluid_mut()
                    .ok_or(RuntimeHostError::Storage(RuntimeError::InvalidBody(
                        ComponentType::FluidContainer,
                    )))? = fluid;
                body.fluid_mut()
                    .expect("immutable FluidContainer class")
                    .cache_invalidated = true;
                Ok(pz_entity::RetiredComponents::new())
            }
            Ok(None) | Err(RuntimeError::NeedsPromotion { .. }) => {
                if self.components.is_bound()
                    || self
                        .components
                        .has_live_owners()
                        .map_err(RuntimeHostError::Storage)?
                {
                    return Err(RuntimeHostError::Unsupported {
                        operation: "add fluid without item lifecycle host",
                        kind: Some(ComponentType::FluidContainer),
                    });
                }
                let component = RuntimeComponent::new_unowned(Component::FluidContainer(fluid))
                    .map_err(|rejected| RuntimeHostError::Storage(rejected.error))?;
                let _old_encoded = self
                    .components
                    .runtime_mut()
                    .remove_encoded_type_unordered(ComponentType::FluidContainer.id());
                self.components
                    .runtime_mut()
                    .push_ref_if_absent(component)
                    .map_err(|rejected| RuntimeHostError::Storage(rejected.error))?;
                Ok(pz_entity::RetiredComponents::new())
            }
            Err(error) => Err(RuntimeHostError::Storage(error)),
        }
    }
}

fn index(l: &[String], i: i32) -> Option<String> {
    usize::try_from(i).ok().and_then(|i| l.get(i)).cloned()
}

/// Component blobs back to `saveEntity` bytes (count byte and blocks).
pub(crate) fn blob_bytes(blobs: &[ComponentBlob]) -> Vec<u8> {
    let mut bytes = vec![blobs.len() as u8];
    for b in blobs {
        bytes.extend_from_slice(&(b.data.len() as i32 + 2).to_be_bytes());
        bytes.extend_from_slice(&b.type_id.to_be_bytes());
        bytes.extend_from_slice(&b.data);
    }
    bytes
}

/// Decodes component blobs (stored in the current format).
pub(crate) fn decode_blobs(blobs: &[ComponentBlob], ctx: &dyn EntityContext) -> Option<Components> {
    let bytes = blob_bytes(blobs);
    let (c, info) =
        pz_entity::entity::load_components(&bytes, COMPONENT_WORLD_VERSION, ctx).ok()?;
    (info.issues.is_empty() && info.consumed == bytes.len()).then_some(c)
}

/// Encodes components into blobs.
pub(crate) fn encode_blobs(c: &Components) -> Vec<ComponentBlob> {
    let bytes = pz_entity::entity::save_components(c);
    let mut out = Vec::new();
    let mut p = 1;
    while p < bytes.len() {
        let len = i32::from_be_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
        let block = &bytes[p + 4..p + 4 + len];
        out.push(ComponentBlob {
            type_id: i16::from_be_bytes([block[0], block[1]]),
            data: block[2..].to_vec(),
        });
        p += 4 + len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::ItemEnv;
    use crate::io::Writer;
    use crate::item::EggData;
    use crate::saved::SavedItem;
    use crate::template::{ItemClass, ItemTemplate};

    fn list(v: &[&str]) -> Option<Rc<Vec<String>>> {
        Some(Rc::new(v.iter().map(|s| s.to_string()).collect()))
    }

    struct Env(Rc<ItemTemplate>, std::cell::Cell<i32>);
    impl ItemEnv for Env {
        fn template(&self, _id: i16) -> Option<Rc<ItemTemplate>> {
            Some(self.0.clone())
        }
        fn rand_next(&self, bound: i32) -> i32 {
            let v = self.1.get();
            self.1.set(v + 1);
            v % bound.max(1)
        }
    }

    #[test]
    fn model_lists_and_getters() {
        let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "Mug");
        t.static_model = Some("Base.Mug".into());
        t.world_static_model = Some("Base.MugWorld".into());
        t.static_models_by_index = list(&["Base.MugRed", "Base.MugBlue"]);
        t.world_static_models_by_index = list(&[]);
        let t = Rc::new(t);
        let mut i = InventoryItem::create(t.clone(), 1);
        // An empty script list is not copied to the instance.
        assert!(i.get_world_static_models_by_index().is_none());
        assert_eq!(i.get_static_model(&|_| None).as_deref(), Some("Base.Mug"));
        i.pick_model_index(&mut |max| max - 1);
        assert_eq!(i.model_index, 1);
        assert_eq!(
            i.get_static_model(&|_| None).as_deref(),
            Some("Base.MugBlue")
        );
        assert_eq!(i.get_world_static_model().as_deref(), Some("Base.MugWorld"));
        // Mod data overrides both.
        i.table.get_or_insert_with(Default::default).rawset(
            LuaKey::Str("staticModel".into()),
            Some(LuaValue::Str("Base.Other".into())),
        );
        assert_eq!(i.get_static_model(&|_| None).as_deref(), Some("Base.Other"));
        // An item that already has an index keeps it.
        i.pick_model_index(&mut |_| panic!("no draw"));
    }

    #[test]
    fn weapon_sprites_and_parts() {
        let mut t = ItemTemplate::new(ItemClass::Weapon, "Base", "Gun");
        t.weapon_sprite = Some("Gun".into());
        t.weapon_sprites_by_index = list(&["GunA", "GunB"]);
        t.model_weapon_part = Some(Rc::new(vec![ModelWeaponPart {
            part_type: "Base.Scope".into(),
            model_name: "ScopeModel".into(),
            attachment_name_self: None,
            attachment_parent: Some("scope".into()),
        }]));
        let mut i = InventoryItem::create(Rc::new(t), 1);
        assert_eq!(i.get_weapon_sprite().as_deref(), Some("Gun"));
        assert_eq!(
            i.get_model_weapon_part().unwrap()[0].model_name,
            "ScopeModel"
        );
        i.model_index = 1;
        assert_eq!(i.get_weapon_sprite().as_deref(), Some("GunB"));
        assert_eq!(i.get_static_model(&|_| None).as_deref(), Some("GunB"));
        // Past the list's end (the original throws).
        i.model_index = 5;
        assert_eq!(i.get_static_model(&|_| None), None);
    }

    #[test]
    fn food_models_and_fertilized_eggs() {
        let mut t = ItemTemplate::new(ItemClass::Food, "Base", "Egg");
        t.static_model = Some("Base.Egg".into());
        let t = Rc::new(t);
        let mut i = InventoryItem::create(t.clone(), 1);
        let scripts =
            |n: &str| (n == "Base.EggRotten" || n == "Base.EggCooked").then(|| n.to_string());
        if let ItemKind::Food(f) = &mut i.kind {
            f.age = 1000.0;
            f.off_age_max = 10;
            f.cooked = true;
            f.egg = Some(EggData {
                time_to_hatch: 3,
                fertilized_time: 1,
                animal_hatch: "chick".into(),
                animal_hatch_breed: "leghorn".into(),
                genome: Vec::new(),
                mother_id: 7,
            });
        }
        // Egg data alone does not make it fertilized, and is not saved.
        assert!(!i.is_fertilized() && i.is_rotten());
        assert_eq!(
            i.get_static_model(&scripts).as_deref(),
            Some("Base.EggRotten")
        );
        let env = Env(t, std::cell::Cell::new(0));
        let mut w = Writer::new();
        i.save(&mut w, &env).unwrap();
        let SavedItem::Item(back) = SavedItem::decode(&w.buf, 249, &env) else {
            panic!("raw");
        };
        assert!(!back.is_fertilized());
        assert!(matches!(&back.kind, ItemKind::Food(f) if f.egg.is_none()));
        // Fertilized: never rotten, so the cooked model applies; a load sets it.
        i.set_fertilized(true);
        assert!(!i.is_rotten() && i.is_fresh());
        assert_eq!(
            i.get_static_model(&scripts).as_deref(),
            Some("Base.EggCooked")
        );
        let mut w = Writer::new();
        i.save(&mut w, &env).unwrap();
        let SavedItem::Item(back) = SavedItem::decode(&w.buf, 249, &env) else {
            panic!("raw");
        };
        assert!(back.is_fertilized());
        assert!(matches!(&back.kind, ItemKind::Food(f) if f.egg.as_ref().unwrap().mother_id == 7));
    }

    #[test]
    fn created_items_draw_in_order() {
        let t = Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock"));
        let env = Env(t, std::cell::Cell::new(0));
        // Outfit seed, then the id.
        let i = crate::env::create_item(&env, 1).unwrap();
        assert_eq!(i.id, 1 + 1233423);
        assert_eq!(env.1.get(), 2);
    }
}
