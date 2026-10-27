//! Genuine static ItemCodeOnCreate entry points over actual item allocations.
//!
//! Only onCreateRecipeMagazine is implemented. Other declared handlers, the
//! public constructor, inherited helpers and factory/callback phase parity are
//! separate capabilities; no replacement Lua callbacks are installed here.

use pz_entity::RetiredComponents;
use pz_inventory::moddata::{get_mod_data_ref, ModDataError};
use pz_kahlua::table::RetiredTableValues;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};

use super::{ItemRef, LuaInventoryItem};

const CLASS: &str = "zombie.scripting.logic.ItemCodeOnCreate";
const SUPERCLASS: &str = "zombie.scripting.logic.RecipeCodeHelper";
const LITERATURE: &str = "zombie.inventory.types.Literature";

/// Native bytecode 2d9de682: getModData (PC1), getFullType (PC7), then
/// rawset("literatureTitle", fullType) (PC10). No name, list or RNG changes.
fn on_create_recipe_magazine(item: &ItemRef) -> LuaResult<()> {
    let table = get_mod_data_ref(item).map_err(|error| error.to_lua_error())?;
    let full_type = {
        let body = item.try_borrow().map_err(|_| {
            ModDataError::Borrowed("ItemCodeOnCreate.onCreateRecipeMagazine getFullType")
                .to_lua_error()
        })?;
        body.get_full_type()
    };
    let value = Value::from(full_type);
    let mut values = RetiredTableValues::new();
    let result = table
        .try_rawset(&Value::str("literatureTitle"), &value, &mut values)
        .map_err(|error| ModDataError::from(error).to_lua_error());
    // Both replaced values and incoming owning pins stay alive past all
    // caller host guards, including when the actual table body is held.
    values.push(value);
    values.push(Value::Table(table));
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(values));
    pz_inventory::component_host::retire_components(retired);
    pz_inventory::shared::observe_item(item);
    result
}

/// Native reference conversion precedes the static invoker's excess-argument
/// check. A Java null is a valid reference argument; it faults in the body.
fn literature_argument(call: &NativeCall<'_>) -> LuaResult<Option<ItemRef>> {
    if call.nargs() == 0 {
        return Err(pz_kahlua::LuaError::runtime("expected 1 argument, got 0"));
    }
    let argument = call.arg(0);
    let item = match &argument {
        Value::Nil => None,
        Value::Object(object) => {
            let known = (&**object as &dyn std::any::Any).downcast_ref::<LuaInventoryItem>();
            match known {
                Some(known) if known.class == LITERATURE => Some(known.item.clone()),
                _ => return Err(argument_type_error(&argument)),
            }
        }
        _ => return Err(argument_type_error(&argument)),
    };
    if call.nargs() != 1 {
        return Err(pz_kahlua::LuaError::runtime(format!(
            "expected 1 argument, got {}",
            call.nargs()
        )));
    }
    Ok(item)
}

fn argument_type_error(argument: &Value) -> pz_kahlua::LuaError {
    // Value::Native retains JavaFunction's interface, not its concrete Java
    // implementation class. Do not invent a getSimpleName diagnostic for it.
    if matches!(argument, Value::Native(_)) {
        return ModDataError::Unsupported(
            "native JavaFunction implementation class in Literature argument diagnostic",
        )
        .to_lua_error();
    }
    let class = match argument {
        Value::Object(object) => {
            match (&**object as &dyn std::any::Any).downcast_ref::<LuaInventoryItem>() {
                Some(item) => item.class.to_owned(),
                None => pz_kahlua::stdlib::java_class_name(argument),
            }
        }
        _ => pz_kahlua::stdlib::java_class_name(argument),
    };
    pz_kahlua::LuaError::runtime(format!(
        "expected argument of type Literature, got {}",
        crate::expose::simple_name(&class)
    ))
}

/// An actual null reference reaches PC1, rather than failing conversion. The
/// native MethodCaller catches its reflected NPE and performs diagnostic state
/// updates before returning void. That state is not represented by this host.
#[derive(Debug)]
enum BodyFailure {
    NullReceiver,
    Host(pz_kahlua::LuaError),
}

fn invoke_body(item: Option<&ItemRef>) -> Result<(), BodyFailure> {
    let item = item.ok_or(BodyFailure::NullReceiver)?;
    on_create_recipe_magazine(item).map_err(BodyFailure::Host)
}

