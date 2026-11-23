//! ItemCodeOnCreate's detached mannequins and IsoMannequin.setCustomSettingsToItem.
use super::*;
use pz_inventory::io::{self, NativeWriterFault, SaveResult, Writer};
use pz_kahlua::{LuaError, LuaResult, Value};
use pz_world::save::{SaveCtx, SaveHooks, SaveSettings, SaveTables, WORLD_VERSION};
use std::path::PathBuf;

struct ItemSource {
    game_dir: PathBuf,
    tables: SaveTables,
}

thread_local! {
    static SOURCE: RefCell<Option<Rc<ItemSource>>> = const { RefCell::new(None) };
    // SliceY.SliceBuffer is reused on the serial game thread.
    static SLICE: io::NativeSaveBufferRef = io::native_save_buffer_with_capacity(0xA00000);
}

pub(crate) fn install_item_source(game_dir: PathBuf, jar: Option<&pz_classfile::Jar>) {
    let source = jar
        .and_then(|jar| SaveTables::from_jar(jar).ok())
        .map(|tables| Rc::new(ItemSource { game_dir, tables }));
    SOURCE.with(|slot| *slot.borrow_mut() = source);
}

struct DetachedSave;
impl SaveHooks for DetachedSave {
    fn mannequin_wear_item(&self, object: &IsoObjectRef, item: &ItemRef) -> SaveResult<()> {
        wear_item_for_save(object, item).map(|_| ())
    }
}

pub(crate) fn create_moveable(
    thread: &mut Thread,
    target: Option<&ItemRef>,
    name: &str,
) -> LuaResult<()> {
    let source = SOURCE
        .with(|slot| slot.borrow().clone())
        .ok_or_else(|| LuaError::runtime("missing mannequin save tables"))?;
    let sprites = crate::bind_sprites::manager()?;
    let sprite = {
        let mut sprites = sprites
            .try_borrow_mut()
            .map_err(|_| LuaError::runtime("mannequin sprite manager is borrowed"))?;
        let name = "location_shop_mall_01_68";
        sprites
            .named_map
            .get(name)
            .copied()
            .unwrap_or_else(|| sprites.add_sprite_named(name))
    };
    let body = IsoMannequin::new(source.tables.blood_body_parts);
    let object = pz_world::ObjectSlot::new(pz_world::IsoObject::new(
        0,
        ObjectKind::Mannequin(Box::new(body)),
        Some(sprite),
    ))
    .ref_handle();
    if crate::bind_registry::shared_script_manager()
        .get::<MannequinScript>(ScriptType::Mannequin, name)
        .is_some()
    {
        let sandbox = crate::bind_sandbox::instance(thread)?;
        let Value::Object(sandbox) = sandbox else {
            return Err(LuaError::runtime("missing sandbox options"));
        };
        let sandbox = sandbox
            .as_any()
            .downcast_ref::<crate::bind_sandbox::SandboxObj>()
            .ok_or_else(|| LuaError::runtime("missing sandbox options"))?;
        let no_black = sandbox
            .0
            .borrow()
            .option_by_name("NoBlackClothes")
            .is_some_and(|o| o.config.get_value_as_string() == "true");
        initialize(
            thread,
            &object,
            InitialSettings::Script(name),
            &source.game_dir,
            no_black,
            None,
        )
        .map_err(|e| LuaError::runtime(format!("mannequin initialization: {e}")))?;
    }
    let Some(item) = target else { return Ok(()) };
    if !matches!(
        item.try_borrow()
            .map_err(|_| LuaError::runtime("moveable item is borrowed"))?
            .kind,
        pz_inventory::ItemKind::Moveable(_) | pz_inventory::ItemKind::Radio(_, _)
    ) {
        return Ok(());
    }
    let sprites = sprites
        .try_borrow()
        .map_err(|_| LuaError::runtime("mannequin sprite manager is borrowed"))?;
    let buffer = SLICE.with(Rc::clone);
    io::native_buffer_clear(&buffer).map_err(|e| LuaError::runtime(format!("{e:?}")))?;
    let _deferral = pz_kahlua::gc::defer_collection();
    let result = Writer::write_native(&buffer, |writer| {
        writer.i32(WORLD_VERSION);
        crate::bind_items::with_factory_env(|items, _| {
            let ctx = SaveCtx {
                sprites: &sprites,
                tables: &source.tables,
                settings: &SaveSettings::default(),
                items,
                hooks: &DetachedSave,
                meta: None,
            };
            pz_world::save::object_io::save_object_ref(writer, &ctx, &object)
        })
    });
    drop(sprites);
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("Couldn't set custom settings for {name}: {error}");
            return Ok(());
        }
        Err(NativeWriterFault::Overflow) => {
            return Err(LuaError::runtime("java.nio.BufferOverflowException"))
        }
        Err(NativeWriterFault::Host(error)) => return Err(LuaError::runtime(format!("{error:?}"))),
    }
    let end =
        io::native_buffer_position(&buffer).map_err(|e| LuaError::runtime(format!("{e:?}")))?;
    let bytes = io::native_buffer_bytes(&buffer, 0, end)
        .map_err(|e| LuaError::runtime(format!("{e:?}")))?;
    item.try_borrow_mut()
        .map_err(|_| LuaError::runtime("moveable item is borrowed"))?
        .byte_data = Some(bytes);
    let container = match &object.borrow().kind {
        ObjectKind::Mannequin(m) => m.runtime_container.clone(),
        _ => None,
    };
    if let Some(container) = container {
        let weight = crate::bind_item_container::contents_weight(&container);
        let mut item = item
            .try_borrow_mut()
            .map_err(|_| LuaError::runtime("moveable item is borrowed"))?;
        item.actual_weight = (item.get_actual_weight() + weight).max(0.0);
    }
    Ok(())
}
