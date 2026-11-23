//! Water and shore geometry from loaded square sprites and the map's water zones.
use crate::RenderWorld;
use pz_iso_render::water::{WaterGeometry, WaterInit, WATER_NEIGHBOURS};
use pz_iso_render::water_flow::IsoWaterFlow;
use pz_iso_render::world::RenderSquare;

impl RenderWorld {
    /// Shore corners use the adjacent squares and flow slows within four
    /// squares of land, so loading a chunk can change its neighbours.
    pub(crate) fn water_changed(&mut self, wx: i32, wy: i32) {
        for y in wy - 1..=wy + 1 {
            for x in wx - 1..=wx + 1 { self.water_chunks.insert((x, y)); }
        }
    }

    fn water_geometry(&self, sq: &RenderSquare, flow: &IsoWaterFlow, shader: bool) -> Option<WaterGeometry> {
        let water = sq.has(|f| f.water);
        let floor = sq.floor().and_then(|i| sq.objects[i].sprite.as_ref()).map(|s| s.name.as_str());
        if !water && !floor.is_some_and(|name| name.starts_with("blends_natural")) { return None; }
        let flag_at = |x, y, z| self.squares.get(&(x, y, z)).map(|s| s.has(|f| f.water));
        let neighbours = WATER_NEIGHBOURS.map(|(dx, dy)| flag_at(sq.x + dx, sq.y + dy, sq.z));
        let init = WaterInit { water, exterior: sq.has(|f| f.exterior), shore_zone: flow.get_shore(sq.x, sq.y),
            floor_sprite: floor, neighbours, shader, ..Default::default() };
        let mut geometry = WaterGeometry::init(&init).filter(|g| g.is_valid())?;
        for (i, (angle, speed)) in flow.corner_flows(sq.x, sq.y, |x,y| flag_at(x,y,0).unwrap_or(false)).into_iter().enumerate() {
            geometry.flow[i] = angle;
            geometry.speed[i] = speed;
        }
        Some(geometry)
    }

    /// Changed chunk textures need a redraw when water switches between the
    /// floor and translucent passes or its corner depths change.
    pub fn refresh_water(&mut self, flow: &IsoWaterFlow, shader: bool) -> Vec<(i32,i32)> {
        let settings = (flow.revision(), shader);
        if self.water_settings != Some(settings) {
            self.water_chunks.extend(self.chunks.keys().copied());
            self.water_settings = Some(settings);
        }
        let mut changed = Vec::new();
        for (wx, wy) in std::mem::take(&mut self.water_chunks) {
            let Some(&(low, high)) = self.chunks.get(&(wx, wy)) else { continue };
            let mut redraw = false;
            for z in low..=high {
                for y in wy * 8..wy * 8 + 8 {
                    for x in wx * 8..wx * 8 + 8 {
                        let pos = (x,y,z);
                        let Some(sq) = self.squares.get(&pos) else { continue };
                        let geometry = self.water_geometry(sq, flow, shader);
                        if sq.water != geometry {
                            self.squares.get_mut(&pos).unwrap().water = geometry;
                            redraw = true;
                        }
                    }
                }
            }
            if redraw { changed.push((wx, wy)); }
        }
        changed
    }
}
