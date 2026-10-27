//! Selected READY noarg List.of semantic role. This is not the complete JVM
//! ImmutableCollections initializer, archive import or general ListN backend.
use super::*;

pub type NativeListNRef = Rc<RefCell<NativeListN>>;
pub struct NativeListN {
    elements: Option<NativeArrayRef>,
    allow_nulls: bool,
}
impl NativeListN {
    pub const CLASS: &'static str = "java.util.ImmutableCollections$ListN";
    pub fn elements_slot(&self) -> Option<&NativeArrayRef> {
        self.elements.as_ref()
    }
    pub fn allow_nulls_scalar(&self) -> bool {
        self.allow_nulls
    }
    /// Collector-only physical detach, not List.clear or a native field reset.
    pub fn take_for_gc(&mut self) -> Option<NativeArrayRef> {
        self.elements.take()
    }
}
impl Drop for NativeListN {
    fn drop(&mut self) {
        if let Some(array) = self.elements.take() {
            let mut retired = NativeListRetired::new();
            retired.keep_array(&array);
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadyState {
    Uninstalled,
    Installing,
    Ready,
    Failed,
}
struct ReadyRole {
    list: Option<NativeListNRef>,
    state: ReadyState,
}
thread_local! {
    static READY_EMPTY_LIST: RefCell<ReadyRole> = const { RefCell::new(ReadyRole { list: None, state: ReadyState::Uninstalled }) };
}
struct Installation {
    complete: bool,
}
impl Drop for Installation {
    fn drop(&mut self) {
        if !self.complete {
            let _ = READY_EMPTY_LIST.try_with(|role| {
                if let Ok(mut role) = role.try_borrow_mut() {
                    role.state = ReadyState::Failed;
                }
            });
        }
    }
}
/// Explicit early selection of the reviewed bundled-JDK25 READY-role profile.
/// Repeated calls keep the same static owner; no session-reset API exists.
/// This does not choose or claim to run either original CDS/<clinit> branch.
pub fn install_selected_ready_empty_list_role(retired: &mut NativeListRetired) -> ListResult<()> {
    let existing =
        READY_EMPTY_LIST
            .try_with(|role| -> ListResult<Option<NativeListNRef>> {
                let mut role = role
                    .try_borrow_mut()
                    .map_err(|_| borrow("READY List.of role carrier"))?;
                match role.state {
                    ReadyState::Ready => Ok(Some(role.list.clone().ok_or_else(|| {
                        unsupported("READY List.of role missing actual EMPTY_LIST")
                    })?)),
                    ReadyState::Uninstalled => {
                        role.state = ReadyState::Installing;
                        Ok(None)
                    }
                    ReadyState::Installing => Err(unsupported(
                        "reentrant selected READY List.of role installation",
                    )),
                    ReadyState::Failed => Err(unsupported(
                        "failed selected READY List.of role installation",
                    )),
                }
            })
            .map_err(|_| unsupported("READY List.of role carrier unavailable"))??;
    if let Some(list) = existing {
        retired.keep_list_n(&list);
        observe_list_n(&list).map_err(ListError::Metadata)?;
        return Ok(());
    }
    let mut installation = Installation { complete: false };
    // The selected initializer's NEW ListN precedes its Object[0] argument.
    let list = Rc::new(RefCell::new(NativeListN {
        elements: None,
        allow_nulls: false,
    }));
    retired.keep_list_n(&list);
    observe_list_n(&list).map_err(ListError::Metadata)?;
    let array = new_array(0, retired)?;
    list.try_borrow_mut()
        .map_err(|_| borrow("ListN elements constructor publication"))?
        .elements = Some(array.clone());
    list.try_borrow_mut()
        .map_err(|_| borrow("ListN allowNulls constructor publication"))?
        .allow_nulls = false;
    observe_list_n(&list).map_err(ListError::Metadata)?;
    READY_EMPTY_LIST
        .try_with(|role| -> ListResult<()> {
            let mut role = role
                .try_borrow_mut()
                .map_err(|_| borrow("READY List.of EMPTY_LIST publication"))?;
            role.list = Some(list.clone());
            role.state = ReadyState::Ready;
            Ok(())
        })
        .map_err(|_| unsupported("READY List.of role carrier unavailable"))??;
    installation.complete = true;
    Ok(())
}
/// Native noarg List.of: reads the installed permanent role only. Never allocates
/// or silently installs a missing singleton during a runtime getter.
pub fn list_of_empty_ready(retired: &mut NativeListRetired) -> ListResult<NativeListNRef> {
    let list = READY_EMPTY_LIST
        .try_with(|role| -> ListResult<NativeListNRef> {
            let role = role
                .try_borrow()
                .map_err(|_| borrow("READY List.of EMPTY_LIST field"))?;
            match role.state {
                ReadyState::Ready => role
                    .list
                    .clone()
                    .ok_or_else(|| unsupported("READY List.of role missing actual EMPTY_LIST")),
                ReadyState::Uninstalled => Err(unsupported(
                    "selected READY List.of EMPTY_LIST not installed",
                )),
                ReadyState::Installing => Err(unsupported(
                    "selected READY List.of EMPTY_LIST installation in progress",
                )),
                ReadyState::Failed => Err(unsupported(
                    "failed selected READY List.of role installation",
                )),
            }
        })
        .map_err(|_| unsupported("READY List.of role carrier unavailable"))??;
    retired.keep_list_n(&list);
    observe_list_n(&list).map_err(ListError::Metadata)?;
    Ok(list)
}
/// Selected ListN.toArray performs both physical field reads then Arrays.copyOf.
/// It is not a Vec snapshot, the static array itself, or ArrayList.EMPTY.
pub fn list_n_empty_to_array(
    source: &NativeListNRef,
    retired: &mut NativeListRetired,
) -> ListResult<NativeArrayRef> {
    retired.keep_list_n(source);
    let first = source
        .try_borrow()
        .map_err(|_| borrow("ListN toArray first elements field"))?
        .elements
        .clone()
        .ok_or_else(|| unsupported("ListN missing actual elements"))?;
    retired.keep_array(&first);
    let length_source = source
        .try_borrow()
        .map_err(|_| borrow("ListN toArray length elements field"))?
        .elements
        .clone()
        .ok_or_else(|| unsupported("ListN missing actual elements"))?;
    retired.keep_array(&length_source);
    copy_of(&first, length_source.len(), retired)
}
#[cfg(test)]
mod tests;
