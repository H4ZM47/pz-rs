use super::*;
use crate::bind_world::{set_cell, with_world};
use pz_world::objects::{IsoDoor, IsoObject, ObjectKind};
use pz_world::sprite::{test_tables, IsoSpriteManager, PropertyValueMap, TilePropertyAliasMap};
use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoObjectRef, ObjectSlot};

/// Squares on one row: the character at (1,1), a neighbour at (1,2) and a
/// distant one at (4,1).
struct Fixture {
    thread: Thread,
    character: LuaGameCharacterRef,
    sprites: Rc<RefCell<IsoSpriteManager>>,
    old_sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    old_cell: Option<IsoCell>,
    old_observer: Option<pz_world::object_slot::ObjectObserver>,
}

/// Sprite names and the properties they carry.
const SPRITES: [(&str, &[(&str, &str)]); 7] = [
    ("explicit_true", &[("GenericCraftingSurface", "true"), ("Surface", "10")]),
    ("explicit_false", &[("GenericCraftingSurface", "false"), ("Surface", "30")]),
    ("surface_low", &[("Surface", "23")]),
    ("surface_table", &[("Surface", "30")]),
    ("surface_high", &[("Surface", "48")]),
    ("surface_offset", &[("Surface", "30"), ("IsSurfaceOffset", "true")]),
    ("plain", &[]),
];

impl Fixture {
    fn new() -> Self {
        let old_observer = pz_world::object_slot::install_object_observer(Some(
            crate::component_gc::observe_object,
        ));
        let old_sprites = crate::bind_sprites::manager().ok();
        let old_cell = with_world(|world| world.cell.take());
        let mut values = PropertyValueMap::default();
        for (name, values_of) in [
            ("GenericCraftingSurface", &["true", "false"][..]),
            ("Surface", &["10", "23", "30", "48"][..]),
            ("IsSurfaceOffset", &["true"][..]),
        ] {
            values.entry(name).extend(values_of.iter().map(|v| v.to_string()));
        }
        let mut manager = IsoSpriteManager::new(test_tables());
        manager.alias = TilePropertyAliasMap::generate(&values, &[]);
        let alias = manager.alias.clone();
        for (i, (name, properties)) in SPRITES.iter().enumerate() {
            let sprite = manager.add_sprite(name, 700 + i as i32);
            for (property, value) in *properties {
                manager.sprites[sprite]
                    .properties
                    .set_raw(&alias, property, value);
            }
        }
        let sprites = Rc::new(RefCell::new(manager));
        crate::bind_sprites::install_sprite_manager(Some(sprites.clone()));
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(0, 0);
        for (x, y) in [(1, 1), (1, 2), (4, 1)] {
            chunk.set_square(x, y, 0, Some(IsoGridSquare::new(x, y, 0).into()));
        }
        cell.chunks.insert((0, 0), chunk);
        set_cell(Some(cell));
        let (mut thread, character, _) = tests::setup_with(ScriptManager::new());
        crate::bind_items::register(&mut thread);
        crate::bind_world::register(&mut thread);
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
        character.borrow_mut().path.current_square = Some((1, 1, 0));
        Self { thread, character, sprites, old_sprites, old_cell, old_observer }
    }

    /// Puts an object with the named sprite onto a square, after
    /// `blockers`, and exposes it to Lua as `name`.
    fn place(&mut self, name: &str, square: (i32, i32), sprite: &str, blockers: Vec<ObjectKind>) {
        let sprite = self.sprites.borrow().named_map[sprite];
        let mut objects: Vec<ObjectSlot> = blockers
            .into_iter()
            .enumerate()
            .map(|(i, kind)| ObjectSlot::new(IsoObject::new(10 + i as u32, kind, None)))
            .collect();
        let object = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, Some(sprite)));
        let handle: IsoObjectRef = object.ref_handle();
        objects.push(object);
        with_world(|world| {
            let square = world
                .cell
                .as_mut()
                .unwrap()
                .get_grid_square_mut(square.0, square.1, 0)
                .unwrap();
            let mut square = square.borrow_mut();
            square.objects = objects;
            square.recalc_properties(&self.sprites.borrow());
        });
        let value = crate::bind_world_object::object_value(&handle).unwrap();
        self.thread.globals().set_str(name, value);
    }

    fn run(&mut self, source: &str) {
        tests::run(&mut self.thread, source).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_cell(self.old_cell.take());
        crate::bind_sprites::install_sprite_manager(self.old_sprites.take());
        pz_world::object_slot::install_object_observer(self.old_observer);
    }
}

fn closed_north_door() -> ObjectKind {
    ObjectKind::Door(IsoDoor {
        north: true,
        open: false,
        ..Default::default()
    })
}

#[test]
fn a_reachable_surface_object_can_be_used() {
    let mut fixture = Fixture::new();
    // Next to the character, by surface height.
    fixture.place("table", (1, 2), "surface_table", vec![]);
    // On the character's own square, by explicit property.
    fixture.place("explicit", (1, 1), "explicit_true", vec![]);
    fixture.run(
        r#"
        assert(table:isGenericCraftingSurface() == true)
        assert(explicit:isGenericCraftingSurface() == true)
        assert(player:canUseAsGenericCraftingSurface(table) == true)
        assert(player:canUseAsGenericCraftingSurface(explicit) == true)
        "#,
    );
}

#[test]
fn an_object_that_is_not_a_surface_cannot_be_used() {
    let mut fixture = Fixture::new();
    // The explicit property wins over the surface height; heights outside
    // 24 to 47, offset surfaces and plain sprites do not qualify.
    for sprite in ["explicit_false", "surface_low", "surface_high", "surface_offset", "plain"] {
        fixture.place("object", (1, 2), sprite, vec![]);
        fixture.run(
            r#"
            assert(object:isGenericCraftingSurface() == false)
            assert(player:canUseAsGenericCraftingSurface(object) == false)
            "#,
        );
    }
}

#[test]
fn a_surface_on_an_unreachable_square_cannot_be_used() {
    let mut fixture = Fixture::new();
    // Too far away.
    fixture.place("far", (4, 1), "surface_table", vec![]);
    // Next to the character, behind a closed door on the shared edge.
    fixture.place("behind_door", (1, 2), "surface_table", vec![closed_north_door()]);
    fixture.run(
        r#"
        assert(far:isGenericCraftingSurface() == true)
        assert(behind_door:isGenericCraftingSurface() == true)
        assert(player:canUseAsGenericCraftingSurface(far) == false)
        assert(player:canUseAsGenericCraftingSurface(behind_door) == false)
        "#,
    );
    // The same surface is usable once the door is open.
    fixture.place(
        "behind_door",
        (1, 2),
        "surface_table",
        vec![ObjectKind::Door(IsoDoor {
            north: true,
            open: true,
            ..Default::default()
        })],
    );
    fixture.run("assert(player:canUseAsGenericCraftingSurface(behind_door) == true)");
}

#[test]
fn a_missing_object_or_character_square_raises() {
    let mut fixture = Fixture::new();
    fixture.place("table", (1, 2), "surface_table", vec![]);
    fixture.run(
        r#"
        assert(not pcall(function() return player:canUseAsGenericCraftingSurface(nil) end))
        assert(not pcall(function() return player:canUseAsGenericCraftingSurface(12) end))
        "#,
    );
    fixture.character.borrow_mut().path.current_square = None;
    fixture.run(
        r#"
        assert(not pcall(function() return player:canUseAsGenericCraftingSurface(table) end))
        "#,
    );
}
