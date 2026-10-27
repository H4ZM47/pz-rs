//! Physical ownership metadata for canonical component allocations.
//!
//! Registration changes no game state and invokes no Lua or native lifecycle
//! hooks. Weak pending records retry unavailable metadata at an outer host safe
//! point. Collector detachment is distinct from remove/reset/pool release.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};

use pz_entity::runtime_attributes::{AttributeRef, RuntimeAttributeInstance};
use pz_entity::tables::{AttributeTypeDefinition, AttributeTypeRef};
use pz_entity::{ComponentRef, RetiredComponents, RuntimeComponent, RuntimeEntryView};
use pz_inventory::shared::ItemRef;
use pz_inventory::InventoryItem;
use pz_kahlua::gc::{self, RegistrationError, TraceDescriptor, TraceVisitor, UserdataDescriptor};
use pz_scripts::objects::entity::components::attributes::AttributesScript;
use pz_scripts::objects::entity::GameEntityScript;
use pz_scripts::objects::items::Item as ItemScript;
use pz_world::floor_object::FloorObjectRef;
use pz_world::object_slot::IsoObjectRef;
use pz_world::objects::{IsoObject, IsoWorldInventoryObject, ObjectKind};

use crate::bind_fluid_container::LuaFluidContainer;
use crate::bind_items::LuaInventoryItem;
use crate::bind_misc_classes::Color;
use crate::bind_world_object::LuaWorldObject;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataStatus {
    Complete,
    /// A temporary host borrow or unavailable registry retained conservative
    /// ownership. The weak pending queue must be retried before a safe drain.
    Deferred,
    /// GC slots remain described, but immutable native class metadata is
    /// invalid. The permanent typed fault is separate from retryable borrow.
    InvalidClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationFault {
    pub allocation: &'static str,
    pub address: usize,
    pub error: RegistrationError,
}

impl std::fmt::Display for RegistrationFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} metadata at {:x}: {:?}",
            self.allocation, self.address, self.error
        )
    }
}

impl std::error::Error for RegistrationFault {}

enum Pending {
    Item(Weak<RefCell<InventoryItem>>),
    Object(Weak<RefCell<IsoObject>>),
    Floor(Weak<RefCell<IsoWorldInventoryObject>>),
    Component(Weak<RefCell<RuntimeComponent>>),
    Color(Weak<RefCell<pz_entity::color::Rgba>>),
    Properties(Weak<RefCell<pz_entity::fluids::SealedFluidProperties>>),
    ItemScript(Weak<RefCell<ItemScript>>),
    EntityScript(Weak<RefCell<GameEntityScript>>),
    Attribute(Weak<RefCell<RuntimeAttributeInstance>>),
    AttributeType(Weak<RefCell<AttributeTypeDefinition>>),
    ItemUserdata(Weak<LuaInventoryItem>),
    FluidUserdata(Weak<LuaFluidContainer>),
    ColorUserdata(Weak<Color>),
    WorldUserdata(Weak<LuaWorldObject>),
}

impl Pending {
    fn key(&self) -> (&'static str, usize) {
        match self {
            Self::Item(w) => ("InventoryItem", w.as_ptr() as usize),
            Self::Object(w) => ("IsoObject", w.as_ptr() as usize),
            Self::Floor(w) => ("IsoWorldInventoryObject payload", w.as_ptr() as usize),
            Self::Component(w) => ("Component", w.as_ptr() as usize),
            Self::Color(w) => ("Color", w.as_ptr() as usize),
            Self::Properties(w) => ("SealedFluidProperties", w.as_ptr() as usize),
            Self::ItemScript(w) => ("Item script", w.as_ptr() as usize),
            Self::EntityScript(w) => ("GameEntityScript", w.as_ptr() as usize),
            Self::Attribute(w) => ("AttributeInstance", w.as_ptr() as usize),
            Self::AttributeType(w) => ("AttributeType", w.as_ptr() as usize),
            Self::ItemUserdata(w) => ("InventoryItem userdata", w.as_ptr() as usize),
            Self::FluidUserdata(w) => ("FluidContainer userdata", w.as_ptr() as usize),
            Self::ColorUserdata(w) => ("Color userdata", w.as_ptr() as usize),
            Self::WorldUserdata(w) => ("IsoObject userdata", w.as_ptr() as usize),
        }
    }

