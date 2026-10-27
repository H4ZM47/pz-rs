//! Physical ownership metadata for canonical component allocations.
//!
//! Registration changes no game state and invokes no Lua or native lifecycle
//! hooks. Weak pending records retry unavailable metadata at an outer host safe
//! point. Collector detachment is distinct from remove/reset/pool release.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};

use pz_characters::character::save::SavedInventoryListRef;
use pz_entity::runtime_attributes::{AttributeRef, RuntimeAttributeInstance};
use pz_entity::tables::{AttributeTypeDefinition, AttributeTypeRef};
use pz_entity::{ComponentRef, RetiredComponents, RuntimeComponent, RuntimeEntryView};
use pz_inventory::shared::{ContainerRef, ItemRef, SharedContainer};
use pz_inventory::InventoryItem;
use pz_inventory::ItemKind;
use pz_kahlua::gc::{self, RegistrationError, TraceDescriptor, TraceVisitor, UserdataDescriptor};
use pz_render::{LuaTexture, TextureRef};
use pz_scripts::objects::entity::components::attributes::AttributesScript;
use pz_scripts::objects::entity::GameEntityScript;
use pz_scripts::objects::items::Item as ItemScript;
use pz_world::floor_object::FloorObjectRef;
use pz_world::object_slot::IsoObjectRef;
use pz_world::objects::{IsoObject, IsoWorldInventoryObject, ObjectKind};
use pz_world::sprite::runtime::{
    RuntimeSprite, RuntimeSpriteInstance, RuntimeSpriteRef, SpriteInstanceRef,
};
use pz_world::{IsoGridSquare, SquareRef};

use crate::bind_fluid_container::LuaFluidContainer;
use crate::bind_item_container::LuaItemContainer;
use crate::bind_items::LuaInventoryItem;
use crate::bind_misc_classes::Color;
use crate::bind_world_object::LuaWorldObject;

mod designation;
pub use designation::{install_designation_observer, observe_designation, register_designation_registry, register_designation_list, register_designation_zone, register_designation_pool, register_designation_userdata};
use crate::bind_designation::LuaDesignationZone;
use pz_world::designation::{DesignationRegistry, DesignationList, DesignationZone, DesignationPositionPool};

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
    DesignationRegistry(Weak<RefCell<DesignationRegistry>>),
    DesignationList(Weak<RefCell<DesignationList>>),
    DesignationZone(Weak<RefCell<DesignationZone>>),
    DesignationPool(Weak<RefCell<DesignationPositionPool>>),
    DesignationUserdata(Weak<LuaDesignationZone>),
    Square(Weak<RefCell<IsoGridSquare>>),
    Sprite(Weak<RefCell<RuntimeSprite>>),
    SpriteInstance(Weak<RefCell<RuntimeSpriteInstance>>),
    SpriteManager(Weak<RefCell<pz_world::sprite::IsoSpriteManager>>),
    ChunkHotSave(Weak<RefCell<pz_world::chunk::ChunkHotSaveState>>),
    WorldObjects(Weak<RefCell<pz_world::world_objects::WorldObjects>>),
    Texture(Weak<LuaTexture>),

    Item(Weak<RefCell<InventoryItem>>),
    Container(Weak<RefCell<SharedContainer>>),
    SavedInventory(Weak<RefCell<Vec<Option<ItemRef>>>>),
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
    ContainerUserdata(Weak<LuaItemContainer>),
    FluidUserdata(Weak<LuaFluidContainer>),
    ColorUserdata(Weak<Color>),
    WorldUserdata(Weak<LuaWorldObject>),
}

