//! Actual AttributeContainer and numeric AttributeInstance allocations.
//!
//! Live slots own the canonical instances. Explicit legacy archive values
//! occupy a different enum variant and never mirror live mutable state.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::components::attributes::{Attribute, AttributeContainer, AttributeValue};
use crate::context::{AttributeKind, EntityContext, Lookup};
use crate::io::{EntityError, Reader, Writer};
use crate::runtime_fluid::RuntimeNativeNotice;
use crate::runtime_load::{HostResult, RuntimeHostError};
use crate::tables::{AttributeTypeRef, InitialValue};
use crate::{ComponentRef, ComponentType, RetiredComponents, RuntimeError};

pub type AttributeRef = Rc<RefCell<RuntimeAttributeInstance>>;

#[derive(Debug)]
pub struct RuntimeAttributeInstance {
    kind: AttributeKind,
    type_slot: Option<AttributeTypeRef>,
    value: AttributeValue,
}

impl RuntimeAttributeInstance {
    pub fn kind(&self) -> AttributeKind {
        self.kind
    }
    pub fn type_slot(&self) -> Option<&AttributeTypeRef> {
        self.type_slot.as_ref()
    }
    pub fn value(&self) -> &AttributeValue {
        &self.value
    }
    pub fn value_snapshot(&self) -> AttributeValue {
        self.value.clone()
    }
    pub fn take_type_for_gc(&mut self) -> Option<AttributeTypeRef> {
        self.type_slot.take()
    }

    /// Apply the native instance string setter to this same allocation.
    /// The caller emits the owned notices after releasing its body guards.
    pub fn set_value_from_script_string(
        &mut self,
        value: &str,
        notices: &mut Vec<RuntimeNativeNotice>,
    ) -> Result<bool, RuntimeError> {
        self.set_script_value(value, notices)
    }

    fn set_type(&mut self, type_ref: &AttributeTypeRef) -> Result<(), RuntimeError> {
        let definition = type_ref
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeType"))?;
        if definition.kind != self.kind {
            return Err(unsupported_kind());
        }
        self.value = initial_value(definition.kind, definition.initial.as_ref())?;
        self.type_slot = Some(Rc::clone(type_ref));
        Ok(())
    }

    fn reset(&mut self, retired: &mut RetiredComponents) {
        // Numeric subclasses inherit this exact reset: type=null, value
        // untouched. A retained instance sees its previous numeric value.
        if let Some(old) = self.type_slot.take() {
            retired.push_erased(old);
        }
    }

    fn load(&mut self, reader: &mut Reader<'_>) -> Result<(), RuntimeError> {
        self.value = match self.kind {
            AttributeKind::Float => AttributeValue::Float(reader.f32()?),
            AttributeKind::Int => AttributeValue::Int(reader.i32()?),
            _ => return Err(unsupported_kind()),
        };
        Ok(())
    }

    fn write(&self, writer: &mut Writer) -> Result<(), RuntimeError> {
        match &self.value {
            AttributeValue::Float(value) if self.kind == AttributeKind::Float => writer.f32(*value),
            AttributeValue::Int(value) if self.kind == AttributeKind::Int => writer.i32(*value),
            _ => return Err(unsupported_kind()),
        }
        Ok(())
    }

    fn set_script_value(
        &mut self,
        value: &str,
        notices: &mut Vec<RuntimeNativeNotice>,
    ) -> Result<bool, RuntimeError> {
        let type_ref = self.type_slot.as_ref().ok_or_else(null_pointer)?;
        let definition = type_ref
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeType"))?;
        let parsed = match self.kind {
            AttributeKind::Float => java_float(value).map(AttributeValue::Float),
            AttributeKind::Int => value.parse::<i32>().ok().map(AttributeValue::Int),
            _ => return Err(unsupported_kind()),
        };
        match parsed {
            Some(value) => {
                self.value = validate(&definition.numeric_validation, value)?;
                Ok(true)
            }
            None => {
                notices.push(RuntimeNativeNotice::GeneralException(format!(
                    "java.lang.NumberFormatException: For input string: \"{value}\""
                )));
                self.value = initial_value(definition.kind, definition.initial.as_ref())?;
                Ok(false)
            }
        }
    }
}

