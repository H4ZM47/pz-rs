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
            for x in wx - 1..=wx + 1 {
                self.water_chunks.insert((x, y));
            }
        }
    }

    fn water_geometry(
        &self,
        sq: &RenderSquare,
        flow: &IsoWaterFlow,
        shader: bool,
    ) -> Option<WaterGeometry> {
        let water = sq.has(|f| f.water);
        let floor = sq
            .floor()
            .and_then(|i| sq.objects[i].sprite.as_ref())
            .map(|s| s.name.as_str());
        if !water && !floor.is_some_and(|name| name.starts_with("blends_natural")) {
            return None;
        }
        let flag_at = |x, y, z| self.squares.get(&(x, y, z)).map(|s| s.has(|f| f.water));
        let neighbours = WATER_NEIGHBOURS.map(|(dx, dy)| flag_at(sq.x + dx, sq.y + dy, sq.z));
        let init = WaterInit {
            water,
            exterior: sq.is_exterior(),
            shore_zone: flow.get_shore(sq.x, sq.y),
            floor_sprite: floor,
            neighbours,
            shader,
            ..Default::default()
        };
        let mut geometry = WaterGeometry::init(&init).filter(|g| g.is_valid())?;
        for (i, (angle, speed)) in flow
            .corner_flows(sq.x, sq.y, |x, y| flag_at(x, y, 0).unwrap_or(false))
            .into_iter()
            .enumerate()
        {
            geometry.flow[i] = angle;
            geometry.speed[i] = speed;
        }
        Some(geometry)
    }

    /// Changed chunk textures need a redraw when water switches between the
    /// floor and translucent passes or its corner depths change.
    pub fn refresh_water(&mut self, flow: &IsoWaterFlow, shader: bool) -> Vec<(i32, i32)> {
        let settings = (flow.revision(), shader);
        if self.water_settings != Some(settings) {
            self.water_chunks.extend(self.chunks.keys().copied());
            self.water_settings = Some(settings);
        }
        let mut changed = Vec::new();
        for (wx, wy) in std::mem::take(&mut self.water_chunks) {
            let Some(&(low, high)) = self.chunks.get(&(wx, wy)) else {
                continue;
            };
            let mut redraw = false;
            for z in low..=high {
                for y in wy * 8..wy * 8 + 8 {
                    for x in wx * 8..wx * 8 + 8 {
                        let pos = (x, y, z);
                        let Some(sq) = self.squares.get(&pos) else {
                            continue;
                        };
                        let geometry = self.water_geometry(sq, flow, shader);
                        let sq = self.squares.get_mut(&pos).unwrap();
                        if sq.water != geometry {
                            sq.water = geometry;
                            redraw = true;
                        }
                        for obj in &mut sq.objects {
                            if let Some(sp) = obj.sprite.as_mut() {
                                let hide = shader
                                    && pz_iso_render::water::hides_for_water_render(&sp.name);
                                if sp.hide_for_water_render != hide {
                                    std::rc::Rc::make_mut(sp).hide_for_water_render = hide;
                                    redraw = true;
                                }
                            }
                        }
                    }
                }
            }
            if redraw {
                changed.push((wx, wy));
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_iso_render::world::{ObjectKind, RenderObject, SpriteFlags, SpriteInfo};
    use std::rc::Rc;

    fn floor(x: i32, y: i32, water: bool) -> RenderSquare {
        let mut sq = RenderSquare::new(x, y, 0);
        let sprite = SpriteInfo {
            name: "blends_natural_01_0".into(),
            flags: SpriteFlags {
                solidfloor: true,
                exterior: true,
                water,
                ..Default::default()
            },
            ..Default::default()
        };
        sq.objects
            .push(RenderObject::new(ObjectKind::Plain, Some(Rc::new(sprite))));
        sq
    }

    #[test]
    fn shore_reacts_to_streamed_neighbours_flow_zones_and_quality() {
        let mut w = RenderWorld::default();
        let mut flow = IsoWaterFlow::default();
        w.chunks.insert((0, 0), (0, 0));
        for x in 6..=7 {
            for y in 1..=3 {
                w.squares.insert((x, y, 0), floor(x, y, false));
            }
        }
        w.refresh_water(&flow, true);
        assert!(w.squares[&(7, 2, 0)].water.is_none());
        w.chunks.insert((1, 0), (0, 0));
        for y in 1..=3 {
            w.squares.insert((8, y, 0), floor(8, y, y == 2));
        }
        w.water_changed(1, 0);
        assert_eq!(w.refresh_water(&flow, true), [(0, 0), (1, 0)]);
        let shore = w.squares[&(7, 2, 0)].water.unwrap();
        assert!(shore.shore && shore.shader);
        assert_eq!(shore.depth, [0., 0., 1., 1.]);
        assert!(w.squares[&(8, 2, 0)].water.unwrap().has_water);
        assert!(w.refresh_water(&flow, true).is_empty());
        flow.add_zone(7., 2., 7., 2., 0., 0.);
        w.refresh_water(&flow, false);
        assert!(w.squares[&(7, 2, 0)].water.is_none());
        assert!(!w.squares[&(8, 2, 0)].water.unwrap().shader);
        flow.reset();
        w.refresh_water(&flow, true);
        assert!(w.squares[&(7, 2, 0)].water.unwrap().shore);
        w.remove_chunk(1, 0);
        w.refresh_water(&flow, true);
        assert!(w.squares[&(7, 2, 0)].water.is_none());
    }

    #[test]
    fn water_shader_toggles_hide_for_water_render_on_shore_blends() {
        let mut w = RenderWorld::default();
        let flow = IsoWaterFlow::default();
        w.chunks.insert((0, 0), (0, 0));
        let mut sq = floor(2, 3, true);
        let blend = SpriteInfo {
            name: "blends_natural_02_6".into(),
            ..Default::default()
        };
        sq.objects
            .push(RenderObject::new(ObjectKind::Plain, Some(Rc::new(blend))));
        w.squares.insert((2, 3, 0), sq);
        w.water_changed(0, 0);

        assert_eq!(w.refresh_water(&flow, true), [(0, 0)]);
        assert!(!w.squares[&(2, 3, 0)].objects[0].sprite.as_ref().unwrap().hide_for_water_render);
        assert!(w.squares[&(2, 3, 0)].objects[1].sprite.as_ref().unwrap().hide_for_water_render);

        // Disabling the water shader restores the shore-blend sprite and flags redraw.
        assert_eq!(w.refresh_water(&flow, false), [(0, 0)]);
        assert!(!w.squares[&(2, 3, 0)].objects[1].sprite.as_ref().unwrap().hide_for_water_render);
    }
}
