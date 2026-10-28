//! Thin actual-owner KnownPoison adapter; dependencies are resolved only at
//! the phase where the native overload asks for them.
use crate::expose::ClassBuilder;
use pz_characters::character::known_poison::{self, KnownPoisonHost};
use pz_characters::{Characters, PerkRef};
use pz_inventory::floor_drop::{HostResult, RuntimeHostError};
use pz_inventory::template::ScriptItemRef;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_scripts::native_item_keys::ScriptAccessError;
use pz_scripts::objects::recipes::Recipe;
use pz_scripts::registry::RegistryKey;
use pz_scripts::ScriptManager;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

struct InputPins(pz_entity::RetiredComponents);
impl InputPins {
    fn new(call: &pz_kahlua::NativeCall<'_>) -> Self {
        let mut pins = Self(pz_entity::RetiredComponents::new());
        for index in 0..call.nargs() {
            pins.0.push_payload(Box::new(call.arg(index)));
        }
        pins
    }
    fn keep<T: Clone + 'static>(&mut self, value: &T) {
        self.0.push_payload(Box::new(value.clone()));
    }
}
impl Drop for InputPins {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}

fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}
fn access(error: ScriptAccessError) -> RuntimeHostError {
    match error {
        ScriptAccessError::Borrowed(phase) => RuntimeHostError::Borrowed(phase),
        ScriptAccessError::Unsupported(phase) => unsupported(phase),
        ScriptAccessError::Native(_) => unsupported("native script lookup exception translation"),
    }
}
struct Host<'a> {
    sm: &'a ScriptManager,
    chars: &'a Rc<RefCell<Characters>>,
    thread: RefCell<&'a mut Thread>,
    // The lazy actual Sandbox initializer can return a Lua/native error.
    // Preserve that original domain after all predicate/body guards unwind.
    lua_fault: RefCell<Option<LuaError>>,
}
// The role methods below read Core's actual debug/client/server fields only:
// no user callbacks, Lua, character access or reference replacement. This is
// the required pure-query contract for the character's inline cheat lease.
impl KnownPoisonHost for Host<'_> {
    fn item_tag(&self, field: &'static str) -> HostResult<Rc<RegistryKey>> {
        self.sm
            .native_item_key_statics()
            .ok_or_else(|| unsupported("KnownPoison requires actual ItemTag class statics"))?
            .constant("item_tag", field)
            .cloned()
            .ok_or_else(|| unsupported("missing actual ItemTag static"))
    }
    fn food_item_type(&self) -> HostResult<Rc<RegistryKey>> {
        self.sm
            .native_item_key_statics()
            .ok_or_else(|| unsupported("KnownPoison requires actual ItemType class statics"))?
            .constant("item_type", "FOOD")
            .cloned()
            .ok_or_else(|| unsupported("missing actual ItemType.FOOD"))
    }
    fn recipe(&self, name: &str) -> HostResult<Option<Rc<RefCell<Recipe>>>> {
        pz_scripts::objects::recipes::try_get_recipe(self.sm, name).map_err(access)
    }
    fn is_debug(&self) -> HostResult<bool> {
        let core = crate::bind_core::shared_core();
        let value = core
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("KnownPoison Core.debug"))?
            .debug;
        Ok(value)
    }
    fn is_client(&self) -> HostResult<bool> {
        let core = crate::bind_core::shared_core();
        let value = core
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("KnownPoison GameClient.client"))?
            .is_client;
        Ok(value)
    }
    fn is_server(&self) -> HostResult<bool> {
        let core = crate::bind_core::shared_core();
        let value = core
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("KnownPoison GameServer.server"))?
            .is_server;
        Ok(value)
    }
    fn see_not_learnt_recipe(&self) -> HostResult<bool> {
        let initialized = {
            let mut thread = self
                .thread
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("KnownPoison lazy Sandbox thread"))?;
            crate::bind_sandbox::instance(&mut **thread)
        };
        let value = match initialized {
            Ok(value) => value,
            Err(error) => {
                *self.lua_fault.borrow_mut() = Some(error);
                return Err(unsupported("KnownPoison lazy Sandbox initializer failed"));
            }
        };
        let Value::Object(object) = value else {
            return Err(unsupported("actual SandboxOptions.instance"));
        };
        let object = (&*object as &dyn Any)
            .downcast_ref::<crate::bind_sandbox::SandboxObj>()
            .ok_or_else(|| unsupported("actual SandboxOptions.instance class"))?;
        let options = object
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("KnownPoison SandboxOptions"))?;
        let option = options
            .option_by_name("SeeNotLearntRecipe")
            .ok_or_else(|| unsupported("SandboxOptions.SeeNotLearntRecipe definition"))?;
        match option.config.get_value_as_object() {
            pz_core::config::ConfigObject::Bool(value) => Ok(value),
            _ => Err(unsupported(
                "SandboxOptions.SeeNotLearntRecipe boolean type",
            )),
        }
    }
    fn cooking_perk(&self) -> HostResult<PerkRef> {
        self.chars
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("KnownPoison PerkFactory"))?
            .perks
            .constant("Cooking")
            .ok_or_else(|| unsupported("actual PerkFactory.Perks.Cooking"))
    }
}

