use super::*;

use std::cell::RefCell;
use std::rc::Rc;

use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{ItemClass, ItemTemplate};
use pz_kahlua::TableRef;

fn thread() -> Thread {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_java_util::register(&mut thread);
    crate::bind_items::register(&mut thread);
    crate::expose::expose_enum(&mut thread, COMPONENT_TYPE_CLASS, &["FluidContainer", "Attributes"]);
    thread
}

fn publish(thread: &Thread, name: &str, template: ItemTemplate) -> ItemRef {
    let item = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    thread
        .globals()
        .set_str(name, crate::bind_items::try_item_value(&item).unwrap());
    item
}

fn run(thread: &mut Thread, source: &str) -> Vec<Value> {
    let function =
        pz_kahlua::stdlib::load_string(source, Some("context-menu"), thread.globals().clone())
            .unwrap();
    thread.pcall(&function, &[]).unwrap_or_else(|(error, trace)| {
        panic!("{}\n{trace}", pz_kahlua::exec::java_value_of(&error))
    })
}

fn failure(thread: &mut Thread, source: &str) -> String {
    let function =
        pz_kahlua::stdlib::load_string(source, Some("context-menu"), thread.globals().clone())
            .unwrap();
    let (error, _) = thread.pcall(&function, &[]).unwrap_err();
    pz_kahlua::exec::java_value_of(&error)
}

/// The values as text; a list as `[a, b]`.
fn show(values: &[Value]) -> Vec<String> {
    fn one(v: &Value) -> String {
        match v {
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Object(o) => match o.as_any().downcast_ref::<crate::bind_java_util::JavaList>() {
                Some(list) => format!(
                    "[{}]",
                    list.items.borrow().iter().map(one).collect::<Vec<_>>().join(", ")
                ),
                None => format!("object {}", o.class_name()),
            },
            _ => "other".into(),
        }
    }
    values.iter().map(one).collect()
}

#[test]
fn instance_getters_read_the_fields_set_at_creation() {
    let mut thread = thread();
    let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "Remote");
    t.requires_equipped_both_hands = true;
    t.can_be_remote = true;
    t.remote_controller = true;
    t.activated_item = true;
    let remote = publish(&thread, "remote", t);
    {
        let mut body = remote.borrow_mut();
        body.remote_control_id = 7;
        body.custom_menu_option = Some("Drink".into());
        body.extra_items = Some(vec![3]);
    }
    publish(&thread, "rock", ItemTemplate::new(ItemClass::Normal, "Base", "Rock"));
    let values = run(
        &mut thread,
        "return remote:isRequiresEquippedBothHands(), remote:canBeRemote(), \
         remote:isRemoteController(), remote:getRemoteControlID(), \
         remote:getCustomMenuOption(), remote:haveExtraItems(), remote:canBeActivated()",
    );
    assert_eq!(show(&values), ["true", "true", "true", "7", "Drink", "true", "true"]);
    let values = run(
        &mut thread,
        "return rock:isRequiresEquippedBothHands(), rock:canBeRemote(), \
         rock:isRemoteController(), rock:getRemoteControlID(), \
         rock:getCustomMenuOption(), rock:haveExtraItems(), rock:canBeActivated(), \
         rock:isRecordedMedia(), rock:getMediaData(), rock:getAnimalTracks(), \
         rock:isWaterSource()",
    );
    assert_eq!(
        show(&values),
        ["false", "false", "false", "-1", "nil", "false", "false", "false", "nil", "nil", "false"]
    );
    // An empty extra item list is not "have extra items".
    remote.borrow_mut().extra_items = Some(Vec::new());
    assert_eq!(show(&run(&mut thread, "return remote:haveExtraItems()")), ["false"]);
}

