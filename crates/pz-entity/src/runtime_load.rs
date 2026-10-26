//! Ordered `GameEntity.loadEntity` coordinator over actual component handles.
//!
//! This module owns no entity borrow. The host must implement actual storage,
//! owner transitions, lifecycle, registration and pools, or return an explicit
//! unsupported error. In particular, a Vec insertion is not `addComponent`.
//! This driver does not use the legacy bounded DTO decoder or retain failed
//! native blocks as encoded entries.

use std::rc::Rc;

use crate::io::{EntityError, Reader};
use crate::{ComponentRef, ComponentType, RetiredComponents, RuntimeError};

/// Only `NativeException` is caught inside the authenticated per-block try.
/// Rust borrow conflicts and unimplemented behavior must remain visible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeHostError {
    NativeException(EntityError),
    Borrowed(&'static str),
    Unsupported {
        operation: &'static str,
        kind: Option<ComponentType>,
    },
    InvalidHostState(&'static str),
    Storage(RuntimeError),
}

pub type HostResult<T> = Result<T, RuntimeHostError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeLoadPhase {
    Count,
    BlockStart,
    ReadType,
    ResolveType,
    GetComponent,
    ObtainComponent,
    LoadComponent,
    AddComponent,
    NullComponent,
    LogException,
    BlockEnd,
    Connect,
    PruneSize,
    PruneGet,
    PruneRemove,
    PruneRelease,
    PruneConnect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeComponentIssue {
    pub block_index: usize,
    pub type_id: Option<i16>,
    pub phase: RuntimeLoadPhase,
    pub exception: EntityError,
}

/// All host calls must end their entity/component guards before callbacks.
/// Every detached owning slot goes into `retired` before either Ok or Err;
/// the caller keeps the returned report/failure until its outer guards end.
/// Implementations must not reset/release a fresh component whose load failed.
/// No method here has an implicit no-op lifecycle or pool implementation.
pub trait RuntimeLoadHost {
    /// Read Core.debug at the actual null-component branch, after prior calls.
    fn debug_enabled(&self) -> bool;
    /// Native ComponentType.FromId, including its signed array lookup errors.
    fn resolve_type(&mut self, id: i16) -> HostResult<ComponentType>;
    fn get_component(&mut self, kind: ComponentType) -> HostResult<Option<ComponentRef>>;
    fn component_size(&mut self) -> HostResult<usize>;
    fn component_at(&mut self, index: usize) -> HostResult<Option<ComponentRef>>;
    fn obtain_component(
        &mut self,
        kind: ComponentType,
        retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>>;
    /// Mutate the SAME body with this SAME reader. A body guard may span only
    /// a reviewed callback-free codec/context, never a lifecycle callback.
    fn load_component(
        &mut self,
        component: &ComponentRef,
        reader: &mut Reader<'_>,
        world_version: i32,
        retired: &mut RetiredComponents,
    ) -> HostResult<()>;
    /// Native prechecks, old-owner removal, insertion, owner, onAdded and
    /// manager phases. The loader intentionally ignores a normal false result.
    fn add_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<bool>;
    fn connect_components(&mut self, retired: &mut RetiredComponents) -> HostResult<()>;
    /// Exact membership removal, onRemoved, then owner-clear. Return the
    /// actual removed reference, with partial effects preserved on failure.
    fn remove_component_exact(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>>;
    /// The actual per-class pool/reset policy; this is distinct from Drop.
    fn release_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<()>;
    fn log_exception(&mut self, issue: &NativeComponentIssue) -> HostResult<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeLoadOptions {
    /// Mirrors the optional native loaded list. Item post-load prune requires it.
    pub collect_loaded: bool,
}

#[derive(Debug)]
#[must_use]
pub struct RuntimeLoadReport {
    /// Actual references, including a freshly loaded component rejected by add.
    pub loaded: Vec<ComponentRef>,
    pub issues: Vec<NativeComponentIssue>,
    /// Absolute position in the original caller buffer, including on failure.
    pub position: usize,
    pub retired: RetiredComponents,
    tracks_loaded: bool,
}

#[derive(Debug)]
#[must_use]
pub struct RuntimeLoadFailure {
    pub error: RuntimeHostError,
    pub phase: RuntimeLoadPhase,
    pub block_index: Option<usize>,
    pub component_index: Option<usize>,
    /// Owns the completed prefix and all temporary/detached reference pins.
    pub report: RuntimeLoadReport,
}

fn failure(
    report: RuntimeLoadReport,
    error: RuntimeHostError,
    phase: RuntimeLoadPhase,
    block_index: Option<usize>,
    component_index: Option<usize>,
) -> RuntimeLoadFailure {
    RuntimeLoadFailure {
        error,
        phase,
        block_index,
        component_index,
        report,
    }
}

/// Read from the full original caller buffer. Start/forced End/connect are
/// outside the per-block catch. Even a zero-length block may consume following
/// bytes before End rewinds; an invalid End can fail after owner publication.
pub fn load_native_with<H: RuntimeLoadHost>(
    reader: &mut Reader<'_>,
    world_version: i32,
    host: &mut H,
    options: RuntimeLoadOptions,
) -> Result<RuntimeLoadReport, RuntimeLoadFailure> {
    let mut report = RuntimeLoadReport {
        loaded: Vec::new(),
        issues: Vec::new(),
        position: reader.pos(),
        retired: RetiredComponents::new(),
        tracks_loaded: options.collect_loaded,
    };
    if reader.limit() > i32::MAX as usize {
        return Err(failure(
            report,
            RuntimeHostError::InvalidHostState("original ByteBuffer limit exceeds i32::MAX"),
            RuntimeLoadPhase::Count,
            None,
            None,
        ));
    }
    let count = match reader.i8() {
        Ok(count) => count,
        Err(error) => {
            report.position = reader.pos();
            return Err(failure(
                report,
                RuntimeHostError::NativeException(error),
                RuntimeLoadPhase::Count,
                None,
                None,
            ));
        }
    };
    report.position = reader.pos();
    if count <= 0 {
        return Ok(report);
    }

    for block_index in 0..count as usize {
        let length = match reader.i32() {
            Ok(length) if length >= 0 => length,
            Ok(_) => {
                report.position = reader.pos();
                return Err(failure(
                    report,
                    RuntimeHostError::NativeException(EntityError(
                        "DataBlock possible corruption.".into(),
                    )),
                    RuntimeLoadPhase::BlockStart,
                    Some(block_index),
                    None,
                ));
            }
            Err(error) => {
                report.position = reader.pos();
                return Err(failure(
                    report,
                    RuntimeHostError::NativeException(error),
                    RuntimeLoadPhase::BlockStart,
                    Some(block_index),
                    None,
                ));
            }
        };
        let start = reader.pos() as i32;
        let mut phase = RuntimeLoadPhase::ReadType;
        let mut type_id = None;
        let mut current = None;
        let block_result = (|| {
            let id = reader.i16().map_err(RuntimeHostError::NativeException)?;
            type_id = Some(id);
            phase = RuntimeLoadPhase::ResolveType;
            let kind = host.resolve_type(id)?;
            phase = RuntimeLoadPhase::GetComponent;
            current = host.get_component(kind)?;
            let existing = current.is_some();
            if !existing {
                phase = RuntimeLoadPhase::ObtainComponent;
                current = host.obtain_component(kind, &mut report.retired)?;
            }
            if let Some(component) = &current {
                phase = RuntimeLoadPhase::LoadComponent;
                host.load_component(component, reader, world_version, &mut report.retired)?;
                if !existing {
                    phase = RuntimeLoadPhase::AddComponent;
                    let _added = host.add_component(component, &mut report.retired)?;
                }
                if options.collect_loaded {
                    report.loaded.push(Rc::clone(component));
                }
            } else if host.debug_enabled() {
                phase = RuntimeLoadPhase::NullComponent;
                return Err(RuntimeHostError::NativeException(EntityError(format!(
                    "Component with id '{}' not found.",
                    id
                ))));
            }
            Ok(())
        })();
        // Never perform the final drop of a host/component pin in this driver.
        if let Some(component) = current {
            report.retired.push_component(component);
        }
        report.position = reader.pos();
        match block_result {
            Ok(()) => {}
            Err(RuntimeHostError::NativeException(exception)) => {
                let issue = NativeComponentIssue {
                    block_index,
                    type_id,
                    phase,
                    exception,
                };
                let logged = host.log_exception(&issue);
                report.issues.push(issue);
                if let Err(error) = logged {
                    return Err(failure(
                        report,
                        error,
                        RuntimeLoadPhase::LogException,
                        Some(block_index),
                        None,
                    ));
                }
            }
            Err(error) => return Err(failure(report, error, phase, Some(block_index), None)),
        }
        // Java int overflow and ByteBuffer.position bounds are observable.
        if let Err(error) = reader.seek_java(start.wrapping_add(length)) {
            report.position = reader.pos();
            return Err(failure(
                report,
                RuntimeHostError::NativeException(error),
                RuntimeLoadPhase::BlockEnd,
                Some(block_index),
                None,
            ));
        }
        report.position = reader.pos();
    }
    if let Err(error) = host.connect_components(&mut report.retired) {
        return Err(failure(
            report,
            error,
            RuntimeLoadPhase::Connect,
            None,
            None,
        ));
    }
    Ok(report)
}

/// Invoke only for InventoryItem's entity-data flag after a NORMAL entity
/// load return. Flag-absent callers skip this function. The initial reverse
/// index uses the CURRENT size; each subsequent access sees current storage.
pub fn prune_item_after_entity_load<H: RuntimeLoadHost>(
    host: &mut H,
    mut report: RuntimeLoadReport,
) -> Result<RuntimeLoadReport, RuntimeLoadFailure> {
    if !report.tracks_loaded {
        return Err(failure(
            report,
            RuntimeHostError::InvalidHostState("item prune requires actual loaded tracking"),
            RuntimeLoadPhase::PruneSize,
            None,
            None,
        ));
    }
    let size = match host.component_size() {
        Ok(size) => size,
        Err(error) => {
            return Err(failure(
                report,
                error,
                RuntimeLoadPhase::PruneSize,
                None,
                None,
            ))
        }
    };
    for index in (0..size).rev() {
        let component = match host.component_at(index) {
            Ok(Some(component)) => component,
            Ok(None) => {
                return Err(failure(
                    report,
                    RuntimeHostError::InvalidHostState("current component index is absent"),
                    RuntimeLoadPhase::PruneGet,
                    None,
                    Some(index),
                ))
            }
            Err(error) => {
                return Err(failure(
                    report,
                    error,
                    RuntimeLoadPhase::PruneGet,
                    None,
                    Some(index),
                ))
            }
        };
        let retained = report
            .loaded
            .iter()
            .any(|loaded| Rc::ptr_eq(loaded, &component));
        // Retain the physical index lookup pin through success and all errors.
        report.retired.push_component(Rc::clone(&component));
        if !retained {
            let removed = host.remove_component_exact(&component, &mut report.retired);
            // This local pin has a matching retired pin, so this cannot be a
            // final owner drop even if the host removed membership.
            drop(component);
            match removed {
                Ok(Some(removed)) => {
                    report.retired.push_component(Rc::clone(&removed));
                    let released = host.release_component(&removed, &mut report.retired);
                    drop(removed);
                    if let Err(error) = released {
                        return Err(failure(
                            report,
                            error,
                            RuntimeLoadPhase::PruneRelease,
                            None,
                            Some(index),
                        ));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(failure(
                        report,
                        error,
                        RuntimeLoadPhase::PruneRemove,
                        None,
                        Some(index),
                    ))
                }
            }
            // Native RemoveComponent connects even if the contains check was false.
            if let Err(error) = host.connect_components(&mut report.retired) {
                return Err(failure(
                    report,
                    error,
                    RuntimeLoadPhase::PruneConnect,
                    None,
                    Some(index),
                ));
            }
        }
    }
    for component in report.loaded.drain(..) {
        report.retired.push_component(component);
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