impl Pending {
    fn key(&self) -> (&'static str, usize) {
        match self {
            Self::DesignationRegistry(w) => ("Designation registry", w.as_ptr() as usize),
            Self::DesignationList(w) => ("Designation list", w.as_ptr() as usize),
            Self::DesignationZone(w) => ("Designation zone", w.as_ptr() as usize),
            Self::DesignationPool(w) => ("Designation position pool", w.as_ptr() as usize),
            Self::DesignationUserdata(w) => ("Designation userdata", w.as_ptr() as usize),
            Self::Square(w) => ("Square", w.as_ptr() as usize),
            Self::Sprite(w) => ("RuntimeSprite", w.as_ptr() as usize),
            Self::SpriteInstance(w) => ("RuntimeSpriteInstance", w.as_ptr() as usize),
            Self::SpriteManager(w) => ("IsoSpriteManager", w.as_ptr() as usize),
            Self::ChunkHotSave(w) => ("ChunkHotSaveState", w.as_ptr() as usize),
            Self::WorldObjects(w) => ("WorldObjects", w.as_ptr() as usize),
            Self::Texture(w) => ("Texture", w.as_ptr() as usize),
            Self::Item(w) => ("InventoryItem", w.as_ptr() as usize),
            Self::Container(w) => ("ItemContainer", w.as_ptr() as usize),
            Self::SavedInventory(w) => ("savedInventoryItems", w.as_ptr() as usize),
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
            Self::ContainerUserdata(w) => ("ItemContainer userdata", w.as_ptr() as usize),
            Self::FluidUserdata(w) => ("FluidContainer userdata", w.as_ptr() as usize),
            Self::ColorUserdata(w) => ("Color userdata", w.as_ptr() as usize),
            Self::WorldUserdata(w) => ("IsoObject userdata", w.as_ptr() as usize),
        }
    }

    fn alive(&self) -> bool {
        match self {
            Self::DesignationRegistry(w) => w.strong_count() != 0,
            Self::DesignationList(w) => w.strong_count() != 0,
            Self::DesignationZone(w) => w.strong_count() != 0,
            Self::DesignationPool(w) => w.strong_count() != 0,
            Self::DesignationUserdata(w) => w.strong_count() != 0,
            Self::Square(w) => w.strong_count() != 0,
            Self::Sprite(w) => w.strong_count() != 0,
            Self::SpriteInstance(w) => w.strong_count() != 0,
            Self::SpriteManager(w) => w.strong_count() != 0,
            Self::ChunkHotSave(w) => w.strong_count() != 0,
            Self::WorldObjects(w) => w.strong_count() != 0,
            Self::Texture(w) => w.strong_count() != 0,
            Self::Item(w) => w.strong_count() != 0,
            Self::Container(w) => w.strong_count() != 0,
            Self::SavedInventory(w) => w.strong_count() != 0,
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
            Self::ContainerUserdata(w) => w.strong_count() != 0,
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
        if let Some(container) = item.container_slot() {
            visitor.owner(container);
        }
        if let ItemKind::Container(bag) = &item.kind {
            if let Some(container) = bag.container.slot() {
                visitor.owner(container);
            }
        }
        if let Some(textures) = item.textures.live_slot() {
            for texture in textures.slots() {
                visitor.userdata(texture);
            }
            if let Some(script) = &textures.script_item {
                visitor.owner(&script.0);
            }
        }
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
        // Existing templates, attachments and opaque holder payloads
        // keep their prior conservative ownership. They are not invented
        // transitive component edges or cloned traversal snapshots.
    }

    fn detach(item: &mut InventoryItem) -> Option<Box<dyn std::any::Any>> {
        let bag = match &mut item.kind {
            ItemKind::Container(bag) => bag.container.take_for_gc(),
            _ => None,
        };
        Some(Box::new((
            item.take_container_for_gc(),
            bag,
            item.components.take_entries(),
            item.take_mod_data_for_gc(),
            item.take_world_item_for_gc(),
            item.take_texture_slots_for_gc(),
        )))
    }
}

/// Every list slot is a separate owning edge, including aliases between
/// main items, includingObsoleteItems, and containingItem. Opaque parent/grid
/// wrappers retain their prior conservative ownership; never call getters or
/// trace through those wrappers as if their Values were direct container slots.
pub struct ContainerDescriptor;
impl TraceDescriptor<SharedContainer> for ContainerDescriptor {
    fn trace(container: &SharedContainer, visitor: &mut TraceVisitor) {
        for item in &container.items {
            visitor.owner(item);
        }
        for item in container.including_obsolete_items.iter().flatten() {
            visitor.owner(item);
        }
        if let Some(item) = &container.containing_item {
            visitor.owner(item);
        }
        if let Some(item) = &container.inventory_container {
            visitor.owner(item);
        }
    }
    fn detach(container: &mut SharedContainer) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new((
            std::mem::take(&mut container.items),
            std::mem::take(&mut container.including_obsolete_items),
            container.containing_item.take(),
            container.inventory_container.take(),
            container.parent.take(),
            container.source_grid.take(),
            std::mem::take(&mut container.raw_items),
        )))
    }
}