fn unsupported_kind() -> RuntimeError {
    RuntimeError::UnsupportedRuntime("Native attribute value class is not implemented")
}
fn null_pointer() -> RuntimeError {
    RuntimeError::Codec(EntityError("NullPointerException".into()))
}

fn initial_value(
    kind: AttributeKind,
    initial: Option<&InitialValue>,
) -> Result<AttributeValue, RuntimeError> {
    match (kind, initial) {
        (AttributeKind::Float, Some(InitialValue::Float(value))) => {
            Ok(AttributeValue::Float(*value as f32))
        }
        (AttributeKind::Int, Some(InitialValue::Int(value))) => {
            Ok(AttributeValue::Int(*value as i32))
        }
        (AttributeKind::Float | AttributeKind::Int, _) => Err(RuntimeError::UnsupportedRuntime(
            "AttributeType initial value is unavailable",
        )),
        _ => Err(unsupported_kind()),
    }
}

fn validate(
    validation: &Lookup<crate::tables::NumericAttributeValidation>,
    value: AttributeValue,
) -> Result<AttributeValue, RuntimeError> {
    let Lookup::Found(validation) = validation else {
        return Err(RuntimeError::UnsupportedRuntime(
            "AttributeType numeric validation is unavailable",
        ));
    };
    if !validation.requires_validation {
        return Ok(value);
    }
    let Some(bounds) = &validation.bounds else {
        return Ok(value);
    };
    match (value, &bounds.min, &bounds.max) {
        (AttributeValue::Float(value), InitialValue::Float(min), InitialValue::Float(max)) => {
            let value = java_max(java_min(value, *max as f32), *min as f32);
            Ok(AttributeValue::Float(value))
        }
        (AttributeValue::Int(value), InitialValue::Int(min), InitialValue::Int(max)) => {
            Ok(AttributeValue::Int(value.min(*max as i32).max(*min as i32)))
        }
        _ => Err(RuntimeError::UnsupportedRuntime(
            "AttributeType bounds have an invalid numeric class",
        )),
    }
}

fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() || b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a <= b {
        a
    } else {
        b
    }
}

fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() || b.is_sign_positive() {
            0.0
        } else {
            -0.0
        }
    } else if a >= b {
        a
    } else {
        b
    }
}

/// Java Float.parseFloat's decimal/special/hex syntax. Invalid text remains
/// a parse failure, rather than adopting Rust's additional `inf` spellings.
fn java_float(value: &str) -> Option<f32> {
    let value = value.trim_matches(|c: char| c <= ' ');
    let (negative, unsigned) = if let Some(v) = value.strip_prefix('-') {
        (true, v)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    if unsigned.starts_with(['+', '-']) {
        return None;
    }
    if unsigned == "NaN" {
        return Some(f32::NAN);
    }
    if unsigned == "Infinity" {
        return Some(if negative {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        });
    }
    let unsigned = unsigned
        .strip_suffix(['f', 'F', 'd', 'D'])
        .unwrap_or(unsigned);
    if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        let (mantissa, exponent) = hex.split_once(['p', 'P'])?;
        let exponent: i32 = exponent.parse().ok()?;
        let mut number = 0.0f64;
        let mut fraction = false;
        let mut divisor = 1.0;
        let mut digits = 0;
        for c in mantissa.chars() {
            if c == '.' && !fraction {
                fraction = true;
                continue;
            }
            let digit = c.to_digit(16)? as f64;
            digits += 1;
            if fraction {
                divisor *= 16.0;
                number += digit / divisor;
            } else {
                number = number * 16.0 + digit;
            }
        }
        if digits == 0 {
            return None;
        }
        let value = (number * 2.0f64.powi(exponent)) as f32;
        return Some(if negative { -value } else { value });
    }
    if unsigned.is_empty()
        || !unsigned
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    let value: f32 = unsigned.parse().ok()?;
    Some(if negative { -value } else { value })
}

