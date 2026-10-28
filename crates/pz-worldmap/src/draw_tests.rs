//! Parity of the drawer's feature passes with the original `Drawer`
//! (oracle `draw.txt` in `PZ_WORLDMAP_ORACLE_DIR`, Muldraugh's data from
//! `PZ_WORLDMAP_MAP_DIR`): the visible cells and every VBO run of
//! `filterFeatures`/`renderCellFeatures`, the cell grids, the paint
//! outside the bounds and the world bounds, flat and isometric, for five
//! views with the default `MapUtils.initDefaultStyleV1` style. The
//! geometry triangles carry over between views as in the game.

use crate::data::get_or_create_data;
use crate::draw::{DrawInputs, Drawer};
use crate::renderer::WorldMapRenderer;
use crate::style::{ColorStop, Filter, StyleLayer, WorldMapStyle};
use crate::triangulate::MapFilesInfo;
use crate::world_map::WorldMap;
use std::cell::RefCell;
use std::rc::Rc;

struct Muldraugh;
impl MapFilesInfo for Muldraugh {
    fn map_directory_names(&self) -> Vec<String> {
        vec!["Muldraugh, KY".into()]
    }
}

fn poly(
    s: &mut WorldMapStyle,
    id: &str,
    min_zoom: f32,
    key: &str,
    value: &str,
    fill: &[(f32, i32, i32, i32, i32)],
) {
    let mut l = StyleLayer::new_polygon(id);
    l.min_zoom = min_zoom;
    l.filter = Some(if value == "*" {
        Filter::PolygonHas(key.into())
    } else {
        Filter::PolygonEquals(key.into(), value.into())
    });
    for &(zoom, r, g, b, a) in fill {
        l.fill_mut().push(ColorStop { zoom, r, g, b, a });
    }
    s.add_layer(Rc::new(RefCell::new(l)));
}

/// `MapUtils.initDefaultStyleV1` without the colour-blind patterns.
pub fn default_style() -> WorldMapStyle {
    let mut s = WorldMapStyle::new();
    poly(
        &mut s,
        "forest",
        13.5,
        "natural",
        "forest",
        &[
            (0.0, 189, 197, 163, 0),
            (14.5, 189, 197, 163, 0),
            (15.0, 189, 197, 163, 255),
            (24.0, 189, 197, 163, 255),
        ],
    );
    poly(
        &mut s,
        "water",
        0.0,
        "water",
        "river",
        &[(0.0, 59, 141, 149, 255), (24.0, 59, 141, 149, 255)],
    );
    poly(
        &mut s,
        "road-trail",
        12.0,
        "highway",
        "trail",
        &[
            (12.25, 185, 122, 87, 0),
            (13.0, 185, 122, 87, 255),
            (24.0, 185, 122, 87, 255),
        ],
    );
    poly(
        &mut s,
        "road-tertiary",
        11.0,
        "highway",
        "tertiary",
        &[
            (11.5, 171, 158, 143, 0),
            (13.0, 171, 158, 143, 255),
            (24.0, 171, 158, 143, 255),
        ],
    );
    poly(
        &mut s,
        "road-secondary",
        11.0,
        "highway",
        "secondary",
        &[(0.0, 134, 125, 113, 255), (24.0, 134, 125, 113, 255)],
    );
    poly(
        &mut s,
        "road-primary",
        11.0,
        "highway",
        "primary",
        &[(0.0, 134, 125, 113, 255), (24.0, 134, 125, 113, 255)],
    );
    poly(
        &mut s,
        "railway",
        14.0,
        "railway",
        "*",
        &[(0.0, 200, 191, 231, 255), (24.0, 200, 191, 231, 255)],
    );
    for (id, v, r, g, b) in [
        ("building", "yes", 210, 158, 105),
        ("building-Residential", "Residential", 210, 158, 105),
        (
            "building-CommunityServices",
            "CommunityServices",
            139,
            117,
            235,
        ),
        ("building-Hospitality", "Hospitality", 127, 206, 225),
        ("building-Industrial", "Industrial", 56, 54, 53),
        ("building-Medical", "Medical", 229, 128, 151),
        (
            "building-RestaurantsAndEntertainment",
            "RestaurantsAndEntertainment",
            245,
            225,
            60,
        ),
        (
            "building-RetailAndCommercial",
            "RetailAndCommercial",
            184,
            205,
            84,
        ),
    ] {
        poly(
            &mut s,
            id,
            13.0,
            "building",
            v,
            &[
                (13.0, r, g, b, 0),
                (13.5, r, g, b, 255),
                (24.0, r, g, b, 255),
            ],
        );
    }
    s
}

