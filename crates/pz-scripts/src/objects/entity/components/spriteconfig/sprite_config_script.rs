//! `zombie.scripting.entity.components.spriteconfig.SpriteConfigScript` and
//! its nested `FaceScript`, `ZLayer`, `XRow` and `TileScript`.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{
    array_get, log_error, null_pointer, parse_int, runtime_exception, try_parse_int, ComponentBase, ComponentScript,
    ComponentType,
};
use crate::parser::{Block, BlockElement};
use crate::script_type::ScriptLoadMode;

/// `SpriteConfigManager.GetFaceIdForString`: `-1` for unknown faces.
pub fn get_face_id_for_string(face: &str) -> i32 {
    const FACES: &[(&str, i32)] =
        &[("n", 0), ("n_open", 4), ("e", 3), ("s", 2), ("w", 1), ("w_open", 5), ("single", 0)];
    FACES.iter().find(|(f, _)| java::eq_ignore_case(face, f)).map_or(-1, |&(_, id)| id)
}

/// `TileScript`: one cell of a row; `true`/`false` cells are empty space
/// that does or does not block the square.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileScript {
    pub(crate) tile_name: Option<String>,
    pub(crate) is_empty_space: bool,
    pub(crate) blocks_square: bool,
}

impl TileScript {
    /// `getTileName`.
    pub fn tile_name(&self) -> Option<&str> {
        self.tile_name.as_deref()
    }

    /// `isEmptySpace`.
    pub fn is_empty_space(&self) -> bool {
        self.is_empty_space
    }

    /// `isBlocksSquare`.
    pub fn is_blocks_square(&self) -> bool {
        self.blocks_square
    }
}

/// `XRow`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XRow {
    pub(crate) tiles: Vec<TileScript>,
}

impl XRow {
    /// `getWidth`.
    pub fn width(&self) -> usize {
        self.tiles.len()
    }

    /// `getTile`.
    pub fn tile(&self, x: usize) -> Option<&TileScript> {
        self.tiles.get(x)
    }
}

/// `ZLayer`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZLayer {
    pub(crate) rows: Vec<XRow>,
}

impl ZLayer {
    /// `getHeight`.
    pub fn height(&self) -> usize {
        self.rows.len()
    }

    /// `getRow`.
    pub fn row(&self, y: usize) -> Option<&XRow> {
        self.rows.get(y)
    }
}

/// `loadLayer`: every `row = a b c` value of the block becomes a row.
pub(crate) fn load_layer(layer: &mut ZLayer, block: &Block) {
    for value in &block.values {
        let key = java::trim(value.get_key());
        let val = java::trim(value.get_value());
        if !java::eq_ignore_case(key, "row") {
            continue;
        }
        let mut row = XRow::default();
        for tile_name in java::split_ws(val) {
            let tile = if java::eq_ignore_case(tile_name, "true") || java::eq_ignore_case(tile_name, "false") {
                TileScript {
                    tile_name: None,
                    is_empty_space: true,
                    blocks_square: java::eq_ignore_case(tile_name, "true"),
                }
            } else {
                TileScript { tile_name: Some(tile_name.to_string()), is_empty_space: false, blocks_square: true }
            };
            row.tiles.push(tile);
        }
        layer.rows.push(row);
    }
}

/// The partner face whose size must match (`n`/`n_open`, `w`/`w_open`).
pub(crate) fn complementary_face(face_id: i32) -> Option<usize> {
    match face_id {
        0 => Some(4),
        4 => Some(0),
        1 => Some(5),
        5 => Some(1),
        _ => None,
    }
}

/// `SpriteConfigScript.FaceScript`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceScript {
    face_name: String,
    face_id: i32,
    total_width: i32,
    total_height: i32,
    lightsource_offset_x: i32,
    lightsource_offset_y: i32,
    lightsource_offset_z: i32,
    layers: Vec<ZLayer>,
}

impl FaceScript {
    /// `getFaceName`: lower-cased.
    pub fn face_name(&self) -> String {
        java::to_lower(&self.face_name)
    }

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

    /// `getLightsourceOffsetX`.
    pub fn lightsource_offset_x(&self) -> i32 {
        self.lightsource_offset_x
    }

    /// `getLightsourceOffsetY`.
    pub fn lightsource_offset_y(&self) -> i32 {
        self.lightsource_offset_y
    }

