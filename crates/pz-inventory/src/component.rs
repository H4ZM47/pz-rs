//! ECS component blocks (`GameEntity.saveEntity`/`loadEntity`) behind a
//! codec seam.
//!
//! The components themselves (`FluidContainer`, `Durability`, ...) are
//! ported in `pz-entity`. Until that lands, items keep each component as
//! its type id and payload bytes ([`ComponentBlob`]), which round-trips
//! exactly because every component is framed by a `ByteBlock` length.
//! [`ComponentCodec`] is the swap point: an implementation backed by
//! `pz-entity` decodes the payload into real components on `load_entity`
//! and encodes them back on `save_entity`, and supplies the components
//! `GameEntityFactory.CreateInventoryItemEntity` gives a new item
//! ([`ComponentCodec::factory_components`]).

use crate::io::{err, Reader, SaveResult, Writer};
use crate::template::ItemTemplate;

/// One saved ECS component: its `ComponentType` id and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentBlob {
    pub type_id: i16,
    pub data: Vec<u8>,
}

/// Reads and writes an entity's component list.
pub trait ComponentCodec {
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

    /// Components a newly created item of this script has, as saved. The
    /// opaque codec has no component definitions and returns none; items
    /// whose script defines components then only round-trip when their
    /// file holds them (see the crate notes).
    fn factory_components(&self, _template: &ItemTemplate) -> Vec<ComponentBlob> {
        Vec::new()
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

/// [`ComponentCodec`] backed by `pz-entity`: every component block is
/// decoded into its component and encoded back, so items whose components
/// do not decode cleanly stay raw records. Storage stays [`ComponentBlob`]s
/// (re-encoded bytes); components the entity already has load in place.
/// `factory_components` still adds none: building them needs
/// `pz_entity::factory::create_from_script` with the game's `Rand` and
/// scripts dictionary (not wired yet).
pub struct EntityCodec<'a> {
    pub ctx: &'a dyn pz_entity::EntityContext,
}

impl ComponentCodec for EntityCodec<'_> {
    fn load_entity(
        &self,
        r: &mut Reader<'_>,
        world_version: i32,
        components: &mut Vec<ComponentBlob>,
    ) -> SaveResult<Vec<i16>> {
        let e = |e: pz_entity::EntityError| crate::io::SaveError(format!("entity: {e:?}"));
        let rest = &r.buf[r.pos..];
        let n = pz_entity::entity::entity_len(rest).map_err(e)?;
        // Components the entity already has (its constructor's) load in
        // place, keeping what their `load` does not reset.
        let mut comps = if components.is_empty() {
            pz_entity::Components::new()
        } else {
            let mut w = Writer::new();
            OpaqueComponents.save_entity(&mut w, components)?;
            let (c, info) =
                pz_entity::entity::load_components(&w.buf, BLOB_WORLD_VERSION, self.ctx)
                    .map_err(e)?;
            if !info.issues.is_empty() {
                return err(format!("entity issues: {:?}", info.issues));
            }
            c
        };
        let info = pz_entity::entity::load_components_into(
            &mut comps,
            &rest[..n],
            world_version,
            self.ctx,
        )
        .map_err(e)?;
        if !info.issues.is_empty() || info.consumed != n {
            return err(format!("entity issues: {:?}", info.issues));
        }
        r.bytes(n)?;
        // Re-encode and split into blocks.
        let bytes = pz_entity::entity::save_components(&comps);
        let mut blobs = Vec::new();
        OpaqueComponents.load_entity(&mut Reader::new(&bytes), BLOB_WORLD_VERSION, &mut blobs)?;
        *components = blobs;
        Ok(info.loaded.iter().map(|t| t.id()).collect())
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
