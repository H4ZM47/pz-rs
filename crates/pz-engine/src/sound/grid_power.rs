//! The ambient manager changes the supply at the electricity event's timeline marker.
use pz_lua_api::bind_world;
use pz_sound::emitter::{Ctx, FmodSoundEmitter, ParentObject};
use pz_world::{IsoMetaGrid, RoomRect};

#[derive(Default)]
pub(super) struct GridPower {
    previous: Option<bool>,
    emitter: Option<FmodSoundEmitter>,
    event: i64,
}

impl GridPower {
    pub(super) fn update(&mut self, exists: bool, x: f32, y: f32, count: usize, ctx: &mut Ctx) {
        match self.previous.replace(exists) {
            None => bind_world::power::set_hydro_power_on(exists),
            Some(false) if exists => bind_world::power::set_hydro_power_on(true),
            Some(true) if !exists => {
                let emitter = self
                    .emitter
                    .get_or_insert_with(|| FmodSoundEmitter::new(count));
                if !emitter.is_playing(self.event) {
                    let (px, py) = bind_world::with_world(|w| {
                        w.meta_grid
                            .as_ref()
                            .and_then(|g| nearest_building_point(g, x, y))
                    })
                    .unwrap_or((-1000., -1000.));
                    emitter.set_pos(px, py, 0.);
                    self.event = emitter.play_sound_impl(
                        "WorldEventElectricityShutdown",
                        ParentObject::None,
                        ctx,
                    );
                    if self.event != 0 {
                        ctx.backend.instance_watch_timeline_markers(self.event);
                    }
                }
            }
            _ => {}
        }
        if let Some(emitter) = &mut self.emitter {
            emitter.tick(ctx);
        }
    }

    pub(super) fn marker(&self, id: i64, name: &str) {
        if id != 0 && id == self.event && name == "ElectricityOff" {
            bind_world::power::set_hydro_power_on(false);
        }
    }

    pub(super) fn stop(&mut self, ctx: &mut Ctx) {
        if let Some(mut emitter) = self.emitter.take() {
            emitter.stop_all(ctx);
        }
        self.event = 0;
        self.previous = None;
    }
}

fn nearest_building_point(grid: &IsoMetaGrid, x: f32, y: f32) -> Option<(f32, f32)> {
    let cx = pz_world::java::fastfloor(x / 256.);
    let cy = pz_world::java::fastfloor(y / 256.);
    let mut closest = None;
    let mut distance = f32::MAX;
    for my in cy - 1..=cy + 1 {
        for mx in cx - 1..=cx + 1 {
            let Some(cell) = grid.get_cell_data(mx, my).filter(|c| c.info.is_some()) else {
                continue;
            };
            for &index in &cell.buildings {
                let building = &grid.building_defs[index.0 as usize];
                let dx = (x - building.x as f32)
                    .abs()
                    .min((x - building.x2 as f32).abs());
                let dy = (y - building.y as f32)
                    .abs()
                    .min((y - building.y2 as f32).abs());
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < distance {
                    closest = Some(building);
                    distance = dist;
                }
            }
        }
    }
    let mut point = None;
    let mut best = f32::MAX;
    for &id in &closest?.rooms {
        for rect in &grid.room_defs[id.0 as usize].rects {
            if let Some((dist, position)) = rect_point(rect, x, y) {
                if dist < best {
                    best = dist;
                    point = Some(position);
                }
                if dist <= 0.5 {
                    return point;
                }
            }
        }
    }
    point
}

// RoomRect.getClosestPoint ranks the continuous edge distance before snapping
// along that edge to the center of a tile.
fn rect_point(rect: &RoomRect, x: f32, y: f32) -> Option<(f32, (f32, f32))> {
    let (x1, y1) = (rect.x as f32, rect.y as f32);
    let (x2, y2) = ((rect.x + rect.w) as f32, (rect.y + rect.h) as f32);
    let squared = |px: f32, py: f32| (px - x) * (px - x) + (py - y) * (py - y);
    if rect.w == 1 && rect.h == 1 {
        return Some((squared(x1 + 0.5, y1 + 0.5), (x1 + 0.5, y1 + 0.5)));
    }
    if x >= x1 && x < x2 && y >= y1 && y < y2 {
        let point = (
            pz_world::java::fastfloor(x) as f32 + 0.5,
            pz_world::java::fastfloor(y) as f32 + 0.5,
        );
        return Some((squared(point.0, point.1), point));
    }
    let (mut best, mut point) = (f32::MAX, None);
    if rect.w > 1 {
        let px = x.clamp(x1 + 0.5, x2 - 0.5);
        for py in [y1 + 0.5, y2 - 0.5] {
            let dist = squared(px, py);
            if dist < best {
                best = dist;
                point = Some((px.trunc() + 0.5, py));
            }
        }
    }
    if rect.h > 1 {
        let py = y.clamp(y1 + 0.5, y2 - 0.5);
        for px in [x2 - 0.5, x1 + 0.5] {
            let dist = squared(px, py);
            if dist < best {
                best = dist;
                point = Some((px, py.trunc() + 0.5));
            }
        }
    }
    point.map(|p| (best, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires PZ_GAME_DIR and installed sound banks"]
    fn installed_shutdown_changes_power_at_marker_and_restores_supply() {
        let (mut system, studio, count) = crate::sound::tests::installed_sound(
            "media/scripts/generated/sounds/sounds_ambience.txt",
            "WorldEventElectricityShutdown",
        );
        let mut grid = GridPower::default();
        system.update_player_listener(0, [20., 20., 0.], false);
        system.with(|_, ctx| grid.update(true, 20., 20., count, ctx));
        system.with(|_, ctx| grid.update(false, 20., 20., count, ctx));
        let event = grid.event;
        assert_ne!(event, 0);
        grid.marker(event + 1, "ElectricityOff");
        grid.marker(event, "Lightning");
        assert!(bind_world::power::hydro_power_on());
        let mut block = vec![0.; 960];
        for step in 1..=265 {
            block.fill(0.);
            studio.lock().unwrap().mix(&mut block);
            system.with(|_, ctx| grid.update(false, 20., 20., count, ctx));
            for (id, name, _) in system.backend.take_timeline_markers() {
                grid.marker(id, &name);
            }
            assert_eq!(bind_world::power::hydro_power_on(), step < 265);
        }
        assert_eq!(grid.event, event);
        system.with(|_, ctx| grid.update(true, 20., 20., count, ctx));
        assert!(bind_world::power::hydro_power_on());
        system.with(|_, ctx| grid.stop(ctx));
        bind_world::power::set_hydro_power_on(false);
    }

    #[test]
    fn building_shutdown_source_uses_centers_and_native_edge_distances() {
        let room = RoomRect::new(20, 18, 5, 2);
        assert_eq!(rect_point(&room, 10., 19.1).unwrap().1, (20.5, 19.5));
        assert_eq!(rect_point(&room, 22.25, 16.).unwrap(), (6.25, (22.5, 18.5)));
        assert_eq!(
            rect_point(&room, 24.75, 18.25).unwrap(),
            (0.125, (24.5, 18.5))
        );
        assert_eq!(
            rect_point(&RoomRect::new(10, 11, 1, 1), 10.5, 12.5),
            Some((1., (10.5, 11.5)))
        );
    }
}
