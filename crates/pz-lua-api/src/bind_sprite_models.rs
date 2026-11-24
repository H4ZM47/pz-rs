//! Lua bindings for Build 42.22 sprite model tile definitions:
//! - `zombie.spriteModel.SpriteModelManager` (singleton)
//! - `zombie.iso.SpriteModelsFile$Tileset`
//! - `zombie.iso.SpriteModelsFile$Tile`
//! - `zombie.iso.SpriteModel.new()` constructor (methods live in `bind_scripts::models`)
//!
//! Parses `media/spriteModels.txt` (block format `spriteModel { VERSION = 1, tileset { name = ..., tile { xy = c r, modelScript = ..., translate = ..., rotate = ..., scale = ..., animation = ..., animationTime = ..., texture = ... } } }`)
//! using `pz_scripts::parser` (zero new dependencies) and publishes resolved
//! `SpriteModel` scripts to `ScriptManager` (`toScriptManager`).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, Thread, Value};
use pz_scripts::base::ScriptRef;
use pz_scripts::math::Vector3;
use pz_scripts::objects::models::SpriteModel;
use pz_scripts::parser::{self, Block};

use crate::bind_java_util::list_value;
use crate::bind_registry::shared_script_manager;
use crate::bind_scripts::{handle_arg, script_value};
use crate::expose::{expose_class, int_arg, str_arg, this};

pub const SPRITE_MODEL_MANAGER_CLASS: &str = "zombie.spriteModel.SpriteModelManager";
pub const SPRITE_MODEL_CLASS: &str = "zombie.iso.SpriteModel";
pub const TILESET_CLASS: &str = "zombie.iso.SpriteModelsFile$Tileset";
pub const TILE_CLASS: &str = "zombie.iso.SpriteModelsFile$Tile";

const GAME_MOD_ID: &str = "game";

#[derive(Clone, Debug)]
pub struct SpriteModelTile {
    pub x: i32,
    pub y: i32,
    pub sprite_model: ScriptRef,
}

#[derive(Clone, Debug)]
pub struct SpriteModelTileset {
    pub name: String,
    pub tiles: Vec<SpriteModelTile>,
}

impl SpriteModelTileset {
    pub fn get_tile(&self, col: i32, row: i32) -> Option<&SpriteModelTile> {
        self.tiles.iter().find(|t| t.x == col && t.y == row)
    }

    pub fn set_tile(&mut self, col: i32, row: i32, model: ScriptRef) {
        if let Some(existing) = self.tiles.iter_mut().find(|t| t.x == col && t.y == row) {
            existing.sprite_model = model;
        } else {
            self.tiles.push(SpriteModelTile {
                x: col,
                y: row,
                sprite_model: model,
            });
        }
    }

    pub fn remove_tile(&mut self, col: i32, row: i32) {
        self.tiles.retain(|t| !(t.x == col && t.y == row));
    }
}

#[derive(Default)]
pub struct ModSpriteModels {
    pub mod_id: String,
    pub media_dir: PathBuf,
    pub tilesets: Vec<Rc<RefCell<SpriteModelTileset>>>,
}

impl ModSpriteModels {
    fn find_tileset(&self, name: &str) -> Option<Rc<RefCell<SpriteModelTileset>>> {
        self.tilesets
            .iter()
            .find(|ts| ts.borrow().name == name)
            .cloned()
    }

    fn get_or_create_tileset(&mut self, name: &str) -> Rc<RefCell<SpriteModelTileset>> {
        if let Some(ts) = self.find_tileset(name) {
            return ts;
        }
        let ts = Rc::new(RefCell::new(SpriteModelTileset {
            name: name.to_string(),
            tiles: Vec::new(),
        }));
        self.tilesets.push(ts.clone());
        ts
    }
}