#[test]
fn script_getters_read_the_retained_script_and_fail_without_one() {
    let mut thread = thread();
    let mut script = pz_scripts::objects::items::Item::new();
    script.can_bandage = true;
    script.make_up_type = Some("Lipstick".into());
    script.clothing_item_extra_option = Some(vec!["Hood".into(), "Open".into()]);
    script.attachments_provided = Some(vec!["SmallBeltLeft".into(), "SmallBeltRight".into()]);
    script.attachment_replacement = Some("Bag".into());
    script.attachment_type = Some("Knife".into());
    script.opening_recipe = Some("OpenCannedFood".into());
    script.eat_type = Some("Can".into());
    script.eat_time = 300;
    script.require_in_hand_or_inventory = Some(vec![Some("Lighter".into()), Some("Matches".into())]);
    script.custom_eat_sound = Some("EatingSoup".into());
    let script = ScriptItemRef(Rc::new(RefCell::new(script)));
    let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "Kit");
    t.script = Some(script.clone());
    publish(&thread, "kit", t);
    thread.globals().set_str(
        "script",
        crate::bind_scripts::script_value(Some(&pz_scripts::ScriptRef::from_shared(
            script.0.clone(),
        ))),
    );
    let values = run(
        &mut thread,
        "return kit:isCanBandage(), kit:isTrap(), kit:getMakeUpType(), \
         kit:getClothingItemExtraOption(), kit:getScriptItem() == script, kit:getOpeningRecipe()",
    );
    assert_eq!(show(&values), ["true", "false", "Lipstick", "[Hood, Open]", "true", "OpenCannedFood"]);
    let values = run(
        &mut thread,
        "return kit:getEatType(), kit:getEatTime(), kit:getRequireInHandOrInventory()",
    );
    assert_eq!(show(&values), ["Can", "300", "[Lighter, Matches]"]);
    let values = run(
        &mut thread,
        "return kit:getAttachmentsProvided():size(), kit:getAttachmentsProvided():get(1), \
         kit:getAttachmentReplacement(), kit:getAttachmentType()",
    );
    assert_eq!(show(&values), ["2", "SmallBeltRight", "Bag", "Knife"]);
    let mut t = ItemTemplate::new(ItemClass::Food, "Base", "Stew");
    t.script = Some(script.clone());
    publish(&thread, "stew", t);
    assert_eq!(show(&run(&mut thread, "return stew:getCustomEatSound()")), ["EatingSoup"]);

    // A later script change is visible through the item.
    script.0.borrow_mut().trap = true;
    script.0.borrow_mut().make_up_type = None;
    assert_eq!(show(&run(&mut thread, "return kit:isTrap(), kit:getMakeUpType()")), ["true", "nil"]);

    publish(&thread, "rock", ItemTemplate::new(ItemClass::Normal, "Base", "Rock"));
    assert_eq!(show(&run(&mut thread, "return rock:getScriptItem(), rock:getEatTime()")), ["nil", "0"]);
    assert_eq!(
        show(&run(&mut thread, "return rock:getAttachmentsProvided(), rock:getAttachmentType()")),
        ["nil", "nil"]
    );
    for name in ["isCanBandage", "isTrap", "getMakeUpType", "getClothingItemExtraOption", "getOpeningRecipe", "getEatType", "getRequireInHandOrInventory"] {
        let error = failure(&mut thread, &format!("return rock:{name}()"));
        assert!(error.contains("NullPointerException"), "{name}: {error}");
    }
}

#[test]
fn hand_weapon_getters_read_the_weapon_fields() {
    let mut thread = thread();
    let mut t = ItemTemplate::new(ItemClass::Weapon, "Base", "PipeBomb");
    t.can_be_placed = true;
    t.sensor_range = 2;
    t.explosion_timer = 50;
    t.fire_mode_possibilities = Some(Rc::new(vec!["Single".into(), "Auto".into()]));
    let bomb = publish(&thread, "bomb", t);
    publish(&thread, "bat", ItemTemplate::new(ItemClass::Weapon, "Base", "Bat"));
    let values = run(
        &mut thread,
        "return bomb:canBePlaced(), bomb:getSensorRange(), bomb:getExplosionTimer(), \
         bomb:getFireModePossibilities(), bat:canBePlaced(), bat:getFireModePossibilities(), \
         bat:getDetachableWeaponParts(nil):size()",
    );
    assert_eq!(show(&values), ["true", "2", "50", "[Single, Auto]", "false", "nil", "0"]);

    // Detaching parts needs the parts as shared items.
    let part = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::WeaponPart, "Base", "Scope")),
        2,
    );
    if let ItemKind::Weapon(w) = &mut bomb.borrow_mut().kind {
        w.attachments.push(part);
    }
    let error = failure(&mut thread, "return bomb:getDetachableWeaponParts(nil)");
    assert!(error.contains("pz.host.Unsupported"), "{error}");
}

#[test]
fn has_component_checks_the_item_components() {
    let mut thread = thread();
    let jug = publish(&thread, "jug", ItemTemplate::new(ItemClass::Normal, "Base", "Jug"));
    let component = pz_entity::RuntimeComponent::new_unowned(pz_entity::Component::FluidContainer(
        pz_entity::FluidContainer::default(),
    ))
    .unwrap();
    jug.borrow_mut()
        .components
        .runtime_mut()
        .push_ref_if_absent(component)
        .unwrap();
    publish(&thread, "rock", ItemTemplate::new(ItemClass::Normal, "Base", "Rock"));
    let globals = thread.globals().clone();
    globals.set_str(
        "fluid",
        crate::expose::enum_value(COMPONENT_TYPE_CLASS, "FluidContainer"),
    );
    globals.set_str(
        "attributes",
        crate::expose::enum_value(COMPONENT_TYPE_CLASS, "Attributes"),
    );
    let values = run(
        &mut thread,
        "return jug:hasComponent(fluid), jug:hasComponent(attributes), \
         rock:hasComponent(fluid), jug:hasComponent(nil)",
    );
    assert_eq!(show(&values), ["true", "false", "false", "false"]);
    let error = failure(&mut thread, "return jug:hasComponent('FluidContainer')");
    assert!(error.contains("ClassCastException"), "{error}");
    pz_inventory::component_host::drain_retired_components();
}

