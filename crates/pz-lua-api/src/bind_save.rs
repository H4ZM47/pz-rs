//! Save-game related `GlobalObject` globals. Only `getLatestSave` so far;
//! the save-games workstream owns the rest of this module.

use crate::expose::global;
use crate::LuaManager;
use pz_kahlua::TableRef;
use std::io::BufRead;

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    // Lines of <cache>/latestSave.ini, 1-based; empty when missing.
    global(&mut mgr.thread, "getLatestSave", move |c| {
        let t = TableRef::new();
        let path = fs.borrow().cache_dir().join("latestSave.ini");
        if let Ok(f) = std::fs::File::open(path) {
            for (i, line) in std::io::BufReader::new(f).lines().enumerate() {
                let Ok(line) = line else { break };
                t.set_int(i as i64 + 1, line);
            }
        }
        Ok(c.push(t))
    });
}
