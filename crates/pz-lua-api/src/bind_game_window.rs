//! `zombie.GameWindow`'s Lua-visible statics: the `doRenderEvent` flag the
//! frame loop reads, `isIngameState`, `DoLoadingText`, `getUpdateTime`,
//! `getCoopServerHome`, and the public static fields as they stand when
//! the class is exposed; and `zombie.gameStates.MainScreenState`'s
//! statics.

use std::cell::Cell;

use pz_kahlua::vm::Thread;

use crate::expose::{bool_arg, expose_class, str_arg};

pub const GAME_WINDOW_CLASS: &str = "zombie.GameWindow";
pub const MAIN_SCREEN_STATE_CLASS: &str = "zombie.gameStates.MainScreenState";

/// The Lua handle of `MainScreenState.instance`.
pub struct MainScreenStateObject;

impl pz_kahlua::value::LuaObject for MainScreenStateObject {
    fn class_name(&self) -> &str {
        MAIN_SCREEN_STATE_CLASS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

thread_local! {
    static DO_RENDER_EVENT: Cell<bool> = const { Cell::new(false) };
    static UPDATE_TIME: Cell<i64> = const { Cell::new(0) };
    static MAIN_SCREEN: std::cell::RefCell<Option<std::rc::Rc<MainScreenStateObject>>> = const { std::cell::RefCell::new(None) };
}

/// `MainScreenState.instance = this` (the end of `enter`, after
/// `OnMainMenuEnter`); the original never clears it.
pub fn set_main_screen_instance() {
    MAIN_SCREEN.with(|m| {
        m.borrow_mut().get_or_insert_with(|| std::rc::Rc::new(MainScreenStateObject));
    });
}

/// `GameWindow.doRenderEvent`: whether `GameWindow.render` fires
/// `OnRenderTick` after the frame.
pub fn render_event_enabled() -> bool {
    DO_RENDER_EVENT.with(Cell::get)
}

/// `GameWindow.updateTime` (the logic step's length, as `getUpdateTime`
/// returns it).
pub fn set_update_time(t: i64) {
    UPDATE_TIME.with(|u| u.set(t));
}

/// `GameWindow.version`, the static initializer's string in the jar.
fn version_from_jar(jar: &pz_classfile::Jar) -> Option<String> {
    let cf = pz_classfile::load_class(jar, "zombie/GameWindow.class").ok()?;
    pz_classfile::static_init::static_inits(&cf)
        .ok()?
        .into_iter()
        .find(|i| i.class == cf.this_class && i.field == "version")
        .and_then(|i| i.value.as_str().map(str::to_string))
}

pub fn register(thread: &mut Thread, jar: Option<&pz_classfile::Jar>, cache_dir: &std::path::Path) {
    let b = expose_class(thread, GAME_WINDOW_CLASS, None);
    b.static_fn("doRenderEvent", |c| {
        let v = bool_arg(c, 0);
        DO_RENDER_EVENT.with(|d| d.set(v));
        Ok(0)
    });
    b.static_fn("isIngameState", |c| Ok(c.push(crate::bind_misc::is_ingame_state())));
    // `DoLoadingText(text)`: `DebugLog.log(text)`.
    b.static_fn("DoLoadingText", |c| {
        let s = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_else(|| "null".into());
        pz_core::debug::log(&s);
        Ok(0)
    });
    b.static_fn("getUpdateTime", |c| Ok(c.push(UPDATE_TIME.with(Cell::get) as f64)));
    // `new File(getCacheDir()).getParent()`.
    let parent = cache_dir.parent().map(|p| p.to_string_lossy().into_owned());
    b.static_fn("getCoopServerHome", move |c| Ok(c.push(parent.as_deref().map(pz_kahlua::value::Value::str).unwrap_or_default())));
    // TODO(port): ReadString/WriteString/ReadUUID/WriteUUID/readInt/readLong
    // (need Lua stream and ByteBuffer objects), getEncodedBytesUTF, save,
    // render, LoadTexturePack, setTexturePackLookup, initFonts, InitDisplay,
    // InitGameThread, doEpilepsyWarningText, uncaughtException, and the
    // object fields (states, GameInput, fileSystem, assetManagers,
    // texturePacks, texturePackTextures, averageFPS).
    if let Some(v) = jar.and_then(version_from_jar) {
        b.field("version", pz_kahlua::value::Value::str(&v));
    }
    for name in [
        "DEBUG_SAVE",
        "okToSaveOnExit",
        "serverDisconnected",
        "loadedAsClient",
        "drawReloadingLua",
        "closeRequested",
        "luaDebuggerKeyDown",
        "gameThreadExited",
    ] {
        b.field(name, false);
    }
    let m = expose_class(thread, MAIN_SCREEN_STATE_CLASS, None);
    m.static_fn("getInstance", |c| {
        let v = MAIN_SCREEN.with(|m| m.borrow().clone());
        Ok(c.push(v.map(|o| pz_kahlua::value::Value::Object(o)).unwrap_or_default()))
    });
    // TODO(port): DrawTexture, loadIcons, preloadBackgroundTextures,
    // totalScale.
    if let Some(cf) = jar.and_then(|j| pz_classfile::load_class(j, "zombie/gameStates/MainScreenState.class").ok()) {
        for (name, v) in cf.static_final_strings() {
            if name == "VERSION" {
                m.field(name, pz_kahlua::value::Value::str(v));
            }
        }
    }
    // `isSteamRunningOnSteamDeck()`: false outside Steam mode, and there is
    // no Steam integration.
    crate::expose::global(thread, "isSteamRunningOnSteamDeck", |c| Ok(c.push(false)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn do_render_event_sets_the_flag() {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t, None, std::path::Path::new("/tmp/x/Zomboid"));
        let f = pz_kahlua::stdlib::load_string(
            "GameWindow.doRenderEvent(true); return GameWindow.getCoopServerHome(), GameWindow.okToSaveOnExit",
            Some("t"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?}\n{tr}")).unwrap();
        assert_eq!(format!("{r:?}"), format!("{:?}", [pz_kahlua::value::Value::str("/tmp/x"), pz_kahlua::value::Value::Bool(false)]));
        assert!(render_event_enabled());
    }
}