fn invoke(call: &mut NativeCall<'_>) -> LuaResult<usize> {
    let item = literature_argument(call)?;
    match invoke_body(item.as_ref()) {
        Ok(()) => Ok(0),
        Err(BodyFailure::Host(error)) => Err(error),
        Err(BodyFailure::NullReceiver) => Err(ModDataError::Unsupported(
            "MethodCaller null-receiver NPE diagnostic state for onCreateRecipeMagazine",
        )
        .to_lua_error()),
    }
}

pub(super) fn register(thread: &mut Thread) {
    let class = crate::expose::expose_class(thread, CLASS, Some(SUPERCLASS));
    class.static_fn("onCreateRecipeMagazine", invoke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::{Rc, Weak};

    use pz_inventory::template::ScriptItemRef;
    use pz_inventory::{shared, InventoryItem, ItemClass, ItemKind, ItemTemplate};
    use pz_kahlua::{stdlib, TableRef};

    fn thread() -> Thread {
        let mut thread = Thread::new(TableRef::new());
        stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        crate::bind_globals::register(&mut thread);
        crate::bind_items::register(&mut thread);
        thread
    }

    fn actual(thread: &mut Thread, class: ItemClass, name: &str) -> ItemRef {
        let item = shared::try_new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(class, "Base", name)),
            1,
        ))
        .unwrap();
        thread
            .globals()
            .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
        item
    }

    fn function(thread: &Thread) -> Value {
        crate::bind_item_container::function_object(
            thread,
            "ItemCodeOnCreate.onCreateRecipeMagazine",
        )
    }

    fn run(thread: &mut Thread, source: &str) -> Vec<Value> {
        let function = stdlib::load_string(
            source,
            Some("native-onCreateRecipeMagazine"),
            thread.globals().clone(),
        )
        .unwrap();
        thread
            .pcall(&function, &[])
            .unwrap_or_else(|(error, trace)| {
                panic!("{}\n{trace}", pz_kahlua::exec::java_value_of(&error))
            })
    }

    fn failure(thread: &mut Thread, arguments: &[Value]) -> String {
        // Exercise the ordinary Lua OP_CALL path. Calling a NativeFunction
        // directly through Thread::pcall adds KahluaThread.call's separate
        // native-entry exception wrapper, which is not this Lua caller phase.
        let closure = stdlib::load_string(
            "return ItemCodeOnCreate.onCreateRecipeMagazine(...)",
            Some("native-onCreateRecipeMagazine-arguments"),
            thread.globals().clone(),
        )
        .unwrap();
        let (error, _) = thread.pcall(&closure, arguments).unwrap_err();
        error.as_str().expect("native error string").to_owned()
    }

    #[test]
    fn genuine_static_class_writes_same_actual_table_returns_void_and_keeps_other_item_state() {
        let mut thread = thread();
        let item = actual(&mut thread, ItemClass::Literature, "NativeRecipeMagazine");
        item.borrow_mut().name = "custom display name".into();
        item.borrow_mut().id = 74;
        if let ItemKind::Literature(data) = &mut item.borrow_mut().kind {
            data.learned_recipes = Some(vec!["ExistingRecipe".into()]);
        }
        let table = get_mod_data_ref(&item).unwrap();
        pz_inventory::moddata::set_mod_data_value_ref(&item, "retained", Some(Value::Num(23.0)))
            .unwrap();
        thread.globals().set_str("retained", table.clone());
        assert!(matches!(function(&thread), Value::Native(_)));
        let result = run(
            &mut thread,
            r#"
            local sameClass = ItemCodeOnCreate == zombie.scripting.logic.ItemCodeOnCreate
            local count = select('#', ItemCodeOnCreate.onCreateRecipeMagazine(item))
            return sameClass, count, retained == item:getModData(),
                retained.literatureTitle, retained.retained,
                ItemCodeOnCreate.onCreatePopBottle == nil
            "#,
        );
        assert!(matches!(result[0], Value::Bool(true)));
        assert_eq!(result[1].as_num(), Some(0.0));
        assert!(matches!(result[2], Value::Bool(true)));
        assert_eq!(result[3].as_str(), Some("Base.NativeRecipeMagazine"));
        assert_eq!(result[4].as_num(), Some(23.0));
        assert!(matches!(result[5], Value::Bool(true)));
        let body = item.borrow();
        assert_eq!(body.name, "custom display name");
        assert_eq!(body.id, 74);
        assert!(
            !body.is_initialised,
            "direct static call does not initialize item"
        );
        let ItemKind::Literature(data) = &body.kind else {
            unreachable!()
        };
        assert_eq!(
            data.learned_recipes.as_ref().unwrap(),
            &vec!["ExistingRecipe".to_owned()]
        );
        assert!(table.ptr_eq(
            body.mod_data_slot()
                .as_ref()
                .unwrap()
                .as_live_slot()
                .unwrap()
        ));
    }

    #[test]
    fn initialisation_resolves_the_genuine_native_static_and_publishes_flag_and_title() {
        let mut thread = thread();
        let script = Rc::new(RefCell::new(pz_scripts::objects::items::Item::new()));
        script.borrow_mut().lua_create = Some("ItemCodeOnCreate.onCreateRecipeMagazine".into());
        let mut template = ItemTemplate::new(ItemClass::Literature, "Base", "CallbackMagazine");
        template.script = Some(ScriptItemRef(script));
        let item = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
        assert!(item.borrow().table.is_none());
        crate::bind_items::initialise_item(&mut thread, &item);
        assert!(item.borrow().is_initialised);
        assert_eq!(
            item.borrow()
                .mod_data_string("literatureTitle")
                .unwrap()
                .as_deref(),
            Some("Base.CallbackMagazine")
        );
        assert!(matches!(function(&thread), Value::Native(_)));
    }

    #[test]
    fn native_conversion_orders_type_before_excess_count_and_never_allocates_wrong_item() {
        let mut thread = thread();
        let wrong = actual(&mut thread, ItemClass::Normal, "WrongArgument");
        let value = crate::bind_items::try_item_value(&wrong).unwrap();
        assert_eq!(
            failure(&mut thread, &[]),
            "expected 1 argument, got 0 java.lang.RuntimeException"
        );
        assert_eq!(
            failure(&mut thread, &[Value::Num(1.0)]),
            "expected argument of type Literature, got Double java.lang.RuntimeException"
        );
        assert_eq!(
            failure(&mut thread, &[Value::Bool(true)]),
            "expected argument of type Literature, got Boolean java.lang.RuntimeException"
        );
        assert_eq!(
            failure(&mut thread, &[Value::str("wrong")]),
            "expected argument of type Literature, got String java.lang.RuntimeException"
        );
        assert_eq!(
            failure(&mut thread, &[value.clone(), Value::Nil]),
            "expected argument of type Literature, got ComboItem java.lang.RuntimeException"
        );
        assert!(wrong.borrow().table.is_none());
        let right = actual(&mut thread, ItemClass::Literature, "ExtraArgument");
        let value = crate::bind_items::try_item_value(&right).unwrap();
        assert_eq!(
            failure(&mut thread, &[value, Value::Bool(true)]),
            "expected 1 argument, got 2 java.lang.RuntimeException"
        );
        assert!(right.borrow().table.is_none());
        let value = crate::bind_items::try_item_value(&right).unwrap();
        assert_eq!(
            failure(&mut thread, &[value, Value::Nil]),
            "expected 1 argument, got 2 java.lang.RuntimeException"
        );
        assert!(right.borrow().table.is_none());
        let result = run(
            &mut thread,
            "return pcall(function() ItemCodeOnCreate:onCreateRecipeMagazine(item) end)",
        );
        assert!(matches!(result[0], Value::Bool(false)));
        assert_eq!(result[1].as_str(), Some("expected argument of type Literature, got KahluaTableImpl java.lang.RuntimeException"));
        assert_eq!(
            failure(&mut thread, &[Value::Nil, Value::Bool(true)]),
            "expected 1 argument, got 2 java.lang.RuntimeException"
        );
    }

    #[test]
    fn wrong_table_conversion_does_not_read_its_held_body_and_null_is_explicit_host_boundary() {
        let mut thread = thread();
        let wrong = TableRef::new();
        let guard = wrong.try_borrow_body_mut().unwrap();
        assert_eq!(
            failure(&mut thread, &[Value::Table(wrong.clone())]),
            "expected argument of type Literature, got KahluaTableImpl java.lang.RuntimeException"
        );
        drop(guard);
        let null = failure(&mut thread, &[Value::Nil]);
        assert!(null.ends_with(" pz.host.Unsupported"));
        assert!(null.contains("MethodCaller null-receiver NPE diagnostic state"));
        assert!(matches!(invoke_body(None), Err(BodyFailure::NullReceiver)));
    }

    #[test]
    fn held_actual_bodies_return_host_faults_without_changing_table_or_allocating_missing_slot() {
        let mut thread = thread();
        let item = actual(&mut thread, ItemClass::Literature, "HeldRecipeMagazine");
        let argument = crate::bind_items::try_item_value(&item).unwrap();
        {
            let _guard = item.borrow_mut();
            let message = failure(&mut thread, &[argument.clone()]);
            assert!(message.ends_with(" pz.host.Borrowed"));
        }
        assert!(item.borrow().table.is_none());
        let table = get_mod_data_ref(&item).unwrap();
        pz_inventory::moddata::set_mod_data_value_ref(
            &item,
            "literatureTitle",
            Some(Value::str("old")),
        )
        .unwrap();
        {
            let _guard = table.try_borrow_body_mut().unwrap();
            let message = failure(&mut thread, &[argument]);
            assert!(message.ends_with(" pz.host.Borrowed"));
        }
        assert_eq!(
            table
                .try_get_own(&Value::str("literatureTitle"))
                .unwrap()
                .as_str(),
            Some("old")
        );
        assert!(table.ptr_eq(&get_mod_data_ref(&item).unwrap()));
    }

    #[test]
    fn opaque_wrong_reference_uses_nonvirtual_downcast_and_native_function_class_is_explicit_boundary(
    ) {
        struct Opaque;
        impl pz_kahlua::LuaObject for Opaque {
            fn class_name(&self) -> &str {
                "test.Opaque"
            }
            fn as_any(&self) -> &dyn std::any::Any {
                panic!("Native argument conversion does not call as_any")
            }
        }
        let mut thread = thread();
        assert_eq!(
            failure(&mut thread, &[Value::Object(Rc::new(Opaque))]),
            "expected argument of type Literature, got Opaque java.lang.RuntimeException"
        );
        let native = function(&thread);
        assert!(failure(&mut thread, &[native]).ends_with(" pz.host.Unsupported"));
    }

    struct DropProbe {
        item: Weak<RefCell<InventoryItem>>,
        foreign: Rc<RefCell<()>>,
        drops: Rc<Cell<usize>>,
    }
    impl pz_kahlua::LuaObject for DropProbe {
        fn class_name(&self) -> &str {
            "test.ReplacedRecipeTitle"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl Drop for DropProbe {
        fn drop(&mut self) {
            if let Some(item) = self.item.upgrade() {
                assert!(item.try_borrow_mut().is_ok());
            }
            assert!(
                self.foreign.try_borrow_mut().is_ok(),
                "title owner dropped under outer host guard"
            );
            self.drops.set(self.drops.get() + 1);
        }
    }

    #[test]
    fn replaced_title_owner_is_retired_until_safe_outer_boundary() {
        pz_inventory::component_host::drain_retired_components();
        let mut thread = thread();
        let item = actual(&mut thread, ItemClass::Literature, "RetiredRecipeTitle");
        let foreign = Rc::new(RefCell::new(()));
        let drops = Rc::new(Cell::new(0));
        pz_inventory::moddata::set_mod_data_value_ref(
            &item,
            "literatureTitle",
            Some(Value::Object(Rc::new(DropProbe {
                item: Rc::downgrade(&item),
                foreign: foreign.clone(),
                drops: drops.clone(),
            }))),
        )
        .unwrap();
        pz_inventory::component_host::drain_retired_components();
        {
            let _defer = pz_kahlua::gc::defer_collection();
            let _guard = foreign.borrow_mut();
            let function = function(&thread);
            let argument = crate::bind_items::try_item_value(&item).unwrap();
            assert!(thread.pcall(&function, &[argument]).unwrap().is_empty());
            assert_eq!(drops.get(), 0);
        }
        pz_inventory::component_host::drain_retired_components();
        assert_eq!(drops.get(), 1);
    }
}
