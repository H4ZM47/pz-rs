use super::*;
use pz_sandbox::{OptionDef, OptionKind as DefinitionKind};
use pz_world::sprite::{test_tables, IsoSpriteManager};
use pz_world::world_state::GeneratorPowerRange;
use pz_world::{IsoChunk, IsoGridSquare};

struct Fixture {
    thread: Thread,
    options: SandboxRef,
    sprites: Rc<RefCell<IsoSpriteManager>>,
    old_sandbox: Option<SandboxRef>,
    old_source: Option<world_state::GeneratorRangeSource>,
    old_range: GeneratorPowerRange,
    old_sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    old_cell: Option<IsoCell>,
    old_weather: Option<Rc<RefCell<pz_iso_render::weather_fx::IsoWeatherFx>>>,
}

fn options() -> SandboxRef {
    let defs = [
        (
            "AllowExteriorGenerator",
            DefinitionKind::Boolean { default: true },
        ),
        (
            "GeneratorTileRange",
            DefinitionKind::Integer {
                min: 1,
                max: 100,
                default: 5,
            },
        ),
        (
            "GeneratorVerticalPowerRange",
            DefinitionKind::Integer {
                min: 0,
                max: 32,
                default: 1,
            },
        ),
    ]
    .into_iter()
    .map(|(name, kind)| OptionDef {
        name: name.into(),
        kind,
        translation: None,
        value_translation: None,
    })
    .collect::<Vec<_>>();
    Rc::new(RefCell::new(SandboxOptions::new(&defs)))
}

fn cell_with_generators() -> IsoCell {
    let mut cell = IsoCell::default();
    for (wx, wy, positions, generator) in [
        (0, 0, vec![(1i32, 1i32, 0), (6, 1, 0), (7, 1, 0)], (1, 1, 0)),
        (
            -1,
            -1,
            vec![(-1, -1, -1), (-1, -1, -2), (-1, -1, -3)],
            (-1, -1, -1),
        ),
    ] {
        let mut chunk = IsoChunk::new(wx, wy);
        chunk.set_min_max_level(-4, 1);
        for (x, y, z) in positions {
            chunk.set_square(
                x.rem_euclid(8),
                y.rem_euclid(8),
                z,
                Some(IsoGridSquare::new(x, y, z).into()),
            );
        }
        chunk.saved.generators = Some(vec![generator]);
        cell.chunks.insert((wx, wy), chunk);
    }
    cell
}

impl Fixture {
    fn new() -> Self {
        let old_sandbox = SANDBOX.with(|current| current.borrow().clone());
        let old_source = world_state::install_generator_range_source(None);
        let old_range = world_state::generator_power_range();
        world_state::set_generator_power_range(GeneratorPowerRange::default());
        let old_sprites = crate::bind_sprites::manager().ok();
        let (old_cell, old_weather) =
            with_world(|world| (world.cell.take(), world.weather_fx.take()));
        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(test_tables())));
        crate::bind_sprites::install_sprite_manager(Some(sprites.clone()));
        let options = options();
        install_sandbox(Some(options.clone()));
        set_cell(Some(cell_with_generators()));
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        super::super::register(&mut thread);
        let mut fixture = Self {
            thread,
            options,
            sprites,
            old_sandbox,
            old_source,
            old_range,
            old_sprites,
            old_cell,
            old_weather,
        };
        fixture.run("function assert(v) if not v then error('authored generator query failed') end end; a=getCell():getGridSquare(1,1,0); edge=getCell():getGridSquare(6,1,0); outside=getCell():getGridSquare(7,1,0); query=a.haveElectricity");
        fixture
    }

    fn run(&mut self, code: &str) {
        let closure = pz_kahlua::stdlib::load_string(
            code,
            Some("authored-generator-power"),
            self.thread.globals().clone(),
        )
        .unwrap();
        self.thread.pcall(&closure, &[]).unwrap();
    }

    fn integer(&self, name: &str, value: i32) {
        let mut options = self.options.borrow_mut();
        let OptionKind::Integer(option) =
            &mut options.option_by_name_mut(name).unwrap().config.kind
        else {
            panic!()
        };
        option.value = value;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_cell(self.old_cell.take());
        with_world(|world| world.weather_fx = self.old_weather.take());
        crate::bind_sprites::install_sprite_manager(self.old_sprites.take());
        install_sandbox(self.old_sandbox.take());
        world_state::install_generator_range_source(self.old_source);
        world_state::set_generator_power_range(self.old_range);
    }
}

