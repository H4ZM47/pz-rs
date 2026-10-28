//! Actual `DesignationZone`/`DesignationZoneAnimal` allocations and class lists.
//!
//! These owners are runtime state, distinct from the explicitly archived scalar
//! records in `meta_save`. The two permanent class lists are independent. A
//! constructor ID collision deliberately need not put the same zone in both.
//! Actor scanning, connected zones and ObjectPool operations remain selected
//! capabilities; reaching them returns a host fault after the supported prefix.

use crate::{IsoObjectRef, SquareRef};
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{EntityOwner, RetiredComponents};
use pz_kahlua::java_io::{JavaReader, JavaWrite};
use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;

pub type DesignationRegistryRef = Rc<RefCell<DesignationRegistry>>;
pub type DesignationListRef = Rc<RefCell<DesignationList>>;
pub type DesignationZoneRef = Rc<RefCell<DesignationZone>>;
pub type DesignationPoolRef = Rc<RefCell<DesignationPositionPool>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignationZoneKind {
    Base,
    Animal,
}
impl DesignationZoneKind {
    pub fn native_class_name(self) -> &'static str {
        match self {
            Self::Base => "zombie.iso.areas.DesignationZone",
            Self::Animal => "zombie.iso.areas.DesignationZoneAnimal",
        }
    }
}

