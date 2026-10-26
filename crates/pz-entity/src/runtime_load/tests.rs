use super::*;
use crate::io::Writer;
use crate::{
    BasicContext, Component, EntityOwner, FluidContainer, RuntimeComponent, RuntimeComponents,
};
use std::cell::RefCell;
use std::rc::Weak;

const VERSION: i32 = 249;

// Instrumented HOST PROTOCOL fixture. Its release operation records a call;
// it does not implement or claim native pool/reset parity. The coordinator
// under test must preserve ordering and errors regardless of host policy.
#[derive(Default)]
struct Entity {
    components: RuntimeComponents,
}

#[derive(Default)]
struct Host {
    entity: Rc<RefCell<Entity>>,
    events: Vec<String>,
    created: Vec<Weak<RefCell<RuntimeComponent>>>,
    fail: Option<(RuntimeLoadPhase, RuntimeHostError)>,
    reject_add: bool,
    substitute: Option<ComponentRef>,
    null_obtain: bool,
    debug: bool,
    enable_debug_on_obtain: bool,
    remove_absent: bool,
}

fn fluid(capacity: f32) -> ComponentRef {
    RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer {
        capacity,
        rain_catcher: 0.875,
        custom_drink_sound: Some("UnsavedSound".into()),
        ..Default::default()
    }))
    .unwrap()
}

impl Host {
    fn insert(&mut self, component: &ComponentRef, retired: &mut RetiredComponents) {
        self.entity
            .borrow_mut()
            .components
            .push_ref_if_absent(Rc::clone(component))
            .unwrap();
        if let Some(owner) = component
            .borrow_mut()
            .replace_owner(Some(&EntityOwner::from_ref(&self.entity)))
        {
            retired.push_owner(owner);
        }
    }

    fn fail_at(&mut self, phase: RuntimeLoadPhase) -> HostResult<()> {
        if self.fail.as_ref().is_some_and(|(at, _)| *at == phase) {
            Err(self.fail.take().unwrap().1)
        } else {
            Ok(())
        }
    }

    fn guards_released(&self, component: Option<&ComponentRef>) {
        assert!(
            self.entity.try_borrow_mut().is_ok(),
            "entity guard crossed host callback"
        );
        if let Some(component) = component {
            assert!(
                component.try_borrow_mut().is_ok(),
                "component guard crossed host callback"
            );
        }
    }
}