#[test]
fn literature_getters_read_the_pages_and_the_lock() {
    let mut thread = thread();
    let book = publish(&thread, "book", ItemTemplate::new(ItemClass::Literature, "Base", "Notebook"));
    let read = |thread: &mut Thread| run(thread, "return book:canBeWrite(), book:isEmptyPages(), book:getLockedBy()");
    assert_eq!(show(&read(&mut thread)), ["false", "true", "nil"]);
    for (pages, empty) in [(vec!["", ""], true), (vec!["", "note"], false)] {
        if let ItemKind::Literature(l) = &mut book.borrow_mut().kind {
            l.can_be_write = true;
            l.custom_pages = Some(pages.into_iter().map(String::from).collect());
            l.locked_by = Some("42".into());
        }
        assert_eq!(show(&read(&mut thread)), ["true", &empty.to_string(), "42"]);
    }
}

#[test]
fn has_origin_needs_both_origin_attributes_and_one_not_zero() {
    use pz_entity::components::attributes::AttributeValue;
    use pz_entity::tables::{
        AttributeTypeInfo, InitialValue, NumericAttributeBounds, NumericAttributeValidation,
    };
    use pz_entity::{AttributeKind, BasicContext, Lookup, RetiredComponents, RuntimeComponent};

    struct Restore(Rc<dyn crate::bind_items::ItemLuaEnv>);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::bind_items::install_item_env(self.0.clone());
            pz_inventory::component_host::drain_retired_components();
        }
    }
    let _restore = Restore(crate::bind_items::item_env());
    let mut context = BasicContext::default();
    for (id, name) in [(5, "OriginX"), (6, "OriginY")] {
        context.attributes.insert_with_validation(
            AttributeTypeInfo {
                id,
                name: name.into(),
                kind: AttributeKind::Int,
                initial: Some(InitialValue::Int(0)),
                hidden_ui: false,
            },
            false,
            Lookup::Found(NumericAttributeValidation {
                requires_validation: false,
                bounds: Some(NumericAttributeBounds {
                    min: InitialValue::Int(-100_000),
                    max: InitialValue::Int(100_000),
                }),
            }),
        );
    }
    let mut thread = thread();
    let mut item = |name: &str, values: &[(i16, i32)]| {
        let attributes = RuntimeComponent::new_unowned(pz_entity::Component::Attributes(
            Default::default(),
        ))
        .unwrap();
        let mut retired = RetiredComponents::new();
        for &(id, value) in values {
            attributes
                .borrow_mut()
                .attributes_mut()
                .unwrap()
                .set_value(
                    &context.attributes.get_ref(id).unwrap(),
                    AttributeValue::Int(value),
                    &mut retired,
                )
                .unwrap();
        }
        drop(retired);
        let item = publish(&thread, name, ItemTemplate::new(ItemClass::Normal, "Base", name));
        item.borrow_mut()
            .components
            .runtime_mut()
            .push_ref_if_absent(attributes)
            .unwrap();
    };
    item("onlyX", &[(5, 4)]);
    item("zero", &[(5, 0), (6, 0)]);
    item("placed", &[(5, 0), (6, 3)]);
    publish(&thread, "rock", ItemTemplate::new(ItemClass::Normal, "Base", "Rock"));

    // Without the entity registries only an item without attributes answers.
    assert_eq!(show(&run(&mut thread, "return rock:hasOrigin()")), ["false"]);
    let error = failure(&mut thread, "return placed:hasOrigin()");
    assert!(error.contains("pz.host.Unsupported"), "{error}");

    let mut env = crate::bind_items::GameItemEnv::new(None);
    env.entity = Some(Rc::new(context.clone()));
    crate::bind_items::install_item_env(Rc::new(env));
    let values = run(
        &mut thread,
        "return rock:hasOrigin(), onlyX:hasOrigin(), zero:hasOrigin(), placed:hasOrigin()",
    );
    assert_eq!(show(&values), ["false", "false", "false", "true"]);
}