    fn alive(&self) -> bool {
        match self {
            Self::Item(w) => w.strong_count() != 0,
            Self::Object(w) => w.strong_count() != 0,
            Self::Floor(w) => w.strong_count() != 0,
            Self::Component(w) => w.strong_count() != 0,
            Self::Color(w) => w.strong_count() != 0,
            Self::Properties(w) => w.strong_count() != 0,
            Self::ItemScript(w) => w.strong_count() != 0,
            Self::EntityScript(w) => w.strong_count() != 0,
            Self::Attribute(w) => w.strong_count() != 0,
            Self::AttributeType(w) => w.strong_count() != 0,
            Self::ItemUserdata(w) => w.strong_count() != 0,
            Self::FluidUserdata(w) => w.strong_count() != 0,
            Self::ColorUserdata(w) => w.strong_count() != 0,
            Self::WorldUserdata(w) => w.strong_count() != 0,
        }
    }
}

thread_local! {
    static PENDING: RefCell<HashMap<(&'static str, usize), Pending>> = RefCell::new(HashMap::new());
    // Fault records own no graph references. They are diagnostics, not proof
    // that a conflicting descriptor was replaced or registration succeeded.
    static FAULTS: RefCell<Vec<RegistrationFault>> = const { RefCell::new(Vec::new()) };
}

fn pending(record: Pending) {
    let _ = PENDING.try_with(|queue| {
        if let Ok(mut queue) = queue.try_borrow_mut() {
            queue.retain(|_, record| record.alive());
            queue.insert(record.key(), record);
        }
    });
}

fn remember_fault(fault: RegistrationFault) {
    let _ = FAULTS.try_with(|faults| {
        if let Ok(mut faults) = faults.try_borrow_mut() {
            if !faults.contains(&fault) {
                faults.push(fault);
            }
        }
    });
}

fn registered(
    result: Result<bool, RegistrationError>,
    record: Pending,
) -> Result<MetadataStatus, RegistrationFault> {
    match result {
        Ok(_) => Ok(MetadataStatus::Complete),
        Err(RegistrationError::RegistryUnavailable) => {
            pending(record);
            Ok(MetadataStatus::Deferred)
        }
        Err(error) => {
            let (allocation, address) = record.key();
            let fault = RegistrationFault {
                allocation,
                address,
                error,
            };
            remember_fault(fault);
            Err(fault)
        }
    }
}

fn merged(a: MetadataStatus, b: MetadataStatus) -> MetadataStatus {
    if a == MetadataStatus::InvalidClass || b == MetadataStatus::InvalidClass {
        MetadataStatus::InvalidClass
    } else if a == MetadataStatus::Complete && b == MetadataStatus::Complete {
        MetadataStatus::Complete
    } else {
        MetadataStatus::Deferred
    }
}

pub struct ItemDescriptor;
impl TraceDescriptor<InventoryItem> for ItemDescriptor {
    fn trace(item: &InventoryItem, visitor: &mut TraceVisitor) {
        if let Some(owner) = item.world_item_slot() {
            if owner.is::<IsoObject>() {
                visitor.owner_erased::<IsoObject>(owner.erased_slot());
            }
        }
        if let Some(table) = item
            .mod_data_slot()
            .as_ref()
            .and_then(|data| data.as_live_slot())
        {
            visitor.table(table);
        }
        for entry in item.components.entry_slots() {
            if let RuntimeEntryView::Live { component, .. } = entry {
                visitor.owner(component);
            }
        }
        // Existing templates, bags, attachments and opaque holder payloads
        // keep their prior conservative ownership. They are not invented
        // transitive component edges or cloned traversal snapshots.
    }

    fn detach(item: &mut InventoryItem) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new((
            item.components.take_entries(),
            item.take_mod_data_for_gc(),
            item.take_world_item_for_gc(),
        )))
    }
}

