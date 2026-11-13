//! Genuine static ItemCodeOnCreate entry points over actual item allocations.
//!
//! Implemented: onCreateRecipeMagazine and the custom-colour callbacks
//! (onCreateLipstick, onCreateFabricRoll, onCreateSprayPaint,
//! onCreateToyPlane, onCreateRandomColor). Other declared handlers, the
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


// ---- custom-colour callbacks ------------------------------------------------------------------

/// The `setCustomColor(item, color)` helper: the item's colour, its red,
/// green and blue channels, then the custom-colour flag. Java keeps the
/// Color reference itself in `col`; the port's item stores its value.
fn set_custom_color(item: &ItemRef, color: &crate::bind_misc_classes::Color) -> LuaResult<()> {
    let rgba = [color.r.get(), color.g.get(), color.b.get(), color.a.get()];
    {
        let mut body = item.try_borrow_mut().map_err(|_| {
            ModDataError::Borrowed("ItemCodeOnCreate.setCustomColor item").to_lua_error()
        })?;
        body.col = rgba;
        body.color_red = rgba[0];
        body.color_green = rgba[1];
        body.color_blue = rgba[2];
        body.custom_color = true;
    }
    pz_inventory::shared::observe_item(item);
    Ok(())
}

/// Native reference conversion for an `InventoryItem` parameter: any item
/// class is accepted, Java null faults later in the body.
fn item_argument(call: &NativeCall<'_>) -> LuaResult<Option<ItemRef>> {
    if call.nargs() == 0 {
        return Err(pz_kahlua::LuaError::runtime("expected 1 argument, got 0"));
    }
    let argument = call.arg(0);
    let item = match &argument {
        Value::Nil => None,
        Value::Object(object) => {
            match (&**object as &dyn std::any::Any).downcast_ref::<LuaInventoryItem>() {
                Some(known) => Some(known.item.clone()),
                None => return Err(item_type_error(&argument)),
            }
        }
        _ => return Err(item_type_error(&argument)),
    };
    if call.nargs() != 1 {
        return Err(pz_kahlua::LuaError::runtime(format!(
            "expected 1 argument, got {}",
            call.nargs()
        )));
    }
    Ok(item)
}

fn item_type_error(argument: &Value) -> pz_kahlua::LuaError {
    if matches!(argument, Value::Native(_)) {
        return ModDataError::Unsupported(
            "native JavaFunction implementation class in InventoryItem argument diagnostic",
        )
        .to_lua_error();
    }
    pz_kahlua::LuaError::runtime(format!(
        "expected argument of type InventoryItem, got {}",
        crate::expose::simple_name(&pz_kahlua::stdlib::java_class_name(argument))
    ))
}

/// Which colour a custom-colour callback picks.
#[derive(Clone, Copy)]
enum ColorSource {
    /// `ItemGenerationConstants.<list>.getRandom()`.
    Weighted(&'static str),
    /// `Color.random()`.
    Random,
}

/// Java evaluates the colour argument before `setCustomColor` touches the
/// item, so the draw happens even for a null item; the null receiver then
/// faults inside the helper.
fn invoke_custom_color(call: &mut NativeCall<'_>, source: ColorSource, name: &'static str) -> LuaResult<usize> {
    let item = item_argument(call)?;
    let color = match source {
        ColorSource::Weighted(list) => crate::bind_colors::item_generation_random_color(list)?,
        ColorSource::Random => crate::bind_colors::random_color()?,
    };
    let Some(item) = item else {
        return Err(ModDataError::Unsupported(name).to_lua_error());
    };
    set_custom_color(&item, &color)?;
    Ok(0)
}

const CUSTOM_COLOR_CALLBACKS: [(&str, ColorSource, &str); 5] = [
    ("onCreateLipstick", ColorSource::Weighted("COLOR_LIPSTICK"), "MethodCaller null-receiver NPE diagnostic state for onCreateLipstick"),
    // FabricRoll draws from the lipstick list too.
    ("onCreateFabricRoll", ColorSource::Weighted("COLOR_LIPSTICK"), "MethodCaller null-receiver NPE diagnostic state for onCreateFabricRoll"),
    ("onCreateSprayPaint", ColorSource::Weighted("COLOR_SPRAY_PAINT"), "MethodCaller null-receiver NPE diagnostic state for onCreateSprayPaint"),
    ("onCreateToyPlane", ColorSource::Weighted("COLOR_TOY_PLANE"), "MethodCaller null-receiver NPE diagnostic state for onCreateToyPlane"),
    ("onCreateRandomColor", ColorSource::Random, "MethodCaller null-receiver NPE diagnostic state for onCreateRandomColor"),
];

pub(super) fn register(thread: &mut Thread) {
    let class = crate::expose::expose_class(thread, CLASS, Some(SUPERCLASS));
    class.static_fn("onCreateRecipeMagazine", invoke);
    for (name, source, null_receiver) in CUSTOM_COLOR_CALLBACKS {
        class.static_fn(name, move |c| invoke_custom_color(c, source, null_receiver));
    }
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

#[cfg(test)]
mod custom_color_tests {
    use std::rc::Rc;

    use pz_inventory::{shared, InventoryItem, ItemClass, ItemTemplate};
    use pz_kahlua::{stdlib, TableRef, Thread, Value};

    use crate::bind_misc_classes::Color;

    fn thread() -> Thread {
        let mut thread = Thread::new(TableRef::new());
        stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        crate::bind_globals::register(&mut thread);
        crate::bind_items::register(&mut thread);
        thread
    }

    fn run(thread: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
        let f = stdlib::load_string(src, Some("test"), thread.globals().clone()).unwrap();
        thread.pcall(&f, &[]).map_err(|(m, _)| pz_kahlua::exec::java_value_of(&m))
    }

    fn item(thread: &mut Thread) -> super::ItemRef {
        let item = shared::try_new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Lipstick")),
            1,
        ))
        .unwrap();
        thread.globals().set_str("item", crate::bind_items::try_item_value(&item).unwrap());
        item
    }

