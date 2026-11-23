use super::*;
use crate::bind_world::{set_cell, set_meta_grid, with_world};
use pz_kahlua::{stdlib, TableRef, Thread};
use pz_world::sprite::{test_tables, IsoSpriteManager, SpriteIdx};
use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoObject, ObjectSlot};
use std::cell::RefCell;
use std::rc::Rc;

struct Fixture {
    thread: Thread,
    closed: SpriteIdx,
    open: SpriteIdx,
    old_sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    old_cell: Option<IsoCell>,
    observer: Option<pz_world::object_slot::ObjectObserver>,
}

impl Fixture {
    fn new() -> Self {
        let observer = pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
        let old_sprites = crate::bind_sprites::manager().ok();
        let old_cell = with_world(|world| world.cell.take());
        let mut sprites = IsoSpriteManager::new(test_tables());
        let closed = sprites.add_sprite("window_closed", 701);
        let open = sprites.add_sprite("window_open", 702);
        crate::bind_sprites::install_sprite_manager(Some(Rc::new(RefCell::new(sprites))));
        let mut chunk = IsoChunk::new(0, 0);
        for (x, y) in [(1, 1), (1, 2)] {
            chunk.set_square(x, y, 0, Some(IsoGridSquare::new(x, y, 0).into()));
        }
        let mut cell = IsoCell::default();
        cell.chunks.insert((0, 0), chunk);
        set_cell(Some(cell));
        set_meta_grid(Some(pz_world::IsoMetaGrid::blank(0, 0, 0, 0)));
        let mut thread = Thread::new(TableRef::new());
        stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        crate::bind_world::register(&mut thread);
        crate::bind_world_object::register_classes(&mut thread);
        register(&mut thread);
        crate::expose::expose_enum(
            &mut thread,
            crate::bind_iso_enums::ISO_DIRECTIONS,
            &["N", "NW", "W", "SW", "S", "SE", "E", "NE"],
        );
        let mut fixture = Self { thread, closed, open, old_sprites, old_cell, observer };
        fixture.run("function check(v) if not v then error('climb binding check failed') end end");
        fixture
    }

    fn run(&mut self, source: &str) {
        let closure =
            stdlib::load_string(source, Some("climb-test"), self.thread.globals().clone()).unwrap();
        self.thread.pcall(&closure, &[]).unwrap();
    }

    /// Put a window with the fixture's sprites on square (1, 2).
    fn window(&self, locked: bool, perma_locked: bool) {
        let window = pz_world::objects::IsoWindow {
            north: true,
            health: 50,
            locked,
            perma_locked,
            closed_sprite: Some(self.closed),
            open_sprite: Some(self.open),
            ..Default::default()
        };
        with_world(|world| {
            let square = world.cell.as_mut().unwrap().get_grid_square_mut(1, 2, 0).unwrap();
            square
                .borrow_mut()
                .objects
                .push(ObjectSlot::new(IsoObject::new(1, ObjectKind::Window(window), Some(self.closed.into()))));
        });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_cell(self.old_cell.take());
        set_meta_grid(None);
        crate::bind_sprites::install_sprite_manager(self.old_sprites.take());
        pz_world::object_slot::install_object_observer(self.observer);
        pz_inventory::component_host::drain_retired_components();
        crate::component_gc::retry_pending();
        let _ = pz_kahlua::gc::collect_tables();
    }
}

#[test]
fn toggle_window_opens_unlocks_and_swaps_the_sprite() {
    let mut f = Fixture::new();
    f.window(true, false);
    f.run(
        "w = IsoCell.getInstance():getGridSquare(1,2,0):getObjects():get(0)
         check(w:isLocked() and not w:IsOpen())
         w:ToggleWindow(nil)
         check(w:IsOpen() and not w:isLocked())
         w:ToggleWindow(nil)
         check(not w:IsOpen())",
    );
    let sprite = with_world(|world| {
        let square = world.cell.as_ref().unwrap().get_grid_square(1, 2, 0).unwrap();
        let square = square.borrow();
        let body = square.objects[0].ref_handle().borrow().sprite.clone();
        body
    });
    assert_eq!(sprite, ObjectSprite::Tile(f.closed));
}

#[test]
fn toggle_window_leaves_a_permanently_locked_window_alone() {
    let mut f = Fixture::new();
    f.window(true, true);
    f.run(
        "w = IsoCell.getInstance():getGridSquare(1,2,0):getObjects():get(0)
         w:ToggleWindow(nil)
         check(not w:IsOpen() and w:isLocked())",
    );
}