    /// `getLightsourceOffsetZ`.
    pub fn lightsource_offset_z(&self) -> i32 {
        self.lightsource_offset_z
    }
}

#[derive(Debug)]
pub struct SpriteConfigScript {
    pub component: ComponentBase,
    faces: [Option<FaceScript>; 6],
    is_single_face: bool,
    is_multi_tile: bool,
    is_valid: bool,
    is_prop: bool,
    all_tile_names: Vec<String>,
    corner_sprite: Option<String>,
    health: i32,
    skill_base_health: i32,
    is_thumpable: bool,
    light_radius: i32,
    lightsource_item: Option<String>,
    lightsource_fuel: Option<String>,
    debug_item: Option<String>,
    lightsource_tag_item: Option<Vec<String>>,
    break_sound: String,
    dont_need_frame: bool,
    need_window_frame: bool,
    is_pole: bool,
    previous_stage: Vec<String>,
    bonus_health: i32,
    on_create: Option<String>,
    on_is_valid: Option<String>,
    timed_action_on_is_valid: Option<String>,
    need_to_be_against_wall: bool,
    can_be_padlocked: bool,
}

/// `IsoThumpable.DEFAULT_BREAK_SOUND.getSoundName()`.
const DEFAULT_BREAK_SOUND: &str = "BreakObject";

impl Default for SpriteConfigScript {
    fn default() -> Self {
        Self::new()
    }
}

impl SpriteConfigScript {
    pub fn new() -> Self {
        SpriteConfigScript {
            component: ComponentBase::new(ComponentType::SpriteConfig),
            faces: Default::default(),
            is_single_face: false,
            is_multi_tile: false,
            is_valid: false,
            is_prop: false,
            all_tile_names: Vec::new(),
            corner_sprite: None,
            health: -1,
            skill_base_health: 20,
            is_thumpable: true,
            light_radius: 0,
            lightsource_item: None,
            lightsource_fuel: None,
            debug_item: None,
            lightsource_tag_item: None,
            break_sound: DEFAULT_BREAK_SOUND.to_string(),
            dont_need_frame: false,
            need_window_frame: false,
            is_pole: false,
            previous_stage: Vec::new(),
            bonus_health: 0,
            on_create: None,
            on_is_valid: None,
            timed_action_on_is_valid: None,
            need_to_be_against_wall: false,
            can_be_padlocked: false,
        }
    }

    /// `getFace`.
    pub fn face(&self, id: usize) -> Option<&FaceScript> {
        self.faces.get(id).and_then(Option::as_ref)
    }