pub struct ObjectDescriptor;
impl TraceDescriptor<IsoObject> for ObjectDescriptor {
    fn trace(object: &IsoObject, visitor: &mut TraceVisitor) {
        if let ObjectKind::WorldInventory(floor) = &object.kind {
            if let Some(payload) = floor.payload_slot() {
                visitor.owner(payload);
            }
        }
        for entry in object.components.entry_slots() {
            if let RuntimeEntryView::Live { component, .. } = entry {
                visitor.owner(component);
            }
        }
    }

    fn detach(object: &mut IsoObject) -> Option<Box<dyn std::any::Any>> {
        let floor = match &mut object.kind {
            ObjectKind::WorldInventory(floor) => floor.take_payload_for_gc(),
            _ => None,
        };
        Some(Box::new((object.components.take_entries(), floor)))
    }
}

pub struct FloorDescriptor;
impl TraceDescriptor<IsoWorldInventoryObject> for FloorDescriptor {
    fn trace(floor: &IsoWorldInventoryObject, visitor: &mut TraceVisitor) {
        if let Some(item) = floor.item.item_slot() {
            visitor.owner(item);
        }
    }
    fn detach(floor: &mut IsoWorldInventoryObject) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(floor.item.take_item_for_gc()))
    }
}

pub struct WorldUserdataDescriptor;
impl UserdataDescriptor<LuaWorldObject> for WorldUserdataDescriptor {
    fn trace(world: &LuaWorldObject, visitor: &mut TraceVisitor) -> Result<(), gc::TraceDeferred> {
        visitor.owner(&world.object);
        Ok(())
    }
}

pub struct ComponentDescriptor;
impl TraceDescriptor<RuntimeComponent> for ComponentDescriptor {
    fn trace(component: &RuntimeComponent, visitor: &mut TraceVisitor) {
        if let Some(owner) = component.owner_slot() {
            // Each branch describes the ONE actual erased physical slot.
            // Unknown owner types deliberately stay opaque/conservative.
            if owner.is::<InventoryItem>() {
                visitor.owner_erased::<InventoryItem>(owner.erased_slot());
            } else if owner.is::<IsoObject>() {
                visitor.owner_erased::<IsoObject>(owner.erased_slot());
            }
        }
        if let Some(color) = component.fluid_color_slot() {
            visitor.owner(color);
        }
        if let Some(properties) = component.fluid_properties_slot() {
            visitor.owner(properties);
        }
        if let Some(attributes) = component.attributes() {
            for slot in attributes.entry_slots() {
                // AssocArray's type key and instance value are two real Rc
                // fields, even though the instance also retains that type.
                visitor.owner(slot.type_slot());
                visitor.owner(slot.instance_slot());
            }
        }
        if let Some(script) = component.script_cache_slot() {
            // The actual shared script definition, not a cloned script body.
            // An unregistered/unknown definition remains conservative.
            if script.is::<ItemScript>() {
                visitor.owner_erased::<ItemScript>(script.erased_slot());
            } else if script.is::<GameEntityScript>() {
                visitor.owner_erased::<GameEntityScript>(script.erased_slot());
            }
        }
    }

    fn detach(component: &mut RuntimeComponent) -> Option<Box<dyn std::any::Any>> {
        let mut retired = RetiredComponents::new();
        if let Some(owner) = component.replace_owner(None) {
            retired.push_owner(owner);
        }
        let attributes = component
            .attributes_mut()
            .map(|attributes| attributes.take_slots_for_gc())
            .unwrap_or_default();
        Some(Box::new((retired, attributes)))
    }
}

pub struct AttributeDescriptor;
impl TraceDescriptor<RuntimeAttributeInstance> for AttributeDescriptor {
    fn trace(attribute: &RuntimeAttributeInstance, visitor: &mut TraceVisitor) {
        if let Some(type_ref) = attribute.type_slot() {
            visitor.owner(type_ref);
        }
    }
    fn detach(attribute: &mut RuntimeAttributeInstance) -> Option<Box<dyn std::any::Any>> {
        // Moving the physical field is collector bookkeeping, not native
        // AttributeInstance.reset/release. Keep numeric value/class untouched.
        Some(Box::new(attribute.take_type_for_gc()))
    }
}

pub struct AttributeTypeDescriptor;
impl TraceDescriptor<AttributeTypeDefinition> for AttributeTypeDescriptor {
    fn trace(_: &AttributeTypeDefinition, _: &mut TraceVisitor) {}
}

