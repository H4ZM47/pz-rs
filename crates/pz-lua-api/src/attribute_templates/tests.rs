use super::*;
use pz_entity::tables::{
    AttributeTypeInfo, InitialValue, NumericAttributeBounds, NumericAttributeValidation,
};
use pz_entity::{AttributeKind, Lookup, RuntimeError};
use pz_scripts::objects::entity::components::attributes::AttributeTemplateCreator;

struct Restore {
    context: Option<Rc<BasicContext>>,
    notices: Vec<RuntimeNativeNotice>,
    faults: Vec<RuntimeHostError>,
    creator: Option<AttributeTemplateCreator>,
    item: Option<pz_inventory::shared::ItemObserver>,
    object: Option<pz_world::object_slot::ObjectObserver>,
    debug: Option<pz_inventory::component_host::ComponentDebugQuery>,
    logger: Option<pz_inventory::component_host::NativeNoticeLogger>,
}

impl Restore {
    fn new() -> Self {
        Self {
            context: CONTEXT.with(|slot| slot.borrow_mut().take()),
            notices: NOTICES.with(|slot| std::mem::take(&mut *slot.borrow_mut())),
            faults: FAULTS.with(|slot| std::mem::take(&mut *slot.borrow_mut())),
            creator: install_attribute_template_creator(None),
            item: pz_inventory::shared::install_item_observer(None),
            object: pz_world::object_slot::install_object_observer(None),
            debug: pz_inventory::component_host::install_component_debug_query(None),
            logger: pz_inventory::component_host::install_native_notice_logger(None),
        }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
        drop(
            pz_scripts::objects::entity::components::attributes::take_retired_attribute_templates(),
        );
        CONTEXT.with(|slot| *slot.borrow_mut() = self.context.take());
        NOTICES.with(|slot| *slot.borrow_mut() = std::mem::take(&mut self.notices));
        FAULTS.with(|slot| *slot.borrow_mut() = std::mem::take(&mut self.faults));
        install_attribute_template_creator(self.creator);
        pz_inventory::shared::install_item_observer(self.item);
        pz_world::object_slot::install_object_observer(self.object);
        pz_inventory::component_host::install_component_debug_query(self.debug);
        pz_inventory::component_host::install_native_notice_logger(self.logger);
    }
}

fn registry(validation: Lookup<NumericAttributeValidation>) -> AttributeTypes {
    let mut types = AttributeTypes::default();
    types.insert_with_validation(
        AttributeTypeInfo {
            id: 0,
            name: "Sharpness".into(),
            kind: AttributeKind::Float,
            initial: Some(InitialValue::Float(0.25)),
            hidden_ui: true,
        },
        false,
        validation,
    );
    types
}

fn load_definition() -> pz_scripts::ScriptManager {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "pz-actual-attribute-bridge-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let scripts = dir.join("media/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("test.txt"),
        "module Base { entity BridgeTest { component Attributes { Sharpness = 2.0, } } }",
    )
    .unwrap();
    let manager = pz_scripts::ScriptManager::new();
    assert!(!manager.debug);
    let loaded = manager.load(&pz_fs::GameFs::new(dir.clone()), &[]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        loaded.is_ok(),
        "non-debug ScriptManager catches script-hook errors"
    );
    manager
}

#[test]
fn native_host_fault_survives_script_manager_non_debug_error_catch() {
    let _restore = Restore::new();
    install(registry(Lookup::Unknown));
    let manager = load_definition();
    assert!(manager.has_load_errors(false));
    assert!(matches!(
        flush_notices(),
        Err(RuntimeHostError::Storage(RuntimeError::UnsupportedRuntime(
            _
        )))
    ));
    assert!(
        flush_notices().is_ok(),
        "the fatal host result is consumed once"
    );
}

#[test]
fn eager_script_loading_uses_same_registered_type_and_real_bounded_template() {
    let _restore = Restore::new();
    let types = registry(Lookup::Found(NumericAttributeValidation {
        requires_validation: true,
        bounds: Some(NumericAttributeBounds {
            min: InitialValue::Float(0.0),
            max: InitialValue::Float(1.0),
        }),
    }));
    let type_ref = types.get_ref(0).unwrap();
    install(types);
    let manager = load_definition();
    assert!(!manager.has_load_errors(false));
    flush_notices().unwrap();
    let definition = manager
        .get::<pz_scripts::objects::entity::GameEntityScript>(
            pz_scripts::ScriptType::Entity,
            "Base.BridgeTest",
        )
        .unwrap();
    let definition = definition.borrow();
    let script = definition
        .component_scripts()
        .iter()
        .find_map(|script| script.as_any().downcast_ref::<AttributesScript>())
        .unwrap();
    assert!(script.template_created());
    let cached = script.get_template_container().unwrap().unwrap();
    let template = cached
        .downcast::<RefCell<pz_entity::RuntimeComponent>>()
        .unwrap();
    let template = template.borrow();
    let attributes = template.attributes().unwrap();
    assert_eq!(
        attributes.get_value(0).unwrap(),
        Some(pz_entity::components::attributes::AttributeValue::Float(
            1.0
        ))
    );
    let instance = attributes.get_ref(0).unwrap().unwrap().borrow();
    assert!(Rc::ptr_eq(instance.type_slot().unwrap(), &type_ref));
}
