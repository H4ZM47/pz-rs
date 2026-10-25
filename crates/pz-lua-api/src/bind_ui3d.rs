//! `zombie.ui.UI3DModel`: a `UIElement` subclass (through the pz-ui
//! extension slot) showing an `AnimatedModel` — the character avatar of
//! character creation, the character screen and the animal UIs.
//!
//! The element keeps `UI3DModel`'s own fields (direction, random `Ext`
//! animations, zoom, offsets) and the Lua-visible state of its
//! `AnimatedModel` ([`AnimatedModel`]: anim set, state, variables, the
//! visual it was given, pending action events). Drawing needs the model
//! assets and the GPU, so the engine installs a [`UIModelRenderer`]
//! ([`install_renderer`]) that runs `AnimatedModel.update` and the
//! `Drawer` (`renderMain` + `DoRender`) for one model and returns its draw
//! commands; they go into the UI draw list right after the element's own
//! `render()`, as `SpriteRenderer.drawGeneric` queues the drawer.

use crate::bind_characters::{desc_arg, LuaSurvivorDesc};
use crate::bind_items::LuaInventoryItem;
use crate::bind_iso_enums::ISO_DIRECTIONS;
use crate::bind_ui::{element_of, UIELEMENT};
use crate::expose::{bool_arg, enum_value, expose_class, num_arg, str_arg, JavaEnum};
use pz_anim::{AnimationVariableHandle, AnimationVariableSource, MapVariableSource, VariableValue};
use pz_inventory::visual::{HumanVisual, ItemVisual};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_render::DrawCmd;
use pz_ui::{with_ui, UIElement, UIElementExt, UiDrawCmd};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const UI3DMODEL: &str = "zombie.ui.UI3DModel";
const SUPERS: &[&str] = &[UIELEMENT, "zombie.core.skinnedmodel.population.IClothingItemListener"];

/// `IsoDirections` constants in ordinal order with their `(dx, dy)`.
const DIRECTIONS: [(&str, i32, i32); 8] =
    [("N", 0, -1), ("NW", -1, -1), ("W", -1, 0), ("SW", -1, 1), ("S", 0, 1), ("SE", 1, 1), ("E", 1, 0), ("NE", 1, -1)];

/// `IsoDirections.ToVector()`: the normalised `(dx, dy)` (`Vector2.normalize`).
pub fn direction_vector(name: &str) -> Option<[f32; 2]> {
    let &(_, dx, dy) = DIRECTIONS.iter().find(|d| d.0 == name)?;
    let (x, y) = (dx as f32, dy as f32);
    let len = (x * x + y * y).sqrt();
    Some([x / len, y / len])
}

/// The visual an `AnimatedModel` was given (`setModelData`'s copies of the
/// `HumanVisual` and the item visuals).
#[derive(Clone, Debug)]
pub struct ModelVisual {
    pub human: HumanVisual,
    /// `getWornItems().getItemVisuals(...)`, in the worn order.
    pub items: Vec<ItemVisual>,
    pub female: bool,
    pub zombie: bool,
    pub skeleton: bool,
}

/// What Lua sets on a `UI3DModel`'s `AnimatedModel`.
pub struct AnimatedModel {
    pub anim_set_name: String,
    /// `setOutfitName`: dressed at the next update (`initOutfit`).
    pub outfit_name: Option<String>,
    pub female: bool,
    pub zombie: bool,
    pub visual: Option<ModelVisual>,
    /// `setCharacter`'s character object.
    pub character: Option<Value>,
    pub state: Option<String>,
    /// The model's own animation variables.
    pub vars: MapVariableSource,
    pub isometric: bool,
    pub animate: bool,
    /// `setAngle`: the direction vector.
    pub angle: [f32; 2],
    /// `setOffset`.
    pub offset: [f32; 3],
    /// `getActionContext().reportEvent(..)` calls not yet handed to the
    /// action context.
    pub events: Vec<String>,
    /// Bumped whenever `setModelData` would run (a new visual, character,
    /// outfit or anim set, `clothingItemChanged`): the renderer rebuilds.
    pub model_serial: u64,
}

impl Default for AnimatedModel {
    fn default() -> Self {
        AnimatedModel {
            anim_set_name: "player-avatar".into(),
            outfit_name: None,
            female: false,
            zombie: false,
            visual: None,
            character: None,
            state: None,
            vars: MapVariableSource::new(),
            isometric: true,
            animate: true,
            angle: [0.0, 0.0],
            offset: [0.0, -0.45, 0.0],
            events: Vec::new(),
            model_serial: 0,
        }
    }
}