fn trace_script_templates(script: &GameEntityScript, visitor: &mut TraceVisitor) {
    for component in script.component_scripts() {
        if let Some(attributes) = component.as_any().downcast_ref::<AttributesScript>() {
            if let Ok(template) = attributes.template_slot() {
                if let Some(template) = template.as_ref() {
                    // ComponentScript's Box is inside the registered script
                    // body; this reports its one physical Rc field directly.
                    visitor.owner_erased::<RuntimeComponent>(template);
                }
            }
            // A held/unknown nested field stays conservative, never a
            // fabricated edge. A held parent script guard defers collection.
        }
    }
}

fn detach_script_templates(script: &GameEntityScript) -> Option<Box<dyn std::any::Any>> {
    let mut retired = Vec::new();
    for component in script.component_scripts() {
        if let Some(attributes) = component.as_any().downcast_ref::<AttributesScript>() {
            if let Ok(Some(template)) = attributes.replace_template(None) {
                retired.push(template);
            }
        }
    }
    // Collector field detachment is not native PreReload. Leave kv pairs and
    // hasCreatedContainer alone and never reset/release pooled components.
    Some(Box::new(retired))
}

pub struct ItemScriptDescriptor;
impl TraceDescriptor<ItemScript> for ItemScriptDescriptor {
    fn trace(script: &ItemScript, visitor: &mut TraceVisitor) {
        trace_script_templates(&script.entity, visitor);
    }
    fn detach(script: &mut ItemScript) -> Option<Box<dyn std::any::Any>> {
        detach_script_templates(&script.entity)
    }
}

pub struct EntityScriptDescriptor;
impl TraceDescriptor<GameEntityScript> for EntityScriptDescriptor {
    fn trace(script: &GameEntityScript, visitor: &mut TraceVisitor) {
        trace_script_templates(script, visitor);
    }
    fn detach(script: &mut GameEntityScript) -> Option<Box<dyn std::any::Any>> {
        detach_script_templates(script)
    }
}

pub struct ColorDescriptor;
impl TraceDescriptor<pz_entity::color::Rgba> for ColorDescriptor {
    fn trace(_: &pz_entity::color::Rgba, _: &mut TraceVisitor) {}
}

pub struct PropertiesDescriptor;
impl TraceDescriptor<pz_entity::fluids::SealedFluidProperties> for PropertiesDescriptor {
    fn trace(_: &pz_entity::fluids::SealedFluidProperties, _: &mut TraceVisitor) {}
}

pub struct ItemUserdataDescriptor;
impl UserdataDescriptor<LuaInventoryItem> for ItemUserdataDescriptor {
    fn trace(item: &LuaInventoryItem, visitor: &mut TraceVisitor) -> Result<(), gc::TraceDeferred> {
        visitor.owner(&item.item);
        Ok(())
    }
}

pub struct FluidUserdataDescriptor;
impl UserdataDescriptor<LuaFluidContainer> for FluidUserdataDescriptor {
    fn trace(
        fluid: &LuaFluidContainer,
        visitor: &mut TraceVisitor,
    ) -> Result<(), gc::TraceDeferred> {
        visitor.owner(&fluid.component);
        Ok(())
    }
}

pub struct ColorUserdataDescriptor;
impl UserdataDescriptor<Color> for ColorUserdataDescriptor {
    fn trace(color: &Color, visitor: &mut TraceVisitor) -> Result<(), gc::TraceDeferred> {
        visitor.owner(color.actual_slot());
        Ok(())
    }
}

pub fn register_color(
    color: &Rc<RefCell<pz_entity::color::Rgba>>,
) -> Result<MetadataStatus, RegistrationFault> {
    registered(
        gc::register_existing::<_, ColorDescriptor>(color),
        Pending::Color(Rc::downgrade(color)),
    )
}

fn register_properties(
    properties: &Rc<RefCell<pz_entity::fluids::SealedFluidProperties>>,
) -> Result<MetadataStatus, RegistrationFault> {
    registered(
        gc::register_existing::<_, PropertiesDescriptor>(properties),
        Pending::Properties(Rc::downgrade(properties)),
    )
}

