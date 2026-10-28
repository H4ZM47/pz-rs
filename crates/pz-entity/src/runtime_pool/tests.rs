use super::*;
use crate::components::EntityScriptInfo;
use crate::fluids::{FluidContainer, FluidInstance, FluidRef, SealedFluidProperties};
use crate::{
    AttributeKind, EncodedComponent, EntityContext, EntityOwner, FluidInfo, Lookup,
    RuntimeComponents,
};

struct Context {
    properties_known: bool,
}
impl EntityContext for Context {
    fn attribute_kind(&self, _: i16) -> Option<AttributeKind> {
        None
    }
    fn fluid_by_id(&self, id: i8) -> Lookup<FluidInfo> {
        Lookup::Found(FluidInfo {
            name: format!("Fluid{id}"),
            color: if id == 0 {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [0.0, 0.0, 1.0, 1.0]
            },
            ..Default::default()
        })
    }
    fn fluid_by_name(&self, name: &str) -> Lookup<FluidInfo> {
        Lookup::Found(FluidInfo {
            name: name.into(),
            color: [1.0; 4],
            ..Default::default()
        })
    }
    fn fluid_properties(&self, _: &FluidRef) -> Lookup<Option<SealedFluidProperties>> {
        if self.properties_known {
            Lookup::Found(Some(SealedFluidProperties {
                calories: 2.0,
                poison: 3.0,
                food_sickness_change: 99,
                ..Default::default()
            }))
        } else {
            Lookup::Unknown
        }
    }
}

fn context() -> Context {
    Context {
        properties_known: true,
    }
}
fn body(capacity: f32) -> ComponentRef {
    RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer {
        capacity,
        ..Default::default()
    }))
    .unwrap()
}

#[test]
fn lazy_factory_has_no_prefill_and_obtains_actual_fifo_references() {
    let mut factory = RuntimeComponentFactory::new();
    assert_eq!(factory.initial_size(), 1024);
    assert_eq!(factory.max_size(), i32::MAX);
    assert!(!factory.has_pool(ComponentType::FluidContainer));
    factory.ensure_pool(ComponentType::FluidContainer).unwrap();
    assert_eq!(factory.pool_len(ComponentType::FluidContainer), Some(0));
    let first = factory.obtain(ComponentType::FluidContainer).unwrap();
    let second = factory.obtain(ComponentType::FluidContainer).unwrap();
    assert!(!Rc::ptr_eq(&first, &second));
    let mut retired = RetiredComponents::new();
    factory.release_detached(&first, &mut retired).unwrap();
    factory.release_detached(&second, &mut retired).unwrap();
    assert_eq!(factory.peak_size(ComponentType::FluidContainer), Some(2));
    assert_eq!(factory.pool_slots().count(), 2);
    assert!(Rc::ptr_eq(
        &factory.obtain(ComponentType::FluidContainer).unwrap(),
        &first
    ));
    assert!(Rc::ptr_eq(
        &factory.obtain(ComponentType::FluidContainer).unwrap(),
        &second
    ));
}

#[test]
fn missing_pool_does_not_reset_or_even_require_detached_owner() {
    let component = body(17.5);
    let owner = Rc::new(RefCell::new(5i32));
    let old = component
        .borrow_mut()
        .replace_owner(Some(&EntityOwner::from_ref(&owner)));
    assert!(old.is_none());
    let mut factory = RuntimeComponentFactory::new();
    let mut retired = RetiredComponents::new();
    assert_eq!(
        factory.release_detached(&component, &mut retired).unwrap(),
        ReleaseOutcome::MissingPool
    );
    assert_eq!(component.borrow().fluid().unwrap().capacity, 17.5);
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&owner)));
    factory.ensure_pool(ComponentType::FluidContainer).unwrap();
    assert_eq!(
        factory
            .release_detached(&component, &mut retired)
            .unwrap_err(),
        PoolError::OwnerPresent
    );
    let old = component.borrow_mut().replace_owner(None);
    drop(old);
}

