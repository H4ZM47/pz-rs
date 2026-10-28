//! ECS component blocks (`GameEntity.saveEntity`/`loadEntity`) behind a
//! codec seam.
//!
//! Actual shared items store canonical runtime component references in
//! [`crate::item_components::ItemComponents`]. [`ComponentCodec`] supplies
//! their native factory and context seams. [`ComponentBlob`] and the value
//! codec methods remain explicit unowned archive/legacy carriers; they do
//! not provide an editable mirror of a live component.

pub use pz_entity::factory::FactoryRng;

use crate::io::{err, Reader, SaveResult, Writer};
use crate::model::{decode_blobs, encode_blobs};
use crate::template::ItemTemplate;

/// The world version component blobs decoded by
/// [`EntityCodec`] are stored in (re-encoded in the current format,
/// `IsoWorld.getWorldVersion()` of this build).
pub const COMPONENT_WORLD_VERSION: i32 = 249;

/// One saved ECS component: its `ComponentType` id and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentBlob {
    pub type_id: i16,
    pub data: Vec<u8>,
}

/// Reads and writes an entity's component list.
pub trait ComponentCodec {
    /// Legacy unowned value/archive load. Actual shared receivers must use
    /// the native ItemComponentHost coordinator instead of this operation.
    fn load_archive_entity(
        &self,
        reader: &mut Reader<'_>,
        world_version: i32,
        components: &mut crate::item_components::ItemComponents,
    ) -> SaveResult<pz_entity::RetiredComponents> {
        components
            .load_encoded_archive(reader, world_version)
            .map(|(_, retired)| retired)
    }
    /// Actual-owner production factory. Unsupported codecs cannot pretend
    /// that their archive bytes are missing native components.
    fn factory_runtime(
        &self,
        _item: &crate::shared::ItemRef,
        template: &ItemTemplate,
        _rng: &mut dyn FactoryRng,
        _retired: &mut pz_entity::RetiredComponents,
        _native_notices: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
    ) -> pz_entity::runtime_load::HostResult<()> {
        if let Some(script) = template.script.as_ref() {
            let script = script.0.try_borrow().map_err(|_| {
                pz_entity::runtime_load::RuntimeHostError::Borrowed(
                    "component factory script definition",
                )
            })?;
            if !script.entity.has_components() {
                return Ok(());
            }
        } else {
            return Ok(());
        }
        Err(pz_entity::runtime_load::RuntimeHostError::Unsupported {
            operation: "actual item component factory",
            kind: None,
        })
    }
    /// `loadEntity`: reads the components into `components` (one already
    /// there of the same type loads in place, others are appended) and
    /// returns the type ids loaded.
    fn load_entity(
        &self,
        r: &mut Reader<'_>,
        _world_version: i32,
        components: &mut Vec<ComponentBlob>,
    ) -> SaveResult<Vec<i16>> {
        let n = r.i8()?;
        let mut loaded = Vec::new();
        for _ in 0..n.max(0) {
            let len = r.i32()?;
            if len < 2 {
                return err("DataBlock possible corruption.");
            }
            let block = r.bytes(len as usize)?;
            let c = ComponentBlob {
                type_id: i16::from_be_bytes([block[0], block[1]]),
                data: block[2..].to_vec(),
            };
            loaded.push(c.type_id);
            match components.iter_mut().find(|e| e.type_id == c.type_id) {
                Some(e) => *e = c,
                None => components.push(c),
            }
        }
        Ok(loaded)
    }

    /// `saveEntity`.
    fn save_entity(&self, w: &mut Writer, components: &[ComponentBlob]) -> SaveResult<()> {
        w.i8(components.len() as i8);
        for c in components {
            w.i32(c.data.len() as i32 + 2);
            w.i16(c.type_id);
            w.bytes(&c.data);
        }
        Ok(())
    }