#[test]
fn electricity_reads_real_generator_membership_and_constructor_latched_ranges() {
    let mut f = Fixture::new();
    // Installing current sandbox values does not perform a constructor.
    assert_eq!(
        world_state::generator_power_range(),
        GeneratorPowerRange::default()
    );
    f.run("assert(a:haveElectricity()); assert(edge:haveElectricity()); assert(outside:haveElectricity())");
    world_state::generator_constructed().unwrap();
    f.run("assert(edge:haveElectricity()); assert(not outside:haveElectricity())");
    f.integer("GeneratorTileRange", 1);
    f.run("assert(edge:haveElectricity())");
    world_state::generator_constructed().unwrap();
    f.run("assert(a:haveElectricity()); assert(not edge:haveElectricity())");
    with_world(|world| {
        world
            .cell
            .as_mut()
            .unwrap()
            .chunks
            .get_mut(&(0, 0))
            .unwrap()
            .saved
            .generators = None
    });
    // Hydro/grid power does not substitute for an actual touching generator.
    f.run("assert(not a:haveElectricity())");
    with_world(|world| {
        world
            .cell
            .as_mut()
            .unwrap()
            .chunks
            .get_mut(&(0, 0))
            .unwrap()
            .saved
            .generators = Some(vec![(1, 1, 0)])
    });
    f.run("assert(a:haveElectricity())");
}

#[test]
fn electricity_uses_live_exterior_flag_and_negative_coordinate_level_ranges() {
    let mut f = Fixture::new();
    world_state::generator_constructed().unwrap();
    f.run("assert(getCell():getGridSquare(-1,-1,-2):haveElectricity()); assert(not getCell():getGridSquare(-1,-1,-3):haveElectricity())");
    let exterior = f.sprites.borrow().tables.flag.exterior;
    with_world(|world| {
        let sq = world
            .cell
            .as_ref()
            .unwrap()
            .get_grid_square(1, 1, 0)
            .unwrap();
        let mut sq = sq.borrow_mut();
        sq.properties.set_flag(exterior);
        sq.room_id = 99;
        sq.have_roof = true;
    });
    let mut options = f.options.borrow_mut();
    let OptionKind::Boolean(option) = &mut options
        .option_by_name_mut("AllowExteriorGenerator")
        .unwrap()
        .config
        .kind
    else {
        panic!()
    };
    option.value = false;
    drop(options);
    f.run("assert(not a:haveElectricity())");
    with_world(|world| {
        world
            .cell
            .as_ref()
            .unwrap()
            .get_grid_square(1, 1, 0)
            .unwrap()
            .borrow_mut()
            .properties
            .unset_flag(exterior)
    });
    f.run("assert(a:haveElectricity())");
}

#[test]
fn electricity_reports_real_owner_conflicts_and_null_stale_unloaded_receivers() {
    let mut f = Fixture::new();
    f.run("local ok,err=pcall(function() query(nil) end); assert(not ok and string.find(tostring(err),'NullPointerException',1,true))");
    {
        let options = f.options.clone();
        let _held = options.borrow_mut();
        f.run("local ok,err=pcall(function() a:haveElectricity() end); assert(not ok and string.find(tostring(err),'sandbox owner is in use',1,true))");
        assert!(world_state::generator_constructed()
            .unwrap_err()
            .0
            .contains("in use"));
    }
    {
        let sprites = f.sprites.clone();
        let _held = sprites.borrow_mut();
        f.run("local ok,err=pcall(function() a:haveElectricity() end); assert(not ok and string.find(tostring(err),'IsoSpriteManager is in use',1,true))");
    }
    let square = with_world(|world| {
        world
            .cell
            .as_ref()
            .unwrap()
            .get_grid_square(1, 1, 0)
            .unwrap()
    });
    {
        let _held = square.borrow_mut();
        f.run("local ok,err=pcall(function() a:haveElectricity() end); assert(not ok and string.find(tostring(err),'IsoGridSquare is in use',1,true))");
    }
    install_sandbox(None);
    f.run("local ok,err=pcall(function() a:haveElectricity() end); assert(not ok and string.find(tostring(err),'sandbox owner is not installed',1,true))");
    install_sandbox(Some(f.options.clone()));
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
    f.run("local ok,err=pcall(function() a:haveElectricity() end); assert(not ok and string.find(tostring(err),'no longer loaded',1,true))");
    set_cell(Some(cell_with_generators()));
    f.run("local ok,err=pcall(function() a:haveElectricity() end); assert(not ok and string.find(tostring(err),'stale',1,true)); assert(getCell():getGridSquare(1,1,0):haveElectricity())");
}

#[test]
fn actual_sandbox_missing_radius_keeps_native_vertical_prefix_and_prior_radius() {
    let f = Fixture::new();
    let mut options = f.options.borrow_mut();
    options
        .option_by_name_mut("GeneratorTileRange")
        .unwrap()
        .config
        .kind = pz_core::config::ConfigOption::boolean("GeneratorTileRange", true).kind;
    drop(options);
    let error = world_state::generator_constructed().unwrap_err();
    assert!(error.0.contains("GeneratorTileRange is not an integer"));
    assert_eq!(
        world_state::generator_power_range(),
        GeneratorPowerRange {
            radius: 20,
            vertical: 1
        }
    );
}