#[test]
fn real_fluid_reset_mutates_same_body_color_and_properties_allocations() {
    let mut factory = RuntimeComponentFactory::new();
    let component = factory.obtain(ComponentType::FluidContainer).unwrap();
    let color = Rc::clone(component.borrow().fluid_color_slot().unwrap());
    let properties = Rc::clone(component.borrow().fluid_properties_slot().unwrap());
    {
        let mut component = component.borrow_mut();
        let fluid = component.fluid_mut().unwrap();
        fluid.capacity = 7.125;
        fluid.rain_catcher = 0.75;
        fluid.custom_drink_sound = Some("RetainedCustom".into());
        fluid.fills_with_clean_water = true;
        fluid.input_locked = true;
        fluid.can_player_empty = false;
        fluid.hidden_amount = true;
        fluid.fluids.push(FluidInstance {
            fluid: Some(FluidRef::Type(0)),
            amount: 0.5,
            custom_color: None,
        });
        component.fluid_runtime_mut().unwrap().temperature = 91.0;
        component.fluid_runtime_mut().unwrap().name_cache = Some("Old mixture".into());
        component.initialize_fluid_data(&context()).unwrap();
    }
    assert_eq!(*color.borrow(), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(properties.borrow().calories, 1.0);
    let mut retired = RetiredComponents::new();
    assert_eq!(
        factory.release_detached(&component, &mut retired).unwrap(),
        ReleaseOutcome::Pooled
    );
    assert!(Rc::ptr_eq(
        component.borrow().fluid_color_slot().unwrap(),
        &color
    ));
    assert!(Rc::ptr_eq(
        component.borrow().fluid_properties_slot().unwrap(),
        &properties
    ));
    assert_eq!(*color.borrow(), [1.0; 4]);
    assert_eq!(*properties.borrow(), SealedFluidProperties::default());
    assert_eq!(
        component.borrow().fluid().unwrap(),
        &FluidContainer::default()
    );
    assert_eq!(
        component.borrow().fluid_runtime().unwrap().temperature,
        22.0
    );
    assert!(component
        .borrow()
        .fluid_runtime()
        .unwrap()
        .name_cache
        .is_none());
    assert!(Rc::ptr_eq(
        &factory.obtain(ComponentType::FluidContainer).unwrap(),
        &component
    ));
}

#[test]
fn a_full_pool_still_resets_but_does_not_retain_the_released_allocation() {
    let mut factory = RuntimeComponentFactory::with_limits(1024, 0);
    let component = factory.obtain(ComponentType::FluidContainer).unwrap();
    component.borrow_mut().fluid_mut().unwrap().capacity = 29.0;
    let weak = Rc::downgrade(&component);
    let mut retired = RetiredComponents::new();
    assert_eq!(
        factory.release_detached(&component, &mut retired).unwrap(),
        ReleaseOutcome::Full
    );
    assert_eq!(component.borrow().fluid().unwrap().capacity, 1.0);
    assert_eq!(factory.pool_len(ComponentType::FluidContainer), Some(0));
    drop(component);
    assert!(weak.upgrade().is_none());
}

#[test]
fn script_reset_keeps_fields_and_actual_cached_script_even_when_pool_is_full() {
    for max in [0, 2] {
        let mut factory = RuntimeComponentFactory::with_limits(1024, max);
        let component = factory.obtain(ComponentType::Script).unwrap();
        let script = Rc::new(RefCell::new(pz_scripts::objects::items::Item::new()));
        let cached = EntityOwner::from_ref(&script);
        assert!(component
            .borrow_mut()
            .set_original_script_actual(true, "Base.RetainedScript".into(), &cached)
            .unwrap()
            .is_none());
        let mut retired = RetiredComponents::new();
        factory.release_detached(&component, &mut retired).unwrap();
        let borrowed = component.borrow();
        assert_eq!(
            borrowed.value_snapshot().unwrap(),
            Component::Script(EntityScriptInfo {
                original_is_item: true,
                original_script: Some("Base.RetainedScript".into())
            })
        );
        assert!(borrowed.script_cache_slot().unwrap().ptr_eq(&cached));
        drop(borrowed);
        if max > 0 {
            component
                .borrow_mut()
                .set_original_script_data(false, "Base.MutatedWhilePooled".into())
                .unwrap();
            let obtained = factory.obtain(ComponentType::Script).unwrap();
            assert!(Rc::ptr_eq(&obtained, &component));
            assert!(
                matches!(obtained.borrow().script_info(), Some(script) if !script.original_is_item && script.original_script.as_deref()==Some("Base.MutatedWhilePooled"))
            );
            assert!(obtained
                .borrow()
                .script_cache_slot()
                .unwrap()
                .ptr_eq(&cached));
        }
    }
}

#[test]
fn duplicate_release_matches_explicit_java_assertion_and_debug_policy() {
    let mut factory = RuntimeComponentFactory::new();
    let component = factory.obtain(ComponentType::FluidContainer).unwrap();
    let mut retired = RetiredComponents::new();
    factory.release_detached(&component, &mut retired).unwrap();
    component.borrow_mut().fluid_mut().unwrap().capacity = 41.0;
    assert_eq!(
        factory
            .release_detached_checked(
                &component,
                &mut retired,
                PoolReleaseOptions {
                    assertions_enabled: true,
                    debug: true
                }
            )
            .unwrap_err(),
        PoolError::AlreadyInPool
    );
    assert_eq!(
        component.borrow().fluid().unwrap().capacity,
        41.0,
        "assertion is before reset"
    );
    factory
        .release_detached_checked(
            &component,
            &mut retired,
            PoolReleaseOptions {
                assertions_enabled: true,
                debug: false,
            },
        )
        .unwrap();
    assert_eq!(factory.pool_len(ComponentType::FluidContainer), Some(2));
    let first = factory.obtain(ComponentType::FluidContainer).unwrap();
    let second = factory.obtain(ComponentType::FluidContainer).unwrap();
    assert!(
        Rc::ptr_eq(&first, &second),
        "native nonasserting duplicate entries are not silently deduplicated"
    );
}

#[test]
fn held_component_or_final_cache_borrow_is_explicit_without_pool_publication() {
    let mut factory = RuntimeComponentFactory::new();
    let component = factory.obtain(ComponentType::FluidContainer).unwrap();
    let mut retired = RetiredComponents::new();
    {
        let _guard = component.borrow_mut();
        assert_eq!(
            factory
                .release_detached(&component, &mut retired)
                .unwrap_err(),
            PoolError::BorrowedComponent
        );
    }
    let color = Rc::clone(component.borrow().fluid_color_slot().unwrap());
    let guard = color.borrow_mut();
    component.borrow_mut().fluid_mut().unwrap().capacity = 19.0;
    assert!(matches!(
        factory.release_detached(&component, &mut retired),
        Err(PoolError::Runtime(RuntimeError::BorrowedTransient(
            "FluidContainer.color"
        )))
    ));
    assert_eq!(component.borrow().fluid().unwrap().capacity, 19.0);
    assert_eq!(factory.pool_len(ComponentType::FluidContainer), Some(0));
    drop(guard);
    factory.release_detached(&component, &mut retired).unwrap();
    assert_eq!(component.borrow().fluid().unwrap().capacity, 1.0);
}

#[test]
fn recalculation_uses_raw_sum_for_color_and_properties_before_capacity_clamp() {
    let component = body(1.0);
    {
        let mut component = component.borrow_mut();
        component.fluid_mut().unwrap().fluids = vec![
            FluidInstance {
                fluid: Some(FluidRef::Type(0)),
                amount: 0.6,
                custom_color: None,
            },
            FluidInstance {
                fluid: Some(FluidRef::Type(1)),
                amount: 0.3995,
                custom_color: None,
            },
        ];
        component.initialize_fluid_data(&context()).unwrap();
    }
    let component = component.borrow();
    let raw_sum = 0.3995f32 + 0.6;
    assert_eq!(component.fluid().unwrap().amount_cache, 1.0);
    let color = component.fluid_color_slot().unwrap().borrow();
    assert_eq!(*color, [0.6 / raw_sum, 0.0, 0.3995 / raw_sum, 1.0]);
    let properties = component.fluid_properties_slot().unwrap().borrow();
    let expected = 2.0 * (raw_sum * (0.6 / raw_sum)) + 2.0 * (raw_sum * (0.3995 / raw_sum));
    assert_eq!(properties.calories, expected);
    assert_eq!(
        properties.food_sickness_change, 0,
        "installed addFromMultiplied omits this integer field"
    );
}

#[test]
fn unknown_properties_are_not_fabricated_and_completed_native_prefix_is_kept() {
    let component = body(2.0);
    {
        let mut component = component.borrow_mut();
        component.fluid_mut().unwrap().fluids.push(FluidInstance {
            fluid: Some(FluidRef::Type(0)),
            amount: 0.75,
            custom_color: None,
        });
        let error = component
            .initialize_fluid_data(&Context {
                properties_known: false,
            })
            .unwrap_err();
        assert!(matches!(error, RuntimeError::UnsupportedRuntime(_)));
        assert_eq!(component.fluid().unwrap().amount_cache, 0.75);
        assert!(component.fluid().unwrap().cache_invalidated);
        assert_eq!(*component.fluid_color_slot().unwrap().borrow(), [1.0; 4]);
        component.initialize_fluid_data(&context()).unwrap();
    }
    assert_eq!(
        *component.borrow().fluid_color_slot().unwrap().borrow(),
        [1.0, 0.0, 0.0, 1.0]
    );
}

#[test]
fn data_outcome_pins_actual_owner_only_when_native_data_phase_runs() {
    let component = body(2.0);
    let owner = Rc::new(RefCell::new(7));
    assert!(component
        .borrow_mut()
        .replace_owner(Some(&EntityOwner::from_ref(&owner)))
        .is_none());
    let skipped = component
        .borrow_mut()
        .fluid_recalculate_data(&context(), false)
        .unwrap();
    assert!(!skipped.recalculated && skipped.owner.is_none());
    let performed = component
        .borrow_mut()
        .fluid_recalculate_data(&context(), true)
        .unwrap();
    assert!(performed.recalculated);
    assert!(performed
        .owner
        .as_ref()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&owner)));
    assert!(
        component.try_borrow_mut().is_ok(),
        "owner phase is driven after component guard release"
    );
    drop(component.borrow_mut().replace_owner(None));
}