pub(super) fn register(
    builder: &ClassBuilder,
    sm: &Rc<ScriptManager>,
    chars: &Rc<RefCell<Characters>>,
) {
    let sm = Rc::clone(sm);
    let chars = Rc::clone(chars);
    builder.method("isKnownPoison", move |call| {
        let character = super::chr_of(call)?;
        let mut input_pins = InputPins::new(call);
        input_pins.keep(&character);
        if call.nargs() != 2 {
            return Err(crate::bind_world_object::unsupported(
                "KnownPoison argument count conversion",
            ));
        }
        let argument = call.arg(1);
        let result: LuaResult<bool> = {
            let host = Host {
                sm: &sm,
                chars: &chars,
                thread: RefCell::new(&mut *call.thread),
                lua_fault: RefCell::new(None),
            };
            let result = match &argument {
                Value::Object(object) => {
                    let object = &**object as &dyn Any;
                    if let Some(item) = object.downcast_ref::<crate::bind_items::LuaInventoryItem>()
                    {
                        known_poison::is_known_poison_item(&character, &item.item, &host)
                    } else if let Some(item) =
                        object.downcast_ref::<crate::bind_scripts::ScriptHandle>()
                    {
                        if !item.is_root() {
                            return Err(crate::bind_world_object::unsupported(
                                "KnownPoison nested script argument",
                            ));
                        }
                        let item = item
                            .root
                            .downcast::<pz_scripts::objects::items::Item>()
                            .ok_or_else(|| {
                                crate::bind_java_util::class_cast(
                                    &argument,
                                    crate::bind_scripts::ITEM_CLASS,
                                )
                            })?;
                        known_poison::is_known_poison_script(
                            &character,
                            &ScriptItemRef(item),
                            &host,
                        )
                    } else {
                        return Err(crate::bind_java_util::class_cast(
                            &argument,
                            "zombie.inventory.InventoryItem",
                        ));
                    }
                }
                Value::Nil => return Err(crate::bind_scripts::npe()),
                _ => {
                    return Err(crate::bind_java_util::class_cast(
                        &argument,
                        "zombie.inventory.InventoryItem",
                    ))
                }
            };
            let fault = host.lua_fault.into_inner();
            match fault {
                Some(error) => Err(error),
                None => result.map_err(crate::bind_world_object::host_error),
            }
        };
        Ok(call.push(result?))
    });
}

#[cfg(test)]
mod tests;

pub(crate) fn register_food_fields(builder: &ClassBuilder) {
    type Getter = fn(&pz_inventory::item::FoodData) -> Value;
    for (name, get) in [
        (
            "getPoisonPower",
            (|food: &pz_inventory::item::FoodData| Value::from(food.poison_power)) as Getter,
        ),
        (
            "getPoisonDetectionLevel",
            (|food: &pz_inventory::item::FoodData| Value::from(food.poison_detection_level))
                as Getter,
        ),
        (
            "getHerbalistType",
            (|food: &pz_inventory::item::FoodData| {
                food.herbalist_type
                    .clone()
                    .map(Value::from)
                    .unwrap_or(Value::Nil)
            }) as Getter,
        ),
    ] {
        builder.method(name, move |call| {
            let item = crate::bind_items::item_arg(call, 0)?;
            let mut input_pins = InputPins::new(call);
            input_pins.keep(&item);
            let result = (|| {
                if call.nargs() != 1 {
                    return Err(crate::bind_world_object::unsupported(
                        "Food getter argument count conversion",
                    ));
                }
                let body = item
                    .try_borrow()
                    .map_err(|_| crate::bind_world_object::borrowed("actual Food getter"))?;
                let pz_inventory::ItemKind::Food(food) = &body.kind else {
                    return Err(crate::bind_world_object::unsupported(
                        "actual Food getter class",
                    ));
                };
                Ok(get(food))
            })();
            Ok(call.push(result?))
        });
    }
    for name in [
        "setPoisonPower",
        "setPoisonDetectionLevel",
        "setHerbalistType",
    ] {
        builder.method(name, move |call| {
            let item = crate::bind_items::item_arg(call, 0)?;
            let mut input_pins = InputPins::new(call);
            input_pins.keep(&item);
            let result = (|| {
                if call.nargs() != 2 {
                    return Err(crate::bind_world_object::unsupported(
                        "Food setter argument count conversion",
                    ));
                }
                let (string, value) = if name == "setHerbalistType" {
                    match call.arg(1) {
                        Value::Nil => (None, 0),
                        Value::Str(value) => (Some(value.to_string()), 0),
                        _ => {
                            return Err(crate::bind_world_object::unsupported(
                                "Food nullable String argument conversion",
                            ))
                        }
                    }
                } else {
                    match call.arg(1) {
                        Value::Num(value) => (None, value as i32),
                        _ => {
                            return Err(crate::bind_world_object::unsupported(
                                "Food primitive int argument conversion",
                            ))
                        }
                    }
                };
                let mut body = item
                    .try_borrow_mut()
                    .map_err(|_| crate::bind_world_object::borrowed("actual Food setter"))?;
                let pz_inventory::ItemKind::Food(food) = &mut body.kind else {
                    return Err(crate::bind_world_object::unsupported(
                        "actual Food setter class",
                    ));
                };
                match name {
                    "setPoisonPower" => food.poison_power = value,
                    "setPoisonDetectionLevel" => food.poison_detection_level = value,
                    _ => food.herbalist_type = string,
                }
                Ok(())
            })();
            result?;
            Ok(0)
        });
    }
}
