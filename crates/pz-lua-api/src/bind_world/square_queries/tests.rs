use super::*;
use pz_world::sprite::{test_tables, IsoSpriteManager};
use pz_world::{IsoChunk, IsoGridSquare};

struct Fixture {
    thread: Thread,
    sprites: Rc<RefCell<IsoSpriteManager>>,
    old_sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    old_cell: Option<IsoCell>,
    old_weather: Option<Rc<RefCell<pz_iso_render::weather_fx::IsoWeatherFx>>>,
}

fn authored_cell() -> IsoCell {
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some((IsoGridSquare::new(1, 1, 0)).into()));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    cell
}

impl Fixture {
    fn new() -> Self {
        let old_sprites = crate::bind_sprites::manager().ok();
        let (old_cell, old_weather) =
            with_world(|world| (world.cell.take(), world.weather_fx.take()));
        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(test_tables())));
        crate::bind_sprites::install_sprite_manager(Some(sprites.clone()));
        set_cell(Some(authored_cell()));
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        super::super::register(&mut thread);
        let mut fixture = Self {
            thread,
            sprites,
            old_sprites,
            old_cell,
            old_weather,
        };
        fixture.run("function assert(value) if not value then error('authored square query failed') end end; a = IsoCell.getInstance():getGridSquare(1,1,0); outside = a.isOutside");
        fixture
    }

    fn run(&mut self, source: &str) {
        let closure = pz_kahlua::stdlib::load_string(
            source,
            Some("authored-square-query"),
            self.thread.globals().clone(),
        )
        .unwrap();
        self.thread.pcall(&closure, &[]).unwrap();
    }

    fn square(&self, f: impl FnOnce(&mut IsoGridSquare)) {
        with_world(|world| {
            f(&mut world
                .cell
                .as_mut()
                .unwrap()
                .get_grid_square_mut(1, 1, 0)
                .unwrap()
                .borrow_mut())
        });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_cell(self.old_cell.take());
        with_world(|world| world.weather_fx = self.old_weather.take());
        crate::bind_sprites::install_sprite_manager(self.old_sprites.take());
    }
}

#[test]
fn is_outside_reads_live_cached_flag_without_recalculating_room_or_roof() {
    let mut f = Fixture::new();
    // Fresh properties are empty, although there is no room or roof.
    f.run("assert(not a:isOutside())");
    f.square(|square| square.recalc_properties(&f.sprites.borrow()));
    f.run("assert(a:isOutside())");
    f.square(|square| {
        square.room_id = 7;
        square.have_roof = true;
    });
    // Deliberately contradictory state: the getter must read cached flags.
    f.run("assert(a:isOutside())");
    let exterior = f.sprites.borrow().tables.flag.exterior;
    f.square(|square| square.properties.unset_flag(exterior));
    f.run("assert(not a:isOutside())");
    f.square(|square| square.properties.set_flag(exterior));
    f.run("assert(a:isOutside())");
    f.square(|square| square.recalc_properties(&f.sprites.borrow()));
    f.run("assert(not a:isOutside())");
    f.square(|square| {
        square.room_id = -1;
        square.have_roof = false;
    });
    f.run("assert(not a:isOutside())");
    f.square(|square| square.recalc_properties(&f.sprites.borrow()));
    f.run("assert(a:isOutside())");
}

#[test]
fn is_outside_preserves_null_unloaded_and_stale_cell_contracts() {
    let mut f = Fixture::new();
    let null_call = pz_kahlua::stdlib::load_string(
        "return outside(nil)",
        Some("authored-null-square-query"),
        f.thread.globals().clone(),
    )
    .unwrap();
    let (_, _, error) = f.thread.pcall_ex(&null_call, &[]).unwrap_err();
    assert!(
        matches!(
            error,
            LuaError::Runtime {
                class: "java.lang.NullPointerException",
                ..
            }
        ),
        "unexpected null error: {error:?}"
    );
    f.run(
        r#"
        local ok, err = pcall(function() outside(nil) end)
        assert(not ok and string.find(tostring(err), 'java.lang.NullPointerException', 1, true))
        assert(IsoCell.getInstance():getGridSquare(7,7,0) == nil)
    "#,
    );
    with_world(|world| {
        world
            .cell
            .as_mut()
            .unwrap()
            .chunks
            .get_mut(&(0, 0))
            .unwrap()
            .set_square(1, 1, 0, None)
    });
    f.run(
        r#"
        local ok, err = pcall(function() return a:isOutside() end)
        assert(not ok and string.find(tostring(err), 'no longer loaded', 1, true))
    "#,
    );
    let mut next = authored_cell();
    let exterior = f.sprites.borrow().tables.flag.exterior;
    next.get_grid_square_mut(1, 1, 0)
        .unwrap()
        .borrow_mut()
        .properties
        .set_flag(exterior);
    set_cell(Some(next));
    f.run(
        r#"
        local ok, err = pcall(function() return a:isOutside() end)
        assert(not ok and string.find(tostring(err), 'stale', 1, true))
        local current = IsoCell.getInstance():getGridSquare(1,1,0)
        assert(current:isOutside())
    "#,
    );
}

