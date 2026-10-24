//! Lua bindings for `pz-text`: the `getText` family of globals,
//! `Translator` statics, `Language`, the `UIFont` enum and
//! `TextManager.instance`.
//!
//! The engine installs the shared translator, text manager and the draw
//! list text is recorded into with [`install`]; until then the globals
//! behave as with empty tables (keys come back unchanged, sizes are 0).

use crate::bind_java_util::{list_value, JavaList, JavaMap};
use crate::expose::{
    bool_arg, enum_value, expose_class, expose_enum, global, int_arg, num_arg, str_arg, with_this,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_render::{DrawList, Rgba};
use pz_text::jfmt::JavaArg;
use pz_text::translator::Language;
use pz_text::{TextManager, Translator, UIFont};
use pz_ui::{SpriteRenderer, TextAlign, TextRenderer, UiDrawCmd};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Engine state the bindings act on.
#[derive(Clone)]
pub struct TextState {
    pub translator: Rc<RefCell<Translator>>,
    pub text_manager: Rc<RefCell<TextManager>>,
    /// Where `TextManager.DrawString*` records glyph quads.
    pub draw: Rc<RefCell<DrawList>>,
    /// `ZomboidFileSystem.getString(path) != path`, for
    /// `isUsingNonEnglishFonts`.
    pub known_file: Rc<dyn Fn(&str) -> bool>,
}

thread_local! {
    static STATE: RefCell<Option<TextState>> = const { RefCell::new(None) };
    static LANGUAGES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

/// Install the translator and text manager the bindings use, and make the
/// text manager the renderer `UIElement` draws and measures text with.
pub fn install(state: TextState) {
    let renderer: Rc<dyn TextRenderer> = Rc::new(UiTextRenderer(state.text_manager.clone()));
    pz_ui::UIManager::set_text_renderer(Some(renderer));
    STATE.with(|s| *s.borrow_mut() = Some(state));
}

/// `TextManager.instance` as `UIElement` sees it.
struct UiTextRenderer(Rc<RefCell<TextManager>>);

impl TextRenderer for UiTextRenderer {
    fn measure_x(&self, font: Option<UIFont>, s: &str) -> i32 {
        self.0.borrow().measure_string_x(font, Some(s))
    }

    fn measure_y(&self, font: Option<UIFont>, s: &str) -> i32 {
        self.0.borrow().measure_string_y(font, Some(s))
    }

    fn font_height(&self, font: Option<UIFont>) -> i32 {
        self.0.borrow().get_font_height(font)
    }

    fn draw(
        &self,
        out: &mut SpriteRenderer,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        zoom: f32,
        text: &str,
        color: Rgba,
        align: TextAlign,
        _trimmed: bool,
    ) {
        // DrawStringUntrimmed draws exactly like DrawString in the original.
        let tm = self.0.borrow();
        let mut dl = DrawList::default();
        let (r, g, b, a) = (
            color.0 as f64,
            color.1 as f64,
            color.2 as f64,
            color.3 as f64,
        );
        match align {
            // pz-ui passes zoom 1.0 for the overloads without a zoom, which
            // draw at integer positions; only a real zoom takes the scaled path.
            TextAlign::Left if zoom != 1.0 => {
                tm.draw_string_zoom(&mut dl, font, x, y, zoom as f64, text, r, g, b, a)
            }
            TextAlign::Left => tm.draw_string(&mut dl, font, x, y, text, r, g, b, a),
            TextAlign::Centre => tm.draw_string_centre(&mut dl, font, x, y, text, r, g, b, a),
            TextAlign::Right => tm.draw_string_right(&mut dl, font, x, y, text, r, g, b, a),
        }
        for cmd in dl.cmds {
            out.push(UiDrawCmd::Render(cmd));
        }
    }
}

fn state() -> Option<TextState> {
    STATE.with(|s| s.borrow().clone())
}

const UIFONT: &str = "zombie.ui.UIFont";
const LANGUAGE: &str = "zombie.core.Language";
const TEXT_MANAGER: &str = "zombie.ui.TextManager";

/// Kahlua's conversion of Lua values to the `Object...` varargs.
fn java_arg(v: &Value) -> JavaArg {
    match v {
        Value::Nil => JavaArg::Null,
        Value::Bool(b) => JavaArg::Bool(*b),
        Value::Num(n) => JavaArg::Double(*n),
        Value::Str(s) => JavaArg::Str(s.to_string()),
        other => JavaArg::Object {
            to_string: pz_kahlua::exec::java_value_of(other),
            hash_code: other.ptr_id() as i32,
        },
    }
}

fn varargs(c: &NativeCall<'_>, from: usize) -> Vec<JavaArg> {
    (from..c.nargs()).map(|i| java_arg(&c.arg(i))).collect()
}

/// A `UIFont` argument (`null` when not a UIFont constant).
fn font_arg(c: &NativeCall<'_>, i: usize) -> Option<UIFont> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<crate::expose::JavaEnum>()
            .filter(|e| e.class == UIFONT)
            .map(|e| UIFont::ALL[e.ordinal as usize]),
        _ => None,
    }
}

fn font_value(f: UIFont) -> Value {
    enum_value(UIFONT, f.name())
}

fn opt_str(v: Option<String>) -> Value {
    v.map_or(Value::Nil, Value::from)
}

/// `zombie.core.Language` instance.
struct LanguageObj(Rc<Language>);

impl LuaObject for LanguageObj {
    fn class_name(&self) -> &str {
        LANGUAGE
    }
    fn to_lua_string(&self) -> String {
        self.0.name.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One Lua object per `Language`, so `==` compares identity like Java.
fn language_value(l: Rc<Language>) -> Value {
    let key = Rc::as_ptr(&l) as usize;
    LANGUAGES.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_insert_with(|| Value::Object(Rc::new(LanguageObj(l))))
            .clone()
    })
}

/// `TextManager.instance`.
struct TextManagerObj;

impl LuaObject for TextManagerObj {
    fn class_name(&self) -> &str {
        TEXT_MANAGER
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn with_translator<R>(f: impl FnOnce(&Translator) -> R) -> Option<R> {
    state().map(|s| f(&s.translator.borrow()))
}

fn with_translator_mut<R>(f: impl FnOnce(&mut Translator) -> R) -> Option<R> {
    state().map(|s| f(&mut s.translator.borrow_mut()))
}

fn with_tm<R>(f: impl FnOnce(&TextManager) -> R) -> Option<R> {
    state().map(|s| f(&s.text_manager.borrow()))
}

fn key(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

fn get_text(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let k = key(c, 0);
    let args = varargs(c, 1);
    let r = with_translator(|t| t.get_text(&k, &args)).unwrap_or(k);
    Ok(c.push(r))
}

fn get_text_or_null(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let k = key(c, 0);
    let args = varargs(c, 1);
    let r = with_translator(|t| t.get_text_or_null(&k, &args)).flatten();
    Ok(c.push(opt_str(r)))
}

fn get_text_media_en(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let k = key(c, 0);
    let r = with_translator(|t| t.get_text_media_en(&k)).unwrap_or(k);
    Ok(c.push(r))
}

fn get_display_item_name(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let k = key(c, 0);
    let r = with_translator(|t| t.get_display_item_name(&k)).unwrap_or(k);
    Ok(c.push(r))
}

/// `ScriptManager` is not ported yet, so script items are not consulted
/// and untranslated names fall back to the full type.
fn get_item_name_from_full_type(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let k = key(c, 0);
    if !k.contains('.') {
        return Err(LuaError::Runtime {
            message: "fullType must contain \".\" i.e. module.type".into(),
            class: "java.lang.IllegalArgumentException",
        });
    }
    let r = with_translator_mut(|t| t.get_item_name_from_full_type(&k, |_| None)).unwrap_or(k);
    Ok(c.push(r))
}

pub fn register(thread: &mut Thread) {
    // LuaManager.GlobalObject
    global(thread, "getText", get_text);
    global(thread, "getTextOrNull", get_text_or_null);
    global(thread, "getItemText", get_display_item_name);
    global(thread, "getTextMediaEN", get_text_media_en);
    global(
        thread,
        "getItemNameFromFullType",
        get_item_name_from_full_type,
    );
    global(thread, "getTextManager", |c| {
        Ok(c.push(Value::Object(Rc::new(TextManagerObj))))
    });

    let font = expose_enum(thread, UIFONT, &UIFont::ALL.map(UIFont::name));
    font.static_fn("FromString", |c| {
        let r = str_arg(c, 0)
            .and_then(|s| UIFont::from_string(&s))
            .map_or(Value::Nil, font_value);
        Ok(c.push(r))
    });

    let lang = expose_class(thread, LANGUAGE, None);
    lang.method("name", |c| {
        let v = with_this::<LanguageObj, _>(c, |l| l.0.name.clone())?;
        Ok(c.push(v))
    });
    lang.method("toString", |c| {
        let v = with_this::<LanguageObj, _>(c, |l| l.0.name.clone())?;
        Ok(c.push(v))
    });
    lang.method("text", |c| {
        let v = with_this::<LanguageObj, _>(c, |l| l.0.text.clone())?;
        Ok(c.push(v))
    });
    lang.method("base", |c| {
        let v = with_this::<LanguageObj, _>(c, |l| l.0.base.clone())?;
        Ok(c.push(opt_str(v)))
    });
    lang.method("isAzerty", |c| {
        let v = with_this::<LanguageObj, _>(c, |l| l.0.azerty)?;
        Ok(c.push(v))
    });

    register_translator(thread);
    register_text_manager(thread);
}

fn register_translator(thread: &mut Thread) {
    let t = expose_class(thread, "zombie.core.Translator", None);
    t.field("debug", false);
    t.static_fn("getText", get_text);
    t.static_fn("getTextOrNull", get_text_or_null);
    t.static_fn("getTextMediaEN", get_text_media_en);
    t.static_fn("getDisplayItemName", get_display_item_name);
    t.static_fn("getItemNameFromFullType", get_item_name_from_full_type);
    t.static_fn("loadFiles", |_| {
        if let Some(Err(e)) = with_translator_mut(|t| t.load_files()) {
            return Err(LuaError::Runtime {
                message: e.to_string(),
                class: "java.lang.RuntimeException",
            });
        }
        Ok(0)
    });
    t.static_fn("getLanguage", |c| {
        let r = with_translator(|t| t.get_language()).map_or(Value::Nil, language_value);
        Ok(c.push(r))
    });
    t.static_fn("getDefaultLanguage", |c| {
        let r = with_translator(|t| t.default_language()).map_or(Value::Nil, language_value);
        Ok(c.push(r))
    });
    t.static_fn("setLanguage", |c| {
        let arg = c.arg(0);
        let lang = match &arg {
            Value::Num(_) => {
                let id = int_arg(c, 0);
                with_translator(|t| t.languages.by_index(id))
                    .flatten()
                    .map(Some)
                    .ok_or_else(|| LuaError::Runtime {
                        message: format!("Index {id} out of bounds"),
                        class: "java.lang.IndexOutOfBoundsException",
                    })?
            }
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LanguageObj>()
                .map(|l| l.0.clone()),
            _ => None,
        };
        with_translator_mut(|t| t.set_language(lang));
        Ok(0)
    });
    t.static_fn("getAvailableLanguage", |c| {
        // A fresh list per call; the original caches one ArrayList.
        let langs =
            with_translator_mut(|t| t.get_available_language().to_vec()).unwrap_or_default();
        Ok(c.push(list_value(langs.into_iter().map(language_value).collect())))
    });
    t.static_fn("getAzertyMap", |c| {
        let v = with_translator_mut(|t| t.get_azerty_map().to_vec()).unwrap_or_default();
        Ok(c.push(list_value(v.into_iter().map(Value::from).collect())))
    });
    t.static_fn("getUI", |c| {
        // A snapshot HashMap. Keys go in sorted, so iteration matches Java's
        // bucket order; keys sharing a bucket may differ in relative order.
        let map = JavaMap::hash_map();
        if let Some(mut entries) = with_translator(|t| {
            t.get_ui()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        }) {
            entries.sort();
            for (k, v) in entries {
                map.put(Value::from(k), Value::from(v));
            }
        }
        Ok(c.push(Value::Object(Rc::new(map))))
    });
    t.static_fn("getRecipeName", |c| {
        let k = key(c, 0);
        let r = with_translator_mut(|t| t.get_recipe_name(&k)).unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("getRecipeGroupName", |c| {
        let k = key(c, 0);
        let r = with_translator_mut(|t| t.get_recipe_group_name(&k)).unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("getMoveableDisplayName", |c| {
        // The IsoObject overload needs the world port; strings only for now.
        let k = key(c, 0);
        let r = with_translator(|t| t.get_moveable_display_name(&k)).unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("getMoveableDisplayNameOrNull", |c| {
        let k = key(c, 0);
        let r = with_translator(|t| t.get_moveable_display_name_or_null(&k)).flatten();
        Ok(c.push(opt_str(r)))
    });
    t.static_fn("getAttributeText", |c| {
        let k = key(c, 0);
        let r = with_translator(|t| t.get_attribute_text(&k, false))
            .flatten()
            .unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("getAttributeTextOrNull", |c| {
        let k = key(c, 0);
        let r = with_translator(|t| t.get_attribute_text(&k, true)).flatten();
        Ok(c.push(opt_str(r)))
    });
    t.static_fn("getFluidText", |c| {
        let k = key(c, 0);
        let r = with_translator(|t| t.get_fluid_text(&k)).unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("getEntityText", |c| {
        let k = key(c, 0);
        let r = with_translator(|t| t.get_entity_text(&k)).unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("getMapLabelText", |c| {
        let k = key(c, 0);
        let r = with_translator(|t| t.get_map_label_text(&k)).unwrap_or(k);
        Ok(c.push(r))
    });
    t.static_fn("setDefaultItemEvolvedRecipeName", |c| {
        let (k, e) = (key(c, 0), key(c, 1));
        with_translator_mut(|t| t.set_default_item_evolved_recipe_name(&k, &e));
        Ok(0)
    });
    t.static_fn("getItemEvolvedRecipeName", |c| {
        let k = key(c, 0);
        let r = with_translator_mut(|t| t.get_item_evolved_recipe_name(&k, |_| None)).unwrap_or(k);
        Ok(c.push(r))
    });
    for (name, f) in [
        (
            "debugItemNames",
            Translator::debug_item_names as fn(&mut Translator),
        ),
        ("debugRecipeNames", Translator::debug_recipe_names),
        (
            "debugRecipeGroupNames",
            Translator::debug_recipe_group_names,
        ),
        (
            "debugItemEvolvedRecipeNames",
            Translator::debug_item_evolved_recipe_names,
        ),
        (
            "debugMultiStageBuildNames",
            Translator::debug_multi_stage_build_names,
        ),
    ] {
        t.static_fn(name, move |_| {
            with_translator_mut(f);
            Ok(0)
        });
    }
}

fn rgba_args(c: &NativeCall<'_>, from: usize) -> [f64; 4] {
    [
        num_arg(c, from),
        num_arg(c, from + 1),
        num_arg(c, from + 2),
        num_arg(c, from + 3),
    ]
}

fn draw(f: impl FnOnce(&TextManager, &mut DrawList)) {
    if let Some(s) = state() {
        f(&s.text_manager.borrow(), &mut s.draw.borrow_mut());
    }
}

fn register_text_manager(thread: &mut Thread) {
    let tm = expose_class(thread, TEXT_MANAGER, None);
    tm.field("instance", Value::Object(Rc::new(TextManagerObj)));
    // Instance methods: argument 0 is `self`.
    tm.method("MeasureStringX", |c| {
        let (f, s) = (font_arg(c, 1), str_arg(c, 2));
        let r = with_tm(|tm| tm.measure_string_x(f, s.as_deref())).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("MeasureStringY", |c| {
        let (f, s) = (font_arg(c, 1), str_arg(c, 2));
        let (actual, offset) = if c.nargs() >= 5 {
            (bool_arg(c, 3), bool_arg(c, 4))
        } else {
            (false, false)
        };
        let r = with_tm(|tm| tm.measure_string_y_ex(f, s.as_deref(), actual, offset)).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("MeasureStringYReal", |c| {
        let (f, s) = (font_arg(c, 1), str_arg(c, 2));
        let r = with_tm(|tm| tm.measure_string_y_real(f, s.as_deref())).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("MeasureStringYOffset", |c| {
        let (f, s) = (font_arg(c, 1), str_arg(c, 2));
        let r = with_tm(|tm| tm.measure_string_y_offset(f, s.as_deref())).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("CentreStringYOffset", |c| {
        let (f, s) = (font_arg(c, 1), str_arg(c, 2));
        let r = with_tm(|tm| tm.centre_string_y_offset(f, s.as_deref())).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("getFontHeight", |c| {
        let f = font_arg(c, 1);
        let r = with_tm(|tm| tm.get_font_height(f)).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("MeasureFont", |c| {
        let f = font_arg(c, 1);
        let r = with_tm(|tm| tm.measure_font(f)).unwrap_or(0);
        Ok(c.push(r))
    });
    tm.method("isSdf", |c| {
        let f = font_arg(c, 1);
        let r = with_tm(|tm| tm.is_sdf(f)).unwrap_or(false);
        Ok(c.push(r))
    });
    tm.method("isAnyFontLoading", |c| {
        let r = with_tm(TextManager::is_any_font_loading).unwrap_or(false);
        Ok(c.push(r))
    });
    tm.method("isUsingNonEnglishFonts", |c| {
        let r = state().is_some_and(|s| {
            let lang = s.translator.borrow().get_language().name.clone();
            s.text_manager
                .borrow()
                .is_using_non_english_fonts(&lang, &*s.known_file)
        });
        Ok(c.push(r))
    });
    tm.method("getAllFonts", |c| {
        // Clears `result`, fills it with every UIFont sorted by name ignoring
        // case, and returns it.
        let result = c.arg(1);
        let Value::Object(o) = &result else {
            return Err(LuaError::Runtime {
                message: "null".into(),
                class: "java.lang.NullPointerException",
            });
        };
        let Some(list) = o.as_any().downcast_ref::<JavaList>() else {
            return Err(LuaError::Runtime {
                message: format!("{} cannot be cast to java.util.ArrayList", o.class_name()),
                class: "java.lang.ClassCastException",
            });
        };
        let fonts = with_tm(TextManager::get_all_fonts)
            .unwrap_or_else(|| TextManager::default().get_all_fonts());
        *list.items.borrow_mut() = fonts.into_iter().map(font_value).collect();
        Ok(c.push(result.clone()))
    });
    tm.method("getCurrentCodeFont", |c| {
        let r = with_tm(|tm| tm.current_code_font).unwrap_or(UIFont::CodeSmall);
        Ok(c.push(font_value(r)))
    });
    tm.method("WrapText", |c| {
        let (f, s, w) = (font_arg(c, 1), key(c, 2), int_arg(c, 3));
        let r = if c.nargs() >= 6 {
            let (n, suffix) = (int_arg(c, 4), key(c, 5));
            with_tm(|tm| tm.wrap_text_ex(f, &s, w, n, &suffix))
        } else {
            with_tm(|tm| tm.wrap_text(f, &s, w))
        };
        Ok(c.push(r.unwrap_or(s)))
    });
    tm.method("DrawString", |c| {
        // (x, y, str), (x, y, str, r, g, b, a), (font, x, y, str, r, g, b, a)
        // or (font, x, y, zoom, str, r, g, b, a).
        let n = c.nargs() - 1;
        match n {
            3 => {
                let (x, y, s) = (num_arg(c, 1), num_arg(c, 2), key(c, 3));
                draw(|tm, dl| tm.draw_string_default(dl, x, y, &s, 1.0, 1.0, 1.0, 1.0));
            }
            7 => {
                let (x, y, s, [r, g, b, a]) =
                    (num_arg(c, 1), num_arg(c, 2), key(c, 3), rgba_args(c, 4));
                draw(|tm, dl| tm.draw_string_default(dl, x, y, &s, r, g, b, a));
            }
            9 => {
                let (f, x, y, z, s, [r, g, b, a]) = (
                    font_arg(c, 1),
                    num_arg(c, 2),
                    num_arg(c, 3),
                    num_arg(c, 4),
                    key(c, 5),
                    rgba_args(c, 6),
                );
                draw(|tm, dl| tm.draw_string_zoom(dl, f, x, y, z, &s, r, g, b, a));
            }
            _ => {
                let (f, x, y, s, [r, g, b, a]) = (
                    font_arg(c, 1),
                    num_arg(c, 2),
                    num_arg(c, 3),
                    key(c, 4),
                    rgba_args(c, 5),
                );
                draw(|tm, dl| tm.draw_string(dl, f, x, y, &s, r, g, b, a));
            }
        }
        Ok(0)
    });
    tm.method("DrawStringUntrimmed", |c| {
        let (f, x, y, s, [r, g, b, a]) = (
            font_arg(c, 1),
            num_arg(c, 2),
            num_arg(c, 3),
            key(c, 4),
            rgba_args(c, 5),
        );
        draw(|tm, dl| tm.draw_string(dl, f, x, y, &s, r, g, b, a));
        Ok(0)
    });
    tm.method("DrawStringCentre", |c| {
        // (x, y, str, r, g, b, a) uses the Small font.
        let (f, base) = if c.nargs() - 1 == 7 {
            (None, 1)
        } else {
            (font_arg(c, 1), 2)
        };
        let (x, y, s, [r, g, b, a]) = (
            num_arg(c, base),
            num_arg(c, base + 1),
            key(c, base + 2),
            rgba_args(c, base + 3),
        );
        draw(|tm, dl| tm.draw_string_centre(dl, f, x, y, &s, r, g, b, a));
        Ok(0)
    });
    tm.method("DrawStringRight", |c| {
        let (f, base) = if c.nargs() - 1 == 7 {
            (None, 1)
        } else {
            (font_arg(c, 1), 2)
        };
        let (x, y, s, [r, g, b, a]) = (
            num_arg(c, base),
            num_arg(c, base + 1),
            key(c, base + 2),
            rgba_args(c, base + 3),
        );
        draw(|tm, dl| tm.draw_string_right(dl, f, x, y, &s, r, g, b, a));
        Ok(0)
    });
    tm.method("DrawStringCentreDefered", |c| {
        let (f, x, y, s, [r, g, b, a]) = (
            font_arg(c, 1),
            num_arg(c, 2),
            num_arg(c, 3),
            key(c, 4),
            rgba_args(c, 5),
        );
        if let Some(st) = state() {
            st.text_manager
                .borrow_mut()
                .draw_string_centre_defered(f, x, y, &s, r, g, b, a);
        }
        Ok(0)
    });
    tm.method("DrawTextFromGameWorld", |_| {
        if let Some(st) = state() {
            st.text_manager
                .borrow_mut()
                .draw_text_from_game_world(&mut st.draw.borrow_mut());
        }
        Ok(0)
    });
    // Empty in the original.
    tm.method("DrawStringBBcode", |_| Ok(0));
    tm.method("DrawTextObject", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_text::translator::{format_fixer, Table, TranslatorEnv};

    #[test]
    fn get_text_and_measure_from_lua() {
        let env = pz_kahlua::TableRef::new();
        let mut thread = Thread::new(env.clone());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        register(&mut thread);
        let mut t = Translator::new(TranslatorEnv::default());
        t.table_mut(Table::Ui)
            .insert("UI_hello".into(), format_fixer("Hi %1, %2 items"));
        let mut tm = TextManager::default();
        let fnt = "info\ncommon\nchar id=97 x=0 y=0 width=5 height=6 xoffset=0 yoffset=2 xadvance=6 page=0\n";
        tm.set_font(UIFont::Small, pz_text::AngelCodeFont::parse(fnt).unwrap());
        install(TextState {
            translator: Rc::new(RefCell::new(t)),
            text_manager: Rc::new(RefCell::new(tm)),
            draw: Rc::new(RefCell::new(DrawList::default())),
            known_file: Rc::new(|_| false),
        });
        let src = "return getText('UI_hello', 'Bob', 3), getTextOrNull('UI_nope'), \
                   getTextManager():MeasureStringX(UIFont.Medium, 'aaa'), UIFont.FromString('Title') == UIFont.Title, \
                   Translator.getLanguage():name(), Translator.getAzertyMap():get(0), \
                   getTextManager():getAllFonts(ArrayList.new()):get(0):name(), Translator.getUI():get('UI_hello'), \
                   Translator.getAvailableLanguage():size()";
        let proto = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), env));
        let r = thread.call(&f, &[]).unwrap();
        assert_eq!(r[0].as_str().unwrap(), "Hi Bob, 3 items");
        assert!(r[1].is_nil());
        assert!(r[2].raw_equals(&Value::Num(17.0)));
        assert!(r[3].raw_equals(&Value::Bool(true)));
        assert_eq!(r[4].as_str().unwrap(), "EN");
        assert_eq!(r[5].as_str().unwrap(), "FR");
        assert_eq!(r[6].as_str().unwrap(), "AutoNormLarge");
        assert_eq!(r[7].as_str().unwrap(), "Hi %1$s, %2$s items");
        assert!(r[8].raw_equals(&Value::Num(1.0)));

        // UIElement text goes through the installed TextManager.
        pz_ui::with_ui(|m| m.sprite.clear());
        assert_eq!(pz_ui::draw::measure_string_x(Some(UIFont::Small), "aa"), 11);
        pz_ui::draw::draw_string(
            Some(UIFont::Small),
            3.5,
            4.0,
            1.0,
            "aa",
            Rgba(1.0, 1.0, 1.0, 1.0),
            TextAlign::Left,
            true,
        );
        let quads = pz_ui::with_ui(|m| m.sprite.cmds.len());
        assert_eq!(quads, 2);
    }
}