    /// `getCornerSprite`.
    pub fn corner_sprite(&self) -> Option<&str> {
        self.corner_sprite.as_deref()
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

    /// `isProp`.
    pub fn is_prop(&self) -> bool {
        self.is_prop
    }

    /// `getHealth`.
    pub fn health(&self) -> i32 {
        self.health
    }

    /// `getSkillBaseHealth`.
    pub fn skill_base_health(&self) -> i32 {
        self.skill_base_health
    }

    /// `getIsThumpable`.
    pub fn is_thumpable(&self) -> bool {
        self.is_thumpable
    }

    /// `getCanBePadlocked`.
    pub fn can_be_padlocked(&self) -> bool {
        self.can_be_padlocked
    }

    /// `getBreakSound`.
    pub fn break_sound(&self) -> &str {
        &self.break_sound
    }

    /// `getDontNeedFrame`.
    pub fn dont_need_frame(&self) -> bool {
        self.dont_need_frame
    }

    /// `getLightRadius`.
    pub fn light_radius(&self) -> i32 {
        self.light_radius
    }

    /// `getLightsourceItem`.
    pub fn lightsource_item(&self) -> Option<&str> {
        self.lightsource_item.as_deref()
    }

    /// `getLightsourceFuel`.
    pub fn lightsource_fuel(&self) -> Option<&str> {
        self.lightsource_fuel.as_deref()
    }

    /// `getDebugItem`.
    pub fn debug_item(&self) -> Option<&str> {
        self.debug_item.as_deref()
    }

    /// `getLightsourceTagItem`.
    pub fn lightsource_tag_item(&self) -> Option<&[String]> {
        self.lightsource_tag_item.as_deref()
    }

    /// `getPreviousStages`.
    pub fn previous_stages(&self) -> &[String] {
        &self.previous_stage
    }

    /// `getBonusHealth`.
    pub fn bonus_health(&self) -> i32 {
        self.bonus_health
    }

    /// `getOnCreate`.
    pub fn on_create(&self) -> Option<&str> {
        self.on_create.as_deref()
    }

    /// `getOnIsValid`.
    pub fn on_is_valid(&self) -> Option<&str> {
        self.on_is_valid.as_deref()
    }

    /// `getTimedActionOnIsValid`.
    pub fn timed_action_on_is_valid(&self) -> Option<&str> {
        self.timed_action_on_is_valid.as_deref()
    }

    /// `getNeedWindowFrame`.
    pub fn need_window_frame(&self) -> bool {
        self.need_window_frame
    }

    /// `getNeedToBeAgainstWall`.
    pub fn need_to_be_against_wall(&self) -> bool {
        self.need_to_be_against_wall
    }

    /// `isPole`.
    pub fn is_pole(&self) -> bool {
        self.is_pole
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

    /// `checkScripts`: drops extra faces of a single-face config, sizes the
    /// faces, collects tile names and decides validity.
    fn check_scripts(&mut self, sm: &ScriptManager) -> ScriptResult {
        if self.is_single_face {
            for i in 0..6 {
                if self.faces[i].as_ref().is_some_and(|f| f.face_id != 0) {
                    self.warn_or_error(sm, "SingleFace has other faces defined.")?;
                    self.faces[i] = None;
                }
            }
        }
        let mut has_face = false;
        let mut has_duplicates = false;
        let mut face_count = 0;
        for i in 0..6 {
            let Some(mut face) = self.faces[i].take() else {
                continue;
            };
            has_face = true;
            face_count += 1;
            let mut result = Ok(());
            for layer in &face.layers {
                face.total_height = face.total_height.max(layer.rows.len() as i32);
                for row in &layer.rows {
                    face.total_width = face.total_width.max(row.tiles.len() as i32);
                    for tile in &row.tiles {
                        let Some(name) = &tile.tile_name else {
                            continue;
                        };
                        if self.all_tile_names.contains(name) {
                            result = result.and_then(|_| self.warn_or_error(sm, &format!("Tile duplicate: {name}")));
                            has_duplicates = true;
                            continue;
                        }
                        self.all_tile_names.push(name.clone());
                    }
                }
            }
            let multi = face_count > 1 || face.total_height > 1 || face.total_width > 1;
            self.faces[i] = Some(face);
            result?;
            if multi {
                self.is_multi_tile = true;
            }
        }
        for face in self.faces.iter().flatten() {
            let Some(other) = complementary_face(face.face_id).and_then(|c| self.faces[c].as_ref()) else {
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
        if has_face && !has_duplicates {
            self.is_valid = true;
        }
        Ok(())
    }

    /// `loadFace`: either rows directly in the face block, or `layer` blocks
    /// (with optional light offsets).
    fn load_face(face: &mut FaceScript, block: &Block) -> ScriptResult {
        let mut layer = ZLayer::default();
        load_layer(&mut layer, block);
        if !layer.rows.is_empty() {
            face.layers.push(layer);
            return Ok(());
        }
        for element in &block.elements {
            let BlockElement::Value(i) = element else {
                continue;
            };
            let s = &block.values[*i].string;
            if java::trim(s).is_empty() || !s.contains('=') {
                continue;
            }
            let split = java::split_char(s, '=');
            let k = java::trim(array_get(&split, 0)?);
            let v = java::trim(array_get(&split, 1)?);
            if java::eq_ignore_case(k, "lightOffsetX") {
                face.lightsource_offset_x = parse_int(v)?;
            }
            if java::eq_ignore_case(k, "lightOffsetY") {
                face.lightsource_offset_y = parse_int(v)?;
            }
            if java::eq_ignore_case(k, "lightOffsetZ") {
                face.lightsource_offset_z = parse_int(v)?;
            }
        }
        for child in &block.children {
            if !java::eq_ignore_case(child.type_str(), "layer") {
                continue;
            }
            load_layer(&mut layer, child);
            face.layers.push(std::mem::take(&mut layer));
        }
        Ok(())
    }

    /// The key/value part of `load`, which the original runs once per child block.
    fn load_values(&mut self, block: &Block) {
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            let eq = |s: &str| java::eq_ignore_case(k, s);
            if eq("health") {
                self.health = try_parse_int(v, -1);
            } else if eq("isProp") {
                self.is_prop = java::parse_bool(v);
            } else if eq("skillBaseHealth") {
                self.skill_base_health = try_parse_int(v, 0);
            } else if eq("lightRadius") {
                self.light_radius = try_parse_int(v, 10);
            } else if eq("lightsourceItem") {
                if v.contains("tags") {
                    let v = v.replace("tags[", "").replace(']', "");
                    self.lightsource_tag_item =
                        Some(java::split_char(&v, ';').iter().map(|s| java::trim(s).to_string()).collect());
                } else {
                    self.lightsource_item = Some(v.to_string());
                }
            } else if eq("lightsourceFuel") {
                self.lightsource_fuel = Some(v.to_string());
            } else if eq("debugItem") {
                self.debug_item = Some(v.to_string());
            } else if eq("OnCreate") {
                self.on_create = Some(v.to_string());
            } else if eq("OnIsValid") {
                self.on_is_valid = Some(v.to_string());
            } else if eq("TimedActionOnIsValid") {
                self.timed_action_on_is_valid = Some(v.to_string());
            } else if eq("isThumpable") {
                self.is_thumpable = java::eq_ignore_case(v, "true");
            } else if eq("breakSound") {
                self.break_sound = v.to_string();
            } else if eq("corner") {
                self.corner_sprite = Some(v.to_string());
            } else if eq("dontNeedFrame") {
                self.dont_need_frame = java::eq_ignore_case(v, "true");
            } else if eq("needWindowFrame") {
                self.need_window_frame = java::eq_ignore_case(v, "true");
            } else if eq("isPole") {
                self.is_pole = java::eq_ignore_case(v, "true");
            } else if eq("needToBeAgainstWall") {
                self.need_to_be_against_wall = java::parse_bool(v);
            } else if eq("canBePadlocked") {
                self.can_be_padlocked = java::parse_bool(v);
            } else if eq("previousStage") {
                self.previous_stage.extend(java::split_char(v, ';').iter().map(|s| s.to_string()));
            } else if eq("bonusHealth") {
                self.bonus_health = try_parse_int(v, 0);
            }
        }
    }
}

impl ComponentScript for SpriteConfigScript {
    component_accessors!("zombie.scripting.entity.components.spriteconfig.SpriteConfigScript");

    fn iso_master_only(&self) -> bool {
        false
    }

    fn pre_reload(&mut self) {
        self.faces = Default::default();
        self.corner_sprite = None;
        self.is_single_face = false;
        self.is_multi_tile = false;
        self.is_valid = false;
        self.all_tile_names.clear();
        self.is_prop = false;
        self.health = -1;
        self.skill_base_health = 0;
        self.is_thumpable = true;
        self.break_sound = DEFAULT_BREAK_SOUND.to_string();
        self.dont_need_frame = false;
        self.need_window_frame = false;
        self.is_pole = false;
        self.need_to_be_against_wall = false;
        self.previous_stage.clear();
        self.bonus_health = 0;
    }

    /// `OnScriptsLoaded`. TODO(ScriptsDictionary): registers the script on init.
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        Ok(())
    }

    /// `copyFrom`: not implemented in the original.
    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Err(runtime_exception("Unfinished Todo"))
    }

    /// `load`. The original reads the key/value settings inside its loop
    /// over child blocks, so they apply once per (valid) child.
    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "face") && !self.is_single_face {
                let id = child.id.as_deref().ok_or_else(null_pointer)?;
                let face_id = get_face_id_for_string(id);
                if face_id == -1 {
                    log_error(sm, &format!("Cannot find face for: {id}"));
                    continue;
                }
                let mut face = FaceScript {
                    face_name: id.to_string(),
                    face_id,
                    total_width: 0,
                    total_height: 0,
                    lightsource_offset_x: 0,
                    lightsource_offset_y: 0,
                    lightsource_offset_z: 0,
                    layers: Vec::new(),
                };
                Self::load_face(&mut face, child)?;
                self.faces[face_id as usize] = Some(face);
                if java::eq_ignore_case(id, "single") {
                    self.is_single_face = true;
                }
            } else {
                log_error(
                    sm,
                    &format!("Unknown block '{}' in entity iso script: {}", child.type_str(), self.component.name(sm)?),
                );
            }
            self.load_values(block);
        }
        self.check_scripts(sm)
    }
}
