//! `zombie.scripting.entity.components.spriteconfig.SpriteOverlayConfigScript`
//! and its nested `OverlayStyle` and `FaceScript` (the `ZLayer`, `XRow` and
//! `TileScript` data is the same as `SpriteConfigScript`'s).

use super::sprite_config_script::{complementary_face, get_face_id_for_string, load_layer, ZLayer};
use crate::base::{ScriptError, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::objects::entity::component_script::cast;
use crate::objects::entity::{
    log_error, null_pointer, parse_int, runtime_exception, ComponentBase, ComponentScript, ComponentType,
};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

/// `SpriteOverlayConfigScript.FaceScript`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayFaceScript {
    face_name: String,
    face_id: i32,
    total_width: i32,
    total_height: i32,
    layers: Vec<ZLayer>,
}

impl OverlayFaceScript {
    /// `getFaceName`: lower-cased.
    pub fn face_name(&self) -> String {
        java::to_lower(&self.face_name)
    }

    /// `getFaceID`.
    pub fn face_id(&self) -> i32 {
        self.face_id
    }

    /// `getTotalWidth`.
    pub fn total_width(&self) -> i32 {
        self.total_width
    }

    /// `getTotalHeight`.
    pub fn total_height(&self) -> i32 {
        self.total_height
    }

    /// `getZLayers`.
    pub fn z_layers(&self) -> usize {
        self.layers.len()
    }

    /// `getLayer`.
    pub fn layer(&self, z: usize) -> Option<&ZLayer> {
        self.layers.get(z)
    }
}

/// `OverlayStyle`: one `style <name> { progress <n> { face ... } }` entry,
/// keyed `<name>_<n>`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OverlayStyle {
    style_name: Option<String>,
    progress: i32,
    faces: [Option<OverlayFaceScript>; 6],
    is_single_face: bool,
    is_multi_tile: bool,
    is_valid: bool,
    all_tile_names: Vec<String>,
}

impl OverlayStyle {
    /// `getStyleName`.
    pub fn style_name(&self) -> Option<&str> {
        self.style_name.as_deref()
    }

    /// `getProgress`: never assigned by the original, so always 0.
    pub fn progress(&self) -> i32 {
        self.progress
    }

    /// `getFace`.
    pub fn face(&self, id: usize) -> Option<&OverlayFaceScript> {
        self.faces.get(id).and_then(Option::as_ref)
    }

    /// `getAllTileNames`.
    pub fn all_tile_names(&self) -> &[String] {
        &self.all_tile_names
    }

    /// `isSingleFace`.
    pub fn is_single_face(&self) -> bool {
        self.is_single_face
    }

    /// `isMultiTile`.
    pub fn is_multi_tile(&self) -> bool {
        self.is_multi_tile
    }

    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        self.is_valid
    }
}

#[derive(Debug)]
pub struct SpriteOverlayConfigScript {
    pub component: ComponentBase,
    all_styles: JHashMap<OverlayStyle>,
    all_style_names: Vec<String>,
    is_valid: bool,
}

impl Default for SpriteOverlayConfigScript {
    fn default() -> Self {
        Self::new()
    }
}

impl SpriteOverlayConfigScript {
    pub fn new() -> Self {
        SpriteOverlayConfigScript {
            component: ComponentBase::new(ComponentType::SpriteOverlayConfig),
            all_styles: JHashMap::new(),
            all_style_names: Vec::new(),
            is_valid: false,
        }
    }

    /// `getStyle`.
    pub fn style(&self, style_name: &str) -> Option<&OverlayStyle> {
        self.all_styles.get(style_name)
    }

    /// Every style, in `HashMap` order.
    pub fn styles(&self) -> &JHashMap<OverlayStyle> {
        &self.all_styles
    }

