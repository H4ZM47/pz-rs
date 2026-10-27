//! Physical ownership of designation class lists, zone fields and full food
//! objects. Worklist clones are temporary registration pins, never trace edges.
use super::*;
use crate::bind_designation::LuaDesignationZone;
use pz_world::designation::{
    DesignationList, DesignationListRef, DesignationObserved, DesignationPoolRef,
    DesignationPositionPool, DesignationRegistry, DesignationRegistryRef, DesignationZone,
    DesignationZoneRef,
};

fn trace_erased(owner: &pz_entity::EntityOwner, visitor: &mut TraceVisitor) {
    if owner.is::<IsoObject>() {
        visitor.owner_erased::<IsoObject>(owner.erased_slot());
    } else if owner.is::<InventoryItem>() {
        visitor.owner_erased::<InventoryItem>(owner.erased_slot());
    } else if owner.is::<RuntimeComponent>() {
        visitor.owner_erased::<RuntimeComponent>(owner.erased_slot());
    } else if owner.is::<DesignationZone>() {
        visitor.owner_erased::<DesignationZone>(owner.erased_slot());
    } else if owner.is::<DesignationRegistry>() {
        visitor.owner_erased::<DesignationRegistry>(owner.erased_slot());
    } else if owner.is::<DesignationList>() {
        visitor.owner_erased::<DesignationList>(owner.erased_slot());
    } else if owner.is::<DesignationPositionPool>() {
        visitor.owner_erased::<DesignationPositionPool>(owner.erased_slot());
    }
    // Unported actor/position classes retain conservative ownership. A copied
    // actor DTO or a guessed transitive body would not describe this Rc slot.
}
fn enqueue_erased(
    owner: &pz_entity::EntityOwner,
    work: &mut VecDeque<Node>,
    status: &mut MetadataStatus,
) -> Result<(), RegistrationFault> {
    if let Some(value) = owner.try_handle::<DesignationZone>() {
        work.push_back(Node::Zone(value));
    } else if let Some(value) = owner.try_handle::<DesignationRegistry>() {
        work.push_back(Node::Registry(value));
    } else if let Some(value) = owner.try_handle::<DesignationList>() {
        work.push_back(Node::List(value));
    } else if let Some(value) = owner.try_handle::<DesignationPositionPool>() {
        work.push_back(Node::Pool(value));
    } else if let Some(value) = owner.try_handle::<IsoObject>() {
        *status = merged(*status, super::register_object(&value)?);
    } else if let Some(value) = owner.try_handle::<InventoryItem>() {
        *status = merged(*status, super::register_item(&value)?);
    } else if let Some(value) = owner.try_handle::<RuntimeComponent>() {
        *status = merged(*status, super::register_component(&value)?);
    }
    Ok(())
}
pub struct DesignationRegistryDescriptor;
impl TraceDescriptor<DesignationRegistry> for DesignationRegistryDescriptor {
    fn trace(value: &DesignationRegistry, visitor: &mut TraceVisitor) {
        if let Some(list) = value.all_zones_slot() {
            visitor.owner(list);
        }
        if let Some(list) = value.animal_zones_slot() {
            visitor.owner(list);
        }
    }
    fn detach(value: &mut DesignationRegistry) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(value.take_for_gc()))
    }
}
pub struct DesignationListDescriptor;
impl TraceDescriptor<DesignationList> for DesignationListDescriptor {
    fn trace(value: &DesignationList, visitor: &mut TraceVisitor) {
        match value {
            DesignationList::Zones(zones) => {
                for zone in zones {
                    visitor.owner(zone);
                }
            }
            DesignationList::WorldObjects(objects) => {
                for object in objects {
                    visitor.owner(object);
                }
            }
            DesignationList::Squares(squares) => {
                for square in squares {
                    visitor.owner(square);
                }
            }
            DesignationList::Owners(owners) | DesignationList::RoofAreas(owners) => {
                for owner in owners {
                    trace_erased(owner, visitor);
                }
            }
        }
    }
    fn detach(value: &mut DesignationList) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(value.take_for_gc()))
    }
}
pub struct DesignationZoneDescriptor;
impl TraceDescriptor<DesignationZone> for DesignationZoneDescriptor {
    fn trace(value: &DesignationZone, visitor: &mut TraceVisitor) {
        for list in [
            &value.animals,
            &value.corpses,
            &value.troughs,
            &value.hutchs,
            &value.food_on_ground,
            &value.near_water_squares,
            &value.roof_areas,
        ]
        .into_iter()
        .flatten()
        {
            visitor.owner(list);
        }
        if let Some(pool) = &value.position3d_pool {
            visitor.owner(pool);
        }
    }
    fn detach(value: &mut DesignationZone) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(value.take_owner_slots_for_gc()))
    }
}
pub struct DesignationPoolDescriptor;
impl TraceDescriptor<DesignationPositionPool> for DesignationPoolDescriptor {
    fn trace(value: &DesignationPositionPool, visitor: &mut TraceVisitor) {
        for owner in &value.released_positions {
            trace_erased(owner, visitor);
        }
    }
    fn detach(value: &mut DesignationPositionPool) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(std::mem::take(&mut value.released_positions)))
    }
}
pub struct DesignationUserdataDescriptor;
impl UserdataDescriptor<LuaDesignationZone> for DesignationUserdataDescriptor {
    fn trace(
        value: &LuaDesignationZone,
        visitor: &mut TraceVisitor,
    ) -> Result<(), gc::TraceDeferred> {
        visitor.owner(&value.zone);
        Ok(())
    }
}

