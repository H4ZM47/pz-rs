use super::*;
use pz_characters::CharacterKind;
use pz_vehicles::vehicle::BaseVehicle;
use pz_world::chunk_vehicles::{LoadedVehicle, VehicleOrigin};
use pz_world::IsoChunk;

struct Fixture {
    thread: Thread,
    player: crate::bind_character_sim::LuaGameCharacterRef,
    old_cell: Option<IsoCell>,
}

impl Fixture {
    /// A player at (84.5, 84.5) with chunks (9..=11, 9..=11) loaded.
    fn new() -> Self {
        let old_cell = with_world(|world| world.cell.take());
        let mut cell = IsoCell::default();
        for cx in 9..=11 {
            for cy in 9..=11 {
                cell.chunks.insert((cx, cy), IsoChunk::new(cx, cy));
            }
        }
        set_cell(Some(cell));
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        register(&mut thread);
        let sm = crate::bind_registry::shared_script_manager();
        let mut chr = crate::bind_character_sim::LuaGameCharacter::new(&sm, CharacterKind::Player);
        chr.vars.x = 84.5;
        chr.vars.y = 84.5;
        let player = Rc::new(RefCell::new(chr));
        thread
            .globals()
            .set_str("player", crate::bind_character_sim::character_value(&player));
        let mut f = Self { thread, player, old_cell };
        f.run("function assert(v) if not v then error('assertion failed') end end").unwrap();
        f
    }

    fn run(&mut self, source: &str) -> Result<(), String> {
        let closure = pz_kahlua::stdlib::load_string(
            source,
            Some("player-vehicles"),
            self.thread.globals().clone(),
        )
        .unwrap();
        self.thread
            .pcall(&closure, &[])
            .map(|_| ())
            .map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))
    }

    fn add_to_chunk(&self, chunk: (i32, i32), v: pz_world::chunk_vehicles::VehicleHandle) {
        with_world(|world| {
            world.cell.as_mut().unwrap().chunks.get_mut(&chunk).unwrap().vehicles.add(v)
        });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_cell(self.old_cell.take());
    }
}

#[test]
fn no_vehicle_around_answers_nil() {
    let mut f = Fixture::new();
    f.run("assert(player:getUseableVehicle() == nil); assert(player:getNearVehicle() == nil)")
        .unwrap();
}

#[test]
fn a_vehicle_without_a_script_is_neither_useable_nor_near() {
    let mut f = Fixture::new();
    let car = BaseVehicle::new();
    car.set_position(86.0, 84.5, 0.0);
    f.add_to_chunk((10, 10), pz_vehicles::cell_link::handle(&car));
    f.run("assert(player:getUseableVehicle() == nil); assert(player:getNearVehicle() == nil)")
        .unwrap();
}

#[test]
fn a_player_in_a_vehicle_gets_nil() {
    let mut f = Fixture::new();
    f.player.borrow_mut().path.vehicle = Some(5);
    f.run("assert(player:getUseableVehicle() == nil); assert(player:getNearVehicle() == nil)")
        .unwrap();
}

#[test]
fn a_vehicle_chunk_loading_has_not_adopted_is_unsupported() {
    let mut f = Fixture::new();
    let loaded = LoadedVehicle {
        origin: VehicleOrigin::Database { sql_id: 1 },
        square: None,
        vehicle: saved_vehicle(),
    };
    f.add_to_chunk((10, 10), Rc::new(loaded));
    for call in ["getUseableVehicle", "getNearVehicle"] {
        let err = f.run(&format!("player:{call}()")).unwrap_err();
        assert!(err.contains("not yet a runtime BaseVehicle"), "{call}: {err}");
    }
}

/// A vehicle as chunk loading leaves it in the list.
fn saved_vehicle() -> pz_world::save::vehicle::BaseVehicle {
    let base = pz_world::save::visual::MovingBase {
        offset_x: 0.0,
        offset_y: 0.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        dir: 0,
        mod_data: None,
    };
    pz_vehicles::save::to_save(&BaseVehicle::new(), &base)
}