#[derive(Default)]
struct AttributePools {
    floats: VecDeque<AttributeRef>,
    ints: VecDeque<AttributeRef>,
}
thread_local! { static POOLS: RefCell<AttributePools> = RefCell::new(AttributePools::default()); }

fn allocate(kind: AttributeKind) -> Result<AttributeRef, RuntimeError> {
    POOLS.with(|pools| {
        let mut pools = pools
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeFactory"))?;
        let (pool, zero) = match kind {
            AttributeKind::Float => (&mut pools.floats, AttributeValue::Float(0.0)),
            AttributeKind::Int => (&mut pools.ints, AttributeValue::Int(0)),
            _ => return Err(unsupported_kind()),
        };
        Ok(pool.pop_front().unwrap_or_else(|| {
            Rc::new(RefCell::new(RuntimeAttributeInstance {
                kind,
                type_slot: None,
                value: zero,
            }))
        }))
    })
}

fn release(attribute: &AttributeRef, retired: &mut RetiredComponents) -> Result<(), RuntimeError> {
    let kind = {
        let mut instance = attribute
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
        instance.reset(retired);
        instance.kind
    };
    POOLS.with(|pools| {
        let mut pools = pools
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeFactory"))?;
        let pool = match kind {
            AttributeKind::Float => &mut pools.floats,
            AttributeKind::Int => &mut pools.ints,
            _ => return Err(unsupported_kind()),
        };
        pool.push_back(Rc::clone(attribute));
        Ok(())
    })
}

#[derive(Debug)]
pub struct RuntimeAttributeSlot {
    id: i16,
    type_slot: AttributeTypeRef,
    instance: AttributeRef,
}
impl RuntimeAttributeSlot {
    pub fn type_slot(&self) -> &AttributeTypeRef {
        &self.type_slot
    }
    pub fn instance_slot(&self) -> &AttributeRef {
        &self.instance
    }
}

#[derive(Debug)]
enum AttributeStorage {
    Live {
        slots: Vec<RuntimeAttributeSlot>,
        max_id: i16,
    },
    Archive(AttributeContainer),
}

#[derive(Debug)]
pub struct RuntimeAttributes {
    storage: AttributeStorage,
}
impl Default for RuntimeAttributes {
    fn default() -> Self {
        Self {
            storage: AttributeStorage::Live {
                slots: Vec::new(),
                max_id: -1,
            },
        }
    }
}