impl RuntimeLoadHost for Host {
    fn debug_enabled(&self) -> bool {
        self.debug
    }
    fn resolve_type(&mut self, id: i16) -> HostResult<ComponentType> {
        self.events.push(format!("type:{id}"));
        if id < 0 || id > 26 {
            return Err(RuntimeHostError::NativeException(EntityError(
                "ArrayIndexOutOfBoundsException".into(),
            )));
        }
        Ok(ComponentType::from_id(id))
    }
    fn get_component(&mut self, kind: ComponentType) -> HostResult<Option<ComponentRef>> {
        self.events.push("get".into());
        self.entity
            .borrow()
            .components
            .get_ref(kind)
            .map_err(RuntimeHostError::Storage)
    }
    fn component_size(&mut self) -> HostResult<usize> {
        Ok(self.entity.borrow().components.len())
    }
    fn component_at(&mut self, index: usize) -> HostResult<Option<ComponentRef>> {
        self.events.push(format!("at:{index}"));
        self.entity
            .borrow()
            .components
            .get_ref_at(index)
            .map_err(RuntimeHostError::Storage)
    }
    fn obtain_component(
        &mut self,
        kind: ComponentType,
        _: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        self.events.push("obtain".into());
        self.guards_released(None);
        self.fail_at(RuntimeLoadPhase::ObtainComponent)?;
        self.debug |= self.enable_debug_on_obtain;
        if self.null_obtain {
            return Ok(None);
        }
        let body = Component::create(kind).ok_or(RuntimeHostError::Unsupported {
            operation: "fixture obtain",
            kind: Some(kind),
        })?;
        let component = RuntimeComponent::new_unowned(body)
            .map_err(|error| RuntimeHostError::Storage(error.error))?;
        self.created.push(Rc::downgrade(&component));
        Ok(Some(component))
    }
    fn load_component(
        &mut self,
        component: &ComponentRef,
        reader: &mut Reader<'_>,
        version: i32,
        _: &mut RetiredComponents,
    ) -> HostResult<()> {
        self.events.push("load".into());
        let result = component
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("fixture body"))?
            .load_payload_in_place(reader, version, &BasicContext::default());
        match result {
            Ok(()) => self.fail_at(RuntimeLoadPhase::LoadComponent),
            Err(RuntimeError::Codec(error)) => Err(RuntimeHostError::NativeException(error)),
            Err(error) => Err(RuntimeHostError::Storage(error)),
        }
    }
    fn add_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<bool> {
        self.events.push("add".into());
        self.guards_released(Some(component));
        if self.reject_add {
            if let Some(other) = self.substitute.take() {
                self.insert(&other, retired);
            }
            return Ok(false);
        }
        self.insert(component, retired);
        self.guards_released(Some(component));
        self.fail_at(RuntimeLoadPhase::AddComponent)?;
        Ok(true)
    }
    fn connect_components(&mut self, _: &mut RetiredComponents) -> HostResult<()> {
        self.events.push("connect".into());
        self.guards_released(None);
        self.fail_at(RuntimeLoadPhase::Connect)?;
        self.fail_at(RuntimeLoadPhase::PruneConnect)
    }
    fn remove_component_exact(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        self.events
            .push(format!("remove:{}", component.borrow().kind().id()));
        let removed = self
            .entity
            .borrow_mut()
            .components
            .remove_exact_unordered(component);
        self.guards_released(Some(component));
        if let Some(removed) = &removed {
            retired.push_component(Rc::clone(removed));
        }
        self.fail_at(RuntimeLoadPhase::PruneRemove)?;
        if let Some(removed) = &removed {
            if let Some(owner) = removed.borrow_mut().replace_owner(None) {
                retired.push_owner(owner);
            }
        }
        if self.remove_absent {
            Ok(None)
        } else {
            Ok(removed)
        }
    }
    fn release_component(
        &mut self,
        component: &ComponentRef,
        _: &mut RetiredComponents,
    ) -> HostResult<()> {
        self.events.push("release".into());
        self.guards_released(Some(component));
        self.fail_at(RuntimeLoadPhase::PruneRelease)
    }
    fn log_exception(&mut self, _: &NativeComponentIssue) -> HostResult<()> {
        self.events.push("log".into());
        self.guards_released(None);
        self.fail_at(RuntimeLoadPhase::LogException)
    }
}

fn options() -> RuntimeLoadOptions {
    RuntimeLoadOptions {
        collect_loaded: true,
    }
}

fn block(writer: &mut Writer, id: i16, payload: &[u8]) {
    let start = writer.block_start();
    writer.i16(id);
    writer.bytes(payload);
    writer.block_end(start);
}

fn payload(capacity: f32) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.i16(1);
    writer.f32(capacity);
    writer.buf
}

fn one_fluid(capacity: f32) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.i8(1);
    block(
        &mut writer,
        ComponentType::FluidContainer.id(),
        &payload(capacity),
    );
    writer.buf
}

#[test]
fn shared_cursor_keeps_absolute_completed_prefix_on_error_and_backward_seek() {
    let input = [99, 0x12, 0x34, 88];
    let mut cursor = 1;
    {
        let mut reader = Reader::borrow_cursor(&input, &mut cursor).unwrap();
        assert_eq!(reader.i16().unwrap(), 0x1234);
        assert_eq!(reader.i32().unwrap_err().0, "BufferUnderflowException");
        assert_eq!(reader.pos(), 3);
    }
    assert_eq!(
        cursor, 3,
        "failed caller operation still publishes prior reads"
    );
    {
        let mut reader = Reader::borrow_cursor(&input, &mut cursor).unwrap();
        reader.seek_java(1).unwrap();
        assert_eq!(
            reader.seek_java(-1).unwrap_err().0,
            "IllegalArgumentException"
        );
        assert_eq!(
            reader.seek_java(5).unwrap_err().0,
            "IllegalArgumentException"
        );
        assert_eq!(reader.i8().unwrap(), 0x12);
    }
    assert_eq!(cursor, 2);
    cursor = 5;
    assert!(Reader::borrow_cursor(&input, &mut cursor).is_err());
    assert_eq!(cursor, 5, "invalid initial cursor is not repaired silently");
}