pub struct SavedInventoryDescriptor;
impl TraceDescriptor<Vec<Option<ItemRef>>> for SavedInventoryDescriptor {
    fn trace(items: &Vec<Option<ItemRef>>, visitor: &mut TraceVisitor) {
        for item in items.iter().flatten() {
            visitor.owner(item);
        }
    }
    fn detach(items: &mut Vec<Option<ItemRef>>) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(std::mem::take(items)))
    }
}

pub struct ObjectDescriptor;
impl TraceDescriptor<IsoObject> for ObjectDescriptor {
    fn trace(object: &IsoObject, visitor: &mut TraceVisitor) {
        if let Some(square) = object.square_slot() {
            visitor.owner(square);
        }
        if let Some(sprite) = object.sprite.native_slot() {
            visitor.owner(sprite);
        }
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
        let sprite = std::mem::replace(&mut object.sprite, pz_world::objects::ObjectSprite::None);
        Some(Box::new((
            object.components.take_entries(),
            floor,
            object.take_square_for_gc(),
            sprite,
        )))
    }
}

pub struct SquareDescriptor;
impl TraceDescriptor<IsoGridSquare> for SquareDescriptor {
    fn trace(square: &IsoGridSquare, visitor: &mut TraceVisitor) {
        for object in &square.objects {
            visitor.owner(object.owner_slot());
        }
        visitor.owner(&square.world_objects);
        if let Some(chunk) = &square.chunk_hot_save {
            visitor.owner(chunk);
        }
    }
    fn detach(square: &mut IsoGridSquare) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new((
            std::mem::take(&mut square.objects),
            std::mem::replace(
                &mut square.world_objects,
                pz_world::world_objects::WorldObjects::new_ref(),
            ),
            square.chunk_hot_save.take(),
        )))
    }
}
pub struct SpriteDescriptor;
impl TraceDescriptor<RuntimeSprite> for SpriteDescriptor {
    fn trace(sprite: &RuntimeSprite, visitor: &mut TraceVisitor) {
        if let Some(manager) = sprite.parent_manager_slot() {
            visitor.owner(manager);
        }
        if let Some(tint) = sprite.tint_slot() {
            visitor.owner(tint);
        }
        if let Some(def) = sprite.def_slot() {
            visitor.owner(def);
        }
        if let Some(texture) = sprite.texture_slot() {
            visitor.userdata(texture);
        }
    }
    fn detach(sprite: &mut RuntimeSprite) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(sprite.take_slots_for_gc()))
    }
}
pub struct SpriteInstanceDescriptor;
impl TraceDescriptor<RuntimeSpriteInstance> for SpriteInstanceDescriptor {
    fn trace(instance: &RuntimeSpriteInstance, visitor: &mut TraceVisitor) {
        if let Some(sprite) = instance.parent_sprite_slot() {
            visitor.owner(sprite);
        }
    }
    fn detach(instance: &mut RuntimeSpriteInstance) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(instance.take_parent_for_gc()))
    }
}
pub struct SpriteManagerDescriptor;
impl TraceDescriptor<pz_world::sprite::IsoSpriteManager> for SpriteManagerDescriptor {
    fn trace(_: &pz_world::sprite::IsoSpriteManager, _: &mut TraceVisitor) {}
}
pub struct ChunkHotSaveDescriptor;
impl TraceDescriptor<pz_world::chunk::ChunkHotSaveState> for ChunkHotSaveDescriptor {
    fn trace(_: &pz_world::chunk::ChunkHotSaveState, _: &mut TraceVisitor) {}
}
pub struct TextureDescriptor;
impl UserdataDescriptor<LuaTexture> for TextureDescriptor {
    fn trace(_: &LuaTexture, _: &mut TraceVisitor) -> Result<(), gc::TraceDeferred> {
        Ok(())
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
        if let Some(texture) = script.normal_texture_slot() {
            visitor.userdata(texture);
        }
        if let Some(texture) = script.world_texture_slot() {
            visitor.userdata(texture);
        }
        for texture in script.special_texture_slots().iter().flatten() {
            visitor.userdata(texture);
        }
        trace_script_templates(&script.entity, visitor);
    }
    fn detach(script: &mut ItemScript) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new((
            detach_script_templates(&script.entity),
            script.take_texture_slots_for_gc(),
        )))
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