/// Every variant is the contents of a distinct actual ArrayList allocation.
/// Unsupported actor/pool producers cannot insert a copied actor description.
pub enum DesignationList {
    Zones(Vec<DesignationZoneRef>),
    WorldObjects(Vec<IsoObjectRef>),
    Owners(Vec<EntityOwner>),
    Squares(Vec<SquareRef>),
    RoofAreas(Vec<EntityOwner>),
}
impl DesignationList {
    pub fn len(&self) -> usize {
        match self {
            Self::Zones(v) => v.len(),
            Self::WorldObjects(v) => v.len(),
            Self::Owners(v) => v.len(),
            Self::Squares(v) => v.len(),
            Self::RoofAreas(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Raw collector detach: preserves the allocation/variant and invokes no hooks.
    pub fn take_for_gc(&mut self) -> Self {
        match self {
            Self::Zones(v) => Self::Zones(std::mem::take(v)),
            Self::WorldObjects(v) => Self::WorldObjects(std::mem::take(v)),
            Self::Owners(v) => Self::Owners(std::mem::take(v)),
            Self::Squares(v) => Self::Squares(std::mem::take(v)),
            Self::RoofAreas(v) => Self::RoofAreas(std::mem::take(v)),
        }
    }
}

mod pool;
pub use pool::{
    DesignationPositionPool, Position3d, Position3dRef, PositionAllocator, PositionAllocatorRef,
    PositionDeque, PositionDequeRef, PositionPoolAuthority, PositionSet, PositionSetRef,
};

pub struct DesignationRegistry {
    all_zones: Option<DesignationListRef>,
    animal_zones: Option<DesignationListRef>,
    pub last_update: i64,
}
impl DesignationRegistry {
    pub fn all_zones_slot(&self) -> Option<&DesignationListRef> {
        self.all_zones.as_ref()
    }
    pub fn animal_zones_slot(&self) -> Option<&DesignationListRef> {
        self.animal_zones.as_ref()
    }
    pub fn take_for_gc(&mut self) -> Vec<DesignationListRef> {
        self.all_zones
            .take()
            .into_iter()
            .chain(self.animal_zones.take())
            .collect()
    }
}

pub struct DesignationZone {
    kind: DesignationZoneKind,
    pub id: f64,
    pub hour_last_seen: i32,
    pub last_action_timestamp: i32,
    pub name: Option<String>,
    pub zone_type: Option<String>,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    pub streamed: bool,
    pub animals: Option<DesignationListRef>,
    pub corpses: Option<DesignationListRef>,
    pub troughs: Option<DesignationListRef>,
    pub hutchs: Option<DesignationListRef>,
    pub food_on_ground: Option<DesignationListRef>,
    pub near_water_squares: Option<DesignationListRef>,
    pub roof_areas: Option<DesignationListRef>,
    pub position3d_pool: Option<DesignationPoolRef>,
    pub nb_of_dung: i32,
    pub nb_of_feather: i32,
}
impl DesignationZone {
    pub fn kind(&self) -> DesignationZoneKind {
        self.kind
    }
    fn raw(kind: DesignationZoneKind) -> Self {
        Self {
            kind,
            id: 0.0,
            hour_last_seen: 0,
            last_action_timestamp: 0,
            name: None,
            zone_type: None,
            x: 0,
            y: 0,
            z: 0,
            w: 0,
            h: 0,
            streamed: true,
            animals: None,
            corpses: None,
            troughs: None,
            hutchs: None,
            food_on_ground: None,
            near_water_squares: None,
            roof_areas: None,
            position3d_pool: None,
            nb_of_dung: 0,
            nb_of_feather: 0,
        }
    }
    /// Raw physical detach leaves every scalar and the native class unchanged.
    pub fn take_owner_slots_for_gc(
        &mut self,
    ) -> (Vec<DesignationListRef>, Option<DesignationPoolRef>) {
        let lists = [
            &mut self.animals,
            &mut self.corpses,
            &mut self.troughs,
            &mut self.hutchs,
            &mut self.food_on_ground,
            &mut self.near_water_squares,
            &mut self.roof_areas,
        ]
        .into_iter()
        .filter_map(Option::take)
        .collect();
        (lists, self.position3d_pool.take())
    }
}

pub enum DesignationObserved<'a> {
    Registry(&'a DesignationRegistryRef),
    List(&'a DesignationListRef),
    Zone(&'a DesignationZoneRef, DesignationZoneKind),
    Pool(&'a DesignationPoolRef),
    PoolDeque(&'a PositionDequeRef),
    PoolSet(&'a PositionSetRef),
    PoolAllocator(&'a PositionAllocatorRef),
    Position(&'a Position3dRef),
}
pub type DesignationObserver = for<'a> fn(DesignationObserved<'a>);
thread_local! { static OBSERVER: Cell<Option<DesignationObserver>> = const { Cell::new(None) }; }
pub fn install_designation_observer(
    observer: Option<DesignationObserver>,
) -> Option<DesignationObserver> {
    OBSERVER.with(|slot| slot.replace(observer))
}
fn observe(value: DesignationObserved<'_>) {
    if let Some(f) = OBSERVER.with(Cell::get) {
        f(value);
    }
}
fn observe_zone(zone: &DesignationZoneRef, kind: DesignationZoneKind) {
    observe(DesignationObserved::Zone(zone, kind));
}
fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}
fn pin<T: 'static>(owner: &Rc<RefCell<T>>) {
    let mut retired = RetiredComponents::new();
    retired.push_owner(EntityOwner::from_ref(owner));
    pz_inventory::component_host::retire_components(retired);
}
fn new_list(list: DesignationList) -> DesignationListRef {
    let list = Rc::new(RefCell::new(list));
    pin(&list);
    observe(DesignationObserved::List(&list));
    list
}
pub fn new_registry() -> DesignationRegistryRef {
    let all_zones = new_list(DesignationList::Zones(Vec::new()));
    let animal_zones = new_list(DesignationList::Zones(Vec::new()));
    let registry = Rc::new(RefCell::new(DesignationRegistry {
        all_zones: Some(all_zones),
        animal_zones: Some(animal_zones),
        last_update: 0,
    }));
    pin(&registry);
    observe(DesignationObserved::Registry(&registry));
    registry
}
/// The native no-argument base constructor registers nothing.
pub fn new_base_default() -> DesignationZoneRef {
    let zone = Rc::new(RefCell::new(DesignationZone::raw(
        DesignationZoneKind::Base,
    )));
    pin(&zone);
    observe_zone(&zone, DesignationZoneKind::Base);
    zone
}

fn class_list(registry: &DesignationRegistryRef, animal: bool) -> HostResult<DesignationListRef> {
    pin(registry);
    let body = registry
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation registry"))?;
    let slot = if animal {
        body.animal_zones_slot()
    } else {
        body.all_zones_slot()
    };
    slot.cloned()
        .ok_or_else(|| unsupported("detached Designation class list"))
}
fn zone_at(list: &DesignationListRef, index: usize) -> HostResult<Option<DesignationZoneRef>> {
    let body = list
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation class list"))?;
    match &*body {
        DesignationList::Zones(v) => Ok(v.get(index).cloned()),
        _ => Err(unsupported("Designation class list element type")),
    }
}
fn lookup_fallible(
    registry: &DesignationRegistryRef,
    animal: bool,
    mut predicate: impl FnMut(&DesignationZone) -> HostResult<bool>,
) -> HostResult<Option<DesignationZoneRef>> {
    let list = class_list(registry, animal)?;
    let mut index = 0;
    while let Some(zone) = zone_at(&list, index)? {
        pin(&zone);
        let matched = {
            let body = zone
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("Designation zone"))?;
            if animal && body.kind != DesignationZoneKind::Animal {
                return Err(unsupported("Designation animal list class cast"));
            }
            predicate(&body)?
        };
        if matched {
            return Ok(Some(zone));
        }
        index += 1;
    }
    Ok(None)
}
fn lookup(
    registry: &DesignationRegistryRef,
    animal: bool,
    mut predicate: impl FnMut(&DesignationZone) -> bool,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup_fallible(registry, animal, |zone| Ok(predicate(zone)))
}

pub fn try_get_zone_by_name(
    registry: &DesignationRegistryRef,
    name: Option<&str>,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup_fallible(registry, false, |zone| {
        let value = zone
            .name
            .as_deref()
            .ok_or_else(|| unsupported("Designation getZoneByName null String.equals receiver"))?;
        Ok(Some(value) == name)
    })
}
pub fn try_get_zone_by_name_and_type(
    registry: &DesignationRegistryRef,
    zone_type: Option<&str>,
    name: Option<&str>,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup_fallible(registry, false, |zone| {
        let value = zone
            .name
            .as_deref()
            .ok_or_else(|| unsupported("Designation getZoneByNameAndType null name receiver"))?;
        if Some(value) != name {
            return Ok(false);
        }
        let value = zone
            .zone_type
            .as_deref()
            .ok_or_else(|| unsupported("Designation getZoneByNameAndType null type receiver"))?;
        Ok(Some(value) == zone_type)
    })
}
pub fn try_get_zone_by_type(
    registry: &DesignationRegistryRef,
    zone_type: Option<&str>,
    x: i32,
    y: i32,
    z: i32,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup_fallible(registry, false, |zone| {
        if !(x >= zone.x
            && x < zone.x.wrapping_add(zone.w)
            && y >= zone.y
            && y < zone.y.wrapping_add(zone.h)
            && z == zone.z)
        {
            return Ok(false);
        }
        let wanted = zone_type
            .ok_or_else(|| unsupported("Designation getZoneByType null String.equals receiver"))?;
        Ok(zone.zone_type.as_deref() == Some(wanted))
    })
}

/// `Double.equals`, including canonical NaNs and distinct signed zero.
fn id_equals(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || a.is_nan() && b.is_nan()
}
pub fn try_get_zone(
    registry: &DesignationRegistryRef,
    x: i32,
    y: i32,
    z: i32,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup(registry, true, |v| {
        x >= v.x && x < v.x.wrapping_add(v.w) && y >= v.y && y < v.y.wrapping_add(v.h) && z == v.z
    })
}
pub fn try_get_zone_xy(
    registry: &DesignationRegistryRef,
    x: i32,
    y: i32,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup(registry, true, |v| {
        x >= v.x && x < v.x.wrapping_add(v.w) && y >= v.y && y < v.y.wrapping_add(v.h)
    })
}
pub fn try_get_base_zone(
    registry: &DesignationRegistryRef,
    x: i32,
    y: i32,
    z: i32,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup(registry, false, |v| {
        x >= v.x && x < v.x.wrapping_add(v.w) && y >= v.y && y < v.y.wrapping_add(v.h) && z == v.z
    })
}
pub fn try_get_zone_by_id(
    registry: &DesignationRegistryRef,
    id: f64,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup(registry, true, |v| id_equals(v.id, id))
}
pub fn try_get_base_zone_by_id(
    registry: &DesignationRegistryRef,
    id: f64,
) -> HostResult<Option<DesignationZoneRef>> {
    lookup(registry, false, |v| id_equals(v.id, id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignationCornerKind {
    Near,
    Far,
}
/// Pure field-read continuation. The host must read its CURRENT actual cell
/// before calling read_coordinates, as native getCell precedes zone fields.
pub struct DesignationCorner<'a> {
    zone: &'a DesignationZoneRef,
    kind: DesignationCornerKind,
}
impl DesignationCorner<'_> {
    pub fn kind(&self) -> DesignationCornerKind {
        self.kind
    }
    pub fn read_coordinates(&self) -> HostResult<(i32, i32, i32)> {
        let z = self
            .zone
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("Designation corner coordinates"))?;
        Ok(match self.kind {
            DesignationCornerKind::Near => (z.x, z.y, z.z),
            DesignationCornerKind::Far => (
                z.x.wrapping_add(z.w).wrapping_sub(1),
                z.y.wrapping_add(z.h).wrapping_sub(1),
                z.z,
            ),
        })
    }
}

/// Each corner consults the CURRENT actual cell before its pure zone field read.
pub trait DesignationHost {
    fn rand_next(&mut self, bound: i32) -> HostResult<i32>;
    fn current_cell_present(&mut self) -> HostResult<bool>;
    fn corner_square(&mut self, corner: DesignationCorner<'_>) -> HostResult<Option<SquareRef>>;
    fn sync_zone(&mut self, _zone: &DesignationZoneRef) -> HostResult<()> {
        Err(unsupported("DesignationZone.sync"))
    }
}
fn append_zone(
    registry: &DesignationRegistryRef,
    animal: bool,
    zone: &DesignationZoneRef,
) -> HostResult<()> {
    let list = class_list(registry, animal)?;
    {
        let mut body = list
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation class list add"))?;
        match &mut *body {
            DesignationList::Zones(v) => v.push(zone.clone()),
            _ => return Err(unsupported("Designation class list add type")),
        }
    }
    observe(DesignationObserved::List(&list));
    Ok(())
}
fn initialize_animal_lists(zone: &DesignationZoneRef) -> HostResult<()> {
    for field in 0..7 {
        let list = new_list(match field {
            4 => DesignationList::WorldObjects(Vec::new()),
            5 => DesignationList::Squares(Vec::new()),
            6 => DesignationList::RoofAreas(Vec::new()),
            _ => DesignationList::Owners(Vec::new()),
        });
        let mut body = zone
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation animal constructor"))?;
        *match field {
            0 => &mut body.animals,
            1 => &mut body.corpses,
            2 => &mut body.troughs,
            3 => &mut body.hutchs,
            4 => &mut body.food_on_ground,
            5 => &mut body.near_water_squares,
            _ => &mut body.roof_areas,
        } = Some(list);
    }
    observe_zone(zone, DesignationZoneKind::Animal);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn try_new_zone(
    registry: &DesignationRegistryRef,
    kind: DesignationZoneKind,
    zone_type: Option<&str>,
    name: Option<&str>,
    mut x: i32,
    mut y: i32,
    z: i32,
    mut x2: i32,
    mut y2: i32,
    sync: bool,
    host: &mut dyn DesignationHost,
    retired: &mut RetiredComponents,
) -> HostResult<DesignationZoneRef> {
    retired.push_owner(EntityOwner::from_ref(registry));
    let zone = Rc::new(RefCell::new(DesignationZone::raw(kind)));
    retired.push_owner(EntityOwner::from_ref(&zone));
    observe_zone(&zone, kind);
    if x > x2 {
        std::mem::swap(&mut x, &mut x2);
    }
    if y > y2 {
        std::mem::swap(&mut y, &mut y2);
    }
    let random = host.rand_next(9_999_999)?;
    {
        let mut body = zone
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation constructor"))?;
        body.id = f64::from(random) + 100_000.0;
        body.zone_type = zone_type.map(str::to_owned);
        body.name = Some(name.unwrap_or("").to_owned());
        body.x = x;
        body.y = y;
        body.z = z;
        body.w = x2.wrapping_sub(x);
        body.h = y2.wrapping_sub(y);
    }
    observe_zone(&zone, kind);
    let id = zone
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation constructor id"))?
        .id;
    if try_get_base_zone_by_id(registry, id)?.is_none() {
        append_zone(registry, false, &zone)?;
        if sync {
            host.sync_zone(&zone)?;
        }
    }
    if kind == DesignationZoneKind::Animal {
        initialize_animal_lists(&zone)?;
        let pool = pool::try_construct_native(retired)?;
        zone.try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation pool publication"))?
            .position3d_pool = Some(pool);
        observe_zone(&zone, kind);
        append_zone(registry, true, &zone)?;
        try_check(registry, &zone, host, retired)?;
    }
    Ok(zone)
}
#[allow(clippy::too_many_arguments)]
pub fn try_new_animal(
    registry: &DesignationRegistryRef,
    name: Option<&str>,
    x: i32,
    y: i32,
    z: i32,
    x2: i32,
    y2: i32,
    sync: bool,
    host: &mut dyn DesignationHost,
    retired: &mut RetiredComponents,
) -> HostResult<DesignationZoneRef> {
    try_new_zone(
        registry,
        DesignationZoneKind::Animal,
        Some("AnimalZone"),
        name,
        x,
        y,
        z,
        x2,
        y2,
        sync,
        host,
        retired,
    )
}

/// Explicit test/data projection. This does NOT execute or certify the native
/// constructor, RNG, ObjectPoolCounter/statistics bootstrap, check, or sync.
/// It is unavailable to production builds and must never back a Lua constructor.
#[cfg(any(test, feature = "test-util"))]
pub fn seed_animal_data_projection(
    registry: &DesignationRegistryRef,
    data: &crate::meta_save::DesignationZone,
    retired: &mut RetiredComponents,
) -> HostResult<DesignationZoneRef> {
    retired.push_owner(EntityOwner::from_ref(registry));
    let zone = Rc::new(RefCell::new(DesignationZone::raw(
        DesignationZoneKind::Animal,
    )));
    retired.push_owner(EntityOwner::from_ref(&zone));
    observe_zone(&zone, DesignationZoneKind::Animal);
    {
        let mut body = zone
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("designation data-projection fields"))?;
        body.id = data.id;
        body.name = Some(data.name.clone());
        body.zone_type = Some(data.zone_type.clone());
        body.x = data.x;
        body.y = data.y;
        body.z = data.z;
        body.w = data.w;
        body.h = data.h;
        body.hour_last_seen = data.hour_last_seen;
    }
    initialize_animal_lists(&zone)?;
    let pool = Rc::new(RefCell::new(DesignationPositionPool::data_projection()));
    retired.push_owner(EntityOwner::from_ref(&pool));
    observe(DesignationObserved::Pool(&pool));
    zone.try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("designation data-projection pool slot"))?
        .position3d_pool = Some(pool);
    observe_zone(&zone, DesignationZoneKind::Animal);
    append_zone(registry, false, &zone)?;
    append_zone(registry, true, &zone)?;
    Ok(zone)
}

pub fn try_is_fully_streamed(
    zone: &DesignationZoneRef,
    host: &mut dyn DesignationHost,
) -> HostResult<bool> {
    pin(zone);
    if !host.current_cell_present()? {
        return Ok(false);
    }
    let first = host.corner_square(DesignationCorner {
        zone,
        kind: DesignationCornerKind::Near,
    })?;
    if let Some(square) = &first {
        pin(square);
    }
    let second = host.corner_square(DesignationCorner {
        zone,
        kind: DesignationCornerKind::Far,
    })?;
    if let Some(square) = &second {
        pin(square);
    }
    Ok(first.is_some() && second.is_some())
}
fn clear_list(list: &DesignationListRef, retired: &mut RetiredComponents) -> HostResult<()> {
    let old = list
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("Designation list clear"))?
        .take_for_gc();
    retired.push_payload(Box::new(old));
    observe(DesignationObserved::List(list));
    Ok(())
}
pub fn try_check(
    registry: &DesignationRegistryRef,
    zone: &DesignationZoneRef,
    host: &mut dyn DesignationHost,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    retired.push_owner(EntityOwner::from_ref(registry));
    retired.push_owner(EntityOwner::from_ref(zone));
    if !try_is_fully_streamed(zone, host)? {
        return Ok(());
    }
    if !host.current_cell_present()? {
        registry
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation lastUpdate"))?
            .last_update = 0;
        observe(DesignationObserved::Registry(registry));
        return Ok(());
    }
    let animals = zone
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation check animals"))?
        .animals
        .clone()
        .ok_or_else(|| unsupported("Designation animal fields unavailable"))?;
    if !animals
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation check animals list"))?
        .is_empty()
    {
        return Err(unsupported("Designation animal.setDZone during check"));
    }
    // Empty actor loop is genuine. Publish each subsequent clear before the
    // first unimplemented pool call; retaining removed owners is not rollback.
    for field in 0..5 {
        let list = {
            let z = zone
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("Designation check list"))?;
            match field {
                0 => &z.near_water_squares,
                1 => &z.animals,
                2 => &z.corpses,
                3 => &z.hutchs,
                _ => &z.food_on_ground,
            }
            .clone()
            .ok_or_else(|| unsupported("Designation check detached list"))?
        };
        clear_list(&list, retired)?;
    }
    {
        let mut z = zone
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation check counters"))?;
        z.nb_of_dung = 0;
        z.nb_of_feather = 0;
    }
    observe_zone(zone, DesignationZoneKind::Animal);
    let pool = zone
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation check pool field"))?
        .position3d_pool
        .clone()
        .ok_or_else(|| unsupported("Designation check null pool diagnostic"))?;
    let roofs = zone
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Designation check roof field"))?
        .roof_areas
        .clone()
        .ok_or_else(|| unsupported("Designation check detached roof list"))?;
    pool::try_release_roofs(&pool, &roofs, retired)?;
    clear_list(&roofs, retired)?;
    Err(unsupported(
        "DesignationZoneAnimal.getAllDZones during check",
    ))
}
/// Base removal changes only allZones, even for an animal receiver. Animal
/// connected removal is a different, currently unsupported native operation.
pub fn try_remove_base_zone(
    registry: &DesignationRegistryRef,
    zone: Option<&DesignationZoneRef>,
    sync: bool,
    host: &mut dyn DesignationHost,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let Some(zone) = zone else {
        return Ok(());
    };
    retired.push_owner(EntityOwner::from_ref(registry));
    retired.push_owner(EntityOwner::from_ref(zone));
    let list = class_list(registry, false)?;
    {
        let mut body = list
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation base remove list"))?;
        let DesignationList::Zones(zones) = &mut *body else {
            return Err(unsupported("Designation base remove list type"));
        };
        if let Some(index) = zones.iter().position(|value| Rc::ptr_eq(value, zone)) {
            retired.push_owner(EntityOwner::from_ref(&zones.remove(index)));
        }
    }
    observe(DesignationObserved::List(&list));
    if sync {
        host.sync_zone(zone)?;
    }
    Ok(())
}
pub fn try_remove_base_zone_by_name_and_type(
    registry: &DesignationRegistryRef,
    zone_type: Option<&str>,
    name: Option<&str>,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let Some(zone) = try_get_zone_by_name_and_type(registry, zone_type, name)? else {
        return Ok(());
    };
    // No sync call in the String overload, including no network host lookup.
    struct NoSync;
    impl DesignationHost for NoSync {
        fn rand_next(&mut self, _: i32) -> HostResult<i32> {
            Err(unsupported("unused removal RNG"))
        }
        fn current_cell_present(&mut self) -> HostResult<bool> {
            Err(unsupported("unused removal cell"))
        }
        fn corner_square(&mut self, _: DesignationCorner<'_>) -> HostResult<Option<SquareRef>> {
            Err(unsupported("unused removal square"))
        }
    }
    try_remove_base_zone(registry, Some(&zone), false, &mut NoSync, retired)
}

pub fn try_reset_base(
    registry: &DesignationRegistryRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let list = class_list(registry, false)?;
    clear_list(&list, retired)
}
pub fn try_reset_animal(
    registry: &DesignationRegistryRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let list = class_list(registry, true)?;
    clear_list(&list, retired)
}

/// Exact null-zone branch; found-zone item classification is deliberately a
/// separate selected capability. It must never be skipped by item class/name.
pub fn try_add_item_on_ground(
    registry: &DesignationRegistryRef,
    object: &IsoObjectRef,
    square: &SquareRef,
) -> HostResult<()> {
    pin(object);
    pin(square);
    let (x, y, z) = {
        let sq = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("Designation supplied square"))?;
        (sq.x, sq.y, sq.z)
    };
    let Some(zone) = try_get_zone(registry, x, y, z)? else {
        return Ok(());
    };
    pin(&zone);
    Err(unsupported("DesignationZoneAnimal.isItemFood"))
}

#[derive(Debug)]
pub enum DesignationIoError {
    Io(io::Error),
    Host(RuntimeHostError),
}
impl From<io::Error> for DesignationIoError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<RuntimeHostError> for DesignationIoError {
    fn from(e: RuntimeHostError) -> Self {
        Self::Host(e)
    }
}
impl std::fmt::Display for DesignationIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Host(e) => write!(f, "{e:?}"),
        }
    }
}
impl std::error::Error for DesignationIoError {}