impl RuntimeAttributes {
    pub(crate) fn from_archive(value: AttributeContainer) -> Self {
        if value.attributes.is_empty() {
            Self::default()
        } else {
            Self {
                storage: AttributeStorage::Archive(value),
            }
        }
    }
    pub fn is_resolved(&self) -> bool {
        matches!(self.storage, AttributeStorage::Live { .. })
    }
    pub fn entry_slots(&self) -> impl Iterator<Item = &RuntimeAttributeSlot> {
        match &self.storage {
            AttributeStorage::Live { slots, .. } => slots.as_slice(),
            AttributeStorage::Archive(_) => &[],
        }
        .iter()
    }
    /// Collector-only physical detach. No native reset, release, pool call or
    /// cached max-id mutation belongs to this graph operation.
    pub fn take_slots_for_gc(&mut self) -> Vec<RuntimeAttributeSlot> {
        match &mut self.storage {
            AttributeStorage::Live { slots, .. } => std::mem::take(slots),
            AttributeStorage::Archive(_) => Vec::new(),
        }
    }
    pub fn max_attribute_id(&self) -> Result<i16, RuntimeError> {
        match &self.storage {
            AttributeStorage::Live { max_id, .. } => Ok(*max_id),
            _ => Err(unresolved()),
        }
    }
    pub fn get_ref(&self, id: i16) -> Result<Option<&AttributeRef>, RuntimeError> {
        match &self.storage {
            AttributeStorage::Live { slots, .. } => {
                Ok(slots.iter().find(|s| s.id == id).map(|s| &s.instance))
            }
            _ => Err(unresolved()),
        }
    }
    pub fn get_ref_by_type(
        &self,
        type_ref: &AttributeTypeRef,
    ) -> Result<Option<&AttributeRef>, RuntimeError> {
        match &self.storage {
            AttributeStorage::Live { slots, .. } => Ok(slots
                .iter()
                .find(|s| Rc::ptr_eq(&s.type_slot, type_ref))
                .map(|s| &s.instance)),
            _ => Err(unresolved()),
        }
    }
    pub fn get_value(&self, id: i16) -> Result<Option<AttributeValue>, RuntimeError> {
        self.get_ref(id)?
            .map(|r| {
                r.try_borrow()
                    .map(|a| a.value_snapshot())
                    .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))
            })
            .transpose()
    }
    pub fn value_snapshot(&self) -> Result<AttributeContainer, RuntimeError> {
        match &self.storage {
            AttributeStorage::Archive(value) => Ok(value.clone()),
            AttributeStorage::Live { slots, .. } => {
                let mut attributes = Vec::with_capacity(slots.len());
                for slot in slots {
                    let instance = slot
                        .instance
                        .try_borrow()
                        .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
                    attributes.push(Attribute {
                        id: slot.id,
                        value: instance.value_snapshot(),
                    });
                }
                Ok(AttributeContainer { attributes })
            }
        }
    }
    pub fn check_archive_copy(&self) -> Result<(), RuntimeError> {
        for slot in self.entry_slots() {
            let _body = slot
                .instance
                .try_borrow()
                .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
        }
        Ok(())
    }
    pub fn clear(&mut self, retired: &mut RetiredComponents) -> Result<(), RuntimeError> {
        if let AttributeStorage::Live { slots, .. } = &self.storage {
            // Native release cannot fail because of a Rust borrow. Check
            // every introduced borrow before the first irreversible FIFO
            // publication; a retry must never enqueue an instance twice.
            if !slots.is_empty() {
                POOLS.with(|pools| {
                    let _guard = pools
                        .try_borrow_mut()
                        .map_err(|_| RuntimeError::BorrowedTransient("AttributeFactory"))?;
                    Ok::<_, RuntimeError>(())
                })?;
            }
            for slot in slots {
                let _guard = slot
                    .instance
                    .try_borrow_mut()
                    .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
            }
            for slot in slots {
                release(&slot.instance, retired)?;
            }
        }
        if let AttributeStorage::Live { slots, max_id } = &mut self.storage {
            // Preserve the final associative storage and its capacity.
            for slot in slots.drain(..) {
                retired.push_erased(slot.type_slot);
                retired.push_erased(slot.instance);
            }
            *max_id = -1;
        } else {
            self.storage = AttributeStorage::Live {
                slots: Vec::new(),
                max_id: -1,
            };
        }
        Ok(())
    }
    pub fn get_or_add(
        &mut self,
        type_ref: &AttributeTypeRef,
        retired: &mut RetiredComponents,
    ) -> Result<AttributeRef, RuntimeError> {
        let AttributeStorage::Live { slots, max_id } = &mut self.storage else {
            return Err(unresolved());
        };
        if let Some(slot) = slots.iter().find(|s| Rc::ptr_eq(&s.type_slot, type_ref)) {
            return Ok(Rc::clone(&slot.instance));
        }
        let (id, kind) = {
            let d = type_ref
                .try_borrow()
                .map_err(|_| RuntimeError::BorrowedTransient("AttributeType"))?;
            (d.id, d.kind)
        };
        let instance = allocate(kind)?;
        retired.push_erased(instance.clone());
        instance
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?
            .set_type(type_ref)?;
        *max_id = (*max_id).max(id);
        slots.push(RuntimeAttributeSlot {
            id,
            type_slot: Rc::clone(type_ref),
            instance: Rc::clone(&instance),
        });
        Ok(instance)
    }
    pub fn set_value(
        &mut self,
        type_ref: &AttributeTypeRef,
        value: AttributeValue,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        let instance = self.get_or_add(type_ref, retired)?;
        let definition = type_ref
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeType"))?;
        if definition.read_only {
            return Err(RuntimeError::UnsupportedRuntime(
                "Readonly attribute setter requires native diagnostic host",
            ));
        }
        let value = validate(&definition.numeric_validation, value)?;
        let mut instance = instance
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
        let matches = matches!(
            (&value, instance.kind),
            (AttributeValue::Float(_), AttributeKind::Float)
                | (AttributeValue::Int(_), AttributeKind::Int)
        );
        if !matches {
            return Err(unsupported_kind());
        }
        instance.value = value;
        Ok(())
    }
    fn put_from_script(
        &mut self,
        type_ref: &AttributeTypeRef,
        value: &str,
        notices: &mut Vec<RuntimeNativeNotice>,
        retired: &mut RetiredComponents,
    ) -> Result<bool, RuntimeError> {
        let instance = self.get_or_add(type_ref, retired)?;
        let result = instance
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?
            .set_script_value(value, notices);
        result
    }
    pub(crate) fn copy_from(
        &mut self,
        source: &Self,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        self.clear(retired)?;
        let AttributeStorage::Live {
            slots: source_slots,
            max_id: source_max,
        } = &source.storage
        else {
            return Err(unresolved());
        };
        for slot in source_slots {
            let original = slot
                .instance
                .try_borrow()
                .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
            let type_ref = original.type_slot.as_ref().ok_or_else(null_pointer)?;
            let instance = allocate(original.kind)?;
            retired.push_erased(instance.clone());
            {
                let mut copied = instance
                    .try_borrow_mut()
                    .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
                copied.set_type(type_ref)?;
                copied.value = original.value.clone();
            }
            if let AttributeStorage::Live { slots, .. } = &mut self.storage {
                slots.push(RuntimeAttributeSlot {
                    id: slot.id,
                    type_slot: Rc::clone(type_ref),
                    instance,
                });
            }
        }
        if let AttributeStorage::Live { max_id, .. } = &mut self.storage {
            *max_id = *source_max;
        }
        Ok(())
    }
    pub(crate) fn archive_copy(&self) -> Result<Self, RuntimeError> {
        match &self.storage {
            AttributeStorage::Archive(value) => Ok(Self::from_archive(value.clone())),
            AttributeStorage::Live { slots, max_id } => {
                let mut copied = Vec::with_capacity(slots.len());
                for slot in slots {
                    let instance = slot
                        .instance
                        .try_borrow()
                        .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?;
                    copied.push(RuntimeAttributeSlot {
                        id: slot.id,
                        type_slot: Rc::clone(&slot.type_slot),
                        instance: Rc::new(RefCell::new(RuntimeAttributeInstance {
                            kind: instance.kind,
                            type_slot: instance.type_slot.clone(),
                            value: instance.value.clone(),
                        })),
                    });
                }
                Ok(Self {
                    storage: AttributeStorage::Live {
                        slots: copied,
                        max_id: *max_id,
                    },
                })
            }
        }
    }
    pub(crate) fn load(
        &mut self,
        reader: &mut Reader<'_>,
        ctx: &dyn EntityContext,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        self.clear(retired)?;
        let mode = reader.i8()?;
        if mode == 0 {
            return Ok(());
        }
        if mode == 1 {
            let count = reader.i8()?;
            if count == 0 {
                return Ok(());
            }
            let headers = reader.pos() as i32;
            let mut data = headers.wrapping_add((count as i32).wrapping_mul(8));
            for index in 0..count.max(0) as i32 {
                reader.seek_java(headers.wrapping_add(index.wrapping_mul(8)))?;
                let bits = reader.i64()?;
                reader.seek_java(data)?;
                for bit in 0..64 {
                    if bits & 1i64.wrapping_shl(bit as u32) != 0 {
                        self.load_one(reader, ctx, (index * 64 + bit) as i16, retired)?;
                    }
                }
                data = reader.pos() as i32;
            }
        } else {
            let count = reader.i16()?;
            for _ in 0..count.max(0) {
                let id = if mode == 8 {
                    reader.i8()? as i16
                } else {
                    reader.i16()?
                };
                self.load_one(reader, ctx, id, retired)?;
            }
        }
        Ok(())
    }
    fn load_one(
        &mut self,
        reader: &mut Reader<'_>,
        ctx: &dyn EntityContext,
        id: i16,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        let type_ref = match ctx.attribute_type_ref(id) {
            Lookup::Found(reference) => reference,
            Lookup::Missing => {
                return Err(RuntimeError::Codec(EntityError(
                    "Unable to read attribute type.".into(),
                )))
            }
            Lookup::Unknown => {
                return Err(RuntimeError::UnsupportedRuntime(
                    "Actual AttributeType registry is unavailable",
                ))
            }
        };
        let instance = self.get_or_add(&type_ref, retired)?;
        let result = instance
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?
            .load(reader);
        result
    }
    pub(crate) fn write(&self, writer: &mut Writer) -> Result<(), RuntimeError> {
        let AttributeStorage::Live { slots, max_id } = &self.storage else {
            if let AttributeStorage::Archive(value) = &self.storage {
                value.save(writer);
            }
            return Ok(());
        };
        if *max_id == -1 {
            writer.i8(0);
            return Ok(());
        }
        let id_bits = if *max_id > 127 { 16 } else { 8 };
        let storages = 1 + *max_id as i32 / 64;
        if (slots.len() as i32).wrapping_mul(id_bits).wrapping_add(16) > storages.wrapping_mul(64) {
            writer.i8(1);
            writer.i8(storages as i8);
            let start = writer.pos();
            for _ in 0..storages.max(0) {
                writer.i64(0);
            }
            let mut masks = vec![0i64; storages.max(0) as usize];
            for slot in slots {
                let index = (slot.id as i32 / 64) as usize;
                let Some(mask) = masks.get_mut(index) else {
                    return Err(RuntimeError::Codec(EntityError(
                        "IndexOutOfBoundsException".into(),
                    )));
                };
                *mask |= 1i64.wrapping_shl(slot.id as u32);
                writer.buf[start + index * 8..start + index * 8 + 8]
                    .copy_from_slice(&mask.to_be_bytes());
                slot.instance
                    .try_borrow()
                    .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?
                    .write(writer)?;
            }
        } else {
            writer.i8(id_bits as i8);
            writer.i16(slots.len() as i16);
            for slot in slots {
                if id_bits == 8 {
                    writer.i8(slot.id as i8);
                } else {
                    writer.i16(slot.id);
                }
                slot.instance
                    .try_borrow()
                    .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?
                    .write(writer)?;
            }
        }
        Ok(())
    }
}

