//! `zombie.worldMap.WorldMap`: the data files, image pyramids and street
//! files one map UI shows, and the displayed bounds in squares.

use crate::data::WorldMapData;
use std::rc::Rc;

/// Squares per world-map cell.
pub const CELL_SIZE: i32 = 256;

/// `zombie.worldMap.WorldMap`.
#[derive(Default)]
pub struct WorldMap {
    pub data: Vec<Rc<WorldMapData>>,
    pub min_data_x: i32,
    pub min_data_y: i32,
    pub max_data_x: i32,
    pub max_data_y: i32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    bounds_from_data: bool,
    pub last_data_in_directory: Vec<Rc<WorldMapData>>,
    /// `images.size()`: the image pyramids (`WorldMapImages`) added.
    pub images: Vec<Rc<str>>,
}

impl WorldMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// `setBoundsInCells(minX, minY, maxX, maxY)`.
    pub fn set_bounds_in_cells(&mut self, min_x: i32, min_y: i32, max_x: i32, max_y: i32) {
        self.set_bounds_in_squares(
            min_x.wrapping_mul(CELL_SIZE),
            min_y.wrapping_mul(CELL_SIZE),
            max_x.wrapping_mul(CELL_SIZE).wrapping_add(CELL_SIZE - 1),
            max_y.wrapping_mul(CELL_SIZE).wrapping_add(CELL_SIZE - 1),
        );
    }

    /// `setBoundsInSquares(minX, minY, maxX, maxY)`.
    pub fn set_bounds_in_squares(&mut self, min_x: i32, min_y: i32, max_x: i32, max_y: i32) {
        self.min_x = min_x;
        self.min_y = min_y;
        self.max_x = max_x;
        self.max_y = max_y;
    }

    /// `setBoundsFromData()`: also follows later data loads.
    pub fn set_bounds_from_data(&mut self) {
        self.bounds_from_data = true;
        self.set_bounds_in_cells(
            self.min_data_x,
            self.min_data_y,
            self.max_data_x,
            self.max_data_y,
        );
    }

    /// `setBoundsFromWorld()` with `IsoMetaGrid`'s cell bounds.
    pub fn set_bounds_from_world(&mut self, min_x: i32, min_y: i32, max_x: i32, max_y: i32) {
        self.set_bounds_in_cells(min_x, min_y, max_x, max_y);
    }

    /// `addData` once the file name was resolved: `data` is the shared
    /// asset (`WorldMapData.getOrCreateData`), `None` when the file does not
    /// exist.
    pub fn add_data(&mut self, data: Option<Rc<WorldMapData>>, relative_file_name: &str) {
        let Some(data) = data else { return };
        if self.data.iter().any(|d| Rc::ptr_eq(d, &data)) {
            return;
        }
        *data.relative_file_name.borrow_mut() = relative_file_name.into();
        self.data.push(data.clone());
        if data.is_ready() {
            self.update_data_bounds();
        }
    }

    pub fn get_data_count(&self) -> usize {
        self.data.len()
    }

    pub fn get_data_by_index(&self, index: usize) -> Option<&Rc<WorldMapData>> {
        self.data.get(index)
    }

    /// `isDataLoaded()`: no data file still waiting to load.
    pub fn is_data_loaded(&self) -> bool {
        self.data.iter().all(|d| !d.is_empty())
    }

    /// `clearData()`.
    pub fn clear_data(&mut self) {
        self.data.clear();
        self.last_data_in_directory.clear();
        self.update_data_bounds();
    }

    /// `endDirectoryData()`.
    pub fn end_directory_data(&mut self) {
        if let Some(last) = self.data.last() {
            if !self
                .last_data_in_directory
                .iter()
                .any(|d| Rc::ptr_eq(d, last))
            {
                self.last_data_in_directory.push(last.clone());
            }
        }
    }

    /// `isLastDataInDirectory(data)`.
    pub fn is_last_data_in_directory(&self, data: &Rc<WorldMapData>) -> bool {
        self.last_data_in_directory
            .iter()
            .any(|d| Rc::ptr_eq(d, data))
    }

    /// `updateDataBounds()`.
    pub fn update_data_bounds(&mut self) {
        self.min_data_x = i32::MAX;
        self.min_data_y = i32::MAX;
        self.max_data_x = i32::MIN;
        self.max_data_y = i32::MIN;
        for d in &self.data {
            if !d.is_ready() {
                continue;
            }
            let b = d.bounds.get();
            self.min_data_x = self.min_data_x.min(b[0]);
            self.min_data_y = self.min_data_y.min(b[1]);
            self.max_data_x = self.max_data_x.max(b[2]);
            self.max_data_y = self.max_data_y.max(b[3]);
        }
        if self.min_data_x > self.max_data_x {
            self.min_data_x = 0;
            self.min_data_y = 0;
            self.max_data_x = 0;
            self.max_data_y = 0;
        }
    }

    /// `onStateChanged`: a data file finished loading (or failed).
    pub fn on_data_state_changed(&mut self) {
        self.update_data_bounds();
        if self.bounds_from_data {
            self.set_bounds_in_cells(
                self.min_data_x,
                self.min_data_y,
                self.max_data_x,
                self.max_data_y,
            );
        }
    }

    pub fn has_data(&self) -> bool {
        !self.data.is_empty()
    }

    /// `addImages(directory)` once resolved: the directory's
    /// `pyramid.zip` and `forest.pyramid.zip` (absolute paths), each added
    /// when it exists and is not there yet.
    pub fn add_images(&mut self, pyramid_zip: &str, forest_zip: &str) {
        for p in [pyramid_zip, forest_zip] {
            self.add_image_pyramid(p);
        }
    }

    /// `addImagePyramid(absolutePath)`.
    pub fn add_image_pyramid(&mut self, path: &str) {
        if path.trim().is_empty() || !std::path::Path::new(path).exists() {
            return;
        }
        if !self.images.iter().any(|i| &**i == path) {
            self.images.push(path.into());
        }
    }

    pub fn has_images(&self) -> bool {
        !self.images.is_empty()
    }

    pub fn get_images_count(&self) -> usize {
        self.images.len()
    }

    pub fn clear_images(&mut self) {
        self.images.clear();
    }

    pub fn get_min_x_in_cells(&self) -> i32 {
        self.min_x / CELL_SIZE
    }
    pub fn get_min_y_in_cells(&self) -> i32 {
        self.min_y / CELL_SIZE
    }
    pub fn get_max_x_in_cells(&self) -> i32 {
        self.max_x / CELL_SIZE
    }
    pub fn get_max_y_in_cells(&self) -> i32 {
        self.max_y / CELL_SIZE
    }
    pub fn get_width_in_cells(&self) -> i32 {
        self.get_max_x_in_cells() - self.get_min_x_in_cells() + 1
    }
    pub fn get_height_in_cells(&self) -> i32 {
        self.get_max_y_in_cells() - self.get_min_y_in_cells() + 1
    }
    pub fn get_min_x_in_squares(&self) -> i32 {
        self.min_x
    }
    pub fn get_min_y_in_squares(&self) -> i32 {
        self.min_y
    }
    pub fn get_max_x_in_squares(&self) -> i32 {
        self.max_x
    }
    pub fn get_max_y_in_squares(&self) -> i32 {
        self.max_y
    }
    pub fn get_width_in_squares(&self) -> i32 {
        self.max_x.wrapping_sub(self.min_x).wrapping_add(1)
    }
    pub fn get_height_in_squares(&self) -> i32 {
        self.max_y.wrapping_sub(self.min_y).wrapping_add(1)
    }
    pub fn get_data_width_in_cells(&self) -> i32 {
        self.max_data_x - self.min_data_x + 1
    }
    pub fn get_data_height_in_cells(&self) -> i32 {
        self.max_data_y - self.min_data_y + 1
    }
    pub fn get_data_width_in_squares(&self) -> i32 {
        self.get_data_width_in_cells() * CELL_SIZE
    }
    pub fn get_data_height_in_squares(&self) -> i32 {
        self.get_data_height_in_cells() * CELL_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds() {
        let mut m = WorldMap::new();
        m.set_bounds_in_cells(1, 2, 3, 4);
        assert_eq!((m.min_x, m.min_y, m.max_x, m.max_y), (256, 512, 1023, 1279));
        assert_eq!((m.get_width_in_cells(), m.get_height_in_cells()), (3, 3));
        assert_eq!(m.get_width_in_squares(), 768);
        m.update_data_bounds();
        assert_eq!((m.min_data_x, m.max_data_x), (0, 0));
        m.set_bounds_from_data();
        assert_eq!((m.min_x, m.max_x), (0, 255));
    }
}