// This does not traverse an owner back to its store recursively. Item/object
// observers already visit all physical component slots at the allocation and
// adoption gates; standalone component access ensures its immediate owner too.
fn component_slots(component: &ComponentRef) -> Result<MetadataStatus, RegistrationFault> {
    let mut status = registered(
        gc::register_existing::<_, ComponentDescriptor>(component),
        Pending::Component(Rc::downgrade(component)),
    )?;
    let Ok(body) = component.try_borrow() else {
        pending(Pending::Component(Rc::downgrade(component)));
        return Ok(MetadataStatus::Deferred);
    };
    if let Some(color) = body.fluid_color_slot() {
        status = merged(status, register_color(color)?);
    }
    if let Some(properties) = body.fluid_properties_slot() {
        status = merged(status, register_properties(properties)?);
    }
    if let Some(attributes) = body.attributes() {
        for slot in attributes.entry_slots() {
            status = merged(status, register_attribute_type(slot.type_slot())?);
            status = merged(status, register_attribute(slot.instance_slot())?);
        }
    }
    if let Some(script) = body.script_cache_slot() {
        if let Some(script) = script.try_handle::<ItemScript>() {
            status = merged(status, register_item_script(&script)?);
        } else if let Some(script) = script.try_handle::<GameEntityScript>() {
            status = merged(status, register_entity_script(&script)?);
        }
    }
    Ok(status)
}

pub fn register_attribute_type(
    type_ref: &AttributeTypeRef,
) -> Result<MetadataStatus, RegistrationFault> {
    registered(
        gc::register_existing::<_, AttributeTypeDescriptor>(type_ref),
        Pending::AttributeType(Rc::downgrade(type_ref)),
    )
}

pub fn register_attribute(attribute: &AttributeRef) -> Result<MetadataStatus, RegistrationFault> {
    let mut status = registered(
        gc::register_existing::<_, AttributeDescriptor>(attribute),
        Pending::Attribute(Rc::downgrade(attribute)),
    )?;
    let Ok(body) = attribute.try_borrow() else {
        pending(Pending::Attribute(Rc::downgrade(attribute)));
        return Ok(MetadataStatus::Deferred);
    };
    if let Some(type_ref) = body.type_slot() {
        status = merged(status, register_attribute_type(type_ref)?);
    }
    Ok(status)
}

fn register_script_templates(
    script: &GameEntityScript,
) -> Result<MetadataStatus, RegistrationFault> {
    let mut status = MetadataStatus::Complete;
    for component in script.component_scripts() {
        if let Some(attributes) = component.as_any().downcast_ref::<AttributesScript>() {
            let Ok(template) = attributes.template_slot() else {
                status = MetadataStatus::Deferred;
                continue;
            };
            if let Some(template) = template.as_ref() {
                if let Ok(template) = Rc::clone(template).downcast::<RefCell<RuntimeComponent>>() {
                    status = merged(status, component_slots(&template)?);
                }
                // Unknown erased allocations keep conservative ownership.
            }
        }
    }
    Ok(status)
}

pub fn register_item_script(
    script: &Rc<RefCell<ItemScript>>,
) -> Result<MetadataStatus, RegistrationFault> {
    let mut status = registered(
        gc::register_existing::<_, ItemScriptDescriptor>(script),
        Pending::ItemScript(Rc::downgrade(script)),
    )?;
    let Ok(body) = script.try_borrow() else {
        pending(Pending::ItemScript(Rc::downgrade(script)));
        return Ok(MetadataStatus::Deferred);
    };
    status = merged(status, register_script_templates(&body.entity)?);
    if status == MetadataStatus::Deferred {
        pending(Pending::ItemScript(Rc::downgrade(script)));
    }
    Ok(status)
}

pub fn register_entity_script(
    script: &Rc<RefCell<GameEntityScript>>,
) -> Result<MetadataStatus, RegistrationFault> {
    let mut status = registered(
        gc::register_existing::<_, EntityScriptDescriptor>(script),
        Pending::EntityScript(Rc::downgrade(script)),
    )?;
    let Ok(body) = script.try_borrow() else {
        pending(Pending::EntityScript(Rc::downgrade(script)));
        return Ok(MetadataStatus::Deferred);
    };
    status = merged(status, register_script_templates(&body)?);
    if status == MetadataStatus::Deferred {
        pending(Pending::EntityScript(Rc::downgrade(script)));
    }
    Ok(status)
}