#[test]
fn is_outside_uses_current_sprite_owner_and_reports_unavailable_borrows() {
    let mut f = Fixture::new();
    let exterior = f.sprites.borrow().tables.flag.exterior;
    f.square(|square| square.properties.set_flag(exterior));
    f.run("assert(a:isOutside())");
    {
        let sprites = f.sprites.clone();
        let _held = sprites.borrow_mut();
        f.run(
            r#"
            local ok, err = pcall(function() return a:isOutside() end)
            assert(not ok and string.find(tostring(err), 'IsoSpriteManager is in use', 1, true))
        "#,
        );
    }
    crate::bind_sprites::install_sprite_manager(None);
    f.run(
        r#"
        local ok, err = pcall(function() return a:isOutside() end)
        assert(not ok and string.find(tostring(err), 'tile definitions are not loaded', 1, true))
    "#,
    );
    crate::bind_sprites::install_sprite_manager(Some(f.sprites.clone()));
    f.run("assert(a:isOutside())");
}

#[test]
fn empty_square_lists_and_vehicle_container() {
    let mut f = Fixture::new();
    f.run(
        r#"
        assert(a:getWorldObjects():size() == 0)
        assert(a:getObjects():size() == 0)
        assert(a:getStaticMovingObjects():size() == 0)
        assert(a:getVehicleContainer() == nil)
    "#,
    );
}

#[test]
fn light_level_reads_installed_source_and_defaults_to_white() {
    let mut f = Fixture::new();
    install_square_light(None);
    f.run("assert(a:getLightLevel(0) == 1)");
    f.run("assert(not a:isSeen(0) and not a:isCanSee(0) and not a:isCouldSee(0))");
    install_square_light(Some(Box::new(|player, x, y, z| {
        assert_eq!((x, y, z), (1, 1, 0));
        match player {
            2 => Some(SquareLighting { light_level: 0.25, vis: 2 }),
            3 => None,
            _ => Some(SquareLighting { light_level: 0.5, vis: 5 }),
        }
    })));
    f.run(
        r#"
        assert(a:getLightLevel(2) == 0.25)
        assert(a:getLightLevel(-1) == 0.5)
        assert(a:getLightLevel(3) == 1)
        assert(a:isCanSee(2) and not a:isSeen(2) and not a:isCouldSee(2))
        assert(a:isSeen(0) and not a:isCanSee(0) and a:isCouldSee(0))
        local ok, err = pcall(function() return a:getLightLevel(4) end)
        assert(not ok and string.find(tostring(err), 'ArrayIndexOutOfBounds', 1, true))
        ok, err = pcall(function() return a:isCanSee(-1) end)
        assert(not ok and string.find(tostring(err), 'ArrayIndexOutOfBounds', 1, true))
    "#,
    );
    install_square_light(None);
}

#[test]
fn is_free_needs_a_solid_floor_without_solid_objects() {
    let mut f = Fixture::new();
    let flags = f.sprites.borrow().tables.flag;
    f.run("assert(a:isFree(false) == false)");
    f.square(|square| square.properties.set_flag(flags.solidfloor));
    f.run("assert(a:isFree(false) == true)");
    f.square(|square| square.properties.set_flag(flags.solidtrans));
    f.run("assert(a:isFree(false) == false)");
    f.run("local ok, err = pcall(a.isFree, a, true); assert(not ok and tostring(err):find('movingObjects'))");
}

#[test]
fn objects_listed_from_a_square_give_their_containers_that_source_grid() {
    let old_items = pz_inventory::shared::install_item_observer(Some(
        crate::component_gc::observe_item,
    ));
    let old_objects = pz_world::object_slot::install_object_observer(Some(
        crate::component_gc::observe_object,
    ));
    let mut f = Fixture::new();
    crate::bind_item_container::register(&mut f.thread);
    crate::bind_world_object::register_classes(&mut f.thread);
    f.square(|square| {
        let mut fridge = pz_world::IsoObject::new(9, pz_world::ObjectKind::Object, None);
        fridge.container = Some(pz_inventory::ItemContainer::new(Some("fridge")));
        square.objects.push(pz_world::ObjectSlot::new(fridge).into());
    });
    // The `ItemContainer` constructor for a map object sets the square as
    // the source grid (ItemPicker needs it).
    f.run(
        r#"
        local c = a:getObjects():get(0):getContainer()
        local g = c:getSourceGrid()
        assert(g ~= nil and g:getX() == 1 and g:getY() == 1 and g:getZ() == 0)
        assert(c:getParent() == a:getObjects():get(0))
    "#,
    );
    drop(f);
    pz_inventory::shared::install_item_observer(old_items);
    pz_world::object_slot::install_object_observer(old_objects);
}