fn b(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

/// Java's `Float.toString` for the view lines (whole and simple values).
fn jf(f: f32) -> String {
    pz_core::config::float_to_string(f)
}

#[test]
fn matches_draw_oracle() {
    let (Some(dir), Some(map)) = (
        std::env::var_os("PZ_WORLDMAP_ORACLE_DIR"),
        std::env::var_os("PZ_WORLDMAP_MAP_DIR"),
    ) else {
        eprintln!("PZ_WORLDMAP_ORACLE_DIR / PZ_WORLDMAP_MAP_DIR not set; skipping");
        return;
    };
    let want = std::fs::read_to_string(std::path::Path::new(&dir).join("draw.txt")).unwrap();
    let map = map.to_string_lossy().to_string();
    let forest = get_or_create_data(&format!("{map}/worldmap-forest.xml")).unwrap();
    let main = get_or_create_data(&format!("{map}/worldmap.xml")).unwrap();
    *forest.relative_file_name.borrow_mut() = "media/maps/Muldraugh, KY/worldmap-forest.xml".into();
    *main.relative_file_name.borrow_mut() = "media/maps/Muldraugh, KY/worldmap.xml".into();
    let views = [
        (10700.0f32, 9800.0f32, 15.0f32),
        (10700.0, 9800.0, 13.0),
        (10650.0, 10150.0, 17.25),
        (300.0, 200.0, 14.0),
        (19800.0, 16000.0, 12.5),
    ];
    let mut out: Vec<String> = Vec::new();
    for iso in 0..2 {
        for grid in 0..2 {
            for &(cx, cy, z) in &views {
                let mut wm = WorldMap::new();
                wm.data.push(forest.clone());
                wm.data.push(main.clone());
                wm.update_data_bounds();
                wm.set_bounds_from_data();
                let mut r = WorldMapRenderer::new();
                r.set_boolean("Isometric", iso == 1);
                r.set_boolean("CellGrid", grid == 1);
                r.set_boolean("CellGrid300", grid == 1);
                r.set_map(0, 0, 1280, 720);
                r.reset_view(&wm);
                r.center_on(cx, cy);
                r.set_zoom(&wm, z);
                r.update_view(&wm, 1_000_000, 0.0);
                let style = default_style();
                let inp = DrawInputs {
                    map: &wm,
                    style: &style,
                    fill: [0.858_823_54, 0.843_137_26, 0.752_941_2, 1.0],
                    files: &Muldraugh,
                    debug: false,
                };
                let mut d = Drawer::new(&r, &inp);
                out.push(format!("case {iso} {grid} {} {} {}", jf(cx), jf(cy), jf(z)));
                d.calculate_visible_cells();
                let list: Vec<String> = d.rasterize_xy.iter().map(|v| v.to_string()).collect();
                out.push(format!(
                    "cells {} {{{}}}",
                    d.rasterize_xy.len(),
                    list.join(", ")
                ));
                d.filter_features();
                let _ = d.render_cell_features();
                let (mnx, mny, mxx, mxy) = (
                    wm.get_min_x_in_squares(),
                    wm.get_min_y_in_squares(),
                    wm.get_max_x_in_squares(),
                    wm.get_max_y_in_squares(),
                );
                if grid == 1 {
                    d.render_cell_grid(mnx / 256, mny / 256, mxx / 256, mxy / 256, 256);
                    d.render_cell_grid(mnx / 300, mny / 300, mxx / 300, mxy / 300, 300);
                }
                let ws = d.world_scale;
                d.paint_areas_outside_bounds(mnx, mny, mxx, mxy, ws);
                d.render_world_bounds();
                for run in &d.vbo.runs {
                    out.push(format!(
                        "run {} {} {} {}",
                        run.prim as i32,
                        b(run.line_width),
                        run.texture.is_some(),
                        run.nearest
                    ));
                    for v in &run.verts {
                        out.push(format!(
                            "v {} {} {} {} {} {} {} {} {}",
                            b(v.x),
                            b(v.y),
                            b(v.z),
                            b(v.u),
                            b(v.v),
                            b(v.color[0]),
                            b(v.color[1]),
                            b(v.color[2]),
                            b(v.color[3])
                        ));
                    }
                }
            }
        }
    }
    let want: Vec<&str> = want.lines().collect();
    let mut bad = 0;
    for (i, (g, w)) in out.iter().zip(&want).enumerate() {
        if g != w {
            if bad < 8 {
                eprintln!(
                    "line {i}\n got {}\nwant {}",
                    &g[..g.len().min(400)],
                    &w[..w.len().min(400)]
                );
            }
            bad += 1;
        }
    }
    assert_eq!(bad, 0, "{bad} lines differ");
    assert_eq!(out.len(), want.len());
}

/// The pyramid tile selection against the original `ImagePyramid` and
/// `Drawer` (oracle `pyramid.txt`): bounds and levels from `pyramid.txt`,
/// metres per tile, `getZoom`, `isValidTile`, the tiles covering cells and
/// the visible tiles of five views, flat and isometric, for both of
/// Muldraugh's pyramids.
#[test]
fn matches_pyramid_oracle() {
    use crate::pyramid::WorldMapImages;
    let (Some(dir), Some(map)) = (
        std::env::var_os("PZ_WORLDMAP_ORACLE_DIR"),
        std::env::var_os("PZ_WORLDMAP_MAP_DIR"),
    ) else {
        eprintln!("PZ_WORLDMAP_ORACLE_DIR / PZ_WORLDMAP_MAP_DIR not set; skipping");
        return;
    };
    let want = std::fs::read_to_string(std::path::Path::new(&dir).join("pyramid.txt")).unwrap();
    let map = map.to_string_lossy().to_string();
    let d64 = |v: f64| format!("{:x}", v.to_bits());
    let mut out = Vec::new();
    for name in ["pyramid.zip", "forest.pyramid.zip"] {
        let images = WorldMapImages::get_or_create(&format!("{map}/{name}")).unwrap();
        {
            let p = images.pyramid.borrow();
            out.push(format!(
                "pyr {name} {} {} {} {} {} {} {}",
                p.min_x,
                p.min_y,
                p.max_x,
                p.max_y,
                p.min_z,
                p.max_z,
                b(p.resolution)
            ));
            for z in 0..=5 {
                out.push(format!("mpt {z} {}", d64(p.calculate_meters_per_tile(z))));
            }
        }
        let mut zf = 10.0f32;
        while zf <= 18.0 {
            out.push(format!("zoom {} {}", b(zf), images.get_zoom(zf)));
            zf += 0.25;
        }
        {
            let p = images.pyramid.borrow();
            for z in -1..=5 {
                for x in (-1..90).step_by(7) {
                    for y in (-1..70).step_by(5) {
                        if p.is_valid_tile(x, y, z) {
                            out.push(format!("valid {x} {y} {z}"));
                        }
                    }
                }
            }
            for z in 0..=4 {
                for cx in (0..80).step_by(9) {
                    for cy in (0..65).step_by(8) {
                        match p.tiles_covering_cell(cx, cy, z) {
                            Some(t) => out.push(format!(
                                "cover {cx} {cy} {z} true {} {} {} {}",
                                t[0], t[1], t[2], t[3]
                            )),
                            None => out.push(format!("cover {cx} {cy} {z} false")),
                        }
                    }
                }
            }
        }
        for iso in 0..2 {
            for (cx, cy, z) in [
                (10700.0f32, 9800.0f32, 15.0f32),
                (10700.0, 9800.0, 13.0),
                (10650.0, 10150.0, 17.25),
                (300.0, 200.0, 14.0),
                (19800.0, 16000.0, 12.5),
            ] {
                let mut wm = WorldMap::new();
                wm.set_bounds_in_squares(0, 0, 19967, 16127);
                let mut r = WorldMapRenderer::new();
                r.set_boolean("Isometric", iso == 1);
                r.set_map(0, 0, 1280, 720);
                r.reset_view(&wm);
                r.center_on(cx, cy);
                r.set_zoom(&wm, z);
                r.update_view(&wm, 1_000_000, 0.0);
                let style = WorldMapStyle::new();
                let inp = DrawInputs {
                    map: &wm,
                    style: &style,
                    fill: [1.0; 4],
                    files: &Muldraugh,
                    debug: false,
                };
                let d = Drawer::new(&r, &inp);
                let adj = d.zoom_adjusted(&images);
                let ptz = images.get_zoom(adj as f32);
                let (xy, bounds) = d.calc_visible_pyramid_tiles(&images, ptz);
                let list: Vec<String> = xy.iter().map(|v| v.to_string()).collect();
                out.push(format!(
                    "view {iso} {} {} {} {} {ptz} {{{}}} [{}, {}, {}, {}]",
                    jf(cx),
                    jf(cy),
                    jf(z),
                    d64(adj),
                    list.join(", "),
                    bounds[0],
                    bounds[1],
                    bounds[2],
                    bounds[3]
                ));
            }
        }
    }
    assert_eq!(out, want.lines().collect::<Vec<_>>());
}
