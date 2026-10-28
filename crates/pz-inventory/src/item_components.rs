//! One authoritative component store for item bodies and archive values.
//!
//! `Clone` is the legacy synchronous archive contract: it creates independent
//! unowned component allocations and panics on a borrowed body. Production
//! archive entry points preflight the graph and use the fallible operation.
//! Runtime aliases are explicit `ComponentRef` clones, never store clones.

use pz_entity::{
    Component, ComponentRef, ComponentType, Components, EncodedComponent, EntityContext,
    RejectedEncoded, RejectedValues, RetiredComponents, RuntimeComponents, RuntimeEntryView,
    RuntimeError,
};

use crate::component::ComponentBlob;
use crate::io::{Reader, SaveError, SaveResult, Writer};

#[derive(Debug, Default)]
pub struct ItemComponents {
    runtime: RuntimeComponents,
    // Construction metadata only; no second component data or owner slot.
    // Explicit archive copies always start unbound, including empty stores.
    bound: bool,
}

#[derive(Debug)]
#[must_use]
pub struct RejectedItemComponents {
    pub error: pz_entity::runtime_load::RuntimeHostError,
    pub components: ItemComponents,
}

impl RejectedItemComponents {
    pub fn into_error_and_retire(mut self) -> pz_entity::runtime_load::RuntimeHostError {
        crate::component_host::retire_components(self.components.take_entries());
        self.error
    }
}

impl Clone for ItemComponents {
    fn clone(&self) -> Self {
        self.try_archive_copy_unowned()
            .expect("component archive copy requires unborrowed component bodies")
    }
}

impl PartialEq for ItemComponents {
    fn eq(&self, other: &Self) -> bool {
        self.try_data_eq(other)
            .expect("component data comparison requires unborrowed component bodies")
    }
}

