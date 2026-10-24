//! File-system globals of `LuaManager.GlobalObject`.
//!
//! TODO: the mod globals (`getModInfoByID`, `getModInfo`, `isModActive`,
//! `getActivatedMods`, `getModDirectoryTable`) need mutable access to
//! `GameFs` (mod.info reading is cached lazily, as in `ChooseGameInfo`) and
//! a `java.util.ArrayList` exposure; they land once `LuaManager` holds the
//! file system mutably.

use pz_kahlua::Value;

use crate::expose::global;
use crate::LuaManager;

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    global(&mut mgr.thread, "getMyDocumentFolder", move |c| {
        // `Core.getMyDocumentFolder()` is `getCacheDir()`.
        let dir = fs.cache_dir().to_string_lossy().into_owned();
        Ok(c.push(Value::str(&dir)))
    });
    global(&mut mgr.thread, "getFileSeparator", |c| Ok(c.push(Value::str(std::path::MAIN_SEPARATOR_STR))));
}