    /// `getAllStyleNames`.
    pub fn all_style_names(&self) -> &[String] {
        &self.all_style_names
    }

    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        self.is_valid
    }

    /// `loadStyle`: each `progress <n>` child (case-sensitive type).
    fn load_style(&mut self, sm: &ScriptManager, style_name: Option<&str>, block: &Block) -> ScriptResult {
        for child in &block.children {
            if child.type_str() != "progress" || child.type_.is_none() {
                continue;
            }
            let id = child
                .id
                .as_deref()
                .ok_or_else(|| ScriptError::new("java.lang.NumberFormatException: Cannot parse null string: null"))?;
            let progress = parse_int(id)?;
            let key = format!("{}_{progress}", style_name.unwrap_or("null"));
            if self.all_styles.contains_key(&key) {
                log_error(sm, &format!("Duplicate style name and progress (not allowed): {key}"));
                continue;
            }
            self.load_faces(sm, key, child)?;
        }
        Ok(())
    }

    /// `loadFaces`.
    fn load_faces(&mut self, sm: &ScriptManager, style_name: String, block: &Block) -> ScriptResult {
        let mut style = OverlayStyle { style_name: Some(style_name.clone()), ..Default::default() };
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "face") && !style.is_single_face {
                let id = child.id.as_deref().ok_or_else(null_pointer)?;
                let face_id = get_face_id_for_string(id);
                if face_id == -1 {
                    log_error(sm, &format!("Cannot find face for: {id}"));
                    continue;
                }
                let mut face = OverlayFaceScript {
                    face_name: id.to_string(),
                    face_id,
                    total_width: 0,
                    total_height: 0,
                    layers: Vec::new(),
                };
                Self::load_face(&mut face, child);
                style.faces[face_id as usize] = Some(face);
                if java::eq_ignore_case(id, "single") {
                    style.is_single_face = true;
                }
                continue;
            }
            log_error(
                sm,
                &format!("Unknown block '{}' in entity iso script: {}", child.type_str(), self.component.name(sm)?),
            );
        }
        self.all_styles.insert(style_name, style);
        Ok(())
    }

    /// `loadFace`.
    fn load_face(face: &mut OverlayFaceScript, block: &Block) {
        let mut layer = ZLayer::default();
        load_layer(&mut layer, block);
        if !layer.rows.is_empty() {
            face.layers.push(layer);
            return;
        }
        for child in &block.children {
            if !java::eq_ignore_case(child.type_str(), "layer") {
                continue;
            }
            load_layer(&mut layer, child);
            face.layers.push(std::mem::take(&mut layer));
        }
    }

    /// `warnOrError`.
    fn warn_or_error(&self, sm: &ScriptManager, msg: &str) -> ScriptResult {
        let text = format!("[{}] {msg}", self.component.name(sm)?);
        if sm.debug {
            return Err(runtime_exception(text));
        }
        sm.warn(&text);
        Ok(())
    }

    /// `checkScripts`.
    fn check_scripts(&mut self, sm: &ScriptManager) -> ScriptResult {
        let keys: Vec<String> = self.all_styles.keys().map(str::to_string).collect();
        for key in &keys {
            let mut style = self.all_styles.get(key).unwrap().clone();
            let r = self.check_style(sm, &mut style);
            *self.all_styles.get_mut(key).unwrap() = style;
            r?;
        }
        self.is_valid = true;
        for style in self.all_styles.values() {
            self.all_style_names.push(style.style_name.clone().unwrap_or_default());
            if !style.is_valid {
                self.is_valid = false;
            }
        }
        Ok(())
    }

    fn check_style(&self, sm: &ScriptManager, style: &mut OverlayStyle) -> ScriptResult {
        if style.is_single_face {
            for i in 0..6 {
                if style.faces[i].as_ref().is_some_and(|f| f.face_id != 0) {
                    self.warn_or_error(sm, "SingleFace has other faces defined.")?;
                    style.faces[i] = None;
                }
            }
        }
        let mut has_face = false;
        let mut face_count = 0;
        for face in style.faces.iter_mut().flatten() {
            has_face = true;
            face_count += 1;
            for layer in &face.layers {
                face.total_height = face.total_height.max(layer.rows.len() as i32);
                for row in &layer.rows {
                    face.total_width = face.total_width.max(row.tiles.len() as i32);
                    for tile in &row.tiles {
                        if let Some(name) = &tile.tile_name {
                            if !style.all_tile_names.contains(name) {
                                style.all_tile_names.push(name.clone());
                            }
                        }
                    }
                }
            }
            if face_count > 1 || face.total_height > 1 || face.total_width > 1 {
                style.is_multi_tile = true;
            }
        }
        for face in style.faces.iter().flatten() {
            let Some(other) = complementary_face(face.face_id).and_then(|c| style.faces[c].as_ref()) else {
                continue;
            };
            if face.total_width != other.total_width || face.total_height != other.total_height {
                log_error(
                    sm,
                    &format!(
                        "Complementary faces are different sizes. This is not supported. In entity iso script: {}",
                        self.component.name(sm)?
                    ),
                );
            }
        }
        if has_face {
            style.is_valid = true;
        }
        Ok(())
    }
}

impl ComponentScript for SpriteOverlayConfigScript {
    component_accessors!("zombie.scripting.entity.components.spriteconfig.SpriteOverlayConfigScript");

    fn pre_reload(&mut self) {
        self.all_styles.clear();
        self.all_style_names.clear();
        self.is_valid = false;
    }

    /// `OnScriptsLoaded`. TODO(ScriptsDictionary): registers the script on init.
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        Ok(())
    }

    /// `copyFrom`: copies validity and styles (style names are not copied).
    fn copy_from(&mut self, other: &dyn ComponentScript) -> ScriptResult {
        let other = cast::<SpriteOverlayConfigScript>(other, self)?;
        self.is_valid = other.is_valid;
        self.all_styles.clear();
        for (k, v) in other.all_styles.iter() {
            self.all_styles.insert(k.to_string(), v.clone());
        }
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for child in &block.children {
            if !java::eq_ignore_case(child.type_str(), "style") {
                continue;
            }
            self.load_style(sm, child.id.as_deref(), child)?;
        }
        self.check_scripts(sm)
    }
}