impl ItemComponents {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn runtime(&self) -> &RuntimeComponents {
        &self.runtime
    }
    pub fn runtime_mut(&mut self) -> &mut RuntimeComponents {
        &mut self.runtime
    }
    pub fn is_bound(&self) -> bool {
        self.bound
    }
    pub fn mark_bound(&mut self) {
        self.bound = true;
    }
    pub fn len(&self) -> usize {
        self.runtime.len()
    }
    pub fn is_empty(&self) -> bool {
        self.runtime.is_empty()
    }
    pub fn entry_slots(&self) -> impl Iterator<Item = RuntimeEntryView<'_>> {
        self.runtime.entry_slots()
    }
    pub fn get_ref(&self, kind: ComponentType) -> Result<Option<ComponentRef>, RuntimeError> {
        self.runtime.get_ref(kind)
    }
    pub fn get_ref_at(&self, index: usize) -> Result<Option<ComponentRef>, RuntimeError> {
        self.runtime.get_ref_at(index)
    }
    pub fn take_entries(&mut self) -> RetiredComponents {
        self.runtime.take_entries()
    }

    pub fn from_values_unowned(values: Components) -> Result<Self, RejectedValues> {
        RuntimeComponents::adopt_values_unowned(values).map(|runtime| Self {
            runtime,
            bound: false,
        })
    }

    /// Explicit raw/archive import. This is not native loadEntity recovery.
    pub fn from_encoded_blobs(
        blobs: Vec<ComponentBlob>,
        world_version: i32,
    ) -> Result<Self, RejectedEncoded> {
        Self::from_encoded_entries(
            blobs
                .into_iter()
                .map(|blob| EncodedComponent::new(blob.type_id, blob.data, world_version))
                .collect(),
        )
    }

    pub fn from_encoded_entries(entries: Vec<EncodedComponent>) -> Result<Self, RejectedEncoded> {
        RuntimeComponents::from_encoded_unowned(entries).map(|runtime| Self {
            runtime,
            bound: false,
        })
    }

    pub fn push_encoded_blob(
        &mut self,
        blob: ComponentBlob,
        world_version: i32,
    ) -> Result<(), RejectedEncoded> {
        self.runtime.push_encoded_if_absent(EncodedComponent::new(
            blob.type_id,
            blob.data,
            world_version,
        ))
    }

    /// Only the explicit encoded carrier crosses the old mutable blob codec.
    /// A live body must never acquire a second editable byte authority here.
    pub fn try_encoded_blobs(&self) -> Result<Vec<ComponentBlob>, RuntimeError> {
        self.entry_slots()
            .map(|entry| match entry {
                RuntimeEntryView::Encoded(encoded) => Ok(ComponentBlob {
                    type_id: encoded.type_id(),
                    data: encoded.payload().to_vec(),
                }),
                RuntimeEntryView::Live { kind, .. } => Err(RuntimeError::UnsupportedLive(kind)),
            })
            .collect()
    }

    pub fn try_archive_copy_unowned(&self) -> Result<Self, RuntimeError> {
        self.runtime.try_archive_copy_unowned().map(|runtime| Self {
            runtime,
            bound: false,
        })
    }

    pub fn try_data_eq(&self, other: &Self) -> Result<bool, RuntimeError> {
        self.runtime.try_data_eq(&other.runtime)
    }

    pub fn has_live_owners(&self) -> Result<bool, RuntimeError> {
        for (index, entry) in self.entry_slots().enumerate() {
            if let RuntimeEntryView::Live { kind, component } = entry {
                let body = component
                    .try_borrow()
                    .map_err(|_| RuntimeError::BorrowedComponent {
                        index: Some(index),
                        kind,
                    })?;
                if body.owner_slot().is_some() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Replace an explicitly unowned archive/factory store, returning all
    /// former owning slots. Live entities must use their native host phases.
    pub fn replace_unowned(
        &mut self,
        replacement: Self,
    ) -> Result<RetiredComponents, RejectedItemComponents> {
        use pz_entity::runtime_load::RuntimeHostError;
        let valid = (|| {
            if self.bound
                || replacement.bound
                || self.has_live_owners().map_err(RuntimeHostError::Storage)?
                || replacement
                    .has_live_owners()
                    .map_err(RuntimeHostError::Storage)?
            {
                return Err(RuntimeHostError::Unsupported {
                    operation: "replace owned components without lifecycle host",
                    kind: None,
                });
            }
            Ok(())
        })();
        if let Err(error) = valid {
            return Err(RejectedItemComponents {
                error,
                components: replacement,
            });
        }
        let retired = self.take_entries();
        self.runtime = replacement.runtime;
        Ok(retired)
    }

    pub fn value_snapshot(
        &self,
        kind: ComponentType,
        ctx: &dyn EntityContext,
    ) -> Result<Option<Component>, RuntimeError> {
        for (index, entry) in self.entry_slots().enumerate() {
            match entry {
                RuntimeEntryView::Live {
                    kind: current,
                    component,
                } if current == kind => {
                    let body =
                        component
                            .try_borrow()
                            .map_err(|_| RuntimeError::BorrowedComponent {
                                index: Some(index),
                                kind,
                            })?;
                    return body.value_snapshot().map(Some);
                }
                RuntimeEntryView::Encoded(encoded) if encoded.type_id() == kind.id() => {
                    let mut body = Component::create(kind)
                        .ok_or(RuntimeError::UnknownEncodedType(encoded.type_id()))?;
                    let mut reader = pz_entity::io::Reader::new(encoded.payload());
                    body.load(&mut reader, encoded.world_version(), ctx)?;
                    if reader.remaining() != 0 {
                        return Err(RuntimeError::UnreadEncodedBytes {
                            kind,
                            remaining: reader.remaining(),
                        });
                    }
                    return Ok(Some(body));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Read-only DTO presentation, never a live getter or mutable authority.
    /// Raw entries are decoded only into this detached return value.
    pub fn values_snapshot(&self, ctx: &dyn EntityContext) -> Result<Components, RuntimeError> {
        let mut values = Components::new();
        for (index, entry) in self.entry_slots().enumerate() {
            let body = match entry {
                RuntimeEntryView::Live { kind, component } => component
                    .try_borrow()
                    .map_err(|_| RuntimeError::BorrowedComponent {
                        index: Some(index),
                        kind,
                    })?
                    .value_snapshot()?,
                RuntimeEntryView::Encoded(encoded) => {
                    let kind = ComponentType::from_id(encoded.type_id());
                    let mut body = Component::create(kind)
                        .ok_or(RuntimeError::UnknownEncodedType(encoded.type_id()))?;
                    let mut reader = pz_entity::io::Reader::new(encoded.payload());
                    body.load(&mut reader, encoded.world_version(), ctx)?;
                    if reader.remaining() != 0 {
                        return Err(RuntimeError::UnreadEncodedBytes {
                            kind,
                            remaining: reader.remaining(),
                        });
                    }
                    body
                }
            };
            values.list.push(body);
        }
        Ok(values)
    }

    /// Writes current live bodies directly. Transfer the completed writer
    /// prefix even on failure; a save does not mutate or reload components.
    pub fn write_to(&self, writer: &mut Writer, world_version: i32) -> SaveResult<()> {
        self.write_to_runtime(writer, world_version)
            .map_err(|error| SaveError(format!("entity: {error}")))
    }

    /// Actual save carrier keeps host/component faults typed. The entity
    /// codec's completed byte prefix is transferred even when it fails.
    pub(crate) fn write_to_runtime(
        &self,
        writer: &mut Writer,
        world_version: i32,
    ) -> Result<(), RuntimeError> {
        let mut entity_writer = pz_entity::io::Writer::new();
        let result = self.runtime.write_to(&mut entity_writer, world_version);
        writer.bytes(&entity_writer.buf);
        result
    }

    /// Explicit legacy opaque archive load. Bytes remain the sole encoded
    /// authority. Production native loading uses ItemComponentHost instead.
    pub fn load_encoded_archive(
        &mut self,
        reader: &mut Reader<'_>,
        world_version: i32,
    ) -> SaveResult<(Vec<i16>, RetiredComponents)> {
        if self.bound {
            return Err(SaveError(
                "bound components require the native item load host".into(),
            ));
        }
        let mut blobs = self
            .try_encoded_blobs()
            .map_err(|error| SaveError(format!("entity archive: {error}")))?;
        let loaded = crate::component::ComponentCodec::load_entity(
            &crate::component::OpaqueComponents,
            reader,
            world_version,
            &mut blobs,
        )?;
        for index in (0..blobs.len()).rev() {
            if !loaded.contains(&blobs[index].type_id) {
                blobs.swap_remove(index);
            }
        }
        let replacement = Self::from_encoded_blobs(blobs, world_version)
            .map_err(|error| SaveError(format!("entity archive: {}", error.error)))?;
        // This method is intentionally restricted to an unowned value body.
        for entry in self.entry_slots() {
            if let RuntimeEntryView::Live { kind, component } = entry {
                let body = component.try_borrow().map_err(|_| {
                    SaveError(format!("entity archive: {} is borrowed", kind.name()))
                })?;
                if body.owner_slot().is_some() {
                    return Err(SaveError(
                        "owned components require the native item load host".into(),
                    ));
                }
            }
        }
        let retired = self.take_entries();
        self.runtime = replacement.runtime;
        Ok((loaded, retired))
    }
}
