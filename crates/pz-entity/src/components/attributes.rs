//! `zombie.entity.components.attributes.AttributeContainer`.

use crate::context::{AttributeKind, EntityContext};
use crate::io::{err, Reader, Result, Writer};

/// An attribute value, by `AttributeType` kind.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    Long(i64),
    Int(i32),
    Short(i16),
    Byte(i8),
    Double(f64),
    Float(f32),
    String(String),
    Bool(bool),
    /// `IOEnum.getByteId()` of the value.
    Enum(i8),
    EnumSet(Vec<i8>),
    EnumStringSet {
        enums: Vec<i8>,
        strings: Vec<String>,
    },
}

impl AttributeValue {
    fn load(kind: AttributeKind, r: &mut Reader<'_>) -> Result<AttributeValue> {
        use AttributeKind as K;
        Ok(match kind {
            K::Long => AttributeValue::Long(r.i64()?),
            K::Int => AttributeValue::Int(r.i32()?),
            K::Short => AttributeValue::Short(r.i16()?),
            K::Byte => AttributeValue::Byte(r.i8()?),
            K::Double => AttributeValue::Double(r.f64()?),
            K::Float => AttributeValue::Float(r.f32()?),
            K::String => AttributeValue::String(r.string()?),
            K::Bool => AttributeValue::Bool(r.bool()?),
            K::Enum => AttributeValue::Enum(r.i8()?),
            K::EnumSet => {
                let n = r.i8()?;
                let mut v = Vec::new();
                for _ in 0..n.max(0) {
                    v.push(r.i8()?);
                }
                AttributeValue::EnumSet(v)
            }
            K::EnumStringSet => {
                let n = r.i8()?;
                let mut enums = Vec::new();
                for _ in 0..n.max(0) {
                    enums.push(r.i8()?);
                }
                let n = r.i8()?;
                let mut strings = Vec::new();
                for _ in 0..n.max(0) {
                    strings.push(r.string()?);
                }
                AttributeValue::EnumStringSet { enums, strings }
            }
        })
    }

    fn save(&self, w: &mut Writer) {
        match self {
            AttributeValue::Long(v) => w.i64(*v),
            AttributeValue::Int(v) => w.i32(*v),
            AttributeValue::Short(v) => w.i16(*v),
            AttributeValue::Byte(v) => w.i8(*v),
            AttributeValue::Double(v) => w.f64(*v),
            AttributeValue::Float(v) => w.f32(*v),
            AttributeValue::String(v) => w.string(v),
            AttributeValue::Bool(v) => w.bool(*v),
            AttributeValue::Enum(v) => w.i8(*v),
            AttributeValue::EnumSet(v) => {
                w.i8(v.len() as i8);
                for b in v {
                    w.i8(*b);
                }
            }
            AttributeValue::EnumStringSet { enums, strings } => {
                w.i8(enums.len() as i8);
                for b in enums {
                    w.i8(*b);
                }
                w.i8(strings.len() as i8);
                for s in strings {
                    w.string(s);
                }
            }
        }
    }
}

/// One attribute: its `AttributeType` id and value.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub id: i16,
    pub value: AttributeValue,
}

/// `AttributeContainer`: attributes in insertion order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AttributeContainer {
    pub attributes: Vec<Attribute>,
}

impl AttributeContainer {
    /// `getOrAdd(type)` then set.
    pub fn set(&mut self, id: i16, value: AttributeValue) {
        match self.attributes.iter_mut().find(|a| a.id == id) {
            Some(a) => a.value = value,
            None => self.attributes.push(Attribute { id, value }),
        }
    }

    /// `load`. The bit-mask form lists values in id order, so attributes
    /// come back sorted by id from it.
    pub fn load(&mut self, r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<()> {
        self.attributes.clear();
        let mode = r.i8()?;
        if mode == 0 {
            return Ok(());
        }
        let read_one = |this: &mut Self, r: &mut Reader<'_>, id: i16| -> Result<()> {
            let Some(kind) = ctx.attribute_kind(id) else {
                return err("Unable to read attribute type.");
            };
            let v = AttributeValue::load(kind, r)?;
            this.set(id, v);
            Ok(())
        };
        if mode == 1 {
            let storages = r.i8()? as i16;
            if storages == 0 {
                return Ok(());
            }
            let headers_start = r.pos();
            let mut block = headers_start + storages.max(0) as usize * 8;
            for hi in 0..storages.max(0) {
                r.seek(headers_start + hi as usize * 8)?;
                let header = r.i64()?;
                r.seek(block)?;
                for bit in 0..64 {
                    if header & (1i64 << bit) != 0 {
                        read_one(self, r, hi * 64 + bit)?;
                    }
                }
                block = r.pos();
            }
        } else {
            let n = r.i16()?;
            for _ in 0..n.max(0) {
                let id = if mode == 8 { r.i8()? as i16 } else { r.i16()? };
                read_one(self, r, id)?;
            }
        }
        Ok(())
    }

    /// `save`: an id list, or a bit mask per 64 ids when that is smaller.
    pub fn save(&self, w: &mut Writer) {
        let Some(max_id) = self.attributes.iter().map(|a| a.id).max() else {
            w.i8(0);
            return;
        };
        let id_bits: i32 = if max_id > 127 { 16 } else { 8 };
        let storages = 1 + max_id as i32 / 64;
        let count = self.attributes.len() as i32;
        if count * id_bits + 16 > storages * 64 {
            w.i8(1);
            w.i8(storages as i8);
            let mut headers = vec![0i64; storages as usize];
            for a in &self.attributes {
                headers[a.id as usize / 64] |= 1i64 << (a.id % 64);
            }
            for h in headers {
                w.i64(h);
            }
            for a in &self.attributes {
                a.value.save(w);
            }
        } else {
            w.i8(id_bits as i8);
            w.i16(count as i16);
            for a in &self.attributes {
                if id_bits == 8 {
                    w.i8(a.id as i8);
                } else {
                    w.i16(a.id);
                }
                a.value.save(w);
            }
        }
    }
}
