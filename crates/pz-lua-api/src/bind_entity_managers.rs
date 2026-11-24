//! Build 42.22 `zombie.entity.GameEntityManager` and
//! `zombie.entity.components.spriteconfig.SpriteConfigManager`
//! (`ObjectInfo`, `FaceInfo`).

use crate::bind_java_util::JavaList;
use crate::expose::{expose_class, int_arg, num_arg, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub const GAME_ENTITY_MANAGER_CLASS: &str = "zombie.entity.GameEntityManager";
pub const SPRITE_CONFIG_MANAGER_CLASS: &str =
    "zombie.entity.components.spriteconfig.SpriteConfigManager";
pub const OBJECT_INFO_CLASS: &str =
    "zombie.entity.components.spriteconfig.SpriteConfigManager$ObjectInfo";
pub const FACE_INFO_CLASS: &str =
    "zombie.entity.components.spriteconfig.SpriteConfigManager$FaceInfo";

pub const FACE_SINGLE: &str = "single";
pub const FACE_N: &str = "n";
pub const FACE_W: &str = "w";
pub const FACE_S: &str = "s";
pub const FACE_E: &str = "e";
pub const FACE_N_OPEN: &str = "n_open";
pub const FACE_W_OPEN: &str = "w_open";

pub const FACE_ID_SINGLE: i32 = 0;
pub const FACE_ID_N: i32 = 0;
pub const FACE_ID_W: i32 = 1;
pub const FACE_ID_S: i32 = 2;
pub const FACE_ID_E: i32 = 3;
pub const FACE_ID_CARDINAL_MAX: i32 = 4;
pub const FACE_ID_N_OPEN: i32 = 4;
pub const FACE_ID_W_OPEN: i32 = 5;
pub const FACE_ID_MAX: i32 = 6;

#[derive(Default)]
struct GameEntityManagerState {
    initialized: bool,
    world_version: i32,
    processing: bool,
    need_save: bool,
    entities: BTreeMap<i64, Value>,
}

impl GameEntityManagerState {
    fn init(&mut self, world_version: i32) {
        self.initialized = true;
        self.world_version = world_version;
        self.processing = false;
        self.need_save = false;
    }

    fn reset(&mut self) {
        self.initialized = false;
        self.processing = false;
        self.need_save = false;
        self.entities.clear();
    }
}

#[derive(Clone, Debug)]
struct FaceInfoData {
    face_name: String,
    width: i32,
    height: i32,
    z_layers: i32,
    master_sprite: String,
}

#[derive(Clone, Debug)]
struct ObjectInfoData {
    name: String,
    version: i64,
    main_sprite: String,
    single_face: bool,
    is_prop: bool,
    time: i32,
    need_to_be_learn: bool,
    tags: Vec<String>,
    faces: Vec<FaceInfoData>,
}

struct LuaObjectInfo(ObjectInfoData);
impl LuaObject for LuaObjectInfo {
    fn class_name(&self) -> &str {
        OBJECT_INFO_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaFaceInfo(FaceInfoData);
impl LuaObject for LuaFaceInfo {
    fn class_name(&self) -> &str {
        FACE_INFO_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default)]
struct SpriteConfigManagerState {
    has_load_errors: bool,
    objects: BTreeMap<String, ObjectInfoData>,
    by_sprite: BTreeMap<String, String>,
}

impl SpriteConfigManagerState {
    fn reset(&mut self) {
        self.has_load_errors = false;
        self.objects.clear();
        self.by_sprite.clear();
    }
}

thread_local! {
    static ENTITY_MGR: RefCell<GameEntityManagerState> =
        RefCell::new(GameEntityManagerState::default());
    static SPRITE_CFG: RefCell<SpriteConfigManagerState> =
        RefCell::new(SpriteConfigManagerState::default());
}

/// `GameEntityManager.Init(worldVersion)`.
pub fn init_game_entity_manager(world_version: i32) {
    ENTITY_MGR.with(|m| m.borrow_mut().init(world_version));
}

/// `GameEntityManager.Update()`.
pub fn update_game_entity_manager() {
    ENTITY_MGR.with(|m| {
        let mut m = m.borrow_mut();
        if !m.initialized {
            return;
        }
        m.processing = true;
        m.processing = false;
    });
}

/// `GameEntityManager.Reset()`.
pub fn reset_game_entity_manager() {
    ENTITY_MGR.with(|m| m.borrow_mut().reset());
}

/// `SpriteConfigManager.Reset()`.
pub fn reset_sprite_config_manager() {
    SPRITE_CFG.with(|s| s.borrow_mut().reset());
}

/// `SpriteConfigManager.GetFaceIdForString(name)`.
pub fn face_id_for_string(name: &str) -> i32 {
    if name.eq_ignore_ascii_case(FACE_SINGLE) || name.eq_ignore_ascii_case(FACE_N) {
        FACE_ID_N
    } else if name.eq_ignore_ascii_case(FACE_W) {
        FACE_ID_W
    } else if name.eq_ignore_ascii_case(FACE_S) {
        FACE_ID_S
    } else if name.eq_ignore_ascii_case(FACE_E) {
        FACE_ID_E
    } else if name.eq_ignore_ascii_case(FACE_N_OPEN) {
        FACE_ID_N_OPEN
    } else if name.eq_ignore_ascii_case(FACE_W_OPEN) {
        FACE_ID_W_OPEN
    } else {
        -1
    }
}

/// Reset per-session entity and sprite-config manager state.
pub(crate) fn reset_session_handles() {
    reset_game_entity_manager();
    reset_sprite_config_manager();
}

fn with_obj_info<R>(v: &Value, f: impl FnOnce(&ObjectInfoData) -> R) -> Option<R> {
    if let Value::Object(o) = v {
        if let Some(oi) = o.as_any().downcast_ref::<LuaObjectInfo>() {
            return Some(f(&oi.0));
        }
    }
    None
}

fn with_face_info<R>(v: &Value, f: impl FnOnce(&FaceInfoData) -> R) -> Option<R> {
    if let Value::Object(o) = v {
        if let Some(fi) = o.as_any().downcast_ref::<LuaFaceInfo>() {
            return Some(f(&fi.0));
        }
    }
    None
}

pub fn register(thread: &mut Thread) {
    register_game_entity_manager(thread);
    register_sprite_config_manager(thread);
}

fn register_game_entity_manager(thread: &mut Thread) {
    let m = expose_class(thread, GAME_ENTITY_MANAGER_CLASS, Some("java.lang.Object"));
    m.field("debugMode", Value::Bool(false));
    m.field("needSave", Value::Bool(false));
    m.field("bbBlockSize", Value::Num(4096.0));

    m.static_fn("Init", |c| {
        let ver = int_arg(c, 0);
        init_game_entity_manager(ver);
        Ok(0)
    });
    m.static_fn("Update", |_c| {
        update_game_entity_manager();
        Ok(0)
    });
    m.static_fn("isEngineProcessing", |c| {
        let v = ENTITY_MGR.with(|e| e.borrow().processing);
        Ok(c.push(v))
    });
    m.static_fn("RenderLast", |_c| Ok(0));
    m.static_fn("Reset", |_c| {
        reset_game_entity_manager();
        Ok(0)
    });
    m.static_fn("GetEntity", |c| {
        let id = num_arg(c, 0) as i64;
        let v = ENTITY_MGR.with(|e| e.borrow().entities.get(&id).cloned().unwrap_or(Value::Nil));
        Ok(c.push(v))
    });
    m.static_fn("checkEntityIDChange", |c| {
        let entity = c.arg(0);
        let old_id = num_arg(c, 1) as i64;
        let new_id = num_arg(c, 2) as i64;
        ENTITY_MGR.with(|e| {
            let mut e = e.borrow_mut();
            if old_id != 0 {
                e.entities.remove(&old_id);
            }
            if !entity.is_nil() && new_id != 0 {
                e.entities.insert(new_id, entity);
                e.need_save = true;
            }
        });
        Ok(0)
    });
    m.static_fn("Save", |_c| {
        ENTITY_MGR.with(|e| e.borrow_mut().need_save = false);
        Ok(0)
    });
    m.static_fn("getIsoEntitiesDebug", |c| {
        let items: Vec<Value> = ENTITY_MGR.with(|e| e.borrow().entities.values().cloned().collect());
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    m.static_fn("reloadDebug", |_c| Ok(0));
}

fn register_sprite_config_manager(thread: &mut Thread) {
    let fi = expose_class(thread, FACE_INFO_CLASS, Some("java.lang.Object"));
    fi.method("getFaceName", |c| {
        let s = with_face_info(&c.arg(0), |f| f.face_name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    fi.method("getWidth", |c| {
        let v = with_face_info(&c.arg(0), |f| f.width).unwrap_or(1);
        Ok(c.push(v))
    });
    fi.method("getHeight", |c| {
        let v = with_face_info(&c.arg(0), |f| f.height).unwrap_or(1);
        Ok(c.push(v))
    });
    fi.method("getzLayers", |c| {
        let v = with_face_info(&c.arg(0), |f| f.z_layers).unwrap_or(1);
        Ok(c.push(v))
    });
    fi.method("getMasterOffsetX", |c| Ok(c.push(0)));
    fi.method("getMasterOffsetY", |c| Ok(c.push(0)));
    fi.method("getMasterOffsetZ", |c| Ok(c.push(0)));
    fi.method("getMasterSpriteName", |c| {
        let s = with_face_info(&c.arg(0), |f| f.master_sprite.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });

    let oi = expose_class(thread, OBJECT_INFO_CLASS, Some("java.lang.Object"));
    oi.method("getName", |c| {
        let s = with_obj_info(&c.arg(0), |o| o.name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    oi.method("getVersion", |c| {
        let v = with_obj_info(&c.arg(0), |o| o.version as f64).unwrap_or(0.0);
        Ok(c.push(v))
    });
    oi.method("getMainSpriteNameUI", |c| {
        let s = with_obj_info(&c.arg(0), |o| o.main_sprite.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    oi.method("isSingleFace", |c| {
        let v = with_obj_info(&c.arg(0), |o| o.single_face).unwrap_or(true);
        Ok(c.push(v))
    });
    oi.method("canRotate", |c| {
        let v = with_obj_info(&c.arg(0), |o| !o.single_face && o.faces.len() > 1).unwrap_or(false);
        Ok(c.push(v))
    });
    oi.method("isProp", |c| {
        let v = with_obj_info(&c.arg(0), |o| o.is_prop).unwrap_or(false);
        Ok(c.push(v))
    });
    oi.method("getTime", |c| {
        let v = with_obj_info(&c.arg(0), |o| o.time).unwrap_or(0);
        Ok(c.push(v))
    });
    oi.method("needToBeLearn", |c| {
        let v = with_obj_info(&c.arg(0), |o| o.need_to_be_learn).unwrap_or(false);
        Ok(c.push(v))
    });
    oi.method("getTags", |c| {
        let items: Vec<Value> = with_obj_info(&c.arg(0), |o| {
            o.tags.iter().map(|t| Value::str(t.as_str())).collect()
        })
        .unwrap_or_default();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    oi.method("getRequiredSkillCount", |c| Ok(c.push(0)));
    oi.method("getFace", |c| {
        let arg = c.arg(1);
        let found = with_obj_info(&c.arg(0), |o| match &arg {
            Value::Str(s) => o
                .faces
                .iter()
                .find(|f| f.face_name.eq_ignore_ascii_case(s))
                .cloned(),
            Value::Num(n) => o.faces.get(*n as usize).cloned(),
            _ => None,
        })
        .flatten();
        match found {
            Some(f) => Ok(c.push(Value::Object(Rc::new(LuaFaceInfo(f))))),
            None => Ok(c.push(Value::Nil)),
        }
    });
    oi.method("getFaceForSprite", |c| {
        let sprite = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let found = with_obj_info(&c.arg(0), |o| {
            o.faces
                .iter()
                .find(|f| f.master_sprite == sprite)
                .cloned()
        })
        .flatten();
        match found {
            Some(f) => Ok(c.push(Value::Object(Rc::new(LuaFaceInfo(f))))),
            None => Ok(c.push(Value::Nil)),
        }
    });

    let scm = expose_class(thread, SPRITE_CONFIG_MANAGER_CLASS, Some("java.lang.Object"));
    scm.field("FACE_SINGLE", Value::str(FACE_SINGLE));
    scm.field("FACE_N", Value::str(FACE_N));
    scm.field("FACE_W", Value::str(FACE_W));
    scm.field("FACE_S", Value::str(FACE_S));
    scm.field("FACE_E", Value::str(FACE_E));
    scm.field("FACE_N_OPEN", Value::str(FACE_N_OPEN));
    scm.field("FACE_W_OPEN", Value::str(FACE_W_OPEN));
    scm.field("FACE_ID_SINGLE", Value::Num(FACE_ID_SINGLE as f64));
    scm.field("FACE_ID_N", Value::Num(FACE_ID_N as f64));
    scm.field("FACE_ID_W", Value::Num(FACE_ID_W as f64));
    scm.field("FACE_ID_S", Value::Num(FACE_ID_S as f64));
    scm.field("FACE_ID_E", Value::Num(FACE_ID_E as f64));
    scm.field(
        "FACE_ID_CARDINAL_MAX",
        Value::Num(FACE_ID_CARDINAL_MAX as f64),
    );
    scm.field("FACE_ID_N_OPEN", Value::Num(FACE_ID_N_OPEN as f64));
    scm.field("FACE_ID_W_OPEN", Value::Num(FACE_ID_W_OPEN as f64));
    scm.field("FACE_ID_MAX", Value::Num(FACE_ID_MAX as f64));

    scm.static_fn("GetFaceIdForString", |c| {
        let s = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(face_id_for_string(&s)))
    });
    scm.static_fn("HasLoadErrors", |c| {
        let v = SPRITE_CFG.with(|s| s.borrow().has_load_errors);
        Ok(c.push(v))
    });
    scm.static_fn("GetObjectInfo", |c| {
        let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let found = SPRITE_CFG.with(|s| s.borrow().objects.get(&name).cloned());
        match found {
            Some(oi) => Ok(c.push(Value::Object(Rc::new(LuaObjectInfo(oi))))),
            None => Ok(c.push(Value::Nil)),
        }
    });
    scm.static_fn("getObjectInfoFromSprite", |c| {
        let sprite = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let found = SPRITE_CFG.with(|s| {
            let s = s.borrow();
            s.by_sprite
                .get(&sprite)
                .and_then(|name| s.objects.get(name))
                .cloned()
        });
        match found {
            Some(oi) => Ok(c.push(Value::Object(Rc::new(LuaObjectInfo(oi))))),
            None => Ok(c.push(Value::Nil)),
        }
    });
    scm.static_fn("GetObjectInfoList", |c| {
        let items: Vec<Value> = SPRITE_CFG.with(|s| {
            s.borrow()
                .objects
                .values()
                .cloned()
                .map(|oi| Value::Object(Rc::new(LuaObjectInfo(oi))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    scm.static_fn("Reset", |_c| {
        reset_sprite_config_manager();
        Ok(0)
    });
    scm.static_fn("InitScriptsPostTileDef", |_c| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn game_entity_and_sprite_config_managers_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            -- GameEntityManager
            GameEntityManager.Init(249)
            assert(GameEntityManager.isEngineProcessing() == false)
            local fake = { kind = "bench" }
            GameEntityManager.checkEntityIDChange(fake, 0, 1001)
            assert(GameEntityManager.GetEntity(1001) == fake)
            assert(GameEntityManager.getIsoEntitiesDebug():size() == 1)
            GameEntityManager.checkEntityIDChange(fake, 1001, 1002)
            assert(GameEntityManager.GetEntity(1001) == nil)
            assert(GameEntityManager.GetEntity(1002) == fake)
            GameEntityManager.Update()
            GameEntityManager.Save()
            GameEntityManager.Reset()
            assert(GameEntityManager.GetEntity(1002) == nil)

            -- SpriteConfigManager
            assert(SpriteConfigManager.FACE_SINGLE == "single")
            assert(SpriteConfigManager.FACE_N == "n")
            assert(SpriteConfigManager.FACE_W == "w")
            assert(SpriteConfigManager.FACE_S == "s")
            assert(SpriteConfigManager.FACE_E == "e")
            assert(SpriteConfigManager.FACE_N_OPEN == "n_open")
            assert(SpriteConfigManager.FACE_W_OPEN == "w_open")
            assert(SpriteConfigManager.FACE_ID_SINGLE == 0)
            assert(SpriteConfigManager.FACE_ID_W == 1)
            assert(SpriteConfigManager.FACE_ID_S == 2)
            assert(SpriteConfigManager.FACE_ID_E == 3)
            assert(SpriteConfigManager.FACE_ID_CARDINAL_MAX == 4)
            assert(SpriteConfigManager.FACE_ID_N_OPEN == 4)
            assert(SpriteConfigManager.FACE_ID_W_OPEN == 5)
            assert(SpriteConfigManager.FACE_ID_MAX == 6)

            assert(SpriteConfigManager.GetFaceIdForString("single") == 0)
            assert(SpriteConfigManager.GetFaceIdForString("N") == 0)
            assert(SpriteConfigManager.GetFaceIdForString("W") == 1)
            assert(SpriteConfigManager.GetFaceIdForString("S") == 2)
            assert(SpriteConfigManager.GetFaceIdForString("E") == 3)
            assert(SpriteConfigManager.GetFaceIdForString("n_open") == 4)
            assert(SpriteConfigManager.GetFaceIdForString("w_open") == 5)
            assert(SpriteConfigManager.GetFaceIdForString("bogus") == -1)
            assert(SpriteConfigManager.HasLoadErrors() == false)
            assert(SpriteConfigManager.GetObjectInfoList():size() == 0)
            SpriteConfigManager.Reset()
        "#;
        let f = stdlib::load_string(src, Some("entity_managers"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();
    }
}
