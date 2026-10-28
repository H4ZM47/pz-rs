//! `zombie.entity.MetaEntity` and the meta entity file `GameEntityManager`
//! keeps in the save (`entity_data.bin`, `MetaEntitySystem`).

use crate::context::EntityContext;
use crate::entity::{load_components_into, write_components, Components, LoadInfo};
use crate::io::{Reader, Result, Writer};

/// An entity kept off-map so its components keep running
/// (`saveMetaEntity`/`loadMetaEntity`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MetaEntity {
    pub entity_net_id: i64,
    /// `originalEntityType.getByteId()` (`GameEntityType`).
    pub original_entity_type: i8,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Saved from world version 233 on.
    pub is_outside_cached: bool,
    pub components: Components,
}

impl MetaEntity {
    /// `loadMetaEntity` on a pooled (empty) meta entity.
    pub fn load(
        r: &mut Reader<'_>,
        bytes: &[u8],
        wv: i32,
        ctx: &dyn EntityContext,
    ) -> Result<(MetaEntity, LoadInfo)> {
        let mut m = MetaEntity {
            entity_net_id: r.i64()?,
            original_entity_type: r.i8()?,
            x: r.f32()?,
            y: r.f32()?,
            z: r.f32()?,
            ..Default::default()
        };
        if wv >= 233 {
            m.is_outside_cached = r.i8()? == 1;
        }
        let start = r.pos();
        let info = load_components_into(&mut m.components, &bytes[start..], wv, ctx)?;
        r.seek(start + info.consumed)?;
        Ok((m, info))
    }

    /// `saveMetaEntity`.
    pub fn save(&self, w: &mut Writer) {
        w.i64(self.entity_net_id);
        w.i8(self.original_entity_type);
        w.f32(self.x);
        w.f32(self.y);
        w.f32(self.z);
        w.bool(self.is_outside_cached);
        write_components(w, &self.components);
    }
}

/// `MetaEntitySystem.loadMetaEntities`: an int count, then the entities.
pub fn load_meta_entities(
    bytes: &[u8],
    wv: i32,
    ctx: &dyn EntityContext,
) -> Result<Vec<(MetaEntity, LoadInfo)>> {
    let mut r = Reader::new(bytes);
    let n = r.i32()?;
    let mut out = Vec::new();
    for _ in 0..n.max(0) {
        out.push(MetaEntity::load(&mut r, bytes, wv, ctx)?);
    }
    Ok(out)
}

/// `MetaEntitySystem.saveMetaEntities`.
pub fn save_meta_entities(entities: &[MetaEntity]) -> Vec<u8> {
    let mut w = Writer::new();
    w.i32(entities.len() as i32);
    for m in entities {
        m.save(&mut w);
    }
    w.buf
}