#[test]
fn an_object_read_from_a_square_reports_it_until_it_leaves() {
    let mut f = Fixture::new();
    crate::bind_world_object::register_classes(&mut f.thread);
    crate::world_menu_logic::register(&mut f.thread);
    let slot = pz_world::ObjectSlot::new(pz_world::IsoObject::new(7, pz_world::ObjectKind::Object, None));
    crate::bind_world_object::ensure_class(&slot.ref_handle(), crate::bind_world_object::ISO_OBJECT).unwrap();
    f.square(|sq| sq.objects.push(slot));
    f.run("o = a:getObjects():get(0); assert(o:getSquare() == a)");
    f.square(|sq| drop(sq.objects.pop()));
    f.run("assert(o:getSquare() == nil)");
}

#[test]
fn menu_reads_of_an_outdoor_square() {
    let mut f = Fixture::new();
    crate::bind_world_object::register_classes(&mut f.thread);
    crate::bind_sprites::register(&mut f.thread);
    crate::world_menu_logic::register(&mut f.thread);
    let slot = pz_world::ObjectSlot::new(pz_world::IsoObject::new(7, pz_world::ObjectKind::Object, None));
    crate::bind_world_object::ensure_class(&slot.ref_handle(), crate::bind_world_object::ISO_OBJECT).unwrap();
    f.square(|sq| sq.objects.push(slot));
    f.run(
        r#"
        assert(a:getNextNonItemObjectIndex(0) == 0)
        assert(a:getNextNonItemObjectIndex(1) == -1)
        assert(not a:getProperties():has("fuelAmount"))
        assert(a:getRoom() == nil and a:getBuilding() == nil)
        assert(a:getDeadBody() == nil and a:getAnimalTrack() == nil and not a:haveGraffiti() and not a:haveGrime())
        local o = a:getObjects():get(0)
        assert(o:getProperties() == nil and o:getAttachedAnimSprite() == nil)
        assert(o:getProperty("CustomName") == nil)
        assert(a:getOre() == nil and o:getSpriteName() == nil)
        assert(not o:hasFluid())
    "#,
    );
}

#[test]
fn moving_objects_of_a_square() {
    let mut f = Fixture::new();
    let sm = pz_scripts::ScriptManager::new();
    let chr = Rc::new(RefCell::new(crate::bind_character_sim::LuaGameCharacter::new(
        &sm,
        pz_characters::CharacterKind::Player,
    )));
    chr.borrow_mut().id = 5;
    crate::bind_players::set_player(0, Some(chr.clone()));
    let p = crate::bind_character_sim::character_value(&chr);
    f.thread.globals().rawset(pz_kahlua::Value::str("p"), p);
    f.run("assert(a:getMovingObjects():size() == 0)");
    f.square(|sq| sq.moving_objects.push(5));
    f.run("local l = a:getMovingObjects(); assert(l:size() == 1 and l:get(0) == p)");
    f.square(|sq| sq.moving_objects.push(9));
    f.run("assert(not pcall(function() return a:getMovingObjects() end))");
    crate::bind_players::set_player(0, None);
}

#[test]
fn static_moving_objects_are_the_square_corpses() {
    let mut f = Fixture::new();
    crate::bind_item_container::register(&mut f.thread);
    crate::bind_dead_body::register(&mut f.thread);
    f.square(|sq| {
        let mut body = pz_world::save::visual::IsoDeadBody::for_tests(1.5, 1.5, 0.0);
        let mut c = pz_inventory::ItemContainer::new(Some("inventorymale"));
        c.id = 3;
        body.container = Some(c);
        sq.static_moving.push(Rc::new(RefCell::new(body)));
    });
    f.run(
        r#"
        local list = a:getStaticMovingObjects()
        assert(list:size() == 1)
        local b = list:get(0)
        assert(b == a:getStaticMovingObjects():get(0))
        assert(b:isZombie() and not b:isFemale() and not b:isAnimal() and not b:isSkeleton())
        assert(b:getX() == 1.5 and b:getY() == 1.5 and b:getZ() == 0)
        assert(b:getSquare() == a)
        assert(b:getStaticMovingObjectIndex() == 0)
        assert(b:DistTo(4, 5) == 6)
        assert(b:DistToSquared(3.5, 1.5) == 4)
        assert(b:DistToSquared(b) == 0)
        local c = b:getContainer()
        assert(c:getType() == "inventorymale")
        assert(c:getParent() == b)
        assert(c:getSourceGrid() == a)
        assert(c == b:getContainer())
    "#,
    );
    // The saved container became the live one.
    f.square(|sq| {
        let body = sq.static_moving[0].borrow();
        assert!(body.container.is_none());
        assert_eq!(body.runtime.as_ref().unwrap().container.borrow().id, 3);
    });
    // A corpse taken off the square has no index.
    let body = {
        let mut out = None;
        f.square(|sq| out = Some(sq.static_moving.remove(0)));
        out.unwrap()
    };
    let v = crate::bind_dead_body::dead_body_value(&body);
    f.thread.globals().set_str("gone", v);
    f.run("assert(gone:getStaticMovingObjectIndex() == -1)");
    f.run("assert(a:getStaticMovingObjects():size() == 0)");
}