/// Runtime state of `SpriteModelManager.getInstance()`.
#[derive(Default)]
pub struct SpriteModelManagerState {
    pub initialized: bool,
    pub mod_order: Vec<String>,
    pub mods: HashMap<String, ModSpriteModels>,
    /// Merged lookup `(tileset_name, col, row) -> ScriptRef<SpriteModel>`.
    pub merged: HashMap<(String, i32, i32), ScriptRef>,
}

impl SpriteModelManagerState {
    pub fn reset(&mut self) {
        self.initialized = false;
        self.mod_order.clear();
        self.mods.clear();
        self.merged.clear();
    }

    pub fn ensure_mod(&mut self, mod_id: &str, media_dir: PathBuf) -> &mut ModSpriteModels {
        if !self.mods.contains_key(mod_id) {
            self.mod_order.push(mod_id.to_string());
            self.mods.insert(
                mod_id.to_string(),
                ModSpriteModels {
                    mod_id: mod_id.to_string(),
                    media_dir,
                    tilesets: Vec::new(),
                },
            );
        }
        self.mods.get_mut(mod_id).unwrap()
    }

    pub fn load_file_for_mod(&mut self, mod_id: &str, media_dir: &Path) -> Result<usize, String> {
        let path = media_dir.join("spriteModels.txt");
        let text = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        self.parse_text_for_mod(mod_id, media_dir.to_path_buf(), &text)
    }

    pub fn parse_text_for_mod(
        &mut self,
        mod_id: &str,
        media_dir: PathBuf,
        text: &str,
    ) -> Result<usize, String> {
        let stripped = parser::strip_comments(text);
        let root = parser::parse(&stripped);
        let mut count = 0usize;
        let mut parsed_tilesets = Vec::new();
        for b in &root.children {
            if b.type_.as_deref() != Some("spriteModel") {
                continue;
            }
            for ts_block in &b.children {
                if ts_block.type_.as_deref() != Some("tileset") {
                    continue;
                }
                let Some(ts_name) = ts_block.get_value("name").map(|v| v.get_value().trim()) else {
                    continue;
                };
                if ts_name.is_empty() {
                    continue;
                }
                let mut tiles = Vec::new();
                for tile_block in &ts_block.children {
                    if tile_block.type_.as_deref() != Some("tile") {
                        continue;
                    }
                    if let Some(tile) = parse_tile_block(ts_name, tile_block) {
                        tiles.push(tile);
                        count += 1;
                    }
                }
                parsed_tilesets.push(Rc::new(RefCell::new(SpriteModelTileset {
                    name: ts_name.to_string(),
                    tiles,
                })));
            }
        }
        let m = self.ensure_mod(mod_id, media_dir);
        m.tilesets = parsed_tilesets;
        self.rebuild_merged();
        Ok(count)
    }

    pub fn rebuild_merged(&mut self) {
        self.merged.clear();
        for mod_id in &self.mod_order {
            let Some(m) = self.mods.get(mod_id) else { continue };
            for ts_rc in &m.tilesets {
                let ts = ts_rc.borrow();
                for tile in &ts.tiles {
                    self.merged.insert(
                        (ts.name.clone(), tile.x, tile.y),
                        tile.sprite_model.clone(),
                    );
                }
            }
        }
    }

    pub fn set_tile_properties(
        &mut self,
        mod_id: &str,
        tileset_name: &str,
        col: i32,
        row: i32,
        src: &SpriteModel,
    ) {
        let mut copy = SpriteModel::new();
        copy.set(src);
        let tile_name = format!("{tileset_name}_{}", col + row * 8);
        copy.base.init_load_pp(&tile_name);
        let model_ref = ScriptRef::new(copy);
        let m = self.ensure_mod(mod_id, PathBuf::new());
        let ts = m.get_or_create_tileset(tileset_name);
        ts.borrow_mut().set_tile(col, row, model_ref);
        self.rebuild_merged();
    }

