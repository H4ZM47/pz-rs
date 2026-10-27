//! Actual ArrayList -> fixed Object[] storage and ordinary ArrayList$Itr.
//!
//! Bundled JDK evidence: runtime-native-shared-list-jdk-conversion-v3-current-
//! jar-20261027.json. Raw operations never perform outward Java-to-Lua conversion.
//! General Collection/ListItr/serialization/concurrency/OOME remain unsupported.
use crate::{gc, Value};
use std::cell::RefCell;
use std::rc::Rc;

mod observation;
mod retirement;
pub use observation::{
    install_observer, observe_array, observe_iterator, observe_list, retry_observation,
    ListObserved, ListObserver, ObservationStatus,
};
pub use retirement::{drain_retired, NativeListRetired};

pub type NativeListRef = Rc<RefCell<NativeArrayList>>;
pub type NativeIteratorRef = Rc<RefCell<NativeArrayListItr>>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListError {
    Borrowed(&'static str),
    Unsupported(&'static str),
    Java {
        class: &'static str,
        message: Option<String>,
    },
    Metadata(gc::RegistrationError),
}
pub type ListResult<T> = Result<T, ListError>;
#[derive(Debug)]
pub enum SearchError<E> {
    List(ListError),
    Equals(E),
}
impl<E> From<ListError> for SearchError<E> {
    fn from(e: ListError) -> Self {
        Self::List(e)
    }
}
/// A genuine Java receiver-equals dispatcher. Arbitrary callback failures pass
/// through unchanged in `SearchError::Equals`; the core never formats them.
pub trait JavaEquals {
    type Error;
    /// The SEARCH argument is the receiver. Called with no node/body guard.
    fn equals(&mut self, search: &Value, stored: &Value) -> Result<bool, Self::Error>;
}
fn borrow(p: &'static str) -> ListError {
    ListError::Borrowed(p)
}
fn unsupported(p: &'static str) -> ListError {
    ListError::Unsupported(p)
}
fn java(class: &'static str) -> ListError {
    ListError::Java {
        class,
        message: None,
    }
}
fn index_error(i: i32, n: i32) -> ListError {
    ListError::Java {
        class: "java.lang.IndexOutOfBoundsException",
        message: Some(format!("Index {i} out of bounds for length {n}")),
    }
}
fn is_index_error(e: &ListError) -> bool {
    matches!(
        e,
        ListError::Java {
            class: "java.lang.IndexOutOfBoundsException"
                | "java.lang.ArrayIndexOutOfBoundsException",
            ..
        }
    )
}
/// Deliberate host allocation boundary, not a substituted Java OOME.
const MAX_HOST_CAPACITY: i32 = 1 << 20;

/// One physical owning handle. Length/class are immutable allocation facts,
/// independent of an element-content borrow. No additional adapter Rc exists.
#[derive(Clone)]
pub struct NativeArrayRef {
    owner: Rc<RefCell<NativeObjectArray>>,
    length: i32,
}
impl NativeArrayRef {
    pub fn owner_slot(&self) -> &Rc<RefCell<NativeObjectArray>> {
        &self.owner
    }
    pub fn len(&self) -> i32 {
        self.length
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.owner, &other.owner)
    }
    pub fn ptr_id(&self) -> usize {
        Rc::as_ptr(&self.owner) as usize
    }
}
pub struct NativeObjectArray {
    slots: Option<Box<[Value]>>,
}
impl NativeObjectArray {
    pub fn value_slots(&self) -> Option<&[Value]> {
        self.slots.as_deref()
    }
    pub fn take_for_gc(&mut self) -> Option<Box<[Value]>> {
        self.slots.take()
    }
}
impl Drop for NativeObjectArray {
    fn drop(&mut self) {
        if let Some(values) = self.slots.take() {
            retirement::retire_payload(retirement::Payload::values(values.into_vec()));
        }
    }
}
pub struct NativeArrayList {
    element_data: Option<NativeArrayRef>,
    size: i32,
    mod_count: i32,
}
impl NativeArrayList {
    pub fn array_slot(&self) -> Option<&NativeArrayRef> {
        self.element_data.as_ref()
    }
    pub fn size_scalar(&self) -> i32 {
        self.size
    }
    pub fn mod_count_scalar(&self) -> i32 {
        self.mod_count
    }
    /// Collector-only detach. It does not reset native size/modCount.
    pub fn take_for_gc(&mut self) -> Option<NativeArrayRef> {
        self.element_data.take()
    }
}
impl Drop for NativeArrayList {
    fn drop(&mut self) {
        if let Some(array) = self.element_data.take() {
            let mut p = retirement::Payload::default();
            p.keep_array(&array);
            retirement::retire_payload(p);
        }
    }
}
pub struct NativeArrayListItr {
    list: Option<NativeListRef>,
    cursor: i32,
    last_ret: i32,
    expected_mod_count: i32,
}
impl NativeArrayListItr {
    pub fn list_slot(&self) -> Option<&NativeListRef> {
        self.list.as_ref()
    }
    pub fn cursor(&self) -> i32 {
        self.cursor
    }
    pub fn last_ret(&self) -> i32 {
        self.last_ret
    }
    pub fn expected_mod_count(&self) -> i32 {
        self.expected_mod_count
    }
    pub fn take_for_gc(&mut self) -> Option<NativeListRef> {
        self.list.take()
    }
}
impl Drop for NativeArrayListItr {
    fn drop(&mut self) {
        if let Some(list) = self.list.take() {
            let mut p = retirement::Payload::default();
            p.keep_list(&list);
            retirement::retire_payload(p);
        }
    }
}

#[derive(Clone)]
struct EmptyArrays {
    empty: NativeArrayRef,
    default: NativeArrayRef,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassArrayState {
    Uninitialized,
    Initializing,
    Ready,
    Failed,
}
struct ClassArrays {
    // Two distinct physical Java static fields. A completed EMPTY publication
    // survives a later DEFAULT allocation/observer failure.
    empty: Option<NativeArrayRef>,
    default: Option<NativeArrayRef>,
    state: ClassArrayState,
}
thread_local! {
    static EMPTY_ARRAYS: RefCell<ClassArrays> = const { RefCell::new(ClassArrays {
        empty: None, default: None, state: ClassArrayState::Uninitialized,
    }) };
}
struct ClassArrayInitialization {
    complete: bool,
}
impl Drop for ClassArrayInitialization {
    fn drop(&mut self) {
        if !self.complete {
            let _ = EMPTY_ARRAYS.try_with(|arrays| {
                if let Ok(mut arrays) = arrays.try_borrow_mut() {
                    arrays.state = ClassArrayState::Failed;
                }
            });
        }
    }
}
fn new_array(n: i32, retired: &mut NativeListRetired) -> ListResult<NativeArrayRef> {
    if !(0..=MAX_HOST_CAPACITY).contains(&n) {
        return Err(unsupported("Object[] host allocation capacity bound"));
    }
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(n as usize)
        .map_err(|_| unsupported("Object[] host allocator unavailable"))?;
    slots.resize_with(n as usize, || Value::Nil);
    let array = NativeArrayRef {
        owner: Rc::new(RefCell::new(NativeObjectArray {
            slots: Some(slots.into_boxed_slice()),
        })),
        length: n,
    };
    retired.keep_array(&array);
    observe_array(&array).map_err(ListError::Metadata)?;
    Ok(array)
}
fn empty_arrays(retired: &mut NativeListRetired) -> ListResult<EmptyArrays> {
    let ready = EMPTY_ARRAYS
        .try_with(|arrays| -> ListResult<Option<EmptyArrays>> {
            let mut arrays = arrays
                .try_borrow_mut()
                .map_err(|_| borrow("ArrayList class statics"))?;
            match arrays.state {
                ClassArrayState::Ready => Ok(Some(EmptyArrays {
                    empty: arrays
                        .empty
                        .clone()
                        .ok_or_else(|| unsupported("ArrayList missing EMPTY static field"))?,
                    default: arrays
                        .default
                        .clone()
                        .ok_or_else(|| unsupported("ArrayList missing DEFAULT static field"))?,
                })),
                ClassArrayState::Uninitialized => {
                    arrays.state = ClassArrayState::Initializing;
                    Ok(None)
                }
                ClassArrayState::Initializing => {
                    Err(unsupported("reentrant ArrayList class initialization"))
                }
                ClassArrayState::Failed => Err(unsupported(
                    "incomplete ArrayList class static initialization",
                )),
            }
        })
        .map_err(|_| unsupported("ArrayList class statics unavailable"))??;
    if let Some(arrays) = ready {
        retired.keep_array(&arrays.empty);
        retired.keep_array(&arrays.default);
        return Ok(arrays);
    }
    let mut initialization = ClassArrayInitialization { complete: false };
    // ArrayList.<clinit>: anewarray PC1 -> putstatic EMPTY PC4. Release the
    // carrier guard before the second allocation/observation at PC8.
    let empty = new_array(0, retired)?;
    EMPTY_ARRAYS
        .try_with(|arrays| -> ListResult<()> {
            arrays
                .try_borrow_mut()
                .map_err(|_| borrow("ArrayList EMPTY static publication"))?
                .empty = Some(empty.clone());
            Ok(())
        })
        .map_err(|_| unsupported("ArrayList class statics unavailable"))??;
    let default = new_array(0, retired)?;
    EMPTY_ARRAYS
        .try_with(|arrays| -> ListResult<()> {
            let mut arrays = arrays
                .try_borrow_mut()
                .map_err(|_| borrow("ArrayList DEFAULT static publication"))?;
            arrays.default = Some(default.clone());
            arrays.state = ClassArrayState::Ready;
            Ok(())
        })
        .map_err(|_| unsupported("ArrayList class statics unavailable"))??;
    initialization.complete = true;
    Ok(EmptyArrays { empty, default })
}
fn raw_list(retired: &mut NativeListRetired) -> ListResult<NativeListRef> {
    let list = Rc::new(RefCell::new(NativeArrayList {
        element_data: None,
        size: 0,
        mod_count: 0,
    }));
    retired.keep_list(&list);
    observe_list(&list).map_err(ListError::Metadata)?;
    Ok(list)
}
fn backing(list: &NativeListRef, retired: &mut NativeListRetired) -> ListResult<NativeArrayRef> {
    retired.keep_list(list);
    let array = list
        .try_borrow()
        .map_err(|_| borrow("ArrayList.elementData"))?
        .element_data
        .clone()
        .ok_or_else(|| unsupported("ArrayList missing actual elementData"))?;
    retired.keep_array(&array);
    Ok(array)
}
fn publish(
    list: &NativeListRef,
    array: &NativeArrayRef,
    retired: &mut NativeListRetired,
) -> ListResult<()> {
    retired.keep_list(list);
    retired.keep_array(array);
    let old = list
        .try_borrow_mut()
        .map_err(|_| borrow("ArrayList.elementData publication"))?
        .element_data
        .replace(array.clone());
    if let Some(old) = old {
        retired.keep_array(&old);
    }
    observe_list(list).map_err(ListError::Metadata)?;
    Ok(())
}
fn inc_mod(list: &NativeListRef, retired: &mut NativeListRetired) -> ListResult<()> {
    retired.keep_list(list);
    let mut b = list
        .try_borrow_mut()
        .map_err(|_| borrow("ArrayList.modCount increment"))?;
    b.mod_count = b.mod_count.wrapping_add(1);
    Ok(())
}
fn set_size(list: &NativeListRef, n: i32) -> ListResult<()> {
    list.try_borrow_mut()
        .map_err(|_| borrow("ArrayList.size publication"))?
        .size = n;
    Ok(())
}
fn check_index(i: i32, size: i32) -> ListResult<()> {
    if i < 0 || i >= size {
        Err(index_error(i, size))
    } else {
        Ok(())
    }
}
fn array_get(a: &NativeArrayRef, i: i32, retired: &mut NativeListRetired) -> ListResult<Value> {
    retired.keep_array(a);
    if i < 0 || i >= a.length {
        return Err(java("java.lang.ArrayIndexOutOfBoundsException"));
    }
    let b = a
        .owner
        .try_borrow()
        .map_err(|_| borrow("Object[] element read"))?;
    let slots = b
        .slots
        .as_ref()
        .ok_or_else(|| unsupported("Object[] detached slots"))?;
    let v = slots[i as usize].clone();
    retired.keep_value(&v);
    Ok(v)
}
fn array_set(
    a: &NativeArrayRef,
    i: i32,
    v: &Value,
    retired: &mut NativeListRetired,
) -> ListResult<()> {
    retired.keep_array(a);
    retired.keep_value(v);
    if i < 0 || i >= a.length {
        return Err(java("java.lang.ArrayIndexOutOfBoundsException"));
    }
    let old = {
        let mut b = a
            .owner
            .try_borrow_mut()
            .map_err(|_| borrow("Object[] element write"))?;
        let slots = b
            .slots
            .as_mut()
            .ok_or_else(|| unsupported("Object[] detached slots"))?;
        std::mem::replace(&mut slots[i as usize], v.clone())
    };
    retired.keep_value(&old);
    Ok(())
}
fn arraycopy(
    a: &NativeArrayRef,
    from: i32,
    to: i32,
    n: i32,
    retired: &mut NativeListRetired,
) -> ListResult<()> {
    retired.keep_array(a);
    if from < 0
        || to < 0
        || n < 0
        || i64::from(from) + i64::from(n) > i64::from(a.length)
        || i64::from(to) + i64::from(n) > i64::from(a.length)
    {
        return Err(java("java.lang.ArrayIndexOutOfBoundsException"));
    }
    if n == 0 {
        return Ok(());
    }
    let mut b = a
        .owner
        .try_borrow_mut()
        .map_err(|_| borrow("Object[] arraycopy"))?;
    let slots = b
        .slots
        .as_mut()
        .ok_or_else(|| unsupported("Object[] detached slots"))?;
    // Exact overlap/memmove semantics, preserving reference identities.
    let copied: Vec<_> = slots[from as usize..(from + n) as usize].to_vec();
    for (offset, value) in copied.into_iter().enumerate() {
        let old = std::mem::replace(&mut slots[to as usize + offset], value);
        retired.keep_value(&old);
    }
    Ok(())
}
fn copy_of(
    a: &NativeArrayRef,
    n: i32,
    retired: &mut NativeListRetired,
) -> ListResult<NativeArrayRef> {
    retired.keep_array(a);
    let copy = new_array(n, retired)?;
    for i in 0..a.length.min(n) {
        let v = array_get(a, i, retired)?;
        array_set(&copy, i, &v, retired)?;
    }
    Ok(copy)
}
/// Constructors support the actual ArrayList class, not arbitrary Collection.
pub fn new_default(retired: &mut NativeListRetired) -> ListResult<NativeListRef> {
    let arrays = empty_arrays(retired)?;
    let list = raw_list(retired)?;
    publish(&list, &arrays.default, retired)?;
    Ok(list)
}
pub fn new_capacity(capacity: i32, retired: &mut NativeListRetired) -> ListResult<NativeListRef> {
    let arrays = empty_arrays(retired)?;
    let list = raw_list(retired)?;
    let array = if capacity > 0 {
        new_array(capacity, retired)?
    } else if capacity == 0 {
        arrays.empty
    } else {
        return Err(ListError::Java {
            class: "java.lang.IllegalArgumentException",
            message: Some(format!("Illegal Capacity: {capacity}")),
        });
    };
    publish(&list, &array, retired)?;
    Ok(list)
}
pub fn new_from_actual(
    source: &NativeListRef,
    retired: &mut NativeListRetired,
) -> ListResult<NativeListRef> {
    retired.keep_list(source);
    let arrays = empty_arrays(retired)?;
    let list = raw_list(retired)?;
    let array = to_array(source, retired)?;
    set_size(&list, array.length)?;
    publish(
        &list,
        if array.length == 0 {
            &arrays.empty
        } else {
            &array
        },
        retired,
    )?;
    Ok(list)
}
pub fn size(list: &NativeListRef, retired: &mut NativeListRetired) -> ListResult<i32> {
    retired.keep_list(list);
    Ok(list
        .try_borrow()
        .map_err(|_| borrow("ArrayList.size"))?
        .size)
}
pub fn is_empty(list: &NativeListRef, retired: &mut NativeListRetired) -> ListResult<bool> {
    Ok(size(list, retired)? == 0)
}
pub fn get_raw(
    list: &NativeListRef,
    index: i32,
    retired: &mut NativeListRetired,
) -> ListResult<Value> {
    check_index(index, size(list, retired)?)?;
    let array = backing(list, retired)?;
    array_get(&array, index, retired)
}
pub fn set_raw(
    list: &NativeListRef,
    index: i32,
    value: &Value,
    retired: &mut NativeListRetired,
) -> ListResult<Value> {
    retired.keep_value(value);
    check_index(index, size(list, retired)?)?;
    let array = backing(list, retired)?;
    let old = array_get(&array, index, retired)?;
    array_set(&array, index, value, retired)?;
    Ok(old)
}
fn grown_length(old: i32, min: i32) -> ListResult<i32> {
    let min_growth = min.wrapping_sub(old);
    let preferred = old >> 1;
    let n = old.wrapping_add(min_growth.max(preferred));
    if n > 0 && n <= i32::MAX - 8 {
        return Ok(n);
    }
    let min_length = old.wrapping_add(min_growth);
    if min_length < 0 {
        return Err(unsupported("ArraysSupport huge length/OOME"));
    }
    Ok(min_length.max(i32::MAX - 8))
}
fn grow(
    list: &NativeListRef,
    min: i32,
    retired: &mut NativeListRetired,
) -> ListResult<NativeArrayRef> {
    let old = backing(list, retired)?;
    let arrays = empty_arrays(retired)?;
    let new = if old.length > 0 || !old.ptr_eq(&arrays.default) {
        copy_of(&old, grown_length(old.length, min)?, retired)?
    } else {
        new_array(10.max(min), retired)?
    };
    publish(list, &new, retired)?;
    Ok(new)
}
pub fn add_raw(
    list: &NativeListRef,
    value: &Value,
    retired: &mut NativeListRetired,
) -> ListResult<bool> {
    retired.keep_list(list);
    retired.keep_value(value);
    inc_mod(list, retired)?;
    let mut array = backing(list, retired)?;
    let n = size(list, retired)?;
    if n == array.length {
        array = grow(list, n.wrapping_add(1), retired)?;
    }
    array_set(&array, n, value, retired)?;
    set_size(list, n.wrapping_add(1))?;
    Ok(true)
}
pub fn insert_raw(
    list: &NativeListRef,
    index: i32,
    value: &Value,
    retired: &mut NativeListRetired,
) -> ListResult<()> {
    retired.keep_value(value);
    let n = size(list, retired)?;
    if index < 0 || index > n {
        return Err(ListError::Java {
            class: "java.lang.IndexOutOfBoundsException",
            message: Some(format!("Index: {index}, Size: {n}")),
        });
    }
    inc_mod(list, retired)?;
    let mut array = backing(list, retired)?;
    if n == array.length {
        array = grow(list, n.wrapping_add(1), retired)?;
    }
    arraycopy(
        &array,
        index,
        index.wrapping_add(1),
        n.wrapping_sub(index),
        retired,
    )?;
    array_set(&array, index, value, retired)?;
    set_size(list, n.wrapping_add(1))
}
fn fast_remove(
    list: &NativeListRef,
    array: &NativeArrayRef,
    index: i32,
    retired: &mut NativeListRetired,
) -> ListResult<()> {
    inc_mod(list, retired)?;
    let new_size = size(list, retired)?.wrapping_sub(1);
    if new_size > index {
        arraycopy(
            array,
            index.wrapping_add(1),
            index,
            new_size.wrapping_sub(index),
            retired,
        )?;
    }
    set_size(list, new_size)?;
    array_set(array, new_size, &Value::Nil, retired)
}
pub fn remove_index_raw(
    list: &NativeListRef,
    index: i32,
    retired: &mut NativeListRetired,
) -> ListResult<Value> {
    check_index(index, size(list, retired)?)?;
    let array = backing(list, retired)?;
    let old = array_get(&array, index, retired)?;
    fast_remove(list, &array, index, retired)?;
    Ok(old)
}
pub fn clear(list: &NativeListRef, retired: &mut NativeListRetired) -> ListResult<()> {
    inc_mod(list, retired)?;
    let array = backing(list, retired)?;
    let n = size(list, retired)?;
    set_size(list, 0)?;
    for i in 0..n {
        array_set(&array, i, &Value::Nil, retired)?;
    }
    Ok(())
}
pub fn ensure_capacity(
    list: &NativeListRef,
    min: i32,
    retired: &mut NativeListRetired,
) -> ListResult<()> {
    let array = backing(list, retired)?;
    if min > array.length && (!array.ptr_eq(&empty_arrays(retired)?.default) || min > 10) {
        inc_mod(list, retired)?;
        grow(list, min, retired)?;
    }
    Ok(())
}
pub fn trim_to_size(list: &NativeListRef, retired: &mut NativeListRetired) -> ListResult<()> {
    inc_mod(list, retired)?;
    let n = size(list, retired)?;
    let old = backing(list, retired)?;
    if n < old.length {
        let new = if n == 0 {
            empty_arrays(retired)?.empty
        } else {
            copy_of(&old, n, retired)?
        };
        publish(list, &new, retired)?;
    }
    Ok(())
}
pub fn to_array(
    list: &NativeListRef,
    retired: &mut NativeListRetired,
) -> ListResult<NativeArrayRef> {
    let array = backing(list, retired)?;
    let n = size(list, retired)?;
    copy_of(&array, n, retired)
}
pub fn add_all_actual(
    list: &NativeListRef,
    source: &NativeListRef,
    retired: &mut NativeListRetired,
) -> ListResult<bool> {
    retired.keep_list(list);
    retired.keep_list(source);
    let values = to_array(source, retired)?;
    inc_mod(list, retired)?;
    if values.length == 0 {
        return Ok(false);
    }
    let mut array = backing(list, retired)?;
    let n = size(list, retired)?;
    if values.length > array.length.wrapping_sub(n) {
        array = grow(list, n.wrapping_add(values.length), retired)?;
    }
    for i in 0..values.length {
        let v = array_get(&values, i, retired)?;
        array_set(&array, n.wrapping_add(i), &v, retired)?;
    }
    set_size(list, n.wrapping_add(values.length))?;
    Ok(true)
}

pub fn index_of<E>(
    list: &NativeListRef,
    search: &Value,
    equals: &mut dyn JavaEquals<Error = E>,
    retired: &mut NativeListRetired,
) -> Result<i32, SearchError<E>> {
    retired.keep_value(search);
    let end = size(list, retired)?;
    let array = backing(list, retired)?;
    for i in 0..end {
        let value = array_get(&array, i, retired)?;
        if search.is_nil() {
            if value.is_nil() {
                return Ok(i);
            }
        } else if equals.equals(search, &value).map_err(SearchError::Equals)? {
            return Ok(i);
        }
    }
    Ok(-1)
}
pub fn contains<E>(
    list: &NativeListRef,
    search: &Value,
    equals: &mut dyn JavaEquals<Error = E>,
    retired: &mut NativeListRetired,
) -> Result<bool, SearchError<E>> {
    Ok(index_of(list, search, equals, retired)? >= 0)
}
pub fn remove_object<E>(
    list: &NativeListRef,
    search: &Value,
    equals: &mut dyn JavaEquals<Error = E>,
    retired: &mut NativeListRetired,
) -> Result<bool, SearchError<E>> {
    retired.keep_value(search);
    let array = backing(list, retired)?;
    let n = size(list, retired)?;
    for i in 0..n {
        let value = array_get(&array, i, retired)?;
        let matched = if search.is_nil() {
            value.is_nil()
        } else {
            equals.equals(search, &value).map_err(SearchError::Equals)?
        };
        if matched {
            fast_remove(list, &array, i, retired)?;
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn contains_all_actual<E>(
    list: &NativeListRef,
    source: &NativeListRef,
    equals: &mut dyn JavaEquals<Error = E>,
    retired: &mut NativeListRetired,
) -> Result<bool, SearchError<E>> {
    retired.keep_list(list);
    let it = iterator_new(source, retired)?;
    while iterator_has_next(&it, retired)? {
        let value = iterator_next_raw(&it, retired)?;
        if !contains(list, &value, equals, retired)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn iterator_new(
    list: &NativeListRef,
    retired: &mut NativeListRetired,
) -> ListResult<NativeIteratorRef> {
    retired.keep_list(list);
    // Child Itr stores this$0 BEFORE Object.<init>, lastRet=-1 thereafter.
    let it = Rc::new(RefCell::new(NativeArrayListItr {
        list: Some(list.clone()),
        cursor: 0,
        last_ret: 0,
        expected_mod_count: 0,
    }));
    retired.keep_iterator(&it);
    observe_iterator(&it).map_err(ListError::Metadata)?;
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.lastRet initializer"))?
        .last_ret = -1;
    let count = list
        .try_borrow()
        .map_err(|_| borrow("Itr.expectedModCount source"))?
        .mod_count;
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.expectedModCount publication"))?
        .expected_mod_count = count;
    Ok(it)
}
fn iterator_list(
    it: &NativeIteratorRef,
    retired: &mut NativeListRetired,
) -> ListResult<NativeListRef> {
    retired.keep_iterator(it);
    let list = it
        .try_borrow()
        .map_err(|_| borrow("Itr.this$0"))?
        .list
        .clone()
        .ok_or_else(|| unsupported("Itr detached actual list"))?;
    retired.keep_list(&list);
    Ok(list)
}
fn check_mod(it: &NativeIteratorRef, list: &NativeListRef) -> ListResult<()> {
    let actual = list
        .try_borrow()
        .map_err(|_| borrow("Itr actual modCount"))?
        .mod_count;
    let expected = it
        .try_borrow()
        .map_err(|_| borrow("Itr.expectedModCount"))?
        .expected_mod_count;
    if actual != expected {
        Err(java("java.util.ConcurrentModificationException"))
    } else {
        Ok(())
    }
}
pub fn iterator_has_next(
    it: &NativeIteratorRef,
    retired: &mut NativeListRetired,
) -> ListResult<bool> {
    retired.keep_iterator(it);
    let cursor = it.try_borrow().map_err(|_| borrow("Itr.cursor"))?.cursor;
    let list = iterator_list(it, retired)?;
    Ok(cursor != size(&list, retired)?)
}
pub fn iterator_next_raw(
    it: &NativeIteratorRef,
    retired: &mut NativeListRetired,
) -> ListResult<Value> {
    let list = iterator_list(it, retired)?;
    check_mod(it, &list)?;
    let i = it.try_borrow().map_err(|_| borrow("Itr.cursor"))?.cursor;
    if i >= size(&list, retired)? {
        return Err(java("java.util.NoSuchElementException"));
    }
    let array = backing(&list, retired)?;
    if i >= array.length {
        return Err(java("java.util.ConcurrentModificationException"));
    }
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.cursor publication"))?
        .cursor = i.wrapping_add(1);
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.lastRet publication"))?
        .last_ret = i;
    array_get(&array, i, retired)
}
pub fn iterator_remove(it: &NativeIteratorRef, retired: &mut NativeListRetired) -> ListResult<()> {
    retired.keep_iterator(it);
    let last = it.try_borrow().map_err(|_| borrow("Itr.lastRet"))?.last_ret;
    if last < 0 {
        return Err(java("java.lang.IllegalStateException"));
    }
    let list = iterator_list(it, retired)?;
    check_mod(it, &list)?;
    match remove_index_raw(&list, last, retired) {
        Ok(value) => retired.keep_value(&value),
        Err(e) if is_index_error(&e) => {
            return Err(java("java.util.ConcurrentModificationException"))
        }
        Err(e) => return Err(e),
    }
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.remove cursor publication"))?
        .cursor = last;
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.remove lastRet publication"))?
        .last_ret = -1;
    let count = list
        .try_borrow()
        .map_err(|_| borrow("Itr.remove modCount source"))?
        .mod_count;
    it.try_borrow_mut()
        .map_err(|_| borrow("Itr.remove expectedModCount publication"))?
        .expected_mod_count = count;
    Ok(())
}
/// Proven scalar Object.equals subset only; unknown classes (even identical
/// unknown pointers) must use a genuine dispatcher or return a capability.
pub struct IntrinsicEquals;
impl JavaEquals for IntrinsicEquals {
    type Error = ListError;
    fn equals(&mut self, left: &Value, right: &Value) -> ListResult<bool> {
        Ok(match left {
            Value::Str(a) => matches!(right,Value::Str(b) if a==b),
            Value::Bool(a) => matches!(right,Value::Bool(b) if a==b),
            Value::Num(a) => matches!(right,Value::Num(b) if double_bits(*a)==double_bits(*b)),
            Value::Object(a) => {
                let a = (&**a as &dyn std::any::Any)
                    .downcast_ref::<crate::table::BoxedFloat>()
                    .ok_or_else(|| unsupported("Object.equals unknown receiver class"))?;
                match right {
                    Value::Object(b) => (&**b as &dyn std::any::Any)
                        .downcast_ref::<crate::table::BoxedFloat>()
                        .is_some_and(|b| a.canonical_bits() == b.canonical_bits()),
                    _ => false,
                }
            }
            Value::Nil => return Err(unsupported("Object.equals null receiver")),
            _ => return Err(unsupported("Object.equals unported receiver class")),
        })
    }
}
fn double_bits(n: f64) -> u64 {
    if n.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        n.to_bits()
    }
}

#[cfg(test)]
mod tests;