#[test]
fn zero_and_negative_count_skip_connect_but_flag_present_prune_removes_all() {
    for count in [0, -1] {
        let mut host = Host::default();
        let component = fluid(25.0);
        let mut outer_retired = RetiredComponents::new();
        host.insert(&component, &mut outer_retired);
        let input = [count as u8];
        let report =
            load_native_with(&mut Reader::new(&input), VERSION, &mut host, options()).unwrap();
        assert_eq!(report.position, 1);
        assert!(host.events.is_empty());
        assert_eq!(
            host.entity.borrow().components.len(),
            1,
            "flag-absent caller does not prune"
        );
        let report = prune_item_after_entity_load(&mut host, report).unwrap();
        assert_eq!(host.entity.borrow().components.len(), 0);
        assert!(component.borrow().owner_slot().is_none());
        assert_eq!(host.events, ["at:0", "remove:2", "release", "connect"]);
        assert!(report.loaded.is_empty());
        assert!(!report.retired.is_empty());
    }
}

#[test]
fn duplicate_blocks_load_same_allocation_and_callbacks_hold_no_body_or_entity_guard() {
    let mut writer = Writer::new();
    writer.i8(2);
    block(&mut writer, 2, &payload(7.0));
    block(&mut writer, 2, &payload(11.0));
    let mut host = Host::default();
    let report =
        load_native_with(&mut Reader::new(&writer.buf), VERSION, &mut host, options()).unwrap();
    assert_eq!(report.loaded.len(), 2);
    assert!(Rc::ptr_eq(&report.loaded[0], &report.loaded[1]));
    assert_eq!(report.loaded[0].borrow().fluid().unwrap().capacity, 11.0);
    assert!(report.loaded[0]
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&host.entity)));
    assert_eq!(
        host.events,
        ["type:2", "get", "obtain", "load", "add", "type:2", "get", "load", "connect"]
    );
}

#[test]
fn zero_and_one_byte_blocks_read_following_bytes_then_rewind_original_cursor() {
    for length in [0, 1] {
        let mut writer = Writer::new();
        writer.i16(0x1234); // Caller prefix: the entity does not start at zero.
        writer.i8(1);
        writer.i32(length);
        writer.i16(2);
        writer.bytes(&payload(19.25));
        writer.i8(0x66);
        let mut cursor = 2;
        let mut host = Host::default();
        let report = load_native_with(
            &mut Reader::borrow_cursor(&writer.buf, &mut cursor).unwrap(),
            VERSION,
            &mut host,
            options(),
        )
        .unwrap();
        assert_eq!(cursor, 7 + length as usize);
        assert_eq!(report.position, cursor);
        assert!(report.issues.is_empty());
        assert_eq!(report.loaded[0].borrow().fluid().unwrap().capacity, 19.25);
        assert_eq!(host.events.last().unwrap(), "connect");
    }
}

#[test]
fn failed_existing_body_keeps_partial_mutation_owner_and_unsaved_state() {
    let component = fluid(88.0);
    let mut host = Host::default();
    let mut outer_retired = RetiredComponents::new();
    host.insert(&component, &mut outer_retired);
    let mut writer = Writer::new();
    writer.i8(1);
    block(&mut writer, 2, &[0, 1]); // capacity flag, missing f32
    let report =
        load_native_with(&mut Reader::new(&writer.buf), VERSION, &mut host, options()).unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].phase, RuntimeLoadPhase::LoadComponent);
    assert_eq!(report.issues[0].exception.0, "BufferUnderflowException");
    assert_eq!(component.borrow().fluid().unwrap().capacity, 1.0);
    assert_eq!(component.borrow().fluid().unwrap().rain_catcher, 0.875);
    assert_eq!(
        component
            .borrow()
            .fluid()
            .unwrap()
            .custom_drink_sound
            .as_deref(),
        Some("UnsavedSound")
    );
    assert!(component.borrow().owner_slot().is_some());
    assert!(Rc::ptr_eq(
        &component,
        &host
            .entity
            .borrow()
            .components
            .get_ref_at(0)
            .unwrap()
            .unwrap()
    ));
    assert_eq!(host.events, ["type:2", "get", "load", "log", "connect"]);
}