pub struct ContainerUserdataDescriptor;
impl UserdataDescriptor<LuaItemContainer> for ContainerUserdataDescriptor {
    fn trace(
        container: &LuaItemContainer,
        visitor: &mut TraceVisitor,
    ) -> Result<(), gc::TraceDeferred> {
        visitor.owner(&container.container);
        let cached = container
            .items_list
            .try_borrow()
            .map_err(|_| gc::TraceDeferred)?;
        if let Some(list) = cached.as_ref() {
            visitor.value(list);
        }
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
    for texture in body
        .normal_texture_slot()
        .into_iter()
        .chain(body.world_texture_slot())
        .chain(body.special_texture_slots().iter().flatten())
    {
        status = merged(status, register_texture(texture)?);
    }
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

pub fn register_texture(texture: &TextureRef) -> Result<MetadataStatus, RegistrationFault> {
    registered(
        gc::register_userdata::<_, TextureDescriptor>(texture),
        Pending::Texture(Rc::downgrade(texture)),
    )
}
fn register_sprite_manager(
    manager: &pz_world::sprite::runtime::SpriteManagerRef,
) -> Result<MetadataStatus, RegistrationFault> {
    registered(
        gc::register_existing::<_, SpriteManagerDescriptor>(manager),
        Pending::SpriteManager(Rc::downgrade(manager)),
    )
}
fn register_chunk_hot_save(
    chunk: &pz_world::chunk::ChunkHotSaveRef,
) -> Result<MetadataStatus, RegistrationFault> {
    registered(
        gc::register_existing::<_, ChunkHotSaveDescriptor>(chunk),
        Pending::ChunkHotSave(Rc::downgrade(chunk)),
    )
}

enum OwnerNode {
    Item(ItemRef),
    Container(ContainerRef),
    SavedInventory(SavedInventoryListRef),
    Object(IsoObjectRef),
    Floor(FloorObjectRef),
    Square(SquareRef),
    Sprite(RuntimeSpriteRef),
    SpriteInstance(SpriteInstanceRef),
    WorldObjects(pz_world::WorldObjectsRef),
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
                if let Some(container) = body.container_slot() {
                    work.push_back(OwnerNode::Container(container.clone()));
                }
                if let ItemKind::Container(bag) = &body.kind {
                    if let Some(container) = bag.container.slot() {
                        work.push_back(OwnerNode::Container(container.clone()));
                    }
                }
                for entry in body.components.entry_slots() {
                    if let RuntimeEntryView::Live { component, .. } = entry {
                        status = merged(status, component_slots(component)?);
                    }
                }
                if let Some(textures) = body.textures.live_slot() {
                    for texture in textures.slots() {
                        status = merged(status, register_texture(texture)?);
                    }
                    if let Some(script) = &textures.script_item {
                        status = merged(status, register_item_script(&script.0)?);
                    }
                }
                if let Some(object) = body
                    .world_item_slot()
                    .and_then(|owner| owner.try_handle::<IsoObject>())
                {
                    work.push_back(OwnerNode::Object(object));
                }
            }
            OwnerNode::Container(container) => {
                if !visited.insert(("container", Rc::as_ptr(&container) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, ContainerDescriptor>(&container),
                        Pending::Container(Rc::downgrade(&container)),
                    )?,
                );
                let Ok(body) = container.try_borrow() else {
                    pending(Pending::Container(Rc::downgrade(&container)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for item in &body.items {
                    work.push_back(OwnerNode::Item(item.clone()));
                }
                for item in body.including_obsolete_items.iter().flatten() {
                    work.push_back(OwnerNode::Item(item.clone()));
                }
                if let Some(item) = &body.containing_item {
                    work.push_back(OwnerNode::Item(item.clone()));
                }
                if let Some(item) = &body.inventory_container {
                    work.push_back(OwnerNode::Item(item.clone()));
                }
            }
            OwnerNode::SavedInventory(items) => {
                if !visited.insert(("savedInventoryItems", Rc::as_ptr(&items) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, SavedInventoryDescriptor>(&items),
                        Pending::SavedInventory(Rc::downgrade(&items)),
                    )?,
                );
                let Ok(body) = items.try_borrow() else {
                    pending(Pending::SavedInventory(Rc::downgrade(&items)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for item in body.iter().flatten() {
                    work.push_back(OwnerNode::Item(item.clone()));
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
                if let Some(square) = body.square_slot() {
                    work.push_back(OwnerNode::Square(square.clone()));
                }
                if let Some(sprite) = body.sprite.native_slot() {
                    work.push_back(OwnerNode::Sprite(sprite.clone()));
                }
            }
            OwnerNode::Square(square) => {
                if !visited.insert(("square", Rc::as_ptr(&square) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, SquareDescriptor>(&square),
                        Pending::Square(Rc::downgrade(&square)),
                    )?,
                );
                let Ok(body) = square.try_borrow() else {
                    pending(Pending::Square(Rc::downgrade(&square)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for object in &body.objects {
                    work.push_back(OwnerNode::Object(object.ref_handle()));
                }
                work.push_back(OwnerNode::WorldObjects(body.world_objects.clone()));
                if let Some(chunk) = &body.chunk_hot_save {
                    status = merged(status, register_chunk_hot_save(chunk)?);
                }
            }
            OwnerNode::WorldObjects(list) => {
                if !visited.insert(("worldObjects", Rc::as_ptr(&list) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        pz_world::world_objects::register_world_objects(&list),
                        Pending::WorldObjects(Rc::downgrade(&list)),
                    )?,
                );
                let Ok(body) = list.try_borrow() else {
                    pending(Pending::WorldObjects(Rc::downgrade(&list)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                for entry in body.entry_slots() {
                    if let pz_world::world_objects::WorldObjectEntry::Object(object) = entry {
                        work.push_back(OwnerNode::Object(object.clone()));
                    }
                }
            }
            OwnerNode::Sprite(sprite) => {
                if !visited.insert(("sprite", Rc::as_ptr(&sprite) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, SpriteDescriptor>(&sprite),
                        Pending::Sprite(Rc::downgrade(&sprite)),
                    )?,
                );
                let Ok(body) = sprite.try_borrow() else {
                    pending(Pending::Sprite(Rc::downgrade(&sprite)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                if let Some(manager) = body.parent_manager_slot() {
                    status = merged(status, register_sprite_manager(manager)?);
                }
                if let Some(tint) = body.tint_slot() {
                    status = merged(status, register_color(tint)?);
                }
                if let Some(def) = body.def_slot() {
                    work.push_back(OwnerNode::SpriteInstance(def.clone()));
                }
                if let Some(texture) = body.texture_slot() {
                    status = merged(status, register_texture(texture)?);
                }
            }
            OwnerNode::SpriteInstance(instance) => {
                if !visited.insert(("spriteInstance", Rc::as_ptr(&instance) as usize)) {
                    continue;
                }
                status = merged(
                    status,
                    registered(
                        gc::register_existing::<_, SpriteInstanceDescriptor>(&instance),
                        Pending::SpriteInstance(Rc::downgrade(&instance)),
                    )?,
                );
                let Ok(body) = instance.try_borrow() else {
                    pending(Pending::SpriteInstance(Rc::downgrade(&instance)));
                    status = merged(status, MetadataStatus::Deferred);
                    continue;
                };
                if let Some(sprite) = body.parent_sprite_slot() {
                    work.push_back(OwnerNode::Sprite(sprite.clone()));
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

pub fn register_container(container: &ContainerRef) -> Result<MetadataStatus, RegistrationFault> {
    register_owner_graph(OwnerNode::Container(container.clone()))
}

pub fn register_saved_inventory(
    items: &SavedInventoryListRef,
) -> Result<MetadataStatus, RegistrationFault> {
    register_owner_graph(OwnerNode::SavedInventory(items.clone()))
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

pub fn register_container_userdata(
    container: &Rc<LuaItemContainer>,
) -> Result<MetadataStatus, RegistrationFault> {
    let status = registered(
        gc::register_userdata::<_, ContainerUserdataDescriptor>(container),
        Pending::ContainerUserdata(Rc::downgrade(container)),
    )?;
    Ok(merged(status, register_container(&container.container)?))
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
pub fn observe_container(container: &ContainerRef) {
    let _ = register_container(container);
}
pub fn observe_saved_inventory(items: &SavedInventoryListRef) {
    let _ = register_saved_inventory(items);
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
    install_designation_observer();
    pz_inventory::shared::install_item_observer(Some(observe_item));
    pz_inventory::shared::install_container_observer(Some(observe_container));
    pz_characters::character::save::install_saved_inventory_observer(Some(observe_saved_inventory));
    pz_world::object_slot::install_object_observer(Some(observe_object));
    pz_world::square_slot::install_square_observer(Some(observe_square));
    pz_world::sprite::runtime::install_sprite_observer(Some(observe_sprite));
    pz_world::sprite::runtime::install_instance_observer(Some(observe_sprite_instance));
    install_texture_observer();
    retry_pending();
}

pub fn observe_square(square: &SquareRef) {
    let _ = register_owner_graph(OwnerNode::Square(square.clone()));
}
pub fn observe_sprite(sprite: &RuntimeSpriteRef) {
    let _ = register_owner_graph(OwnerNode::Sprite(sprite.clone()));
}
pub fn observe_sprite_instance(instance: &SpriteInstanceRef) {
    let _ = register_owner_graph(OwnerNode::SpriteInstance(instance.clone()));
}
pub fn observe_texture(texture: &TextureRef) {
    let _ = register_texture(texture);
}
/// Early boot may install the leaf observer before ScriptManager.load;
/// actual item/world observers are installed by ordinary registration.
pub fn install_texture_observer() {
    pz_render::install_texture_observer(Some(observe_texture));
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
            Pending::DesignationRegistry(w) => { if let Some(v)=w.upgrade() {let _=register_designation_registry(&v);} }
            Pending::DesignationList(w) => { if let Some(v)=w.upgrade() {let _=register_designation_list(&v);} }
            Pending::DesignationZone(w) => { if let Some(v)=w.upgrade() {let _=register_designation_zone(&v);} }
            Pending::DesignationPool(w) => { if let Some(v)=w.upgrade() {let _=register_designation_pool(&v);} }
            Pending::DesignationUserdata(w) => { if let Some(v)=w.upgrade() {let _=register_designation_userdata(&v);} }
            Pending::Square(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_owner_graph(OwnerNode::Square(v));
                }
            }
            Pending::Sprite(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_owner_graph(OwnerNode::Sprite(v));
                }
            }
            Pending::SpriteInstance(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_owner_graph(OwnerNode::SpriteInstance(v));
                }
            }
            Pending::WorldObjects(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_owner_graph(OwnerNode::WorldObjects(v));
                }
            }
            Pending::SpriteManager(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_sprite_manager(&v);
                }
            }
            Pending::ChunkHotSave(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_chunk_hot_save(&v);
                }
            }
            Pending::Texture(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_texture(&v);
                }
            }
            Pending::Item(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_item(&v);
                }
            }
            Pending::Container(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_container(&v);
                }
            }
            Pending::SavedInventory(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_saved_inventory(&v);
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
            Pending::ContainerUserdata(w) => {
                if let Some(v) = w.upgrade() {
                    let _ = register_container_userdata(&v);
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
mod sprite_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod container_tests;