#[test]
fn archive_copies_final_caches_independently_and_preflight_is_fallible() {
    let component = body(2.0);
    let color = Rc::clone(component.borrow().fluid_color_slot().unwrap());
    *color.borrow_mut() = [0.1, 0.2, 0.3, 0.4];
    let copy = component.borrow().try_archive_copy_unowned().unwrap();
    let copied_color = Rc::clone(copy.borrow().fluid_color_slot().unwrap());
    assert!(!Rc::ptr_eq(&color, &copied_color));
    assert_eq!(*color.borrow(), *copied_color.borrow());
    *copied_color.borrow_mut() = [0.8; 4];
    assert_eq!(*color.borrow(), [0.1, 0.2, 0.3, 0.4]);
    let guard = color.borrow_mut();
    assert!(matches!(
        component.borrow().check_archive_copy(),
        Err(RuntimeError::BorrowedTransient("FluidContainer.color"))
    ));
    assert!(matches!(
        component.borrow().try_archive_copy_unowned(),
        Err(RuntimeError::BorrowedTransient("FluidContainer.color"))
    ));
    drop(guard);
    component.borrow().check_archive_copy().unwrap();
}

#[test]
fn factory_read_keeps_pooled_mutations_in_fields_the_native_script_does_not_reset() {
    struct NoRand;
    impl crate::factory::FactoryRng for NoRand {
        fn next_int(&mut self, _: i32) -> i32 {
            panic!("default script must not draw")
        }
        fn next_float(&mut self, _: f32, _: f32) -> f32 {
            panic!("default script must not draw")
        }
    }
    let mut factory = RuntimeComponentFactory::new();
    let component = factory.obtain(ComponentType::FluidContainer).unwrap();
    let color = Rc::clone(component.borrow().fluid_color_slot().unwrap());
    let mut retired = RetiredComponents::new();
    factory.release_detached(&component, &mut retired).unwrap();
    {
        let mut component = component.borrow_mut();
        component.fluid_mut().unwrap().fluids.push(FluidInstance {
            fluid: Some(FluidRef::Type(0)),
            amount: 0.25,
            custom_color: None,
        });
        component.fluid_runtime_mut().unwrap().temperature = 87.0;
    }
    let obtained = factory.obtain(ComponentType::FluidContainer).unwrap();
    assert!(Rc::ptr_eq(&component, &obtained));
    obtained
        .borrow_mut()
        .read_fluid_script(
            &pz_scripts::objects::entity::components::fluids::FluidContainerScript::new(),
            &context(),
            &mut NoRand,
            "UnknownScriptName",
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(obtained.borrow().fluid().unwrap().fluids.len(), 1);
    assert_eq!(obtained.borrow().fluid_runtime().unwrap().temperature, 87.0);
    assert!(Rc::ptr_eq(
        obtained.borrow().fluid_color_slot().unwrap(),
        &color
    ));
    assert_eq!(*color.borrow(), [1.0, 0.0, 0.0, 1.0]);
    assert!(
        obtained.borrow().fluid().unwrap().cache_invalidated,
        "native final setCapacity invalidation remains for a subsequent getter"
    );
}

fn notice_script(
    fluids: &str,
) -> pz_scripts::objects::entity::components::fluids::FluidContainerScript {
    use pz_scripts::objects::entity::component_script::ComponentScript;
    let root = pz_scripts::parser::parse(&format!(
        "component FluidContainer {{ Capacity = 4, InitialPercent = 0.5, InputLocked = true, fluids {{ {fluids} }} }}"
    ));
    let mut script = pz_scripts::objects::entity::components::fluids::FluidContainerScript::new();
    script
        .load(&pz_scripts::ScriptManager::new(), &root.children[0])
        .unwrap();
    script
}

struct NoticeContext;
impl EntityContext for NoticeContext {
    fn attribute_kind(&self, _: i16) -> Option<AttributeKind> {
        None
    }
    fn fluid_by_name(&self, name: &str) -> Lookup<FluidInfo> {
        if name == "Unavailable" {
            Lookup::Unknown
        } else {
            Lookup::Missing
        }
    }
}

struct NoNoticeRand;
impl crate::factory::FactoryRng for NoNoticeRand {
    fn next_int(&mut self, _: i32) -> i32 {
        panic!("all initial fluids are selected without RNG")
    }
    fn next_float(&mut self, _: f32, _: f32) -> f32 {
        panic!("equal InitialPercent range returns its bound without RNG")
    }
}

#[test]
fn missing_initial_fluids_keep_native_general_logs_and_successfully_skip() {
    use crate::runtime_fluid::RuntimeNativeNotice;
    let component = body(1.0);
    let script = notice_script("Fluid = MissingOne, Fluid = MissingTwo,");
    let mut notices = Vec::new();
    component
        .borrow_mut()
        .read_fluid_script(
            &script,
            &NoticeContext,
            &mut NoNoticeRand,
            "ActualParentName",
            &mut notices,
        )
        .unwrap();
    assert_eq!(
        notices,
        [
            RuntimeNativeNotice::GeneralLog(
                "FluidContainer -> initial Fluid 'MissingOne' not found! [ActualParentName]".into()
            ),
            RuntimeNativeNotice::GeneralLog(
                "FluidContainer -> initial Fluid 'MissingTwo' not found! [ActualParentName]".into()
            ),
        ]
    );
    assert!(component.borrow().fluid().unwrap().fluids.is_empty());
    assert!(component.borrow().fluid().unwrap().input_locked);
}

#[test]
fn native_warning_prefix_remains_caller_owned_when_later_registry_lookup_fails() {
    use crate::runtime_fluid::RuntimeNativeNotice;
    let component = body(1.0);
    let script = notice_script("Fluid = MissingFirst, Fluid = Unavailable,");
    let mut notices = vec![RuntimeNativeNotice::GeneralLog("earlier native log".into())];
    let result = component.borrow_mut().read_fluid_script(
        &script,
        &NoticeContext,
        &mut NoNoticeRand,
        "ActualParentName",
        &mut notices,
    );
    assert_eq!(
        result,
        Err(RuntimeError::UnsupportedRuntime(
            "Fluid script factory registry is unavailable"
        ))
    );
    assert_eq!(
        notices,
        [
            RuntimeNativeNotice::GeneralLog("earlier native log".into()),
            RuntimeNativeNotice::GeneralLog(
                "FluidContainer -> initial Fluid 'MissingFirst' not found! [ActualParentName]"
                    .into()
            ),
        ]
    );
    assert!(
        !component.borrow().fluid().unwrap().input_locked,
        "a later error retains the native prefix before the final script flags"
    );
}

#[test]
fn encoded_import_edits_preserve_live_identity_and_existing_encoded_order() {
    let live = body(7.0);
    let mut components = RuntimeComponents::new();
    components.push_ref_if_absent(Rc::clone(&live)).unwrap();
    components
        .push_encoded_if_absent(EncodedComponent::new(20, vec![1, 2], 249))
        .unwrap();
    components
        .push_encoded_if_absent(EncodedComponent::new(21, vec![3, 4], 249))
        .unwrap();
    let old = components
        .replace_encoded(20, EncodedComponent::new(20, vec![9], 249))
        .unwrap();
    assert_eq!(old.payload(), &[1, 2]);
    assert!(Rc::ptr_eq(
        &components.get_ref_at(0).unwrap().unwrap(),
        &live
    ));
    let ids: Vec<_> = components
        .entry_slots()
        .map(|slot| match slot {
            crate::RuntimeEntryView::Live { kind, .. } => kind.id(),
            crate::RuntimeEntryView::Encoded(raw) => raw.type_id(),
        })
        .collect();
    assert_eq!(ids, [2, 20, 21]);
    let rejected = components
        .replace_encoded(2, EncodedComponent::new(2, vec![8], 249))
        .unwrap_err();
    assert_eq!(rejected.entries[0].payload(), &[8]);
    assert_eq!(live.borrow().fluid().unwrap().capacity, 7.0);
    let rejected = components
        .push_encoded_if_absent(EncodedComponent::new(2, vec![6], 249))
        .unwrap_err();
    assert_eq!(rejected.entries[0].payload(), &[6]);
    assert!(components.remove_encoded_type_unordered(2).is_none());
    assert_eq!(
        components
            .remove_encoded_type_unordered(20)
            .unwrap()
            .payload(),
        &[9]
    );
}

#[test]
fn unsupported_class_obtain_is_visible_and_does_not_create_a_fake_pool() {
    let mut factory = RuntimeComponentFactory::new();
    assert_eq!(
        factory.obtain(ComponentType::SpriteConfig).unwrap_err(),
        PoolError::UnsupportedClass(ComponentType::SpriteConfig)
    );
    assert!(!factory.has_pool(ComponentType::SpriteConfig));
}

#[test]
fn native_default_color_is_a_captured_mutable_global_and_final_caches_copy_its_current_data() {
    let default = crate::runtime_fluid::fluid_default_color_slot();
    struct Restore(crate::runtime_fluid::FluidColorRef, [f32; 4]);
    impl Drop for Restore {
        fn drop(&mut self) {
            *self.0.borrow_mut() = self.1;
        }
    }
    let previous = *default.borrow();
    let _restore = Restore(Rc::clone(&default), previous);
    *default.borrow_mut() = [0.2, 0.3, 0.4, 0.5];
    let component = body(3.0);
    let cache = Rc::clone(component.borrow().fluid_color_slot().unwrap());
    assert!(!Rc::ptr_eq(&default, &cache));
    assert_eq!(*cache.borrow(), [0.2, 0.3, 0.4, 0.5]);
    *default.borrow_mut() = [0.6, 0.7, 0.8, 0.9];
    assert_eq!(
        *cache.borrow(),
        [0.2, 0.3, 0.4, 0.5],
        "native final cache is a copied Color, not a live default-colour view"
    );
    let outcome = component.borrow_mut().fluid_empty_data(&context()).unwrap();
    assert!(outcome.owner.is_none());
    assert_eq!(*cache.borrow(), [0.6, 0.7, 0.8, 0.9]);
    let guard = default.borrow_mut();
    let rejected = RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer {
        capacity: 17.0,
        ..Default::default()
    }))
    .unwrap_err();
    assert_eq!(
        rejected.error,
        RuntimeError::BorrowedTransient("Color.white")
    );
    assert!(matches!(rejected.component, Component::FluidContainer(fluid) if fluid.capacity==17.0));
    // An archive copy reads its own final cache, not unrelated global white.
    component.borrow().try_archive_copy_unowned().unwrap();
    drop(guard);
}

