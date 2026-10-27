use super::*;
use crate::runtime_pool::{PoolError, ReleaseOutcome, RuntimeComponentFactory};
use crate::{
    BasicContext, Component, ComponentType, EntityOwner, RetiredComponents, RuntimeComponent,
};
use pz_scripts::objects::entity::component_script::ComponentScript;
use std::cell::RefCell;
use std::rc::Rc;

fn actual() -> crate::ComponentRef {
    RuntimeComponent::new_unowned(Component::Durability(Durability::default())).unwrap()
}

#[test]
fn actual_constructor_and_script_read_keep_native_nullable_material_on_same_component() {
    let actual = actual();
    assert_eq!(
        actual.borrow().durability(),
        Some(&RuntimeDurability::default())
    );
    let parsed = pz_scripts::parser::parse("component Durability { MaxHitPoints = 13.5, }");
    let mut script = DurabilityScript::new();
    script
        .load(&pz_scripts::ScriptManager::new(), &parsed.children[0])
        .unwrap();
    actual.borrow_mut().read_durability_script(&script).unwrap();
    let body = actual.borrow();
    let value = body.durability().unwrap();
    assert_eq!(value.current_hit_points, 13.5);
    assert_eq!(value.max_hit_points, 13.5);
    assert_eq!(value.material, None);
    assert!(matches!(
        body.value_snapshot(),
        Err(RuntimeError::UnsupportedRuntime(_))
    ));
    drop(body);
    actual
        .borrow_mut()
        .read_durability_script(&DurabilityScript::new())
        .unwrap();
    let body = actual.borrow();
    assert_eq!(body.durability().unwrap().current_hit_points, 0.0);
    assert_eq!(body.durability().unwrap().max_hit_points, 0.0);
    assert_eq!(body.durability().unwrap().material, None);
}

#[test]
fn durability_pool_release_is_true_inherited_noop_including_full_and_missing_pool() {
    let mut retired = RetiredComponents::new();
    let mut factory = RuntimeComponentFactory::new();
    let actual = actual();
    {
        let mut body = actual.borrow_mut();
        let value = body.durability_mut().unwrap();
        value.current_hit_points = f32::from_bits(0x7fc0_1234);
        value.max_hit_points = -7.0;
        value.material = None;
    }
    assert_eq!(
        factory.release_detached(&actual, &mut retired).unwrap(),
        ReleaseOutcome::MissingPool
    );
    factory.ensure_pool(ComponentType::Durability).unwrap();
    factory.release_detached(&actual, &mut retired).unwrap();
    let reused = factory.obtain(ComponentType::Durability).unwrap();
    assert!(Rc::ptr_eq(&actual, &reused));
    let mut full = RuntimeComponentFactory::with_limits(1024, 0);
    full.ensure_pool(ComponentType::Durability).unwrap();
    assert_eq!(
        full.release_detached(&actual, &mut retired).unwrap(),
        ReleaseOutcome::Full
    );
    let body = actual.borrow();
    let value = body.durability().unwrap();
    assert_eq!(value.current_hit_points.to_bits(), 0x7fc0_1234);
    assert_eq!(value.max_hit_points, -7.0);
    assert_eq!(value.material, None);
}

#[test]
fn durability_load_resets_before_header_then_keeps_each_completed_scalar_prefix() {
    let actual = actual();
    let mut retired = RetiredComponents::new();
    for bytes in [vec![], vec![0], vec![0, 3, 0x40, 0xc0, 0, 0]] {
        {
            let mut body = actual.borrow_mut();
            let value = body.durability_mut().unwrap();
            value.current_hit_points = 50.0;
            value.max_hit_points = 60.0;
            value.material = None;
        }
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            actual.borrow_mut().load_payload_in_place(
                &mut reader,
                249,
                &BasicContext::default(),
                &mut retired
            ),
            Err(RuntimeError::Codec(_))
        ));
        let body = actual.borrow();
        let value = body.durability().unwrap();
        assert_eq!(
            value.current_hit_points,
            if bytes.len() == 6 { 6.0 } else { 1.0 }
        );
        assert_eq!(value.max_hit_points, 1.0);
        assert_eq!(value.material, Some(MaterialType::Default));
        assert_eq!(reader.pos(), if bytes.len() == 6 { 6 } else { 0 });
    }
}

#[test]
fn invalid_material_load_consumes_name_and_retains_native_default_and_scalar_prefix() {
    let actual = actual();
    let mut writer = Writer::new();
    writer.i16(7);
    writer.f32(-0.0);
    writer.f32(32.0);
    writer.string("NotAMaterial");
    let mut reader = Reader::new(&writer.buf);
    let mut retired = RetiredComponents::new();
    let error = actual
        .borrow_mut()
        .load_payload_in_place(&mut reader, 249, &BasicContext::default(), &mut retired)
        .unwrap_err();
    assert_eq!(
        error,
        RuntimeError::Codec(EntityError(
            "No enum constant zombie.iso.enums.MaterialType.NotAMaterial".into()
        ))
    );
    assert_eq!(reader.pos(), writer.buf.len());
    let body = actual.borrow();
    let value = body.durability().unwrap();
    assert_eq!(value.current_hit_points.to_bits(), (-0.0f32).to_bits());
    assert_eq!(value.max_hit_points, 32.0);
    assert_eq!(value.material, Some(MaterialType::Default));
}

