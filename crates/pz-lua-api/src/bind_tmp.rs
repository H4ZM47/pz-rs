//! LOCAL ONLY: temporary stand-ins to explore the menu boot. Not committed.
use crate::expose::*;
use pz_kahlua::Thread;
pub fn register(thread: &mut Thread) {
    let b = expose_enum(thread, "zombie.debug.DebugType", &["General", "Lua", "Mod", "Sound", "ISUI", "Translation", "Script", "Input", "Animation"]);
    for m in ["debugln", "println", "print", "warn", "error", "noise", "trace", "debugOnceln", "warnOnce"] {
        b.method(m, |_| Ok(0));
    }
    b.method("isEnabled", |c| Ok(c.push(false)));
}
