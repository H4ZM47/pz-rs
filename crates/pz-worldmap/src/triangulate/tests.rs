use super::*;
use crate::data::WorldMapData;
use crate::draw::{DrawInputs, Drawer};
use crate::renderer::WorldMapRenderer;
use crate::style::{ColorStop, Filter, StyleLayer, WorldMapStyle};
use crate::world_map::WorldMap;
use std::cell::RefCell;
use std::rc::Rc;

const EMPTY_POLYGON: &[u8] = br#"<world><cell x="0" y="0"><feature><geometry type="Polygon"/><properties><property name="building" value="yes"/></properties></feature></cell></world>"#;
const EMPTY_ABORT: DrawAbort =
    DrawAbort("IndexOutOfBoundsException in WorldMapGeometry.triangulate");

#[test]
fn retained_xml_polygon_without_rings_aborts_before_mutating_existing_geometry_buffers() {
    let cells = crate::xml::read_bytes(EMPTY_POLYGON).unwrap();
    assert_eq!(cells.len(), 1);
    let cell = &cells[0];
    assert_eq!(cell.features.len(), 1);
    let geometry = cell.features[0].geometry.as_ref().unwrap();
    assert!(geometry.points.is_empty());
    cell.triangle_buffer.borrow_mut().extend([7.0, 8.0]);
    cell.index_buffer.borrow_mut().extend([3, 4, 5]);
    cell.priority.set(1);
    struct UnreachedMapFiles;
    impl MapFilesInfo for UnreachedMapFiles {
        fn has_cell300(&self, _: usize, _: i32, _: i32) -> bool {
            panic!("native outer-ring failure precedes map-priority clipping")
        }
    }
    assert_eq!(
        triangulate(cell, geometry, None, &UnreachedMapFiles),
        Err(EMPTY_ABORT),
    );
    assert!(geometry.triangles.borrow().is_none());
    assert!(cell.clip_higher_priority_cells.borrow().is_none());
    assert_eq!(*cell.triangle_buffer.borrow(), [7.0, 8.0]);
    assert_eq!(*cell.index_buffer.borrow(), [3, 4, 5]);
}

#[test]
fn retained_xml_empty_polygon_uses_drawer_abort_and_skips_later_debug_passes() {
    let data = Rc::new(WorldMapData::new("authored-empty-ring.xml"));
    data.set_loaded(crate::xml::read_bytes(EMPTY_POLYGON).unwrap());
    let cell = data.get_cell(0, 0).unwrap();
    let mut map = WorldMap::new();
    map.data.push(data);
    map.update_data_bounds();
    map.set_bounds_from_data();
    let mut layer = StyleLayer::new_polygon("building");
    layer.filter = Some(Filter::PolygonEquals("building".into(), "yes".into()));
    layer.fill_mut().push(ColorStop {
        zoom: 0.0,
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    });
    let mut style = WorldMapStyle::new();
    style.add_layer(Rc::new(RefCell::new(layer)));
    let mut renderer = WorldMapRenderer::new();
    renderer.set_boolean("Isometric", false);
    renderer.set_boolean("Features", true);
    renderer.set_boolean("ImagePyramid", false);
    renderer.set_boolean("CellGrid", true);
    renderer.set_boolean("WorldBounds", true);
    renderer.set_map(0, 0, 640, 480);
    renderer.reset_view(&map);
    renderer.center_on(128.0, 128.0);
    renderer.set_zoom(&map, 15.0);
    renderer.update_view(&map, 1_000_000, 0.0);
    let inputs = DrawInputs {
        map: &map,
        style: &style,
        fill: [1.0; 4],
        files: &NoMapFiles,
        debug: false,
    };
    let frame = Drawer::new(&renderer, &inputs).render();
    assert_eq!(frame.aborted, Some(EMPTY_ABORT));
    assert!(
        frame.vbo.runs.is_empty(),
        "later cell/world grids must not run after abort"
    );
    assert!(cell.features[0]
        .geometry
        .as_ref()
        .unwrap()
        .triangles
        .borrow()
        .is_none());
    assert!(cell.triangle_buffer.borrow().is_empty());
    assert!(cell.index_buffer.borrow().is_empty());
}