/// `UI3DModel.Drawer`'s call of `DoRender(x, y, w, h, sizeV, angle)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UIModelView {
    /// `absX, Core.height - absY - (int)height, (int)width, (int)height`.
    pub viewport: [i32; 4],
    /// `(isometric ? 22 : 25) - zoom`.
    pub size_v: f32,
    /// `Core.width`, `Core.height`.
    pub screen: (i32, i32),
}

/// Draws `AnimatedModel`s for the UI; installed by the engine.
pub trait UIModelRenderer {
    /// `AnimatedModel.update()` then the drawer (`renderMain`, `DoRender`)
    /// for the model `key`: its draw commands (none while it is not ready).
    fn render(&mut self, key: u64, model: &mut AnimatedModel, view: &UIModelView) -> Vec<DrawCmd>;
    /// The model's element is gone.
    fn release(&mut self, _key: u64) {}
}

thread_local! {
    static RENDERER: RefCell<Option<Box<dyn UIModelRenderer>>> = const { RefCell::new(None) };
    static NEXT_KEY: Cell<u64> = const { Cell::new(1) };
}

/// Install the engine's model renderer (`None` removes it).
pub fn install_renderer(r: Option<Box<dyn UIModelRenderer>>) {
    RENDERER.with(|s| *s.borrow_mut() = r);
}

/// `UI3DModel`'s own fields.
struct UI3DState {
    model: AnimatedModel,
    /// The `IsoDirections` constant (`dir`).
    dir: Option<&'static str>,
    do_ext: bool,
    next_ext: i64,
    zoom: f32,
    y_offset: f32,
    x_offset: f32,
}

/// The `UI3DModel` part of the element.
pub struct UI3DModel {
    key: u64,
    st: RefCell<UI3DState>,
}

impl Drop for UI3DModel {
    fn drop(&mut self) {
        let key = self.key;
        let _ = RENDERER.try_with(|r| {
            if let Ok(mut r) = r.try_borrow_mut() {
                if let Some(r) = r.as_mut() {
                    r.release(key);
                }
            }
        });
    }
}

impl UI3DModel {
    pub fn new() -> Self {
        let key = NEXT_KEY.with(|k| {
            let v = k.get();
            k.set(v + 1);
            v
        });
        UI3DModel {
            key,
            st: RefCell::new(UI3DState {
                model: AnimatedModel::default(),
                dir: Some("E"),
                do_ext: false,
                next_ext: -1,
                zoom: 0.0,
                y_offset: 0.0,
                x_offset: 0.0,
            }),
        }
    }

    /// Runs `f` on the model state.
    pub fn with_model<R>(&self, f: impl FnOnce(&mut AnimatedModel) -> R) -> R {
        f(&mut self.st.borrow_mut().model)
    }

    pub fn zoom(&self) -> f32 {
        self.st.borrow().zoom
    }

    pub fn offsets(&self) -> (f32, f32) {
        let s = self.st.borrow();
        (s.x_offset, s.y_offset)
    }

    pub fn is_do_ext(&self) -> bool {
        self.st.borrow().do_ext
    }

    /// The `Drawer.init` / `Drawer.render` view for an element at absolute
    /// `(abs_x, abs_y)` of `w` x `h` on a `screen`: sets the model offset
    /// and returns the `DoRender` arguments.
    pub fn drawer_view(&self, abs_x: i32, abs_y: i32, w: f64, h: f64, screen: (i32, i32)) -> UIModelView {
        let mut s = self.st.borrow_mut();
        let mut y_off = if s.model.isometric { -0.45 } else { -0.5 };
        if s.y_offset != 0.0 {
            y_off = s.y_offset;
        }
        s.model.offset = [s.x_offset, y_off, 0.0];
        let size = if s.model.isometric { 22.0f32 } else { 25.0 };
        UIModelView {
            viewport: [abs_x, screen.1 - abs_y - h as i32, w as i32, h as i32],
            size_v: size - s.zoom,
            screen,
        }
    }

