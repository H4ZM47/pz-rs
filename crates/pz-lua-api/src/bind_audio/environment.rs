//! The game-thread queries FMODSoundEmitter makes while ticking a clip.
use crate::bind_character_sim::LuaGameCharacterRef;
use crate::bind_world::with_world_state_cell;
use pz_sound::emitter::{EmitterWorld, PlayerInfo, RoomInfo, SquareInfo};
use pz_world::java::fastfloor;

/// Reads the same world, players, scripts and lighting as Lua.
pub struct GameEmitterWorld;

/// Hearing locals at a simulation boundary, while all player owners are available.
#[derive(Default)]
pub struct HearingPlayers([Option<(f32, f32)>; crate::bind_players::MAX_PLAYERS]);

impl HearingPlayers {
    pub fn capture() -> Self {
        let sound = super::sound_system();
        let system = sound.borrow();
        if system.sounds.env.sound_disabled || system.world.is_server() { return Self::default(); }
        let n = crate::bind_players::num_players().max(0) as usize;
        let scripts = crate::bind_registry::shared_script_manager();
        Self(std::array::from_fn(|i| {
            if i >= n { return None; }
            let actor = crate::bind_players::player(i)?;
            let actor = actor.borrow();
            (!actor.has_trait(&scripts, "DEAF").unwrap_or(false)).then_some((actor.vars.x, actor.vars.y))
        }))
    }

    pub fn in_range(&self, x: f32, y: f32, range: f32) -> bool {
        self.0.iter().flatten().any(|&(px, py)| {
            let squared = f64::from(x - px).powi(2) + f64::from(y - py).powi(2);
            (squared as f32) < range * range
        })
    }
}

impl GameEmitterWorld {
    fn square_at(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        let (room, zone_type) = with_world_state_cell(|state| {
            let state = state.borrow();
            let cell = state.cell.as_ref()?;
            let owner = cell.get_grid_square(x, y, z)?;
            let square = owner.borrow();
            let room = state
                .meta_grid
                .as_ref()
                .and_then(|m| square.room_def(m))
                .map(|r| {
                    // Runtime rooms contain each loaded square once, including
                    // rooms whose definition crosses a chunk or overlaps itself.
                    let mut count = 0;
                    for (index, rect) in r.rects.iter().enumerate() {
                        for yy in rect.y..rect.get_y2() {
                            for xx in rect.x..rect.get_x2() {
                                if r.rects[..index]
                                    .iter()
                                    .any(|p| p.contains(xx as f32, yy as f32))
                                {
                                    continue;
                                }
                                if cell
                                    .get_grid_square(xx, yy, r.level)
                                    .is_some_and(|s| s.borrow().room_id == r.id)
                                {
                                    count += 1;
                                }
                            }
                        }
                    }
                    RoomInfo {
                        id: r.id as u64,
                        building: r.building.map_or(u64::MAX, |b| u64::from(b.0)),
                        num_squares: count,
                        level: r.level,
                    }
                });
            let zone = state.meta_grid.as_ref().and_then(|m| {
                m.get_zone_at(x, y, z)
                    .map(|id| m.zone_defs[id.0 as usize].zone_type.clone())
            });
            Some((room, zone))
        })?;
        let could_see = std::array::from_fn(|i| {
            crate::bind_world::square_queries::square_vis(i as i32, x, y, z) & 4 != 0
        });
        Some(SquareInfo {
            x,
            y,
            z,
            building: room.and_then(|r| (r.building != u64::MAX).then_some(r.building)),
            room,
            zone_type,
            could_see,
        })
    }

    fn player_info(&self, player: LuaGameCharacterRef) -> PlayerInfo {
        let p = player.borrow();
        let square = p
            .current_square_xyz()
            .and_then(|(x, y, z)| self.square_at(x, y, z));
        PlayerInfo {
            x: p.vars.x,
            y: p.vars.y,
            z: p.vars.z,
            zi: fastfloor(p.vars.z),
            deaf: p
                .has_trait(&crate::bind_registry::shared_script_manager(), "DEAF")
                .unwrap_or(false),
            num_nearby_buildings_rooms: p.player.num_nearby_buildings_rooms,
            building: square.as_ref().and_then(|s| s.building),
            square,
        }
    }
}

impl EmitterWorld for GameEmitterWorld {
    fn num_players(&self) -> usize {
        crate::bind_players::num_players().clamp(0, crate::bind_players::MAX_PLAYERS as i32)
            as usize
    }

    fn player(&self, i: usize) -> Option<PlayerInfo> {
        crate::bind_players::player(i).map(|p| self.player_info(p))
    }

    fn main_player(&self) -> Option<(usize, PlayerInfo)> {
        let p = crate::bind_players::player_instance()?;
        let i = usize::try_from(p.borrow().player.player_index).ok()?;
        Some((i, self.player_info(p)))
    }

    fn square(&self, x: f32, y: f32, z: f32) -> Option<SquareInfo> {
        self.square_at(fastfloor(x), fastfloor(y), fastfloor(z))
    }

    fn meta_room_at(&self, x: i32, y: i32, z: i32) -> bool {
        with_world_state_cell(|w| {
            w.borrow()
                .meta_grid
                .as_ref()
                .is_some_and(|m| m.get_room_at(x, y, z).is_some())
        })
    }

    fn current_time_ms(&self) -> i64 {
        crate::player_db::current_time_millis()
    }

    fn is_server(&self) -> bool {
        crate::bind_core::shared_core().borrow().is_server
    }

    fn sound_timeline_position(&self, event: &str, name: &str) -> i32 {
        use pz_scripts::{objects::models::SoundTimelineScript, ScriptType};
        crate::bind_registry::shared_script_manager()
            .get::<SoundTimelineScript>(ScriptType::SoundTimeline, event)
            .map_or(-1, |script| script.borrow().position(name))
    }
}
