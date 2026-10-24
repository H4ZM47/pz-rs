//! `zombie.entity.components.resources.Resources` and its `Resource`
//! subclasses (`ResourceItem`, `ResourceFluid`, `ResourceEnergy`).

use pz_scripts::objects::entity::components::resources::{ResourceChannel, ResourceType};

use crate::context::EntityContext;
use crate::fluids::FluidContainer;
use crate::io::{err, read_header, HeaderSize, HeaderWrite, Reader, Result, Writer};

/// One `CompressIdenticalItems` entry: an item saved once (`saveWithSize`,
/// kept as bytes) standing for `identical` items, with the ids of the
/// copies after the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRecord {
    pub identical: i32,
    /// The item's data after its `int` length (`InventoryItem.save`).
    pub data: Vec<u8>,
    pub extra_ids: Vec<i32>,
}

/// `CompressIdenticalItems.save`/`load` of an item list.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ItemList {
    pub entries: Vec<ItemRecord>,
}

impl ItemList {
    pub fn load(r: &mut Reader<'_>) -> Result<ItemList> {
        let n = r.i16()?;
        let mut entries = Vec::new();
        for _ in 0..n.max(0) {
            let identical = r.i32()?;
            let len = r.i32()?;
            if len <= 0 {
                return err(format!(
                    "InventoryItem.loadItem() invalid item data length: {len}"
                ));
            }
            let data = r.bytes(len as usize)?.to_vec();
            let mut extra_ids = Vec::new();
            for _ in 1..identical.max(1) {
                extra_ids.push(r.i32()?);
            }
            entries.push(ItemRecord {
                identical,
                data,
                extra_ids,
            });
        }
        Ok(ItemList { entries })
    }

    pub fn save(&self, w: &mut Writer) {
        w.i16(self.entries.len() as i16);
        for e in &self.entries {
            w.i32(e.identical);
            w.i32(e.data.len() as i32);
            w.bytes(&e.data);
            for id in &e.extra_ids {
                w.i32(*id);
            }
        }
    }
}

/// `Energy.saveEnergy`'s reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnergyRef {
    Type(i8),
    Modded(String),
}

/// The subclass part of a resource.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceBody {
    Item {
        capacity: f32,
        stack_any_item: bool,
        stored_items: ItemList,
    },
    Fluid(Box<FluidContainer>),
    Energy {
        energy: Option<EnergyRef>,
        capacity: f32,
        stored_energy: f32,
    },
}

/// `Resource`'s saved state.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub id: String,
    /// `resourceType.getId()`.
    pub resource_type: i8,
    /// `resourceIo.getId()`.
    pub resource_io: i8,
    pub is_locked: bool,
    pub progress: f64,
    /// `channel.getByteId()`.
    pub channel: i8,
    /// `flags` (`EnumBitStore<ResourceFlag>` bits).
    pub flags: i32,
    pub body: ResourceBody,
}

impl Resource {
    /// `ResourceFactory.createBlancResource` + `load`; the type byte the
    /// group writes picks the subclass (unknown types get a `ResourceItem`).
    fn load(
        r: &mut Reader<'_>,
        outer_type: i8,
        wv: i32,
        ctx: &dyn EntityContext,
    ) -> Result<Resource> {
        let h = read_header(r, HeaderSize::Byte)?;
        let id = r.string()?;
        let resource_type = r.i8()?;
        let resource_io = r.i8()?;
        let is_locked = h & 1 != 0;
        let progress = if h & 2 != 0 { r.f64()? } else { 0.0 };
        let channel = if h & 4 != 0 {
            r.i8()?
        } else {
            ResourceChannel::NO_CHANNEL.byte_id()
        };
        let flags = if h & 8 != 0 { r.i32()? } else { 0 };
        let body = if outer_type == ResourceType::Fluid.id() {
            let mut fc = FluidContainer::default();
            fc.load(r, ctx)?;
            ResourceBody::Fluid(Box::new(fc))
        } else if outer_type == ResourceType::Energy.id() {
            let energy = if r.bool()? {
                Some(if r.bool()? {
                    EnergyRef::Modded(r.string()?)
                } else {
                    EnergyRef::Type(r.i8()?)
                })
            } else {
                None
            };
            ResourceBody::Energy {
                energy,
                capacity: r.f32()?,
                stored_energy: r.f32()?,
            }
        } else {
            let capacity = r.f32()?;
            let stack_any_item = if wv >= 238 { r.bool()? } else { false };
            ResourceBody::Item {
                capacity,
                stack_any_item,
                stored_items: ItemList::load(r)?,
            }
        };
        Ok(Resource {
            id,
            resource_type,
            resource_io,
            is_locked,
            progress,
            channel,
            flags,
            body,
        })
    }