#[test]
fn failed_new_body_is_not_added_released_reset_or_retained_as_encoded() {
    let mut writer = Writer::new();
    writer.i8(1);
    block(&mut writer, 2, &[0, 1]);
    let mut host = Host::default();
    let report =
        load_native_with(&mut Reader::new(&writer.buf), VERSION, &mut host, options()).unwrap();
    assert!(host.entity.borrow().components.is_empty());
    assert!(report.loaded.is_empty());
    assert_eq!(
        host.events,
        ["type:2", "get", "obtain", "load", "log", "connect"]
    );
    let obtained = &host.created[0];
    assert!(
        obtained.upgrade().is_some(),
        "failed pin lives in outer report retirement"
    );
    drop(report);
    assert!(obtained.upgrade().is_none());
}

#[test]
fn caught_unknown_block_skips_to_declared_end_and_positive_all_bad_still_connects() {
    for later_valid in [false, true] {
        let mut writer = Writer::new();
        writer.i8(if later_valid { 2 } else { 1 });
        block(&mut writer, -1, &[0x66, 0x77]);
        if later_valid {
            block(&mut writer, 2, &payload(8.5));
        }
        let mut host = Host::default();
        let report =
            load_native_with(&mut Reader::new(&writer.buf), VERSION, &mut host, options()).unwrap();
        assert_eq!(report.position, writer.buf.len());
        assert_eq!(report.issues[0].phase, RuntimeLoadPhase::ResolveType);
        assert_eq!(report.issues[0].type_id, Some(-1));
        assert_eq!(
            host.entity.borrow().components.len(),
            usize::from(later_valid)
        );
        assert_eq!(report.loaded.len(), usize::from(later_valid));
        assert_eq!(host.events.last().unwrap(), "connect");
    }
}

#[test]
fn null_component_reads_live_debug_and_uses_authenticated_original_id_message() {
    for enable_debug_on_obtain in [false, true] {
        let mut writer = Writer::new();
        writer.i8(1);
        block(&mut writer, 0, &[]);
        let mut host = Host {
            null_obtain: true,
            enable_debug_on_obtain,
            ..Default::default()
        };
        let report =
            load_native_with(&mut Reader::new(&writer.buf), VERSION, &mut host, options()).unwrap();
        assert!(report.loaded.is_empty());
        assert_eq!(report.issues.len(), usize::from(enable_debug_on_obtain));
        if enable_debug_on_obtain {
            assert_eq!(report.issues[0].phase, RuntimeLoadPhase::NullComponent);
            assert_eq!(
                report.issues[0].exception.0,
                "Component with id '0' not found."
            );
        }
        assert_eq!(host.events.last().unwrap(), "connect");
    }
}

#[test]
fn invalid_end_fails_after_add_owner_and_loaded_publication_without_connect() {
    for length in [999, i32::MAX] {
        let mut bytes = one_fluid(44.0);
        bytes[1..5].copy_from_slice(&length.to_be_bytes());
        let mut host = Host::default();
        let failure =
            load_native_with(&mut Reader::new(&bytes), VERSION, &mut host, options()).unwrap_err();
        assert_eq!(failure.phase, RuntimeLoadPhase::BlockEnd);
        assert_eq!(
            failure.error,
            RuntimeHostError::NativeException(EntityError("IllegalArgumentException".into()))
        );
        assert_eq!(failure.report.position, bytes.len());
        assert_eq!(failure.report.loaded.len(), 1);
        assert_eq!(host.entity.borrow().components.len(), 1);
        assert!(failure.report.loaded[0].borrow().owner_slot().is_some());
        assert_eq!(host.events.last().unwrap(), "add");
        assert!(!host
            .events
            .iter()
            .any(|event| event == "connect" || event == "release"));
    }
}