#[test]
fn null_material_save_keeps_zero_header_and_written_floats_without_patching() {
    let actual = actual();
    {
        let mut body = actual.borrow_mut();
        let value = body.durability_mut().unwrap();
        value.current_hit_points = 2.5;
        value.max_hit_points = -0.0;
        value.material = None;
    }
    let mut writer = Writer::new();
    writer.i8(0x55);
    assert_eq!(
        actual.borrow().write_payload(&mut writer).unwrap_err(),
        RuntimeError::Codec(EntityError("NullPointerException".into()))
    );
    let mut expected = vec![0x55, 0, 0];
    expected.extend(2.5f32.to_be_bytes());
    expected.extend((-0.0f32).to_be_bytes());
    assert_eq!(writer.buf, expected);
    assert_eq!(actual.borrow().durability().unwrap().material, None);
}

#[test]
fn successful_durability_save_load_preserves_raw_nan_and_material_and_archive_is_unowned() {
    let actual = actual();
    let owner = Rc::new(RefCell::new(31u8));
    let pin = EntityOwner::from_ref(&owner);
    assert!(actual.borrow_mut().replace_owner(Some(&pin)).is_none());
    {
        let mut body = actual.borrow_mut();
        let value = body.durability_mut().unwrap();
        value.current_hit_points = f32::from_bits(0x7fc0_1234);
        value.max_hit_points = -12.0;
        value.material = Some(MaterialType::Wood);
    }
    let mut writer = Writer::new();
    actual.borrow().write_payload(&mut writer).unwrap();
    assert_eq!(&writer.buf[..2], &[0, 7]);
    let copy = actual.borrow().try_archive_copy_unowned().unwrap();
    assert!(!Rc::ptr_eq(&actual, &copy));
    assert!(copy.borrow().owner_slot().is_none());
    copy.borrow_mut().durability_mut().unwrap().material = None;
    assert_eq!(
        actual.borrow().durability().unwrap().material,
        Some(MaterialType::Wood)
    );
    let mut retired = RetiredComponents::new();
    copy.borrow_mut()
        .load_payload_in_place(
            &mut Reader::new(&writer.buf),
            249,
            &BasicContext::default(),
            &mut retired,
        )
        .unwrap();
    assert_eq!(
        copy.borrow()
            .durability()
            .unwrap()
            .current_hit_points
            .to_bits(),
        0x7fc0_1234
    );
    assert_eq!(
        copy.borrow().durability().unwrap().material,
        Some(MaterialType::Wood)
    );
    let old = actual.borrow_mut().replace_owner(None);
    drop(old);
}

#[test]
fn nullable_archive_copy_preserves_state_without_a_fabricated_nonnull_dto() {
    let actual = actual();
    actual.borrow_mut().durability_mut().unwrap().material = None;
    let copy = actual.borrow().try_archive_copy_unowned().unwrap();
    assert_eq!(copy.borrow().durability().unwrap().material, None);
    assert!(copy.borrow().value_snapshot().is_err());
    actual.borrow_mut().durability_mut().unwrap().material = Some(MaterialType::Metal);
    assert_eq!(copy.borrow().durability().unwrap().material, None);
}

#[test]
fn fieldless_context_menu_uses_actual_class_fifo_and_inherited_empty_load_save_reset() {
    let mut factory = RuntimeComponentFactory::new();
    let actual = factory.obtain(ComponentType::ContextMenuConfig).unwrap();
    assert_eq!(actual.borrow().kind(), ComponentType::ContextMenuConfig);
    let owner = Rc::new(RefCell::new(1u8));
    let owner_pin = EntityOwner::from_ref(&owner);
    assert!(actual
        .borrow_mut()
        .replace_owner(Some(&owner_pin))
        .is_none());
    let mut retired = RetiredComponents::new();
    assert_eq!(
        factory.release_detached(&actual, &mut retired).unwrap_err(),
        PoolError::OwnerPresent
    );
    retired.push_owner(actual.borrow_mut().replace_owner(None).unwrap());
    let mut reader = Reader::new(&[99]);
    actual
        .borrow_mut()
        .load_payload_in_place(&mut reader, 249, &BasicContext::default(), &mut retired)
        .unwrap();
    assert_eq!(reader.pos(), 0);
    let mut writer = Writer::new();
    actual.borrow().write_payload(&mut writer).unwrap();
    assert!(writer.buf.is_empty());
    factory.release_detached(&actual, &mut retired).unwrap();
    let reused = factory.obtain(ComponentType::ContextMenuConfig).unwrap();
    assert!(Rc::ptr_eq(&actual, &reused));
    assert!(reused.borrow().owner_slot().is_none());
    assert_eq!(
        reused.borrow().value_snapshot().unwrap(),
        Component::Empty(ComponentType::ContextMenuConfig)
    );
}