    fn save(&self, w: &mut Writer) {
        let mut h = HeaderWrite::start(w, HeaderSize::Byte);
        w.string(&self.id);
        w.i8(self.resource_type);
        w.i8(self.resource_io);
        if self.is_locked {
            h.add(1);
        }
        if self.progress > 0.0 {
            h.add(2);
            w.f64(self.progress);
        }
        if self.channel != ResourceChannel::NO_CHANNEL.byte_id() {
            h.add(4);
            w.i8(self.channel);
        }
        if self.flags != 0 {
            h.add(8);
            w.i32(self.flags);
        }
        h.write(w);
        match &self.body {
            ResourceBody::Item {
                capacity,
                stack_any_item,
                stored_items,
            } => {
                w.f32(*capacity);
                w.bool(*stack_any_item);
                stored_items.save(w);
            }
            ResourceBody::Fluid(fc) => fc.save(w),
            ResourceBody::Energy {
                energy,
                capacity,
                stored_energy,
            } => {
                w.bool(energy.is_some());
                match energy {
                    Some(EnergyRef::Modded(s)) => {
                        w.bool(true);
                        w.string(s);
                    }
                    Some(EnergyRef::Type(id)) => {
                        w.bool(false);
                        w.i8(*id);
                    }
                    None => {}
                }
                w.f32(*capacity);
                w.f32(*stored_energy);
            }
        }
    }
}

/// A named `ResourceGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceGroup {
    pub name: String,
    pub resources: Vec<Resource>,
}

/// `zombie.entity.components.resources.Resources`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Resources {
    /// `namedGroups`, in creation order.
    pub groups: Vec<ResourceGroup>,
}

impl Resources {
    /// Groups of the same name merge; a resource whose id is already
    /// present is dropped (`addResourceInternal`).
    pub fn load(&mut self, r: &mut Reader<'_>, wv: i32, ctx: &dyn EntityContext) -> Result<()> {
        self.groups.clear();
        let n = r.i32()?;
        for _ in 0..n.max(0) {
            let name = r.string()?;
            let count = r.i32()?;
            for _ in 0..count.max(0) {
                let outer = r.i8()?;
                let res = Resource::load(r, outer, wv, ctx)?;
                self.add(&name, res);
            }
        }
        Ok(())
    }

    fn add(&mut self, group: &str, res: Resource) {
        if self
            .groups
            .iter()
            .any(|g| g.resources.iter().any(|x| x.id == res.id))
        {
            return;
        }
        match self.groups.iter_mut().find(|g| g.name == group) {
            Some(g) => g.resources.push(res),
            None => self.groups.push(ResourceGroup {
                name: group.to_string(),
                resources: vec![res],
            }),
        }
    }

    pub fn save(&self, w: &mut Writer) {
        w.i32(self.groups.len() as i32);
        for g in &self.groups {
            w.string(&g.name);
            w.i32(g.resources.len() as i32);
            for res in &g.resources {
                w.i8(res.resource_type);
                res.save(w);
            }
        }
    }
}