#[test]
fn count_and_block_start_errors_are_fatal_without_native_block_logging() {
    for (input, phase, position) in [
        (vec![], RuntimeLoadPhase::Count, 0),
        (vec![1, 0, 0], RuntimeLoadPhase::BlockStart, 1),
        (vec![1, 255, 255, 255, 255], RuntimeLoadPhase::BlockStart, 5),
    ] {
        let mut host = Host::default();
        let failure =
            load_native_with(&mut Reader::new(&input), VERSION, &mut host, options()).unwrap_err();
        assert_eq!(failure.phase, phase);
        assert_eq!(failure.report.position, position);
        assert!(failure.report.issues.is_empty());
        assert!(host.events.is_empty());
    }
}

#[test]
fn rejected_add_loaded_ref_does_not_retain_same_kind_distinct_component() {
    let substitute = fluid(91.0);
    let mut host = Host {
        reject_add: true,
        substitute: Some(Rc::clone(&substitute)),
        ..Default::default()
    };
    let bytes = one_fluid(17.0);
    let report = load_native_with(&mut Reader::new(&bytes), VERSION, &mut host, options()).unwrap();
    assert_eq!(
        report.loaded.len(),
        1,
        "normal false add still appends actual loaded ref"
    );
    assert!(!Rc::ptr_eq(&report.loaded[0], &substitute));
    assert!(report.loaded[0].borrow().owner_slot().is_none());
    assert_eq!(host.entity.borrow().components.len(), 1);
    let report = prune_item_after_entity_load(&mut host, report).unwrap();
    assert!(host.entity.borrow().components.is_empty());
    assert!(substitute.borrow().owner_slot().is_none());
    assert!(report.loaded.is_empty());
    assert_eq!(
        &host.events[host.events.len() - 4..],
        ["at:0", "remove:2", "release", "connect"]
    );
}

#[test]
fn native_add_exception_preserves_prefix_but_unsupported_add_is_fatal_before_end() {
    let native = RuntimeHostError::NativeException(EntityError(
        "authored native lifecycle exception".into(),
    ));
    let unsupported = RuntimeHostError::Unsupported {
        operation: "onAdded",
        kind: Some(ComponentType::FluidContainer),
    };
    for (error, caught) in [(native, true), (unsupported, false)] {
        let mut bytes = one_fluid(9.0);
        bytes[1..5].copy_from_slice(&12i32.to_be_bytes());
        bytes.extend([0x66; 4]); // End would skip these bytes after successful catch.
        let mut host = Host {
            fail: Some((RuntimeLoadPhase::AddComponent, error.clone())),
            ..Default::default()
        };
        let result = load_native_with(&mut Reader::new(&bytes), VERSION, &mut host, options());
        assert_eq!(
            host.entity.borrow().components.len(),
            1,
            "completed add prefix remains"
        );
        if caught {
            let report = result.unwrap();
            assert_eq!(report.issues.len(), 1);
            assert_eq!(report.position, bytes.len());
            assert!(report.loaded.is_empty());
            assert_eq!(&host.events[host.events.len() - 2..], ["log", "connect"]);
        } else {
            let failure = result.unwrap_err();
            assert_eq!(failure.phase, RuntimeLoadPhase::AddComponent);
            assert_eq!(failure.error, error);
            assert_eq!(failure.report.position, bytes.len() - 4);
            assert!(failure.report.issues.is_empty());
            assert_eq!(host.events.last().unwrap(), "add");
        }
    }
}

#[test]
fn borrowed_component_is_explicit_host_failure_and_never_logged_as_native_exception() {
    let component = fluid(50.0);
    let mut host = Host::default();
    let mut retired = RetiredComponents::new();
    host.insert(&component, &mut retired);
    let body_guard = component.borrow_mut();
    let bytes = one_fluid(8.0);
    let failure =
        load_native_with(&mut Reader::new(&bytes), VERSION, &mut host, options()).unwrap_err();
    assert_eq!(failure.phase, RuntimeLoadPhase::LoadComponent);
    assert_eq!(failure.error, RuntimeHostError::Borrowed("fixture body"));
    assert_eq!(failure.report.position, 7);
    assert!(failure.report.issues.is_empty());
    assert_eq!(host.events, ["type:2", "get", "load"]);
    drop(body_guard);
    assert_eq!(component.borrow().fluid().unwrap().capacity, 50.0);
}