    pub fn get_tile_properties(
        &self,
        mod_id: &str,
        tileset_name: &str,
        col: i32,
        row: i32,
    ) -> Option<ScriptRef> {
        let m = self.mods.get(mod_id)?;
        let ts = m.find_tileset(tileset_name)?;
        let borrowed = ts.borrow();
        borrowed.get_tile(col, row).map(|t| t.sprite_model.clone())
    }

    pub fn clear_tile_properties(
        &mut self,
        mod_id: &str,
        tileset_name: &str,
        col: i32,
        row: i32,
    ) {
        if let Some(m) = self.mods.get_mut(mod_id) {
            if let Some(ts) = m.find_tileset(tileset_name) {
                ts.borrow_mut().remove_tile(col, row);
            }
        }
        self.rebuild_merged();
    }

    pub fn find_tileset(
        &self,
        mod_id: &str,
        tileset_name: &str,
    ) -> Option<Rc<RefCell<SpriteModelTileset>>> {
        self.mods.get(mod_id)?.find_tileset(tileset_name)
    }

    pub fn serialize_mod(&self, mod_id: &str) -> String {
        let mut out = String::from("spriteModel\n{\n    VERSION = 1,\n");
        if let Some(m) = self.mods.get(mod_id) {
            for ts_rc in &m.tilesets {
                let ts = ts_rc.borrow();
                if ts.tiles.is_empty() {
                    continue;
                }
                out.push_str(&format!("\n    tileset\n    {{\n        name = {},\n", ts.name));
                for tile in &ts.tiles {
                    if let Some(sm_rc) = tile.sprite_model.downcast::<SpriteModel>() {
                        if let Ok(sm) = sm_rc.try_borrow() {
                            out.push_str(&format!(
                                "\n        tile\n        {{\n            xy = {} {},\n",
                                tile.x, tile.y
                            ));
                            if let Some(ms) = sm.model_script_name.as_deref() {
                                out.push_str(&format!("            modelScript = {ms},\n"));
                            }
                            if let Some(tex) = sm.texture_name.as_deref() {
                                out.push_str(&format!("            texture = {tex},\n"));
                            }
                            out.push_str(&format!(
                                "            translate = {:.4} {:.4} {:.4},\n",
                                sm.translate.x, sm.translate.y, sm.translate.z
                            ));
                            out.push_str(&format!(
                                "            rotate = {:.4} {:.4} {:.4},\n",
                                sm.rotate.x, sm.rotate.y, sm.rotate.z
                            ));
                            out.push_str(&format!("            scale = {:.4},\n", sm.scale));
                            if let Some(anim) = sm.animation_name.as_deref() {
                                out.push_str(&format!("            animation = {anim},\n"));
                            }
                            if sm.animation_time >= 0.0 {
                                out.push_str(&format!(
                                    "            animationTime = {:.4},\n",
                                    sm.animation_time
                                ));
                            }
                            out.push_str("        }\n");
                        }
                    }
                }
                out.push_str("    }\n");
            }
        }
        out.push_str("}\n");
        out
    }

    pub fn publish_to_script_manager(&self, only_mod: Option<&str>) {
        let sm = shared_script_manager();
        let base_mod = sm.get_module("Base");
        for mod_id in &self.mod_order {
            if let Some(want) = only_mod {
                if mod_id != want {
                    continue;
                }
            }
            let Some(m) = self.mods.get(mod_id) else { continue };
            for ts_rc in &m.tilesets {
                let ts = ts_rc.borrow();
                for tile in &ts.tiles {
                    if let Some(bm) = base_mod.as_ref() {
                        if let Ok(mut s) = tile.sprite_model.try_borrow_mut() {
                            if s.base().module().is_none() {
                                s.base_mut().set_module(bm);
                            }
                        }
                    }
                    let _ = sm.add_runtime_script(tile.sprite_model.clone());
                }
            }
        }
    }
}