fn unresolved() -> RuntimeError {
    RuntimeError::UnsupportedRuntime(
        "Archive attribute values require actual registry resolution before live access",
    )
}

/// Native lazy script template creation. The cache owns the actual component
/// allocation obtained from the same component class pool as item receivers.
pub fn get_or_create_attributes_template(
    script: &pz_scripts::objects::entity::components::attributes::AttributesScript,
    ctx: &dyn EntityContext,
    debug: &dyn Fn() -> HostResult<bool>,
    notices: &mut Vec<RuntimeNativeNotice>,
    retired: &mut RetiredComponents,
) -> HostResult<Option<ComponentRef>> {
    if script.template_created() {
        let slot = script
            .template_slot()
            .map_err(|_| RuntimeHostError::Borrowed("AttributesScript template"))?;
        let cached = slot.as_ref().cloned();
        drop(slot);
        return cached
            .map(|reference| {
                Rc::downcast::<RefCell<crate::RuntimeComponent>>(reference).map_err(|reference| {
                    retired.push_erased(reference);
                    RuntimeHostError::InvalidHostState(
                        "AttributesScript template has wrong actual class",
                    )
                })
            })
            .transpose();
    }
    if script.kv_pairs().is_empty() {
        script.mark_template_created();
        return get_or_create_attributes_template(script, ctx, debug, notices, retired);
    }
    let template = crate::runtime_pool::obtain_component(ComponentType::Attributes)
        .map_err(attribute_pool_error)?;
    retired.push_component(Rc::clone(&template));
    let erased: Rc<dyn std::any::Any> = template.clone();
    if let Some(old) = script
        .replace_template(Some(&erased))
        .map_err(|_| RuntimeHostError::Borrowed("AttributesScript template"))?
    {
        retired.push_erased(old);
    }
    for (key, value) in script.kv_pairs().iter() {
        let (ok, type_name) = match ctx.attribute_type_by_name_ref(key) {
            Lookup::Found(type_ref) => {
                let name = type_ref
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("AttributeType"))?
                    .name
                    .clone();
                let ok = template
                    .try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("Attribute template component"))?
                    .attributes_mut()
                    .ok_or(RuntimeHostError::InvalidHostState(
                        "Attribute template class mismatch",
                    ))?
                    .put_from_script(&type_ref, value, notices, retired)
                    .map_err(RuntimeHostError::Storage)?;
                (ok, name)
            }
            Lookup::Missing => (false, "null".to_string()),
            Lookup::Unknown => {
                return Err(RuntimeHostError::Storage(RuntimeError::UnsupportedRuntime(
                    "Actual AttributeType registry is unavailable",
                )))
            }
        };
        if !ok {
            if debug()? {
                notices.push(RuntimeNativeNotice::GeneralException(format!(
                    "java.lang.RuntimeException: Attribute '{type_name}' could not be added."
                )));
                if let Some(old) = script
                    .replace_template(None)
                    .map_err(|_| RuntimeHostError::Borrowed("AttributesScript template"))?
                {
                    retired.push_erased(old);
                }
                script.mark_template_created();
                return Ok(None);
            }
            notices.push(RuntimeNativeNotice::GeneralError(format!(
                "WARNING: Item - > Attribute '{type_name}' could not be added."
            )));
        }
    }
    script.mark_template_created();
    Ok(Some(template))
}

fn attribute_pool_error(error: crate::runtime_pool::PoolError) -> RuntimeHostError {
    use crate::runtime_pool::PoolError;
    match error {
        PoolError::BorrowedFactory => RuntimeHostError::Borrowed("native component factory"),
        PoolError::BorrowedComponent => RuntimeHostError::Borrowed("native component pool body"),
        PoolError::UnsupportedClass(kind) => RuntimeHostError::Unsupported {
            operation: "native component class pool",
            kind: Some(kind),
        },
        PoolError::Runtime(RuntimeError::Codec(error)) => RuntimeHostError::NativeException(error),
        PoolError::Runtime(error) => RuntimeHostError::Storage(error),
        PoolError::OwnerPresent => {
            RuntimeHostError::InvalidHostState("native component pool still has owner")
        }
        PoolError::AlreadyInPool => {
            RuntimeHostError::InvalidHostState("AssertionError: Object already in pool.")
        }
    }
}

#[cfg(test)]
mod tests;