#[test]
fn connect_exception_escapes_without_entering_native_block_catch() {
    let error = RuntimeHostError::NativeException(EntityError("authored connect exception".into()));
    let mut host = Host {
        fail: Some((RuntimeLoadPhase::Connect, error.clone())),
        ..Default::default()
    };
    let bytes = one_fluid(4.0);
    let failure =
        load_native_with(&mut Reader::new(&bytes), VERSION, &mut host, options()).unwrap_err();
    assert_eq!(failure.phase, RuntimeLoadPhase::Connect);
    assert_eq!(failure.error, error);
    assert_eq!(failure.report.loaded.len(), 1);
    assert!(failure.report.issues.is_empty());
    assert_eq!(host.events.last().unwrap(), "connect");
}

#[test]
fn prune_uses_reverse_current_indices_and_connects_after_each_remove_even_absent() {
    for remove_absent in [false, true] {
        let mut host = Host {
            remove_absent,
            ..Default::default()
        };
        let mut retired = RetiredComponents::new();
        for kind in [
            ComponentType::FluidContainer,
            ComponentType::MetaTag,
            ComponentType::Durability,
        ] {
            host.insert(
                &RuntimeComponent::new_unowned(Component::create(kind).unwrap()).unwrap(),
                &mut retired,
            );
        }
        let report =
            load_native_with(&mut Reader::new(&[0]), VERSION, &mut host, options()).unwrap();
        let report = prune_item_after_entity_load(&mut host, report).unwrap();
        assert!(host.entity.borrow().components.is_empty());
        let indices: Vec<_> = host
            .events
            .iter()
            .filter(|e| e.starts_with("at:"))
            .cloned()
            .collect();
        assert_eq!(indices, ["at:2", "at:1", "at:0"]);
        assert_eq!(host.events.iter().filter(|e| *e == "connect").count(), 3);
        assert_eq!(
            host.events.iter().filter(|e| *e == "release").count(),
            if remove_absent { 0 } else { 3 }
        );
        assert!(report.loaded.is_empty());
    }
}

#[test]
fn prune_remove_and_release_failures_keep_actual_completed_prefix_and_pin_owners() {
    for phase in [
        RuntimeLoadPhase::PruneRemove,
        RuntimeLoadPhase::PruneRelease,
        RuntimeLoadPhase::PruneConnect,
    ] {
        let component = fluid(23.0);
        let error =
            RuntimeHostError::NativeException(EntityError("authored prune exception".into()));
        let mut host = Host {
            fail: Some((phase, error.clone())),
            ..Default::default()
        };
        let mut retired = RetiredComponents::new();
        host.insert(&component, &mut retired);
        let report =
            load_native_with(&mut Reader::new(&[0]), VERSION, &mut host, options()).unwrap();
        let failure = prune_item_after_entity_load(&mut host, report).unwrap_err();
        assert_eq!(failure.phase, phase);
        assert_eq!(failure.component_index, Some(0));
        assert_eq!(failure.error, error);
        assert!(host.entity.borrow().components.is_empty());
        assert_eq!(
            component.borrow().owner_slot().is_some(),
            phase == RuntimeLoadPhase::PruneRemove
        );
        assert!(!failure.report.retired.is_empty());
        assert!(
            failure.report.issues.is_empty(),
            "prune errors are outside the entity block catch"
        );
    }
}

#[test]
fn prune_refuses_an_entity_load_without_actual_loaded_tracking() {
    let mut host = Host::default();
    let report = load_native_with(
        &mut Reader::new(&[0]),
        VERSION,
        &mut host,
        RuntimeLoadOptions::default(),
    )
    .unwrap();
    let failure = prune_item_after_entity_load(&mut host, report).unwrap_err();
    assert_eq!(failure.phase, RuntimeLoadPhase::PruneSize);
    assert!(matches!(
        failure.error,
        RuntimeHostError::InvalidHostState(_)
    ));
    assert!(host.events.is_empty());
}