    /// Components a newly created item of this script has, as saved
    /// (`GameEntityFactory.CreateInventoryItemEntity`), drawing from `rng`
    /// where the original calls `Rand`. The opaque codec has no component
    /// definitions and returns none; items whose script defines components
    /// then only round-trip when their file holds them (see the crate
    /// notes).
    fn factory_components(
        &self,
        _template: &ItemTemplate,
        _rng: &mut dyn FactoryRng,
    ) -> Vec<ComponentBlob> {
        Vec::new()
    }

    /// The decoded form of [`factory_components`](Self::factory_components)
    /// for codecs that decode (`None`: use the blobs).
    fn factory_entity(
        &self,
        _template: &ItemTemplate,
        _rng: &mut dyn FactoryRng,
    ) -> Option<pz_entity::Components> {
        None
    }

    /// The registries decoded components resolve against (`None` for a
    /// codec that keeps bytes).
    fn entity_context(&self) -> Option<&dyn pz_entity::EntityContext> {
        None
    }

    /// Whether the codec can build this script's components. Items whose
    /// script has components it cannot build are left as raw records.
    fn knows_factory_components(&self, _template: &ItemTemplate) -> bool {
        true
    }
}

/// [`ComponentCodec`] keeping components as bytes.
pub struct OpaqueComponents;
impl ComponentCodec for OpaqueComponents {}

/// [`ComponentCodec`] backed by `pz-entity`. Actual factories obtain pooled
/// runtime references and read the actual script into those allocations.
/// The unowned archive methods retain the existing value codec boundary;
/// actual shared loads use [`crate::component_host::load_item_ref`] instead.
pub struct EntityCodec<'a> {
    pub ctx: &'a dyn pz_entity::EntityContext,
}

