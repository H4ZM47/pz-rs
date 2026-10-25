//! File-system and mod globals of `LuaManager.GlobalObject`, and
//! `ChooseGameInfo$Mod`.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_fs::{GameFs, ModInfo};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, TableRef, Value};

use crate::bind_java_util::list_value;
use crate::expose::{expose_class, global, int_arg, str_arg, this};
use crate::LuaManager;

pub const MOD: &str = "zombie.gameStates.ChooseGameInfo$Mod";

/// A `ChooseGameInfo.Mod`: one object per mod directory, reading the
/// file system's copy so later changes show through.
pub struct LuaMod {
    pub dir: String,
    fs: Rc<RefCell<GameFs>>,
}

impl LuaObject for LuaMod {
    fn class_name(&self) -> &str {
        MOD
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static MODS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

fn mod_value(fs: &Rc<RefCell<GameFs>>, dir: Option<String>) -> Value {
    let Some(dir) = dir else { return Value::Nil };
    MODS.with(|m| {
        m.borrow_mut()
            .entry(dir.clone())
            .or_insert_with(|| {
                Value::Object(Rc::new(LuaMod {
                    dir,
                    fs: fs.clone(),
                }))
            })
            .clone()
    })
}

/// Runs `f` on the mod behind `self`.
fn with_mod<R>(c: &NativeCall<'_>, f: impl FnOnce(&ModInfo) -> R) -> LuaResult<R> {
    let o = this::<LuaMod>(c)?;
    let m = o.as_any().downcast_ref::<LuaMod>().unwrap();
    let fs = m.fs.borrow();
    let info = fs
        .mod_info(&m.dir)
        .ok_or_else(|| pz_kahlua::LuaError::runtime("mod not read"))?;
    Ok(f(info))
}

fn opt_list(v: &Option<Vec<String>>) -> Value {
    match v {
        Some(items) => list_value(items.iter().map(|s| Value::str(s)).collect()),
        None => Value::Nil,
    }
}

/// `isModActive`'s id: `mod.getId()` unless blank, else `mod.getDir()`.
pub(crate) fn mod_id_of(c: &NativeCall<'_>, i: usize) -> Option<String> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<LuaMod>().map(|m| {
            let fs = m.fs.borrow();
            match fs.mod_info(&m.dir) {
                Some(i) if !pz_fs::java::is_null_or_whitespace(&i.id) => i.id.clone(),
                _ => m.dir.clone(),
            }
        }),
        _ => None,
    }
}

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    let thread = &mut mgr.thread;
    let f = fs.clone();
    global(thread, "getMyDocumentFolder", move |c| {
        // `Core.getMyDocumentFolder()` is `getCacheDir()`.
        let dir = f.borrow().cache_dir().to_string_lossy().into_owned();
        Ok(c.push(Value::str(&dir)))
    });
    global(thread, "getFileSeparator", |c| {
        Ok(c.push(Value::str(std::path::MAIN_SEPARATOR_STR)))
    });

    let f = fs.clone();
    global(thread, "getModInfoByID", move |c| {
        let id = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let dir = f.borrow_mut().mod_details(&id);
        Ok(c.push(mod_value(&f, dir)))
    });
    let f = fs.clone();
    global(thread, "getModInfo", move |c| {
        let dir = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let found = f.borrow_mut().read_mod_info(&dir).map(|m| m.dir.clone());
        Ok(c.push(mod_value(&f, found)))
    });
    let f = fs.clone();
    global(thread, "isModActive", move |c| {
        let id = mod_id_of(c, 0);
        let active = id.is_some_and(|id| f.borrow().mod_ids().contains(&id));
        Ok(c.push(active))
    });
    let f = fs.clone();
    global(thread, "getActivatedMods", move |c| {
        let ids: Vec<Value> = f.borrow().mod_ids().iter().map(|s| Value::str(s)).collect();
        Ok(c.push(list_value(ids)))
    });
    let f = fs.clone();
    global(thread, "getModDirectoryTable", move |c| {
        let t = TableRef::new();
        let dirs = f.borrow_mut().all_mod_folders();
        for (i, dir) in dirs.iter().enumerate() {
            t.set_int(i as i64 + 1, Value::str(dir));
        }
        Ok(c.push(t))
    });

    let b = expose_class(thread, MOD, None);
    let f = fs.clone();
    b.ctor(move |c| {
        // `new Mod(dir)`: an unread mod for the directory.
        let dir = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        Ok(c.push(Value::Object(Rc::new(LuaMod {
            dir: dir.to_string(),
            fs: f.clone(),
        }))))
    });
    macro_rules! getter {
        ($name:literal, |$m:ident| $e:expr) => {
            b.method($name, |c| {
                let v: Value = with_mod(c, |$m| $e)?;
                Ok(c.push(v))
            });
        };
    }
    getter!("getDir", |m| Value::str(&m.dir));
    getter!("getCommonDir", |m| Value::str(&m.common_dir));
    getter!("getVersionDir", |m| Value::str(&m.version_dir));
    getter!("getName", |m| Value::str(&m.name));
    getter!("getId", |m| Value::str(&m.id));
    getter!("getDescription", |m| Value::str(&m.description));
    getter!("getUrl", |m| Value::str(m.url.as_deref().unwrap_or("")));
    getter!("getAuthor", |m| Value::str(
        m.author.as_deref().unwrap_or("")
    ));
    getter!("getModVersion", |m| Value::str(
        m.mod_version.as_deref().unwrap_or("")
    ));
    getter!("getIcon", |m| Value::str(m.icon.as_deref().unwrap_or("")));
    getter!("getCategory", |m| Value::str(
        m.category.as_deref().unwrap_or("")
    ));
    getter!("getSource", |m| Value::str(&m.source));
    getter!("getWorkshopID", |m| Value::str(&m.workshop_id));
    getter!("getPosterCount", |m| Value::Num(m.posters.len() as f64));
    getter!("getRequire", |m| opt_list(&m.require));
    getter!("getIncompatible", |m| opt_list(&m.incompatible));
    getter!("getLoadAfter", |m| opt_list(&m.load_after));
    getter!("getLoadBefore", |m| opt_list(&m.load_before));
    b.method("getPoster", |c| {
        let i = int_arg(c, 1);
        let v = with_mod(c, |m| {
            usize::try_from(i)
                .ok()
                .and_then(|i| m.posters.get(i))
                .map_or(Value::Nil, |p| Value::str(p))
        })?;
        Ok(c.push(v))
    });
    b.method("isAvailableSelf", |c| {
        let o = this::<LuaMod>(c)?;
        let m = o.as_any().downcast_ref::<LuaMod>().unwrap();
        let fs = m.fs.borrow();
        let ok = fs
            .mod_info(&m.dir)
            .is_some_and(|i| i.is_available_self(&fs.game_version));
        Ok(c.push(ok))
    });
    b.method("isAvailable", |c| {
        let o = this::<LuaMod>(c)?;
        let m = o.as_any().downcast_ref::<LuaMod>().unwrap();
        let read = m.fs.borrow().mod_info(&m.dir).is_some();
        let ok = read && m.fs.borrow_mut().is_available(&m.dir);
        Ok(c.push(ok))
    });
    b.method("getTexture", |c| {
        // The first poster, else the white texture.
        let o = this::<LuaMod>(c)?;
        let m = o.as_any().downcast_ref::<LuaMod>().unwrap();
        let poster =
            m.fs.borrow()
                .mod_info(&m.dir)
                .and_then(|i| i.posters.first().cloned());
        let tm = crate::bind_render::textures();
        let fs = m.fs.borrow();
        let mut t = poster
            .filter(|p| !pz_fs::java::is_null_or_whitespace(p))
            .and_then(|p| tm.borrow_mut().shared_texture(&fs, &p));
        if t.as_ref().is_none_or(|t| !t.ready) {
            t = Some(tm.borrow_mut().white());
        }
        Ok(c.push(crate::bind_render::texture_value(t)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_globals_from_lua() {
        let dir = std::env::temp_dir().join(format!("pz-bind-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game/media")).unwrap();
        std::fs::create_dir_all(dir.join("user/mods/M/common")).unwrap();
        std::fs::write(
            dir.join("user/mods/M/common/mod.info"),
            "name=My Mod\nid=M\nrequire=X,Y\n",
        )
        .unwrap();
        let mut gfs = GameFs::new(dir.join("game"));
        gfs.set_cache_dir(dir.join("user"));
        let fs = Rc::new(RefCell::new(gfs));
        let mut mgr = LuaManager::init(fs).unwrap();
        let call = |mgr: &mut LuaManager, g: &str, args: &[Value]| {
            let f = mgr.thread.globals().get_str(g);
            mgr.thread
                .call(&f, args)
                .unwrap()
                .into_iter()
                .next()
                .unwrap_or_default()
        };
        let m = call(&mut mgr, "getModInfoByID", &[Value::str("M")]);
        assert!(m.raw_equals(&call(&mut mgr, "getModInfoByID", &[Value::str("M")])));
        let meta = mgr
            .thread
            .class_metatables
            .get_str(MOD)
            .as_table()
            .unwrap()
            .get_str("__index");
        let get_name = meta.as_table().unwrap().get_str("getName");
        assert_eq!(
            mgr.thread
                .call(&get_name, std::slice::from_ref(&m))
                .unwrap()[0]
                .as_str(),
            Some("My Mod")
        );
        assert!(!call(&mut mgr, "isModActive", &[m]).truthy());
        assert!(call(&mut mgr, "getModInfoByID", &[Value::str("Nope")]).is_nil());
        assert_eq!(
            call(&mut mgr, "getModDirectoryTable", &[])
                .as_table()
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