#[test]
fn script_load_null_name_retains_actual_cached_script_at_the_authenticated_branch() {
    let component =
        RuntimeComponent::new_unowned(Component::Script(EntityScriptInfo::default())).unwrap();
    let script = Rc::new(RefCell::new(pz_scripts::objects::items::Item::new()));
    let cached = EntityOwner::from_ref(&script);
    assert!(component
        .borrow_mut()
        .set_original_script_actual(true, "Base.BeforeLoad".into(), &cached)
        .unwrap()
        .is_none());
    let mut retired = RetiredComponents::new();
    component
        .borrow_mut()
        .load_payload_in_place(
            &mut crate::io::Reader::new(&[0, 0]),
            249,
            &context(),
            &mut retired,
        )
        .unwrap();
    assert!(
        matches!(component.borrow().script_info(), Some(script) if !script.original_is_item && script.original_script.is_none())
    );
    assert!(component
        .borrow()
        .script_cache_slot()
        .unwrap()
        .ptr_eq(&cached));
    let archive = component.borrow().try_archive_copy_unowned().unwrap();
    assert!(archive.borrow().owner_slot().is_none());
    assert!(archive
        .borrow()
        .script_cache_slot()
        .unwrap()
        .ptr_eq(&cached));
    let old = component
        .borrow_mut()
        .replace_script_cache(None)
        .unwrap()
        .unwrap();
    assert!(old.ptr_eq(&cached));
    assert!(component.borrow().script_cache_slot().is_none());
}

