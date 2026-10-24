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
//!
//! With [`EntityCodec`] the blobs are always the current format and decode
//! through [`InventoryItem::decode_components`](crate::InventoryItem::decode_components) /
//! [`get_fluid_container`](crate::InventoryItem::get_fluid_container).

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

/// [`ComponentCodec`] backed by `pz-entity`: every component block is
/// decoded into its component (loading in place into the components the
/// item already has, as `loadEntity` does) and encoded back, so items whose
/// components do not decode cleanly stay raw records. Storage stays
/// [`ComponentBlob`]s, in the current format (decode them with
/// [`InventoryItem::decode_components`](crate::InventoryItem::decode_components)).
/// New items get their components from
/// `pz_entity::factory::create_from_script`.
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