    /// `render()`'s random `Ext` animation timer at time `now` (ms).
    fn ext_tick(&self, now: i64) {
        let mut s = self.st.borrow_mut();
        if !s.do_ext {
            return;
        }
        if s.next_ext < 0 {
            s.next_ext = now + pz_core::random::next_range(5000, 10000) as i64;
        }
        if s.next_ext < now {
            s.model.events.push("EventDoExt".into());
            let ext = pz_core::random::next_range(0, 6) + 1;
            s.model.vars.set_variable("Ext", VariableValue::Float(ext as f32));
            s.next_ext = -1;
        }
    }
}

impl Default for UI3DModel {
    fn default() -> Self {
        Self::new()
    }
}

impl UIElementExt for UI3DModel {
    fn class_name(&self) -> &'static str {
        UI3DMODEL
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `UI3DModel.render()`.
    fn render(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        el.render_base(t);
        if let Some(p) = el.get_parent() {
            let mdh = p.get_max_draw_height();
            if mdh != -1.0 && mdh <= el.get_y() {
                return;
            }
        }
        self.ext_tick(pz_ui::manager::now_ms());
        let screen = {
            let core = crate::bind_core::shared_core();
            let c = core.borrow();
            (c.get_screen_width(), c.get_screen_height())
        };
        let view = self.drawer_view(
            el.get_absolute_x() as i32,
            el.get_absolute_y() as i32,
            el.get_width(),
            el.get_height(),
            screen,
        );
        let key = self.key;
        let cmds = RENDERER.with(|r| {
            let mut r = r.borrow_mut();
            match r.as_mut() {
                Some(r) => r.render(key, &mut self.st.borrow_mut().model, &view),
                None => Vec::new(),
            }
        });
        if !cmds.is_empty() {
            with_ui(|m| {
                for c in cmds {
                    m.sprite.push(UiDrawCmd::Render(c));
                }
            });
        }
    }
}

// ---- Lua ----

fn ui3d<R>(c: &NativeCall<'_>, f: impl FnOnce(&UI3DModel) -> R) -> LuaResult<R> {
    let e = element_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("Expected UI3DModel"))?;
    let m = e.ext::<UI3DModel>().ok_or_else(|| {
        LuaError::runtime(format!("Expected UI3DModel but got {}", e.class_name()))
    })?;
    Ok(f(m))
}

fn model<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut AnimatedModel) -> R) -> LuaResult<R> {
    ui3d(c, |m| m.with_model(f))
}

fn npe() -> LuaError {
    LuaError::Runtime { message: "null".into(), class: "java.lang.NullPointerException" }
}

/// The item visual a worn item contributes (`InventoryItem.getVisual()`;
/// an item without one yet gets a fresh visual of its clothing item).
fn worn_item_visual(v: &Value) -> Option<ItemVisual> {
    let Value::Object(o) = v else { return None };
    let item = o.as_any().downcast_ref::<LuaInventoryItem>()?.item.borrow();
    if let Some(vis) = &item.visual {
        return Some(vis.clone());
    }
    let name = item.template.clothing_item.clone()?;
    let mut vis = ItemVisual::new(&item.get_full_type());
    vis.clothing_item_name = name;
    vis.base_texture = 0;
    vis.texture_choice = 0;
    Some(vis)
}

/// `setSurvivorDesc`'s copies of the desc's visual and worn items.
pub fn model_visual_of(desc: &LuaSurvivorDesc) -> ModelVisual {
    ModelVisual {
        human: desc.human_visual.clone(),
        items: desc.worn_items.items().iter().filter_map(|w| worn_item_visual(&w.item.0)).collect(),
        female: desc.is_female(),
        zombie: false,
        skeleton: false,
    }
}

fn variable_to_value(v: VariableValue) -> Value {
    match v {
        VariableValue::String(s) => Value::str(&s),
        VariableValue::Float(f) => Value::Num(f as f64),
        VariableValue::Int(i) => Value::Num(i as f64),
        VariableValue::Bool(b) => Value::Bool(b),
    }
}

