//! Interpreter safe points may still be inside actual WORLD/sprite borrows.

use super::*;
use pz_kahlua::gc::{self, CollectionResult};
use pz_world::sprite::{test_tables, IsoSpriteManager};
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Fixture {
    old_world: Option<WorldState>,
    old_cell_generation: u64,
    old_handles: HashMap<Key, Weak<Handle>>,
    old_sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    sprites: Rc<RefCell<IsoSpriteManager>>,
}

impl Fixture {
    fn new() -> Self {
        let old_world = with_world(std::mem::take);
        let old_sprites = crate::bind_sprites::manager().ok();
        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(test_tables())));
        crate::bind_sprites::install_sprite_manager(Some(sprites.clone()));
        Self {
            old_world: Some(old_world),
            old_cell_generation: cell_generation(),
            old_handles: HANDLES.with(|handles| handles.borrow().clone()),
            old_sprites,
            sprites,
        }
    }

    fn unreachable_probe(&self, dropped: &Rc<Cell<bool>>) {
        let table = TableRef::new();
        table.set_str("self", table.clone());
        table.set_str(
            "probe",
            Value::Object(Rc::new(WorldSpriteDrop {
                sprites: self.sprites.clone(),
                dropped: dropped.clone(),
            })),
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Every test guard has ended before restoration. Retired payloads
        // remain outside WORLD while their final destructors run.
        let retired = with_world(|world| std::mem::replace(world, self.old_world.take().unwrap()));
        CELL_GEN.with(|generation| generation.set(self.old_cell_generation));
        let retired_handles = HANDLES.with(|handles| {
            std::mem::replace(
                &mut *handles.borrow_mut(),
                std::mem::take(&mut self.old_handles),
            )
        });
        drop(retired);
        drop(retired_handles);
        crate::bind_sprites::install_sprite_manager(self.old_sprites.take());
    }
}

struct WorldSpriteDrop {
    sprites: Rc<RefCell<IsoSpriteManager>>,
    dropped: Rc<Cell<bool>>,
}

impl LuaObject for WorldSpriteDrop {
    fn class_name(&self) -> &str {
        "test.WorldSpriteDrop"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for WorldSpriteDrop {
    fn drop(&mut self) {
        with_world(|world| {
            let _sprites = self.sprites.borrow_mut();
            assert!(world.cell.is_none());
            self.dropped.set(true);
        });
    }
}

#[test]
fn interpreter_drain_waits_until_actual_world_and_sprite_guards_end() {
    let fixture = Fixture::new();
    let dropped = Rc::new(Cell::new(false));
    fixture.unreachable_probe(&dropped);
    let function = Value::Native(NativeFunction::new("request under world", |_| {
        gc::request_collection();
        Ok(0)
    }));
    let mut thread = Thread::new(TableRef::new());
    with_world(|_| {
        let _sprites = fixture.sprites.borrow_mut();
        thread.call(&function, &[]).unwrap();
        assert_eq!(gc::collect_pending(), CollectionResult::Deferred);
        assert!(!dropped.get());
    });
    assert!(!dropped.get(), "host guard Drop must not start collection");
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
    assert!(dropped.get());
}

#[test]
fn actual_world_unwind_restores_gate_and_retains_the_request() {
    let fixture = Fixture::new();
    let dropped = Rc::new(Cell::new(false));
    fixture.unreachable_probe(&dropped);
    let failure = catch_unwind(AssertUnwindSafe(|| {
        with_world(|_| {
            let _sprites = fixture.sprites.borrow_mut();
            assert_eq!(gc::collect_tables(), CollectionResult::Deferred);
            panic!("authored world callback failure");
        });
    }));
    assert!(failure.is_err());
    assert!(!dropped.get());
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
    assert!(dropped.get());
}

#[test]
fn fallible_world_access_reports_the_actual_borrow_without_running_a_fallback() {
    let _fixture = Fixture::new();
    let called = Cell::new(false);
    with_world(|_| {
        assert_eq!(
            try_with_world(|_| {
                called.set(true);
                Ok(())
            }),
            Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
                "Lua actual world state"
            ))
        );
    });
    assert!(!called.get());
    try_with_world(|world| {
        assert!(world.cell.is_none() && world.meta_grid.is_none());
        called.set(true);
        Ok(())
    })
    .unwrap();
    assert!(called.get());
}

#[test]
fn fallible_world_error_retains_collection_request_until_real_guards_end() {
    let fixture = Fixture::new();
    let dropped = Rc::new(Cell::new(false));
    fixture.unreachable_probe(&dropped);
    let error = try_with_world::<()>(|_| {
        let _sprites = fixture.sprites.borrow_mut();
        assert_eq!(gc::collect_tables(), CollectionResult::Deferred);
        assert!(!dropped.get());
        Err(pz_entity::runtime_load::RuntimeHostError::Unsupported {
            operation: "authored native floor phase",
            kind: None,
        })
    })
    .unwrap_err();
    assert_eq!(
        error,
        pz_entity::runtime_load::RuntimeHostError::Unsupported {
            operation: "authored native floor phase",
            kind: None,
        }
    );
    assert!(!dropped.get());
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
    assert!(dropped.get());
}

#[test]
fn cell_session_token_changes_even_when_clearing_an_absent_cell() {
    let _fixture = Fixture::new();
    let before = cell_generation();
    set_cell(None);
    assert_ne!(cell_generation(), before);
    let cleared = cell_generation();
    set_cell(None);
    assert_ne!(cell_generation(), cleared);
}
