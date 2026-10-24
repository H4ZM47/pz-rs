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