/// Register `zombie.ui.UI3DModel` (after `UIElement`).
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, UI3DMODEL, Some(UIELEMENT));
    b.ctor(|c| {
        let table = match c.arg(0) {
            Value::Table(t) => Some(t),
            _ => None,
        };
        let e = UIElement::new_ext(table, Box::new(UI3DModel::new()));
        Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
    });
    b.method("setDirection", |c| {
        let dir = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<JavaEnum>().filter(|e| e.class == ISO_DIRECTIONS).map(|e| e.name),
            _ => None,
        };
        ui3d(c, |m| {
            let mut s = m.st.borrow_mut();
            s.dir = dir;
            if let Some(v) = dir.and_then(direction_vector) {
                s.model.angle = v;
            }
        })?;
        Ok(0)
    });
    b.method("getDirection", |c| {
        let dir = ui3d(c, |m| m.st.borrow().dir)?;
        Ok(c.push(dir.map(|d| enum_value(ISO_DIRECTIONS, d)).unwrap_or_default()))
    });
    b.method("setAnimate", |c| {
        let v = bool_arg(c, 1);
        model(c, |m| m.animate = v)?;
        Ok(0)
    });
    b.method("setAnimSetName", |c| {
        let name = str_arg(c, 1).filter(|s| !s.trim().is_empty()).ok_or_else(|| {
            LuaError::Runtime {
                message: format!("invalid AnimSet \"{}\"", str_arg(c, 1).as_deref().unwrap_or("null")),
                class: "java.lang.IllegalArgumentException",
            }
        })?;
        model(c, |m| m.anim_set_name = name.to_string())?;
        Ok(0)
    });
    b.method("setDoRandomExtAnimations", |c| {
        let v = bool_arg(c, 1);
        ui3d(c, |m| m.st.borrow_mut().do_ext = v)?;
        Ok(0)
    });
    b.method("setIsometric", |c| {
        let v = bool_arg(c, 1);
        model(c, |m| m.isometric = v)?;
        Ok(0)
    });
    b.method("setOutfitName", |c| {
        let name = str_arg(c, 1).map(|s| s.to_string());
        let (female, zombie) = (bool_arg(c, 2), bool_arg(c, 3));
        model(c, |m| {
            m.outfit_name = name;
            m.female = female;
            m.zombie = zombie;
        })?;
        Ok(0)
    });
    b.method("setCharacter", |c| {
        let ch = c.arg(1);
        model(c, |m| {
            m.outfit_name = None;
            m.visual = None;
            if !ch.is_nil() {
                m.character = Some(ch);
                m.model_serial += 1;
            }
        })?;
        Ok(0)
    });
    b.method("getCharacter", |c| {
        let ch = model(c, |m| m.character.clone())?;
        Ok(c.push(ch.unwrap_or_default()))
    });
    b.method("setSurvivorDesc", |c| {
        let desc = match c.arg(1) {
            Value::Nil => return Err(npe()),
            _ => desc_arg(c, 1)?,
        };
        let v = model_visual_of(&desc.borrow());
        model(c, |m| {
            m.outfit_name = None;
            m.female = v.female;
            m.zombie = v.zombie;
            m.visual = Some(v);
            m.model_serial += 1;
        })?;
        Ok(0)
    });
    b.method("setState", |c| {
        let s = str_arg(c, 1).map(|s| s.to_string());
        model(c, |m| m.state = s)?;
        Ok(0)
    });
    b.method("getState", |c| {
        let s = model(c, |m| m.state.clone())?;
        Ok(c.push(s.map(|s| Value::str(&s)).unwrap_or_default()))
    });
    b.method("setVariable", |c| {
        let Some(key) = str_arg(c, 1) else { return Ok(0) };
        let v = match c.arg(2) {
            Value::Bool(b) => VariableValue::Bool(b),
            Value::Num(n) => VariableValue::Float(n as f32),
            Value::Str(s) => VariableValue::String(s.to_string()),
            _ => VariableValue::String(String::new()),
        };
        model(c, |m| m.vars.set_variable(&key, v))?;
        Ok(0)
    });
    b.method("getVariable", |c| {
        let Some(key) = str_arg(c, 1) else { return Ok(c.push(Value::Nil)) };
        let v = model(c, |m| AnimationVariableHandle::alloc(&key).and_then(|h| m.vars.get_variable(&h)))?;
        Ok(c.push(v.map(variable_to_value).unwrap_or_default()))
    });
    b.method("clearVariable", |c| {
        let Some(key) = str_arg(c, 1) else { return Ok(0) };
        model(c, |m| m.vars.clear_variable(&key))?;
        Ok(0)
    });
    b.method("clearVariables", |c| {
        model(c, |m| m.vars.remove_all_variables())?;
        Ok(0)
    });
    b.method("reportEvent", |c| {
        let Some(ev) = str_arg(c, 1).filter(|s| !s.trim().is_empty()) else { return Ok(0) };
        model(c, |m| m.events.push(ev.to_string()))?;
        Ok(0)
    });
    b.method("clothingItemChanged", |c| {
        model(c, |m| m.model_serial += 1)?;
        Ok(0)
    });
    b.method("setZoom", |c| {
        let v = num_arg(c, 1) as f32;
        ui3d(c, |m| m.st.borrow_mut().zoom = v)?;
        Ok(0)
    });
    b.method("setYOffset", |c| {
        let v = num_arg(c, 1) as f32;
        ui3d(c, |m| m.st.borrow_mut().y_offset = v)?;
        Ok(0)
    });
    b.method("setXOffset", |c| {
        let v = num_arg(c, 1) as f32;
        ui3d(c, |m| m.st.borrow_mut().x_offset = v)?;
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expose::expose_enum;
    use pz_kahlua::TableRef;
    use pz_ui::UIManager;

    /// Records each render call.
    struct Recorder(Rc<RefCell<Vec<(u64, UIModelView, [f32; 3], [f32; 2], Vec<String>, Option<String>)>>>);

    impl UIModelRenderer for Recorder {
        fn render(&mut self, key: u64, m: &mut AnimatedModel, view: &UIModelView) -> Vec<DrawCmd> {
            let events = std::mem::take(&mut m.events);
            self.0.borrow_mut().push((key, *view, m.offset, m.angle, events, m.state.clone()));
            vec![DrawCmd::Clear(256)]
        }
    }

    fn setup() -> Thread {
        UIManager::reset();
        with_ui(|m| m.time_override_ms = Some(1_000_000));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_ui::register(&mut t);
        register(&mut t);
        let names: Vec<&'static str> = DIRECTIONS.iter().map(|d| d.0).collect();
        expose_enum(&mut t, ISO_DIRECTIONS, &names);
        t
    }

    fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    fn eval(t: &mut Thread, expr: &str) -> Value {
        run(t, &format!("__r = {expr}"));
        t.globals().get_str("__r")
    }

    /// The `ISUI3DModel` / `CharacterCreationAvatar` calls on a child of a
    /// plain element.
    const SCRIPT: &str = r#"
        Panel = {}
        Panel.__index = Panel
        parent = setmetatable({}, Panel)
        parent.javaObject = UIElement.new(parent)
        parent.javaObject:setX(100); parent.javaObject:setY(50)
        parent.javaObject:setWidth(400); parent.javaObject:setHeight(600)
        avatar = setmetatable({}, Panel)
        avatar.javaObject = UI3DModel.new(avatar)
        avatar.javaObject:setX(20); avatar.javaObject:setY(30)
        avatar.javaObject:setWidth(200); avatar.javaObject:setHeight(400)
        parent.javaObject:AddChild(avatar.javaObject)
        UIManager.AddUI(parent.javaObject)
        avatar.javaObject:setState("idle")
        avatar.javaObject:setDirection(IsoDirections.S)
        avatar.javaObject:setIsometric(false)
        avatar.javaObject:setDoRandomExtAnimations(true)
        avatar.javaObject:setZoom(-3)
        avatar.javaObject:setYOffset(0)
        avatar.javaObject:setVariable("Female", true)
        avatar.javaObject:setVariable("Speed", 0.5)
        avatar.javaObject:setVariable("Name", "x")
    "#;

    #[test]
    fn ui3dmodel_is_a_uielement_with_its_methods() {
        let mut t = setup();
        run(&mut t, SCRIPT);
        let v = eval(&mut t, "avatar.javaObject");
        let Value::Object(o) = &v else { panic!() };
        assert_eq!(o.class_name(), UI3DMODEL);
        assert!(o.super_classes().contains(&UIELEMENT));
        assert_eq!(eval(&mut t, "parent.javaObject:getControls():get(0) == avatar.javaObject").truthy(), true);
        assert_eq!(eval(&mut t, "avatar.javaObject:getDirection() == IsoDirections.S").truthy(), true);
        assert_eq!(eval(&mut t, "avatar.javaObject:getState()").as_str(), Some("idle"));
        assert_eq!(eval(&mut t, "avatar.javaObject:getVariable('Female')").truthy(), true);
        assert_eq!(eval(&mut t, "avatar.javaObject:getVariable('speed')").as_num(), Some(0.5));
        assert_eq!(eval(&mut t, "avatar.javaObject:getVariable('Name')").as_str(), Some("x"));
        run(&mut t, "avatar.javaObject:clearVariable('Name')");
        assert_eq!(eval(&mut t, "avatar.javaObject:getVariable('Name')").as_str(), Some(""));
        run(&mut t, "avatar.javaObject:clearVariables()");
        assert!(eval(&mut t, "avatar.javaObject:getVariable('Female')").is_nil());
        // UIElement methods still work on it.
        assert_eq!(eval(&mut t, "avatar.javaObject:getWidth()").as_num(), Some(200.0));
        // setAnimSetName rejects blank names; setState(nil) clears.
        run(&mut t, "ok = pcall(function() avatar.javaObject:setAnimSetName(' ') end)");
        assert_eq!(eval(&mut t, "ok").truthy(), false);
        run(&mut t, "avatar.javaObject:setState(nil)");
        assert!(eval(&mut t, "avatar.javaObject:getState()").is_nil());
        // Direction vector: S is (0, 1); NE normalised.
        assert_eq!(direction_vector("S"), Some([0.0, 1.0]));
        let ne = direction_vector("NE").unwrap();
        assert!((ne[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-7 && (ne[1] + std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-7);
    }

    /// `render()`: the base render, then the drawer's `DoRender` view
    /// (GL viewport from the absolute rectangle, `25 - zoom`, the
    /// non-isometric -0.5 offset) handed to the renderer, whose commands
    /// follow the element's own; reported events reach it once.
    #[test]
    fn render_hands_the_drawer_view_to_the_renderer() {
        let mut t = setup();
        let core = crate::bind_core::shared_core();
        core.borrow_mut().set_screen_size(1280, 720);
        let log = Rc::new(RefCell::new(Vec::new()));
        install_renderer(Some(Box::new(Recorder(log.clone()))));
        run(&mut t, SCRIPT);
        run(&mut t, "avatar.javaObject:reportEvent('EventWalk'); avatar.javaObject:reportEvent(' ')");
        with_ui(|m| m.time_override_ms = Some(1_000_000));
        UIManager::update(&mut t);
        UIManager::render(&mut t);
        {
            let l = log.borrow();
            assert_eq!(l.len(), 1);
            let (_, view, offset, angle, events, state) = &l[0];
            assert_eq!(view.viewport, [120, 720 - 80 - 400, 200, 400]);
            assert_eq!(view.size_v, 28.0);
            assert_eq!(view.screen, (1280, 720));
            assert_eq!(*offset, [0.0, -0.5, 0.0]);
            assert_eq!(*angle, [0.0, 1.0]);
            assert_eq!(events, &["EventWalk"]);
            assert_eq!(state.as_deref(), Some("idle"));
        }
        assert!(with_ui(|m| matches!(m.sprite.cmds.last(), Some(UiDrawCmd::Render(DrawCmd::Clear(256))))));
        // Isometric with a y offset and zoom: 22 - zoom, the set offset.
        run(&mut t, "avatar.javaObject:setIsometric(true); avatar.javaObject:setZoom(14); avatar.javaObject:setYOffset(-0.85); avatar.javaObject:setXOffset(0.25)");
        UIManager::render(&mut t);
        {
            let l = log.borrow();
            let (_, view, offset, _, events, _) = &l[1];
            assert_eq!(view.size_v, 8.0);
            assert_eq!(*offset, [0.25, -0.85, 0.0]);
            assert!(events.is_empty());
        }
        // Random Ext animations: armed 5-10 s ahead, then EventDoExt with
        // Ext in 1..=6.
        with_ui(|m| m.time_override_ms = Some(1_004_000));
        UIManager::render(&mut t);
        assert!(log.borrow()[2].4.is_empty());
        with_ui(|m| m.time_override_ms = Some(1_010_001));
        UIManager::render(&mut t);
        assert_eq!(log.borrow()[3].4, ["EventDoExt"]);
        let ext = eval(&mut t, "avatar.javaObject:getVariable('Ext')").as_num().unwrap();
        assert!((1.0..=6.0).contains(&ext), "{ext}");
        // Hidden: nothing rendered.
        run(&mut t, "avatar.javaObject:setVisible(false)");
        UIManager::render(&mut t);
        assert_eq!(log.borrow().len(), 4);
        install_renderer(None);
    }
}
