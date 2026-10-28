use super::*;
use std::cell::Cell;

struct FinalOwner {
    released: Rc<Cell<bool>>,
    enclosing_owner: Rc<RefCell<()>>,
}

impl Drop for FinalOwner {
    fn drop(&mut self) {
        let _guard = self
            .enclosing_owner
            .try_borrow_mut()
            .expect("a retired final owner must be able to reenter its former enclosing host");
        self.released.set(true);
    }
}

fn queue_owner(enclosing_owner: &Rc<RefCell<()>>) -> Rc<Cell<bool>> {
    let released = Rc::new(Cell::new(false));
    let owner = Rc::new(RefCell::new(FinalOwner {
        released: released.clone(),
        enclosing_owner: enclosing_owner.clone(),
    }));
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_owner(pz_entity::EntityOwner::from_ref(&owner));
    pz_inventory::component_host::retire_components(retired);
    drop(owner);
    assert!(!released.get());
    released
}

fn unbooted_app() -> App {
    App {
        launch: Launch {
            game_dir: PathBuf::new(),
            cache_dir: PathBuf::new(),
            start: Start::MainMenu,
        },
        window: None,
        gpu: None,
        lua: None,
        core: Rc::new(RefCell::new(Core::default())),
        states: GameStateMachine::default(),
        world: None,
        error: None,
        retired_models: Vec::new(),
        last_frame: Instant::now(),
        gamepads: None,
        audio: None,
        ui_models: None,
    }
}

#[test]
fn error_only_frame_and_close_without_lua_release_retired_final_owners() {
    let enclosing_owner = Rc::new(RefCell::new(()));
    let mut app = unbooted_app();
    app.error = Some("failed before Lua boot".into());
    let frame_owner = queue_owner(&enclosing_owner);
    app.frame();
    assert!(frame_owner.get());
    let close_owner = queue_owner(&enclosing_owner);
    app.close();
    assert!(close_owner.get());
}

#[test]
fn failed_outer_scope_releases_retired_owners_after_its_host_guard_unwinds() {
    let enclosing_owner = Rc::new(RefCell::new(()));
    let released = queue_owner(&enclosing_owner);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _retirement_boundary = HostRetirementBoundary;
        let _enclosing_guard = enclosing_owner.borrow_mut();
        panic!("authored boot failure while a host is borrowed");
    }));
    assert!(result.is_err());
    assert!(released.get());
}