enum OwnerNode {
    Item(ItemRef),
    Object(IsoObjectRef),
    Floor(FloorObjectRef),
}

/// Metadata traversal follows actual slots using temporary pins. The visited
/// set is local and owns no game allocations; cycles never recurse or grow a
/// persistent strong mirror. Trace descriptors themselves borrow physical
/// slots and never report these traversal pins.
fn register_owner_graph(root: OwnerNode) -> Result<MetadataStatus, RegistrationFault> {
    let mut status = MetadataStatus::Complete;
    let mut work = VecDeque::from([root]);
    let mut visited = HashSet::new();
    while let Some(node) = work.pop_front() {
        match node {
            OwnerNode::Item(item) => {
                if !visited.insert(("item", Rc::as_ptr(&item) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, ItemDescriptor>(&item),
                        Pending::Item(Rc::downgrade(&item)),
                    )?,
                );
                let Ok(body) = item.try_borrow() else {
                    pending(Pending::Item(Rc::downgrade(&item)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for entry in body.components.entry_slots() {
                    if let RuntimeEntryView::Live { component, .. } = entry {
                        status = merged(status, component_slots(component)?);
                    }
                }
                if let Some(object) = body
                    .world_item_slot()
                    .and_then(|owner| owner.try_handle::<IsoObject>())
                {
                    work.push_back(OwnerNode::Object(object));
                }
            }
            OwnerNode::Object(object) => {
                if !visited.insert(("object", Rc::as_ptr(&object) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, ObjectDescriptor>(&object),
                        Pending::Object(Rc::downgrade(&object)),
                    )?,
                );
                let Ok(body) = object.try_borrow() else {
                    pending(Pending::Object(Rc::downgrade(&object)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                if let Err(error) =
                    crate::bind_world_object::ensure_class(&object, body.kind.native_class_name())
                {
                    if matches!(
                        error,
                        pz_kahlua::LuaError::Runtime {
                            class: "pz.host.Borrowed",
                            ..
                        }
                    ) {
                        pending(Pending::Object(Rc::downgrade(&object)));
                        status = merged(status, MetadataStatus::Deferred);
                    } else {
                        status = MetadataStatus::InvalidClass;
                    }
                }
                for entry in body.components.entry_slots() {
                    if let RuntimeEntryView::Live { component, .. } = entry {
                        status = merged(status, component_slots(component)?);
                    }
                }
                if let ObjectKind::WorldInventory(floor) = &body.kind {
                    if let Some(payload) = floor.payload_slot() {
                        work.push_back(OwnerNode::Floor(Rc::clone(payload)));
                    }
                }
            }
            OwnerNode::Floor(floor) => {
                if !visited.insert(("floor", Rc::as_ptr(&floor) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, FloorDescriptor>(&floor),
                        Pending::Floor(Rc::downgrade(&floor)),
                    )?,
                );
                let Ok(body) = floor.try_borrow() else {
                    pending(Pending::Floor(Rc::downgrade(&floor)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                if let Some(item) = body.item.item_slot() {
                    work.push_back(OwnerNode::Item(Rc::clone(item)));
                }
            }
        }
    }
    Ok(status)
}

pub fn register_item(item: &ItemRef) -> Result<MetadataStatus, RegistrationFault> {
    register_owner_graph(OwnerNode::Item(Rc::clone(item)))
}

pub fn register_object(object: &IsoObjectRef) -> Result<MetadataStatus, RegistrationFault> {
    register_owner_graph(OwnerNode::Object(Rc::clone(object)))
}

pub fn register_component(component: &ComponentRef) -> Result<MetadataStatus, RegistrationFault> {
    let status = component_slots(component)?;
    let owner = component
        .try_borrow()
        .ok()
        .and_then(|body| body.owner_slot().cloned());
    if let Some(owner) = owner {
        if let Some(item) = owner.try_handle::<InventoryItem>() {
            return Ok(merged(status, register_item(&item)?));
        }
        if let Some(object) = owner.try_handle::<IsoObject>() {
            return Ok(merged(status, register_object(&object)?));
        }
    }
    Ok(status)
}

pub fn register_item_userdata(
    item: &Rc<LuaInventoryItem>,
) -> Result<MetadataStatus, RegistrationFault> {
    let status = registered(
        gc::register_userdata::<_, ItemUserdataDescriptor>(item),
        Pending::ItemUserdata(Rc::downgrade(item)),
    )?;
    Ok(merged(status, register_item(&item.item)?))
}

pub fn register_fluid_userdata(
    fluid: &Rc<LuaFluidContainer>,
) -> Result<MetadataStatus, RegistrationFault> {
    let status = registered(
        gc::register_userdata::<_, FluidUserdataDescriptor>(fluid),
        Pending::FluidUserdata(Rc::downgrade(fluid)),
    )?;
    Ok(merged(status, register_component(&fluid.component)?))
}

pub fn register_color_userdata(color: &Rc<Color>) -> Result<MetadataStatus, RegistrationFault> {
    let status = registered(
        gc::register_userdata::<_, ColorUserdataDescriptor>(color),
        Pending::ColorUserdata(Rc::downgrade(color)),
    )?;
    Ok(merged(status, register_color(color.actual_slot())?))
}

pub fn register_world_userdata(
    world: &Rc<LuaWorldObject>,
) -> Result<MetadataStatus, RegistrationFault> {
    let status = registered(
        gc::register_userdata::<_, WorldUserdataDescriptor>(world),
        Pending::WorldUserdata(Rc::downgrade(world)),
    )?;
    Ok(merged(status, register_object(&world.object)?))
}

/// Capture-free callbacks installed before any actual entity owner cycle is
/// attached. Deferred registrations remain weak and conservative until retry.
pub fn observe_item(item: &ItemRef) {
    let _ = register_item(item);
}
pub fn observe_object(object: &IsoObjectRef, native_class: &'static str) {
    if matches!(
        crate::bind_world_object::remember_class(object, native_class),
        Err(pz_kahlua::LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ) {
        pending(Pending::Object(Rc::downgrade(object)));
    }
    let _ = register_object(object);
}

pub fn install_observers() {
    pz_inventory::shared::install_item_observer(Some(observe_item));
    pz_world::object_slot::install_object_observer(Some(observe_object));
    retry_pending();
}

/// Retry only metadata. Hosts call this after world/sprite/entity guards end
/// and BEFORE their explicit pending collection drain. Never collects itself.
pub fn retry_pending() {
    let records = PENDING
        .try_with(|queue| {
            queue
                .try_borrow_mut()
                .ok()
                .map(|mut q| std::mem::take(&mut *q))
        })
        .ok()
        .flatten()
        .unwrap_or_default();
    for (_, record) in records {
        match record {
            Pending::Item(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_item(&v);
                }
            }
            Pending::Object(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_object(&v);
                }
            }
            Pending::Floor(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_owner_graph(OwnerNode::Floor(v));
                }
            }
            Pending::Component(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_component(&v);
                }
            }
            Pending::Color(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_color(&v);
                }
            }
            Pending::Properties(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_properties(&v);
                }
            }
            Pending::ItemScript(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_item_script(&v);
                }
            }
            Pending::EntityScript(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_entity_script(&v);
                }
            }
            Pending::Attribute(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_attribute(&v);
                }
            }
            Pending::AttributeType(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_attribute_type(&v);
                }
            }
            Pending::ItemUserdata(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_item_userdata(&v);
                }
            }
            Pending::FluidUserdata(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_fluid_userdata(&v);
                }
            }
            Pending::ColorUserdata(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_color_userdata(&v);
                }
            }
            Pending::WorldUserdata(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_world_userdata(&v);
                }
            }
        }
    }
}

/// Drain explicit implementation diagnostics at a safe host boundary. A hard
/// conflict never replaces another descriptor or clears live data.
pub fn take_registration_faults() -> Vec<RegistrationFault> {
    FAULTS
        .try_with(|faults| std::mem::take(&mut *faults.borrow_mut()))
        .unwrap_or_default()
}

#[cfg(test)]
mod world_item_tests;

#[cfg(test)]
mod tests;