fn parse_vec3(s: &str) -> Vector3 {
    let mut parts = s.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
    Vector3 {
        x: parts.next().unwrap_or(0.0),
        y: parts.next().unwrap_or(0.0),
        z: parts.next().unwrap_or(0.0),
    }
}

fn parse_tile_block(tileset: &str, block: &Block) -> Option<SpriteModelTile> {
    let xy_str = block.get_value("xy")?.get_value();
    let mut xy = xy_str.split_whitespace().filter_map(|p| p.parse::<i32>().ok());
    let x = xy.next()?;
    let y = xy.next()?;
    let mut sm = SpriteModel::new();
    sm.base.init_load_pp(&format!("{tileset}_{}", x + y * 8));
    if let Some(v) = block.get_value("modelScript") {
        let s = v.get_value().trim();
        if !s.is_empty() {
            sm.model_script_name = Some(s.to_string());
        }
    }
    if let Some(v) = block.get_value("texture") {
        let s = v.get_value().trim();
        if !s.is_empty() {
            sm.texture_name = Some(s.to_string());
        }
    }
    if let Some(v) = block.get_value("translate") {
        sm.translate = parse_vec3(v.get_value());
    }
    if let Some(v) = block.get_value("rotate") {
        sm.rotate = parse_vec3(v.get_value());
    }
    if let Some(v) = block.get_value("scale") {
        if let Ok(sc) = v.get_value().trim().parse::<f32>() {
            sm.scale = sc;
        }
    }
    if let Some(v) = block.get_value("animation") {
        let s = v.get_value().trim();
        if !s.is_empty() {
            sm.animation_name = Some(s.to_string());
        }
    }
    if let Some(v) = block.get_value("animationTime") {
        if let Ok(t) = v.get_value().trim().parse::<f32>() {
            sm.animation_time = t;
        }
    }
    Some(SpriteModelTile {
        x,
        y,
        sprite_model: ScriptRef::new(sm),
    })
}

thread_local! {
    static STATE: Rc<RefCell<SpriteModelManagerState>> =
        Rc::new(RefCell::new(SpriteModelManagerState::default()));
}

pub fn shared_sprite_model_manager() -> Rc<RefCell<SpriteModelManagerState>> {
    STATE.with(Rc::clone)
}

pub fn reset() {
    STATE.with(|s| s.borrow_mut().reset());
}

// ---- userdata wrappers ------------------------------------------------------

struct LuaSpriteModelManager(Rc<RefCell<GameFs>>);