/// Original caller reader, complete local header first, then actual registry
/// lookup/constructor; errors preserve both the cursor and published owners.
pub fn try_load_zone(
    registry: &DesignationRegistryRef,
    r: &mut JavaReader<'_>,
    host: &mut dyn DesignationHost,
    retired: &mut RetiredComponents,
) -> Result<Option<DesignationZoneRef>, DesignationIoError> {
    retired.push_owner(EntityOwner::from_ref(registry));
    let id = r.get_f64()?;
    let (x, y, z, h, w) = (
        r.get_i32()?,
        r.get_i32()?,
        r.get_i32()?,
        r.get_i32()?,
        r.get_i32()?,
    );
    let zone_type = r.read_string_utf()?;
    let name = r.read_string_utf()?;
    let hour = r.get_i32()?;
    if zone_type != "AnimalZone" {
        return Ok(None);
    }
    let zone = if let Some(zone) = try_get_zone_by_id(registry, id)? {
        let mut body = zone
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation loaded existing zone"))?;
        body.name = Some(name);
        body.w = w;
        body.h = h;
        drop(body);
        zone
    } else {
        try_new_animal(
            registry,
            Some(&name),
            x,
            y,
            z,
            x.wrapping_add(w),
            y.wrapping_add(h),
            false,
            host,
            retired,
        )?
    };
    {
        let mut body = zone
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation loaded id/hour"))?;
        body.id = id;
        body.hour_last_seen = hour;
    }
    observe_zone(&zone, DesignationZoneKind::Animal);
    Ok(Some(zone))
}
pub fn try_save_registry(registry: &DesignationRegistryRef, out: &mut Vec<u8>) -> HostResult<()> {
    let list = class_list(registry, false)?;
    let count = {
        let body = list
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("Designation save list count"))?;
        match &*body {
            DesignationList::Zones(v) => v.len(),
            _ => return Err(unsupported("Designation save list type")),
        }
    };
    out.put_i32(count as i32);
    let mut index = 0;
    while let Some(zone) = zone_at(&list, index)? {
        pin(&zone);
        let z = zone
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("Designation save zone"))?;
        out.put_f64(z.id);
        out.put_i32(z.x);
        out.put_i32(z.y);
        out.put_i32(z.z);
        out.put_i32(z.h);
        out.put_i32(z.w);
        out.write_string_utf(z.zone_type.as_deref());
        out.write_string_utf(z.name.as_deref());
        out.put_i32(z.hour_last_seen);
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