enum Node {
    Registry(DesignationRegistryRef),
    List(DesignationListRef),
    Zone(DesignationZoneRef),
    Pool(DesignationPoolRef),
}
fn register_graph(root: Node) -> Result<MetadataStatus, RegistrationFault> {
    let mut work = VecDeque::from([root]);
    let mut visited = HashSet::new();
    let mut status = MetadataStatus::Complete;
    while let Some(node) = work.pop_front() {
        match node {
            Node::Registry(value) => {
                if !visited.insert(("designation registry", Rc::as_ptr(&value) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, DesignationRegistryDescriptor>(&value),
                        Pending::DesignationRegistry(Rc::downgrade(&value)),
                    )?,
                );
                let Ok(body) = value.try_borrow() else {
                    pending(Pending::DesignationRegistry(Rc::downgrade(&value)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for list in body
                    .all_zones_slot()
                    .into_iter()
                    .chain(body.animal_zones_slot())
                {
                    work.push_back(Node::List(list.clone()));
                }
            }
            Node::List(value) => {
                if !visited.insert(("designation list", Rc::as_ptr(&value) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, DesignationListDescriptor>(&value),
                        Pending::DesignationList(Rc::downgrade(&value)),
                    )?,
                );
                let Ok(body) = value.try_borrow() else {
                    pending(Pending::DesignationList(Rc::downgrade(&value)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                match &*body {
                    DesignationList::Zones(zones) => {
                        for zone in zones {
                            work.push_back(Node::Zone(zone.clone()));
                        }
                    }
                    DesignationList::WorldObjects(objects) => {
                        for object in objects {
                            status = merged(status, super::register_object(object)?);
                        }
                    }
                    DesignationList::Squares(squares) => {
                        for square in squares {
                            status = merged(
                                status,
                                super::register_owner_graph(OwnerNode::Square(square.clone()))?,
                            );
                        }
                    }
                    DesignationList::Owners(owners) | DesignationList::RoofAreas(owners) => {
                        for owner in owners {
                            enqueue_erased(owner, &mut work, &mut status)?;
                        }
                    }
                }
            }
            Node::Zone(value) => {
                if !visited.insert(("designation zone", Rc::as_ptr(&value) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, DesignationZoneDescriptor>(&value),
                        Pending::DesignationZone(Rc::downgrade(&value)),
                    )?,
                );
                let Ok(body) = value.try_borrow() else {
                    pending(Pending::DesignationZone(Rc::downgrade(&value)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                if let Err(error) = crate::bind_designation::remember_kind(&value, body.kind()) {
                    if matches!(
                        error,
                        pz_kahlua::LuaError::Runtime {
                            class: "pz.host.Borrowed",
                            ..
                        }
                    ) {
                        pending(Pending::DesignationZone(Rc::downgrade(&value)));
                        status = merged(status, MetadataStatus::Deferred);
                    } else {
                        status = MetadataStatus::InvalidClass;
                    }
                }
                for list in [
                    &body.animals,
                    &body.corpses,
                    &body.troughs,
                    &body.hutchs,
                    &body.food_on_ground,
                    &body.near_water_squares,
                    &body.roof_areas,
                ]
                .into_iter()
                .flatten()
                {
                    work.push_back(Node::List(list.clone()));
                }
                if let Some(pool) = &body.position3d_pool {
                    work.push_back(Node::Pool(pool.clone()));
                }
            }
            Node::Pool(value) => {
                if !visited.insert(("designation pool", Rc::as_ptr(&value) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, DesignationPoolDescriptor>(&value),
                        Pending::DesignationPool(Rc::downgrade(&value)),
                    )?,
                );
                let Ok(body) = value.try_borrow() else {
                    pending(Pending::DesignationPool(Rc::downgrade(&value)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for owner in &body.released_positions {
                    enqueue_erased(owner, &mut work, &mut status)?;
                }
            }
        }
    }
    Ok(status)
}
pub fn register_designation_registry(
    value: &DesignationRegistryRef,
) -> Result<MetadataStatus, RegistrationFault> {
    register_graph(Node::Registry(value.clone()))
}
pub fn register_designation_list(
    value: &DesignationListRef,
) -> Result<MetadataStatus, RegistrationFault> {
    register_graph(Node::List(value.clone()))
}
pub fn register_designation_zone(
    value: &DesignationZoneRef,
) -> Result<MetadataStatus, RegistrationFault> {
    register_graph(Node::Zone(value.clone()))
}
pub fn register_designation_pool(
    value: &DesignationPoolRef,
) -> Result<MetadataStatus, RegistrationFault> {
    register_graph(Node::Pool(value.clone()))
}
pub fn register_designation_userdata(
    value: &Rc<LuaDesignationZone>,
) -> Result<MetadataStatus, RegistrationFault> {
    let status = registered(
        gc::register_userdata::<_, DesignationUserdataDescriptor>(value),
        Pending::DesignationUserdata(Rc::downgrade(value)),
    )?;
    Ok(merged(status, register_designation_zone(&value.zone)?))
}
pub fn observe_designation(value: DesignationObserved<'_>) {
    match value {
        DesignationObserved::Registry(value) => {
            let _ = register_designation_registry(value);
        }
        DesignationObserved::List(value) => {
            let _ = register_designation_list(value);
        }
        DesignationObserved::Zone(value, kind) => {
            if matches!(
                crate::bind_designation::remember_kind(value, kind),
                Err(pz_kahlua::LuaError::Runtime {
                    class: "pz.host.Borrowed",
                    ..
                })
            ) {
                pending(Pending::DesignationZone(Rc::downgrade(value)));
            }
            let _ = register_designation_zone(value);
        }
        DesignationObserved::Pool(value) => {
            let _ = register_designation_pool(value);
        }
    }
}
/// Install before the first class-list/zone allocation, including early boot.
pub fn install_designation_observer() {
    pz_world::designation::install_designation_observer(Some(observe_designation));
}

#[cfg(test)]
mod tests;