impl LuaObject for LuaSpriteModelManager {
    fn class_name(&self) -> &str {
        SPRITE_MODEL_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaTileset(Rc<RefCell<SpriteModelTileset>>);

impl LuaObject for LuaTileset {
    fn class_name(&self) -> &str {
        TILESET_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaTile(SpriteModelTile);

impl LuaObject for LuaTile {
    fn class_name(&self) -> &str {
        TILE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registers `SpriteModelManager`, `SpriteModelsFile$Tileset`,
/// `SpriteModelsFile$Tile`, and `SpriteModel.new()` on `thread`.
pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>) {
    // Ensure `SpriteModel.new()` constructor exists on `zombie.iso.SpriteModel`.
    let sm_cls = expose_class(thread, SPRITE_MODEL_CLASS, None);
    sm_cls.ctor(|c| Ok(c.push(script_value(Some(&ScriptRef::new(SpriteModel::new()))))));

    if let Value::Table(meta) = thread.class_metatables.get_str(TILE_CLASS) {
        if let Value::Table(methods) = meta.get_str("__pz_methods") {
            meta.set_str("__index", methods);
        }
    }
    let tile_b = expose_class(thread, TILE_CLASS, None);
    let tile_methods = tile_b.methods.clone();
    if let Value::Table(meta) = thread.class_metatables.get_str(TILE_CLASS) {
        meta.set_str("__pz_methods", tile_methods.clone());
        meta.set_str(
            "__index",
            Value::Native(pz_kahlua::NativeFunction::new("__index", move |c| {
                let key = c.arg(1);
                let found = tile_methods.rawget(&key);
                if !found.is_nil() {
                    return Ok(c.push(found));
                }
                if key.as_str() == Some("spriteModel") {
                    let o = this::<LuaTile>(c)?;
                    let t = o.as_any().downcast_ref::<LuaTile>().unwrap();
                    return Ok(c.push(script_value(Some(&t.0.sprite_model))));
                }
                Ok(c.push(Value::Nil))
            })),
        );
    }

    let ts_b = expose_class(thread, TILESET_CLASS, None);
    ts_b.method("getName", |c| {
        let o = this::<LuaTileset>(c)?;
        let ts = o.as_any().downcast_ref::<LuaTileset>().unwrap().0.borrow();
        Ok(c.push(Value::str(ts.name.as_str())))
    });
    ts_b.method("getTile", |c| {
        let o = this::<LuaTileset>(c)?;
        let col = int_arg(c, 1);
        let row = int_arg(c, 2);
        let ts = o.as_any().downcast_ref::<LuaTileset>().unwrap().0.borrow();
        let out = ts
            .get_tile(col, row)
            .cloned()
            .map(|t| Value::Object(Rc::new(LuaTile(t))))
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });

    let b = expose_class(thread, SPRITE_MODEL_MANAGER_CLASS, None);
    let fs_inst = fs.clone();
    b.static_fn("getInstance", move |c| {
        Ok(c.push(Value::Object(Rc::new(LuaSpriteModelManager(fs_inst.clone())))))
    });
    b.method("init", |c| {
        let o = this::<LuaSpriteModelManager>(c)?;
        let fs_ref = &o.as_any().downcast_ref::<LuaSpriteModelManager>().unwrap().0;
        let media = fs_ref.borrow().base.join("media");
        let st = shared_sprite_model_manager();
        let mut st = st.borrow_mut();
        let _ = st.load_file_for_mod(GAME_MOD_ID, &media);
        st.initialized = true;
        Ok(0)
    });
    b.method("initGameData", |c| {
        let o = this::<LuaSpriteModelManager>(c)?;
        let fs_ref = &o.as_any().downcast_ref::<LuaSpriteModelManager>().unwrap().0;
        let media = fs_ref.borrow().base.join("media");
        let _ = shared_sprite_model_manager()
            .borrow_mut()
            .load_file_for_mod(GAME_MOD_ID, &media);
        Ok(0)
    });
    b.method("getModIDs", |c| {
        let ids: Vec<Value> = shared_sprite_model_manager()
            .borrow()
            .mod_order
            .iter()
            .map(|s| Value::str(s.as_str()))
            .collect();
        Ok(c.push(list_value(ids)))
    });
    b.method("setTileProperties", |c| {
        let Some(mod_id) = str_arg(c, 1) else { return Ok(0) };
        let Some(tileset) = str_arg(c, 2) else { return Ok(0) };
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let Some(h) = handle_arg(c, 5)? else { return Ok(0) };
        let copy = h.read(|m: &SpriteModel| {
            let mut s = SpriteModel::new();
            s.set(m);
            s
        })?;
        shared_sprite_model_manager()
            .borrow_mut()
            .set_tile_properties(&mod_id, &tileset, col, row, &copy);
        Ok(0)
    });
    b.method("getTileProperties", |c| {
        let (Some(mod_id), Some(tileset)) = (str_arg(c, 1), str_arg(c, 2)) else {
            return Ok(c.push(Value::Nil));
        };
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let out = shared_sprite_model_manager()
            .borrow()
            .get_tile_properties(&mod_id, &tileset, col, row);
        Ok(c.push(script_value(out.as_ref())))
    });
    b.method("clearTileProperties", |c| {
        let (Some(mod_id), Some(tileset)) = (str_arg(c, 1), str_arg(c, 2)) else {
            return Ok(0);
        };
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        shared_sprite_model_manager()
            .borrow_mut()
            .clear_tile_properties(&mod_id, &tileset, col, row);
        Ok(0)
    });
    b.method("findTileset", |c| {
        let (Some(mod_id), Some(tileset)) = (str_arg(c, 1), str_arg(c, 2)) else {
            return Ok(c.push(Value::Nil));
        };
        let out = shared_sprite_model_manager()
            .borrow()
            .find_tileset(&mod_id, &tileset)
            .map(|ts| Value::Object(Rc::new(LuaTileset(ts))))
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.method("toScriptManager", |c| {
        let only_mod = str_arg(c, 1);
        shared_sprite_model_manager()
            .borrow()
            .publish_to_script_manager(only_mod.as_deref());
        Ok(0)
    });
    b.method("loadedTileDefinitions", |_c| Ok(0));
    b.method("initSprites", |_c| Ok(0));
    b.method("write", |c| {
        let Some(mod_id) = str_arg(c, 1) else { return Ok(0) };
        let (media_dir, text) = {
            let st = shared_sprite_model_manager();
            let st = st.borrow();
            let dir = st.mods.get(&*mod_id).map(|m| m.media_dir.clone());
            (dir, st.serialize_mod(&mod_id))
        };
        if let Some(dir) = media_dir.filter(|d| !d.as_os_str().is_empty()) {
            let path = dir.join("spriteModels.txt");
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, text)
                .map_err(|e| LuaError::runtime(format!("SpriteModelManager.write: {e}")))?;
        }
        Ok(0)
    });
    b.method("Reset", |_c| {
        reset();
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn sprite_model_manager_parses_mutates_and_writes_from_lua() {
        reset();
        let root = std::env::temp_dir().join(format!(
            "pz-lua-api-sprite-models-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let media = root.join("media");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(
            media.join("spriteModels.txt"),
            r#"
spriteModel
{
    VERSION = 1,

    tileset
    {
        name = test_radio_01,

        tile
        {
            xy = 1 2,
            modelScript = Base.TestRadio,
            translate = 0.25 -0.5 0.1,
            rotate = 0 90 0,
            scale = 1.5,
        }
    }
}
"#,
        )
        .unwrap();

        let fs = Rc::new(RefCell::new(GameFs::new(&root)));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_scripts::register(&mut t, shared_script_manager());
        register(&mut t, fs);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            local mgr = SpriteModelManager.getInstance()
            mgr:init()
            assert(mgr:getModIDs():size() == 1)
            local ts = mgr:findTileset("game", "test_radio_01")
            assert(ts ~= nil and ts:getName() == "test_radio_01")
            local tile = ts:getTile(1, 2)
            assert(tile ~= nil and tile.spriteModel:getModelScriptName() == "Base.TestRadio")

            local sm = mgr:getTileProperties("game", "test_radio_01", 1, 2)
            assert(sm ~= nil and sm:getScale() == 1.5)

            local custom = SpriteModel.new()
            custom:setModelScriptName("Base.LampModel")
            custom:setScale(2.0)
            mgr:setTileProperties("game", "test_radio_01", 3, 0, custom)
            local got = mgr:getTileProperties("game", "test_radio_01", 3, 0)
            assert(got ~= nil and got:getModelScriptName() == "Base.LampModel" and got:getScale() == 2.0)

            mgr:write("game")
            mgr:clearTileProperties("game", "test_radio_01", 3, 0)
            assert(mgr:getTileProperties("game", "test_radio_01", 3, 0) == nil)

            mgr:Reset()
            assert(mgr:getModIDs():size() == 0)
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("bind_sprite_models_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
        let written = std::fs::read_to_string(media.join("spriteModels.txt")).unwrap();
        assert!(written.contains("Base.LampModel"));
        let _ = std::fs::remove_dir_all(&root);
        reset();
    }
}