    #[test]
    fn weighted_callbacks_set_the_drawn_custom_colour() {
        let mut thread = thread();
        let red = Rc::new(Color::rgba(0.75, 0.25, 0.5, 1.0));
        let blue = Rc::new(Color::rgba(0.0, 0.0, 1.0, 0.5));
        crate::bind_colors::install_item_generation_colors_for_test(vec![
            ("COLOR_LIPSTICK", vec![(red.clone(), 3), (blue.clone(), 1)]),
            ("COLOR_SPRAY_PAINT", vec![(blue.clone(), 5)]),
            ("COLOR_TOY_PLANE", vec![(blue.clone(), 5)]),
        ]);
        for callback in ["onCreateLipstick", "onCreateFabricRoll", "onCreateSprayPaint", "onCreateToyPlane"] {
            let item = item(&mut thread);
            let r = run(&mut thread, &format!("return select('#', ItemCodeOnCreate.{callback}(item))")).unwrap();
            assert_eq!(r[0].as_num(), Some(0.0), "{callback}");
            let body = item.borrow();
            assert!(body.custom_color, "{callback}");
            let got = [body.col[0], body.col[1], body.col[2], body.col[3]];
            assert!(got == [0.75, 0.25, 0.5, 1.0] || got == [0.0, 0.0, 1.0, 0.5], "{callback}: {got:?}");
            assert_eq!([body.color_red, body.color_green, body.color_blue], [got[0], got[1], got[2]]);
        }
    }

    #[test]
    fn weighted_draw_follows_the_running_sum() {
        let a = Rc::new(Color::rgba(1.0, 0.0, 0.0, 1.0));
        let b = Rc::new(Color::rgba(0.0, 1.0, 0.0, 1.0));
        crate::bind_colors::install_item_generation_colors_for_test(vec![("COLOR_LIPSTICK", vec![(a.clone(), 1), (b.clone(), 1)])]);
        let draws = (0..8)
            .map(|_| {
                pz_core::random::set_seed(7);
                let expected = pz_core::random::with_standard(|r| r.next(2));
                pz_core::random::set_seed(7);
                let got = crate::bind_colors::item_generation_random_color("COLOR_LIPSTICK").unwrap();
                (expected, Rc::ptr_eq(&got, &a))
            })
            .collect::<Vec<_>>();
        for (roll, is_a) in draws {
            assert_eq!(roll == 0, is_a);
        }
    }

    #[test]
    fn bad_arguments_and_missing_lists_fail() {
        let mut thread = thread();
        crate::bind_colors::install_item_generation_colors_for_test(vec![]);
        let _ = item(&mut thread);
        let e = run(&mut thread, "ItemCodeOnCreate.onCreateSprayPaint(item)").unwrap_err();
        assert!(e.contains("COLOR_SPRAY_PAINT"), "{e}");
        let e = run(&mut thread, "ItemCodeOnCreate.onCreateLipstick('x')").unwrap_err();
        assert!(e.contains("expected argument of type InventoryItem"), "{e}");
        let e = run(&mut thread, "ItemCodeOnCreate.onCreateLipstick()").unwrap_err();
        assert!(e.contains("expected 1 argument, got 0"), "{e}");
    }

    /// With an install, the jar's lists load and every entry resolves to a
    /// `Colors` field.
    #[test]
    fn installed_jar_lists_load() {
        let Ok(dir) = std::env::var("PZ_GAME_DIR") else {
            eprintln!("PZ_GAME_DIR not set; skipping");
            return;
        };
        let jar = pz_classfile::Jar::open(std::path::Path::new(&dir).join("projectzomboid.jar")).unwrap();
        let mut thread = thread();
        crate::bind_colors::register(&mut thread, Some(&jar));
        for list in ["COLOR_LIPSTICK", "COLOR_SPRAY_PAINT", "COLOR_TOY_PLANE"] {
            crate::bind_colors::item_generation_random_color(list).unwrap();
        }
        crate::bind_colors::random_color().unwrap();
        let item = item(&mut thread);
        run(&mut thread, "ItemCodeOnCreate.onCreateRandomColor(item)").unwrap();
        assert!(item.borrow().custom_color);
    }
}