#[test]
fn fluid_native_load_does_not_publish_amount_only_cache_before_real_data_phase() {
    let component = body(1.0);
    component.borrow_mut().fluid_mut().unwrap().amount_cache = 17.0;
    let saved = FluidContainer {
        fluids: vec![
            FluidInstance {
                fluid: Some(FluidRef::Type(0)),
                amount: 0.6,
                custom_color: None,
            },
            FluidInstance {
                fluid: Some(FluidRef::Type(1)),
                amount: 0.3995,
                custom_color: None,
            },
        ],
        ..Default::default()
    };
    let mut writer = crate::io::Writer::new();
    saved.save(&mut writer);
    let mut retired = RetiredComponents::new();
    component
        .borrow_mut()
        .load_payload_in_place(
            &mut crate::io::Reader::new(&writer.buf),
            249,
            &context(),
            &mut retired,
        )
        .unwrap();
    assert_eq!(
        component.borrow().fluid().unwrap().amount_cache,
        0.0,
        "native Empty(false) clears the old cache; field decode has not reached recalculateCaches"
    );
    let _update = component
        .borrow_mut()
        .fluid_recalculate_data(&context(), true)
        .unwrap();
    let raw_sum = 0.3995f32 + 0.6;
    let borrowed = component.borrow();
    assert_eq!(borrowed.fluid().unwrap().amount_cache, 1.0);
    assert_eq!(
        *borrowed.fluid_color_slot().unwrap().borrow(),
        [0.6 / raw_sum, 0.0, 0.3995 / raw_sum, 1.0]
    );
}

