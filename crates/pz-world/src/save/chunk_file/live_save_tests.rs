use super::*;
use crate::objects::{IsoMannequin, IsoObject, ObjectName};
use crate::save::{NoSaveHooks, SaveHooks, SaveSettings, SaveTables};
use crate::{IsoObjectRef, ObjectSlot, SquareRef, SquareSlot};
use pz_inventory::item_texture::CapturedScriptTextures;
use pz_inventory::worn_items::{BodyLocationGroup, WornItems};
use pz_inventory::{shared, ItemClass, ItemTemplate};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}
struct Env(Rc<ItemTemplate>);
impl pz_inventory::ItemEnv for Env {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        (id == 1).then(|| self.0.clone())
    }
}
fn fixture() -> (Env, shared::ItemRef, IsoObjectRef, SquareRef) {
    let mut template = ItemTemplate::new(ItemClass::Normal, "Base", "SquareSave");
    template.texture_capture = Some(CapturedScriptTextures::new(None, Vec::new()));
    let env = Env(Rc::new(template));
    let item = pz_inventory::env::create_from_template_ref(&env, env.0.clone(), 1).unwrap();
    let container = shared::SharedContainer::new(18, "mannequin").into_ref();
    shared::add_item(&container, &item);
    let worn = Rc::new(RefCell::new(WornItems::new(Rc::new(RefCell::new(
        BodyLocationGroup::new("fixture").unwrap(),
    )))));
    let mut mannequin = IsoMannequin::new(7);
    drop(mannequin.install_runtime_inventory(container, worn));
    let object = ObjectSlot::new(IsoObject::new(
        1,
        ObjectKind::Mannequin(Box::new(mannequin)),
        None,
    ));
    let receiver = object.ref_handle();
    let square = Rc::new(RefCell::new(IsoGridSquare::new(0, 0, 0)));
    square.borrow_mut().objects.push(object);
    let mut second = IsoObject::new(2, ObjectKind::Object, None);
    second.name = Some(ObjectName::Str("before".into()));
    square.borrow_mut().objects.push(ObjectSlot::new(second));
    (env, item, receiver, square)
}
struct Callback {
    square: SquareRef,
    replacement: IsoObjectRef,
    calls: Cell<usize>,
    fail: bool,
}
impl SaveHooks for Callback {
    fn mannequin_wear_item(&self, object: &IsoObjectRef, item: &shared::ItemRef) -> SaveResult<()> {
        assert!(object.try_borrow_mut().is_ok());
        assert!(item.try_borrow_mut().is_ok());
        let mut square = self
            .square
            .try_borrow_mut()
            .expect("square guard crossed native object callback");
        square.overlay_done = true;
        square.have_roof = true;
        square.burnt_out = true;
        square.seen[2] = true;
        square.objects[1] = ObjectSlot::retain(&self.replacement);
        self.calls.set(self.calls.get() + 1);
        // This normal item has no visual, so the native wear body makes no
        // worn mutation. The authored callback probes the host boundary.
        if self.fail {
            return err("fixture native mannequin callback failure");
        }
        Ok(())
    }
}
fn replacement() -> IsoObjectRef {
    let mut object = IsoObject::new(3, ObjectKind::Object, None);
    object.name = Some(ObjectName::Str("after".into()));
    ObjectSlot::new(object).ref_handle()
}

#[test]
fn actual_chunk_save_releases_square_and_reads_later_objects_and_square_fields() {
    let _boundary = Boundary;
    let (env, item, _receiver, square) = fixture();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(0, 0, 0, Some(SquareSlot::retain(&square)));
    let hooks = Callback {
        square,
        replacement: replacement(),
        calls: Cell::new(0),
        fail: false,
    };
    let sprites = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = SaveTables::for_tests();
    let settings = SaveSettings::default();
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &env,
        hooks: &hooks,
        meta: None,
    };
    let bytes = save_chunk(&chunk, &ctx).unwrap();
    assert!(validate_header(&bytes));
    assert_eq!(hooks.calls.get(), 1);
    let load_ctx = SaveCtx {
        hooks: &NoSaveHooks,
        ..ctx
    };
    let loaded = load_chunk(&bytes, 0, 0, &load_ctx).unwrap();
    let saved_square = loaded.chunk.get_grid_square(0, 0, 0).unwrap();
    let saved_square = saved_square.borrow();
    assert!(saved_square.overlay_done && saved_square.have_roof && saved_square.burnt_out);
    assert!(saved_square.seen[2]);
    assert_eq!(saved_square.objects.len(), 2);
    assert_eq!(
        saved_square.objects[1].borrow().name,
        Some(ObjectName::Str("after".into()))
    );
    shared::try_set_container(&item, None).unwrap();
}

#[test]
fn failed_actual_object_callback_keeps_square_prefix_unpatched_and_mutations_visible() {
    let _boundary = Boundary;
    let (env, item, receiver, square) = fixture();
    let hooks = Callback {
        square: square.clone(),
        replacement: replacement(),
        calls: Cell::new(0),
        fail: true,
    };
    let sprites = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = SaveTables::for_tests();
    let settings = SaveSettings::default();
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &env,
        hooks: &hooks,
        meta: None,
    };
    let mut writer = Writer::new();
    let error = save_square_ref(&mut writer, &ctx, &square).unwrap_err();
    assert_eq!(error.0, "fixture native mannequin callback failure");
    assert!(writer.buf.len() > 2);
    assert_eq!(
        writer.buf[1], 0,
        "failure must not finalize the earlier square header"
    );
    assert!(square.borrow().overlay_done);
    assert_eq!(hooks.calls.get(), 1);
    assert!(receiver.try_borrow_mut().is_ok());
    shared::try_set_container(&item, None).unwrap();
}
