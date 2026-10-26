//! Interpreter safe points may still be inside actual WORLD/sprite borrows.

use super::*;
use pz_kahlua::gc::{self, CollectionResult};
use pz_world::sprite::{test_tables, IsoSpriteManager};
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Fixture {
    old_world: Option<WorldState>,
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
        drop(retired);
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