#[test]
fn failed_native_load_clears_final_cache_contents_before_reading_header_without_notifying() {
    let component = body(5.0);
    let color = Rc::clone(component.borrow().fluid_color_slot().unwrap());
    let properties = Rc::clone(component.borrow().fluid_properties_slot().unwrap());
    {
        let mut component = component.borrow_mut();
        let fluid = component.fluid_mut().unwrap();
        fluid.rain_catcher = 0.75;
        fluid.custom_drink_sound = Some("UnsavedSound".into());
        fluid.fluids.push(FluidInstance {
            fluid: Some(FluidRef::Type(0)),
            amount: 0.5,
            custom_color: None,
        });
        component.initialize_fluid_data(&context()).unwrap();
    }
    assert_eq!(*color.borrow(), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(properties.borrow().calories, 1.0);
    let owner = Rc::new(RefCell::new(9));
    assert!(component
        .borrow_mut()
        .replace_owner(Some(&EntityOwner::from_ref(&owner)))
        .is_none());
    let mut reader = crate::io::Reader::new(&[]);
    let mut retired = RetiredComponents::new();
    let error = component
        .borrow_mut()
        .load_payload_in_place(&mut reader, 249, &context(), &mut retired)
        .unwrap_err();
    assert!(matches!(error, RuntimeError::Codec(_)));
    assert_eq!(reader.pos(), 0);
    assert_eq!(*color.borrow(), [1.0; 4]);
    assert_eq!(*properties.borrow(), SealedFluidProperties::default());
    let borrowed = component.borrow();
    assert!(Rc::ptr_eq(borrowed.fluid_color_slot().unwrap(), &color));
    let fluid = borrowed.fluid().unwrap();
    assert_eq!(fluid.amount_cache, 0.0);
    assert_eq!(fluid.capacity, 1.0);
    assert!(fluid.fluids.is_empty());
    assert_eq!(fluid.rain_catcher, 0.75);
    assert_eq!(fluid.custom_drink_sound.as_deref(), Some("UnsavedSound"));
    assert!(borrowed
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&owner)));
    drop(borrowed);
    let skipped = component
        .borrow_mut()
        .fluid_recalculate_data(&context(), false)
        .unwrap();
    assert!(
        !skipped.recalculated && skipped.owner.is_none(),
        "Empty(false) did not invalidate/notify"
    );
    drop(component.borrow_mut().replace_owner(None));
}