impl ComponentCodec for EntityCodec<'_> {
    fn load_archive_entity(
        &self,
        reader: &mut Reader<'_>,
        world_version: i32,
        components: &mut crate::item_components::ItemComponents,
    ) -> SaveResult<pz_entity::RetiredComponents> {
        if components.is_bound() {
            return err("actual shared item requires the native component load coordinator");
        }
        let rest = &reader.buf[reader.pos..];
        let length = pz_entity::entity::entity_len(rest)
            .map_err(|error| crate::io::SaveError(format!("entity archive: {error}")))?;
        let mut values = components
            .values_snapshot(self.ctx)
            .map_err(|error| crate::io::SaveError(format!("entity archive: {error}")))?;
        let loaded = pz_entity::entity::load_components_into(
            &mut values,
            &rest[..length],
            world_version,
            self.ctx,
        )
        .map_err(|error| crate::io::SaveError(format!("entity archive: {error}")))?;
        if !loaded.issues.is_empty() || loaded.consumed != length {
            return err(format!("entity archive issues: {:?}", loaded.issues));
        }
        for index in (0..values.list.len()).rev() {
            if !loaded.loaded.contains(&values.list[index].kind()) {
                values.list.swap_remove(index);
            }
        }
        let replacement = crate::item_components::ItemComponents::from_values_unowned(values)
            .map_err(|rejected| {
                crate::io::SaveError(format!("entity archive: {}", rejected.error))
            })?;
        reader.bytes(length)?;
        components.replace_unowned(replacement).map_err(|rejected| {
            crate::io::SaveError(format!(
                "entity archive: {:?}",
                rejected.into_error_and_retire()
            ))
        })
    }
    fn factory_runtime(
        &self,
        item: &crate::shared::ItemRef,
        template: &ItemTemplate,
        rng: &mut dyn FactoryRng,
        retired: &mut pz_entity::RetiredComponents,
        native_notices: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
    ) -> pz_entity::runtime_load::HostResult<()> {
        use pz_entity::runtime_load::RuntimeHostError;
        use pz_entity::{ComponentType, EntityOwner};
        use pz_scripts::objects::entity::components::attributes::AttributesScript;
        use pz_scripts::objects::entity::components::combat::DurabilityScript;
        use pz_scripts::objects::entity::components::fluids::FluidContainerScript;
        let Some(script_ref) = template.script.as_ref() else {
            return Ok(());
        };
        let script = script_ref
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual factory item script"))?;
        if !script.entity.has_components() {
            return Ok(());
        }
        // CreateInventoryItemEntity protects createEntity only. The script
        // checks above and the InventoryItem constructor remain outside its
        // native Exception catch; Rust capability/borrow faults stay fatal.
        let created: pz_entity::runtime_load::HostResult<()> = (|| {
            if !item
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("actual component factory receiver"))?
                .components
                .is_empty()
            {
                return Err(RuntimeHostError::NativeException(pz_entity::EntityError(
                    "Calling CreateEntity on entity that already has components.".into(),
                )));
            }
            let mut host = crate::component_host::ItemComponentHost::new(item, self.ctx);
            for component_script in script.entity.component_scripts() {
                let kind = component_script.component_type();
                let component = pz_entity::runtime_pool::obtain_component(kind)
                    .map_err(crate::component_host::pool_error)?;
                retired.push_component(component.clone());
                match kind {
                    ComponentType::ContextMenuConfig => {
                        // This concrete class has no fields and inherits the
                        // exact return-only Component.readFromScript method.
                        // Entries remain on the owner's current script.
                    }
                    ComponentType::Durability => {
                        let durability_script = component_script
                            .as_any()
                            .downcast_ref::<DurabilityScript>()
                            .ok_or(RuntimeHostError::InvalidHostState(
                                "Durability script class mismatch",
                            ))?;
                        component
                            .try_borrow_mut()
                            .map_err(|_| {
                                RuntimeHostError::Borrowed("pooled durability script body")
                            })?
                            .read_durability_script(durability_script)
                            .map_err(crate::component_host::native_component_error)?;
                    }
                    ComponentType::Attributes => {
                        let attributes_script = component_script
                            .as_any()
                            .downcast_ref::<AttributesScript>()
                            .ok_or(RuntimeHostError::InvalidHostState(
                                "Attributes script class mismatch",
                            ))?;
                        // Native obtains this instance before a lazy template
                        // allocation. Template construction runs without an
                        // instance guard; its debug query occurs only on the
                        // native failed-parse branch.
                        let attributes_template =
                            pz_entity::runtime_attributes::get_or_create_attributes_template(
                                attributes_script,
                                self.ctx,
                                &crate::component_host::component_debug_enabled,
                                native_notices,
                                retired,
                            )?;
                        component
                            .try_borrow_mut()
                            .map_err(|_| {
                                RuntimeHostError::Borrowed("pooled attributes script body")
                            })?
                            .read_attributes_template(
                                attributes_template.as_ref(),
                                script.entity.name().unwrap_or("null"),
                                native_notices,
                                retired,
                            )
                            .map_err(crate::component_host::native_component_error)?;
                    }
                    ComponentType::FluidContainer => {
                        let fluid_script = component_script
                            .as_any()
                            .downcast_ref::<FluidContainerScript>()
                            .ok_or(RuntimeHostError::InvalidHostState(
                                "FluidContainer script class mismatch",
                            ))?;
                        component
                            .try_borrow_mut()
                            .map_err(|_| RuntimeHostError::Borrowed("pooled fluid script body"))?
                            .read_fluid_script(
                                fluid_script,
                                self.ctx,
                                rng,
                                script.entity.name().unwrap_or("null"),
                                native_notices,
                            )
                            .map_err(crate::component_host::native_component_error)?;
                    }
                    _ => {
                        return Err(RuntimeHostError::Unsupported {
                            operation: "native component readFromScript",
                            kind: Some(kind),
                        })
                    }
                }
                let _added = host.add(&component, retired)?;
            }
            let component = pz_entity::runtime_pool::obtain_component(ComponentType::Script)
                .map_err(crate::component_host::pool_error)?;
            retired.push_component(component.clone());
            let former_script = component
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("pooled entity script-info body"))?
                .set_original_script_actual(
                    true,
                    script.full_name(),
                    &EntityOwner::from_ref(&script_ref.0),
                )
                .map_err(RuntimeHostError::Storage)?;
            if let Some(former_script) = former_script {
                retired.push_owner(former_script);
            }
            let _added = host.add(&component, retired)?;
            host.connect(retired)?;
            // The following bool=true first-creation dispatch is authenticated
            // return-only for these exact supported classes. No extra state is
            // written by GameEntity's indexed dispatch or their inherited hook.
            Ok(())
        })();
        match created {
            Err(RuntimeHostError::NativeException(error)) => {
                native_notices
                    .push(pz_entity::runtime_fluid::RuntimeNativeNotice::ExceptionLogger(error.0));
                Ok(())
            }
            result => result,
        }
    }
    fn load_entity(
        &self,
        r: &mut Reader<'_>,
        world_version: i32,
        components: &mut Vec<ComponentBlob>,
    ) -> SaveResult<Vec<i16>> {
        let rest = &r.buf[r.pos..];
        let n = pz_entity::entity::entity_len(rest)
            .map_err(|e| crate::io::SaveError(format!("entity: {e:?}")))?;
        let Some(mut comps) = decode_blobs(components, self.ctx) else {
            return err("entity: existing components do not decode");
        };
        let info = pz_entity::entity::load_components_into(
            &mut comps,
            &rest[..n],
            world_version,
            self.ctx,
        )
        .map_err(|e| crate::io::SaveError(format!("entity: {e:?}")))?;
        if !info.issues.is_empty() || info.consumed != n {
            return err(format!("entity issues: {:?}", info.issues));
        }
        r.bytes(n)?;
        *components = encode_blobs(&comps);
        Ok(info.loaded.iter().map(|t| t.id()).collect())
    }

    fn entity_context(&self) -> Option<&dyn pz_entity::EntityContext> {
        Some(self.ctx)
    }

    fn factory_components(
        &self,
        template: &ItemTemplate,
        rng: &mut dyn FactoryRng,
    ) -> Vec<ComponentBlob> {
        self.factory_entity(template, rng)
            .map_or_else(Vec::new, |c| encode_blobs(&c))
    }

    fn factory_entity(
        &self,
        template: &ItemTemplate,
        rng: &mut dyn FactoryRng,
    ) -> Option<pz_entity::Components> {
        let script = template.script.as_ref()?;
        let script = script.0.borrow();
        // `CreateInventoryItemEntity`: nothing without component scripts.
        if !script.entity.has_components() {
            return None;
        }
        let dictionary = |_: pz_entity::ScriptRegister, _: &str| None;
        let mut input = pz_entity::factory::FactoryInput {
            ctx: self.ctx,
            rng,
            dictionary: &dictionary,
        };
        let comps = pz_entity::factory::create_from_script(
            &script.entity,
            &script.full_name(),
            pz_entity::factory::EntityKind::InventoryItem,
            &mut input,
        );
        Some(comps)
    }
}

/// [`FactoryRng`] drawing from an [`ItemEnv`](crate::ItemEnv)'s `Rand`.
pub struct EnvRng<'a>(pub &'a dyn crate::ItemEnv);

impl FactoryRng for EnvRng<'_> {
    fn next_int(&mut self, max: i32) -> i32 {
        self.0.rand_next(max)
    }

    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        self.0.rand_float(min, max)
    }
}

/// The world version [`ComponentBlob`] payloads are encoded at (saving
/// always writes the current format).
pub const BLOB_WORLD_VERSION: i32 = 249;

/// One component's saved form, as [`ComponentBlob`].
pub fn encode_component(c: pz_entity::Component) -> ComponentBlob {
    let mut comps = pz_entity::Components::new();
    comps.add(c);
    let bytes = pz_entity::entity::save_components(&comps);
    let mut blobs = Vec::new();
    OpaqueComponents
        .load_entity(&mut Reader::new(&bytes), BLOB_WORLD_VERSION, &mut blobs)
        .expect("saved component block");
    blobs.remove(0)
}
