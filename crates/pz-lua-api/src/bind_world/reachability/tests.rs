use super::*;
use pz_world::objects::{IsoBarricade, IsoDoor, IsoObject, IsoThumpable, IsoWindow, ObjectKind};
use pz_world::sprite::{test_tables, IsoSpriteManager, SpriteIdx};
use pz_world::{IsoChunk, IsoGridSquare};

struct Fixture {
    thread: Thread,
    sprites: Rc<RefCell<IsoSpriteManager>>,
    cut: SpriteIdx,
    old_sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    old_cell: Option<IsoCell>,
    old_weather: Option<Rc<RefCell<pz_iso_render::weather_fx::IsoWeatherFx>>>,
}

impl Fixture {
    fn new() -> Self {
        let old_sprites = crate::bind_sprites::manager().ok();
        let (old_cell, old_weather) =
            with_world(|world| (world.cell.take(), world.weather_fx.take()));
        let mut sprites = IsoSpriteManager::new(test_tables());
        let cut = sprites.add_sprite("authored_cut", 501);
        sprites.sprites[cut].name = Some("authored_cut".into());
        sprites.sprites[cut].cut_n = true;
        let sprites = Rc::new(RefCell::new(sprites));
        crate::bind_sprites::install_sprite_manager(Some(sprites.clone()));
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(0, 0);
        for (x, y, z) in [(1, 1, 0), (1, 2, 0), (2, 1, 0), (4, 1, 0), (1, 1, 1)] {
            chunk.set_square(x, y, z, Some((IsoGridSquare::new(x, y, z)).into()));
        }
        cell.chunks.insert((0, 0), chunk);
        set_cell(Some(cell));
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        super::super::register(&mut thread);
        crate::bind_iso_enums::register_with(
            &mut thread,
            &crate::bind_iso_enums::IsoClassData {
                ignore_blocking_sprites: Some(pz_classfile::Value::New {
                    class: "java/util/ArrayList".into(),
                    args: Vec::new(),
                }),
                ..Default::default()
            },
        );
        let mut fixture = Self {
            thread,
            sprites,
            cut,
            old_sprites,
            old_cell,
            old_weather,
        };
        fixture.run(r#"
            function check(value) if not value then error('authored reachability check failed') end end
            local cell = IsoCell.getInstance()
            a, b, east, far, above = cell:getGridSquare(1,1,0), cell:getGridSquare(1,2,0),
                cell:getGridSquare(2,1,0), cell:getGridSquare(4,1,0), cell:getGridSquare(1,1,1)
        "#);
        fixture
    }

    fn run(&mut self, source: &str) {
        let closure = pz_kahlua::stdlib::load_string(
            source,
            Some("reachability-test"),
            self.thread.globals().clone(),
        )
        .unwrap();
        self.thread.pcall(&closure, &[]).unwrap();
    }

    fn object(&self, object: ObjectKind, sprite: Option<SpriteIdx>) {
        with_world(|world| {
            let square = world
                .cell
                .as_mut()
                .unwrap()
                .get_grid_square_mut(1, 2, 0)
                .unwrap();
            let mut square = square.borrow_mut();
            square.objects = vec![pz_world::ObjectSlot::new(IsoObject::new(1, object, sprite))];
            square.special_objects = vec![1];
            square.recalc_properties(&self.sprites.borrow());
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
fn reachability_preserves_same_square_range_null_and_stale_errors() {
    let mut fixture = Fixture::new();
    fixture.run(r#"
        local ok, error = pcall(function() a:canReachTo(nil) end)
        check(not ok and string.find(tostring(error), 'java.lang.NullPointerException', 1, true) ~= nil)
    "#);
    fixture.run("check(a:canReachTo(a)); check(a:canReachTo(b)); check(not a:canReachTo(far)); check(not a:canReachTo(above))");
    crate::bind_sprites::install_sprite_manager(None);
    fixture.run("check(a:canReachTo(a)); check(not a:canReachTo(far)); check(not a:canReachTo(above)); check(not pcall(function() a:canReachTo(b) end))");
    let closure = pz_kahlua::stdlib::load_string(
        "return a:canReachTo(nil)",
        Some("null-square-test"),
        fixture.thread.globals().clone(),
    )
    .unwrap();
    assert!(matches!(
        fixture.thread.call(&closure, &[]).unwrap_err(),
        LuaError::Runtime {
            class: "java.lang.NullPointerException",
            ..
        }
    ));
    crate::bind_sprites::install_sprite_manager(Some(fixture.sprites.clone()));
    set_cell(Some(IsoCell::default()));
    fixture.run("check(not pcall(function() a:canReachTo(b) end))");
}

#[test]
fn reachability_reads_live_nullable_sprite_exclusions_without_disabling_other_edges() {
    let mut fixture = Fixture::new();
    fixture.object(ObjectKind::Object, Some(fixture.cut));
    fixture.run(
        r#"
        exclusions = IsoGridSquare.ignoreBlockingSprites
        check(not a:canReachTo(b) and not b:canReachTo(a))
        exclusions:add(true) exclusions:add({})
        check(not a:canReachTo(b))
        exclusions:add('authored_cut')
        check(a:canReachTo(b) and b:canReachTo(a))
        exclusions:remove('authored_cut')
        check(not a:canReachTo(b))
    "#,
    );
    fixture.sprites.borrow_mut().sprites[fixture.cut].name = None;
    fixture.run("check(not a:canReachTo(b)); exclusions:add(nil); check(a:canReachTo(b)); exclusions:remove(nil); check(not a:canReachTo(b))");
    fixture.run("exclusions:add(nil)");
    let flag = fixture.sprites.borrow().tables.flag.collideN;
    fixture.sprites.borrow_mut().sprites[fixture.cut]
        .properties
        .set_flag(flag);
    fixture.object(ObjectKind::Object, Some(fixture.cut));
    fixture.run("check(not a:canReachTo(b))");
}

#[test]
fn reachability_uses_live_doors_windows_thumpables_and_barricades() {
    let mut fixture = Fixture::new();
    fixture.object(
        ObjectKind::Door(IsoDoor {
            north: true,
            open: false,
            ..Default::default()
        }),
        None,
    );
    fixture.run("check(not a:canReachTo(b))");
    fixture.object(
        ObjectKind::Door(IsoDoor {
            north: true,
            open: true,
            ..Default::default()
        }),
        None,
    );
    fixture.run("check(a:canReachTo(b))");
    fixture.object(
        ObjectKind::Window(IsoWindow {
            north: true,
            open: false,
            ..Default::default()
        }),
        None,
    );
    fixture.run("check(not a:canReachTo(b))");
    fixture.object(
        ObjectKind::Window(IsoWindow {
            north: true,
            open: true,
            ..Default::default()
        }),
        None,
    );
    fixture.run("check(a:canReachTo(b))");
    with_world(|world| {
        let square = world
            .cell
            .as_mut()
            .unwrap()
            .get_grid_square_mut(1, 2, 0)
            .unwrap();
        let mut square = square.borrow_mut();
        square
            .objects
            .push(pz_world::ObjectSlot::new(IsoObject::new(
                2,
                ObjectKind::Barricade(IsoBarricade {
                    dir: 0,
                    plank_health: [10, 0, 0, 0],
                    metal_health: 0,
                    metal_bar_health: 0,
                }),
                None,
            )));
        square.special_objects.push(2);
    });
    fixture.run("check(not a:canReachTo(b))");
    fixture.object(
        ObjectKind::Thumpable(Box::new(IsoThumpable {
            north: true,
            is_door: true,
            open: false,
            ..Default::default()
        })),
        None,
    );
    fixture.run("check(not a:canReachTo(b))");
    fixture.object(
        ObjectKind::Thumpable(Box::new(IsoThumpable {
            north: true,
            is_door: true,
            open: true,
            ..Default::default()
        })),
        None,
    );
    fixture.run("check(a:canReachTo(b))");
}