#[test]
fn corpse_inventory_weight_counts_worn_and_attached_items() {
    use pz_inventory::{InventoryItem, ItemClass, ItemTemplate, SavedItem};
    let mut env = crate::bind_items::GameItemEnv::new(None);
    env.encumbrance_multiplier = Some(0.3);
    crate::bind_items::install_item_env(Rc::new(env));
    let mut f = Fixture::new();
    crate::bind_item_container::register(&mut f.thread);
    crate::bind_dead_body::register(&mut f.thread);
    let item = |ty: &str, weight: f32, slot: i32| {
        let mut t = ItemTemplate::new(ItemClass::Normal, "Base", ty);
        t.actual_weight = weight;
        let mut i = InventoryItem::create(Rc::new(t), 1);
        i.attached_slot = slot;
        SavedItem::Item(Box::new(i))
    };
    f.square(|sq| {
        let mut body = pz_world::save::visual::IsoDeadBody::for_tests(1.5, 1.5, 0.0);
        let mut c = pz_inventory::ItemContainer::new(Some("inventorymale"));
        // Unequipped, worn (equipped) and attached but not worn.
        c.items.push(item("Rock", 2.0, -1));
        c.items.push(item("Shirt", 1.0, -1));
        c.items.push(item("Knife", 1.0, 0));
        body.container = Some(c);
        body.worn_items = vec![("Shirt".into(), 1)];
        sq.static_moving.push(Rc::new(RefCell::new(body)));
    });
    f.run(
        r#"
        local b = a:getStaticMovingObjects():get(0)
        local w = b:getInventoryWeight()
        assert(math.abs(w - 3.0) < 1e-5)
        assert(math.abs(b:getContainer():getCapacityWeight() - w) < 1e-6)
    "#,
    );
    // Without a container the weight is 0.
    f.square(|sq| sq.static_moving[0].borrow_mut().runtime = None);
    f.run("assert(a:getStaticMovingObjects():get(0):getInventoryWeight() == 0)");
}

#[test]
fn corpse_has_the_iso_object_methods() {
    let mut f = Fixture::new();
    crate::bind_item_container::register(&mut f.thread);
    crate::bind_dead_body::register(&mut f.thread);
    // A method of `IsoObject` registered after the corpse class.
    let object = crate::expose::expose_class(&mut f.thread, crate::bind_world_object::ISO_OBJECT, None);
    object.method("parentMethod", |c| Ok(c.push(Value::Num(7.0))));
    f.square(|sq| {
        let body = pz_world::save::visual::IsoDeadBody::for_tests(1.5, 1.5, 0.0);
        sq.static_moving.push(Rc::new(RefCell::new(body)));
    });
    f.run(
        r#"
        local b = a:getStaticMovingObjects():get(0)
        assert(b:parentMethod() == 7)
        assert(not b:isPropaneBBQ() and not b:hasPropaneTank())
        assert(not b:isFireInteractionObject() and not b:isLit())
        assert(b:getName() == nil and b:getSprite() == nil)
    "#,
    );
}

#[test]
fn special_objects_floor_stairs_dead_body_and_vehicle_queries() {
    let mut f = Fixture::new();
    crate::bind_dead_body::register(&mut f.thread);
    f.square(|sq| {
        let body = pz_world::save::visual::IsoDeadBody::for_tests(1.5, 1.5, 0.0);
        sq.static_moving.push(Rc::new(RefCell::new(body)));
    });
    f.run(
        r#"
        assert(a:getSpecialObjects():size() == 0)
        assert(a:isSolidTrans() == false)
        assert(a:TreatAsSolidFloor() == false)
        assert(a:HasStairs() == false)
        assert(a:HasStairsNorth() == false)
        assert(a:HasStairsWest() == false)
        assert(a:hasFloor() == false)
        assert(a:isVehicleIntersecting() == false)
        assert(a:getDeadBody() == a:getStaticMovingObjects():get(0))
        a:RecalcProperties()
    "#,
    );
}