#[test]
fn get_object_index_is_the_position_in_the_square() {
    let mut f = Fixture::new();
    f.window(false, false);
    f.window(false, false);
    f.run(
        "objects = IsoCell.getInstance():getGridSquare(1,2,0):getObjects()
         check(objects:get(0):getObjectIndex() == 0)
         check(objects:get(1):getObjectIndex() == 1)",
    );
}

#[test]
fn square_get_room_is_one_object_per_room() {
    let mut f = Fixture::new();
    with_world(|world| {
        let meta = world.meta_grid.as_mut().unwrap();
        let mut meta_cell = pz_world::IsoMetaCell::new(0, 0);
        meta.room_defs.push(pz_world::RoomDef::new(7, "test room".into()));
        meta_cell.rooms.insert(7, pz_world::RoomIdx(0));
        meta.set_cell(0, 0, Some(meta_cell));
        let cell = world.cell.as_mut().unwrap();
        cell.get_grid_square_mut(1, 1, 0).unwrap().borrow_mut().room_id = 7;
        cell.get_grid_square_mut(1, 2, 0).unwrap().borrow_mut().room_id = 7;
    });
    f.run(
        "cell = IsoCell.getInstance()
         a, b = cell:getGridSquare(1,1,0), cell:getGridSquare(1,2,0)
         check(a:getRoom() ~= nil and a:getRoom() == b:getRoom())",
    );
    with_world(|world| {
        world.cell.as_mut().unwrap().get_grid_square_mut(1, 2, 0).unwrap().borrow_mut().room_id = -1;
    });
    f.run("check(b:getRoom() == nil and a:getRoom() ~= b:getRoom())");
}

#[test]
fn square_get_adjacent_square_steps_one_tile_and_answers_nil_when_not_loaded() {
    let mut f = Fixture::new();
    f.run(
        "cell = IsoCell.getInstance()
         b = cell:getGridSquare(1,2,0)
         north = b:getAdjacentSquare(IsoDirections.N)
         check(north ~= nil and north:getX() == 1 and north:getY() == 1)
         check(north:getAdjacentSquare(IsoDirections.N) == nil)",
    );
}

#[test]
fn window_frame_and_thumpable_climb_checks_use_the_live_object_and_actor() {
    use crate::bind_character_sim::{character_value, LuaGameCharacter};
    use pz_characters::character::CharacterKind;
    let mut f = Fixture::new();
    f.window(false, false);
    let manager = crate::bind_sprites::manager().unwrap();
    let mut sprites = manager.borrow_mut();
    let flag = sprites.tables.flag.WindowN;
    sprites.sprites[f.closed].properties.set_flag(flag);
    drop(sprites);
    with_world(|world| {
        let square = world.cell.as_mut().unwrap().get_grid_square_mut(1, 2, 0).unwrap();
        let mut square = square.borrow_mut();
        for (id, kind, sprite) in [
            (41, ObjectKind::WindowFrame { north: true }, None),
            (42, ObjectKind::Thumpable(Default::default()), Some(f.closed.into())),
            (43, ObjectKind::Thumpable(Default::default()), Some(f.open.into())),
        ] {
            square.objects.push(ObjectSlot::new(IsoObject::new(id, kind, sprite)));
        }
    });
    let sm = pz_scripts::ScriptManager::new();
    let animal = Rc::new(RefCell::new(LuaGameCharacter::new(&sm, CharacterKind::Animal)));
    let player = Rc::new(RefCell::new(LuaGameCharacter::new(&sm, CharacterKind::Player)));
    f.thread.globals().set_str("animal", character_value(&animal));
    f.thread.globals().set_str("player", character_value(&player));
    f.run(r#"
        local objects = IsoCell.getInstance():getGridSquare(1,2,0):getObjects()
        w, frame, wall, other = objects:get(0), objects:get(1), objects:get(2), objects:get(3)
        check(w:isWindow() and wall:isWindow() and not other:isWindow())
        check(not w:canClimbThrough(nil))
        w:ToggleWindow(nil)
        check(w:canClimbThrough(nil) and w:canClimbThrough(player))
        check(frame:canClimbThrough(nil) and frame:canClimbThrough(player))
        check(wall:canClimbThrough(nil) and not other:canClimbThrough(nil))
        check(not frame:canClimbThrough(animal) and not w:canClimbThrough(animal))
        check(not wall:canClimbThrough(animal))
    "#);
    with_world(|world| {
        let square = world.cell.as_mut().unwrap().get_grid_square_mut(1, 2, 0).unwrap();
        square.borrow_mut().objects.remove(1);
    });
    f.run("check(not frame:canClimbThrough(nil))");
}
