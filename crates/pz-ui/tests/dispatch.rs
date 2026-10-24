//! Lua-driven dispatch tests. pz-ui cannot depend on pz-lua-api (which
//! binds it), so this file installs a minimal `UIElement.new` + a few
//! methods directly on the thread.

use pz_kahlua::{LuaObject, NativeCall, NativeFunction, TableRef, Thread, Value};
use pz_ui::{with_ui, UIElement, UIManager};
use std::rc::Rc;

fn el(v: &Value) -> Rc<UIElement> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<UIElement>().unwrap().rc(),
        _ => panic!("not a UIElement"),
    }
}

fn num(c: &NativeCall<'_>, i: usize) -> f64 {
    c.arg(i).as_num().unwrap_or(0.0)
}

fn setup() -> Thread {
    UIManager::reset();
    with_ui(|m| m.time_override_ms = Some(5_000));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    let methods = TableRef::new();
    let meta = TableRef::new();
    meta.set_str("__index", methods.clone());
    t.class_metatables.set_str("zombie.ui.UIElement", meta);
    let reg = |name: &str, f: fn(&mut NativeCall<'_>) -> pz_kahlua::LuaResult<usize>| {
        methods.set_str(name, Value::Native(NativeFunction::new(name, f)));
    };
    reg("setX", |c| {
        el(&c.arg(0)).set_x(num(c, 1));
        Ok(0)
    });
    reg("setY", |c| {
        el(&c.arg(0)).set_y(num(c, 1));
        Ok(0)
    });
    reg("setWidth", |c| {
        el(&c.arg(0)).set_width(num(c, 1));
        Ok(0)
    });
    reg("setHeight", |c| {
        el(&c.arg(0)).set_height(num(c, 1));
        Ok(0)
    });
    reg("AddChild", |c| {
        let p = el(&c.arg(0));
        p.add_child(&el(&c.arg(1)));
        Ok(0)
    });
    reg("setConsumeMouseEvents", |c| {
        el(&c.arg(0)).set_consume_mouse_events(matches!(c.arg(1), Value::Bool(true)));
        Ok(0)
    });
    let ui = TableRef::new();
    ui.set_str(
        "new",
        Value::Native(NativeFunction::new("new", |c| {
            let e = UIElement::new(c.arg(0).as_table().cloned());
            Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
        })),
    );
    t.globals().set_str("UIElement", ui);
    run(
        &mut t,
        r#"
        log = {}
        function L(s) log[#log + 1] = s end
        Base = {}
        Base.__index = Base
        function Base:derive(n) local o = setmetatable({}, self) o.__index = o o.Type = n return o end
        Panel = Base:derive("Panel")
        function Panel:new(name, x, y, w, h)
            local o = setmetatable({}, self)
            o.name = name
            o.javaObject = UIElement.new(o)
            o.javaObject:setX(x); o.javaObject:setY(y)
            o.javaObject:setWidth(w); o.javaObject:setHeight(h)
            return o
        end
        function Panel:add(c) self.javaObject:AddChild(c.javaObject) return c end
        -- Inherited through two metatable levels: found via tableget.
        function Base:prerender() L("pre " .. self.name) end
        function Panel:render() L("render " .. self.name) end
        function Panel:onMouseDown(x, y) L("down " .. self.name .. " " .. x .. "," .. y) end
        function Panel:onMouseDoubleClick(x, y) L("dbl " .. self.name) end
    "#,
    );
    t
}

fn run(t: &mut Thread, src: &str) {
    let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
    let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
    t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?} {tr}")).unwrap();
}

fn log(t: &mut Thread) -> Vec<String> {
    let l = t.globals().get_str("log");
    let l = l.as_table().unwrap().clone();
    let v = (1..=l.len()).map(|i| l.get_int(i).as_str().unwrap().to_string()).collect();
    t.globals().set_str("log", TableRef::new());
    v
}

fn global_el(t: &Thread, name: &str) -> Rc<UIElement> {
    let tb = t.globals().get_str(name);
    el(&tb.as_table().unwrap().get_str("javaObject"))
}

#[test]
fn render_calls_inherited_prerender_then_children_then_render() {
    let mut t = setup();
    run(&mut t, r#"root = Panel:new("root", 0, 0, 100, 100); kid = root:add(Panel:new("kid", 1, 1, 5, 5))"#);
    global_el(&t, "root").render(&mut t);
    assert_eq!(log(&mut t), ["pre root", "pre kid", "render kid", "render root"]);
    global_el(&t, "kid").set_visible(false);
    global_el(&t, "root").render(&mut t);
    assert_eq!(log(&mut t), ["pre root", "render root"]);
}

#[test]
fn nil_mouse_down_result_consumes_per_flag_and_double_click() {
    let mut t = setup();
    run(&mut t, r#"p = Panel:new("p", 10, 10, 50, 50); q = p:add(Panel:new("q", 20, 0, 10, 10))"#);
    let p = global_el(&t, "p");
    // Child q gets coordinates local to it; it returns nil and consumes
    // (consumeMouseEvents defaults to true), so p's own onMouseDown is skipped.
    assert!(p.on_mouse_down(&mut t, 25.0, 5.0));
    assert_eq!(log(&mut t), ["down q 5,5"]);
    global_el(&t, "q").set_consume_mouse_events(false);
    with_ui(|m| m.time_override_ms = Some(10_000));
    assert!(p.on_mouse_down(&mut t, 25.0, 5.0));
    assert_eq!(log(&mut t), ["down q 5,5", "down p 25,5"]);
    // Second click within 500 ms and 5 px is a double click.
    with_ui(|m| m.time_override_ms = Some(10_200));
    assert!(p.on_mouse_down(&mut t, 26.0, 6.0));
    assert_eq!(log(&mut t), ["dbl p"]);
}

#[test]
fn absolute_coordinates_follow_scroll_children() {
    let mut t = setup();
    run(&mut t, r#"p = Panel:new("p", 10.7, 20.2, 100, 100); c = p:add(Panel:new("c", 3.9, 4, 10, 10))"#);
    let (p, c) = (global_el(&t, "p"), global_el(&t, "c"));
    assert_eq!((c.get_absolute_x(), c.get_absolute_y()), (13.0, 24.0));
    p.set_scroll_children(true);
    p.set_y_scroll(&mut t, -30.5);
    assert_eq!(c.get_absolute_y(), 20.0 + 4.0 - 30.0);
    assert_eq!(c.get_y_scrolled(Some(&p)), 4.0 - 30.5);
    c.set_scroll_with_parent(false);
    assert_eq!(c.get_absolute_y(), 24.0);
}

#[test]
fn point_over_respects_later_siblings_and_top_level_order() {
    let mut t = setup();
    run(
        &mut t,
        r#"a = Panel:new("a", 0, 0, 100, 100); b = Panel:new("b", 50, 50, 100, 100)
           x = a:add(Panel:new("x", 0, 0, 40, 40)); y = a:add(Panel:new("y", 20, 20, 40, 40))"#,
    );
    let (a, b) = (global_el(&t, "a"), global_el(&t, "b"));
    UIManager::add_ui(a.clone());
    UIManager::add_ui(b.clone());
    UIManager::update_before_fade_out();
    assert!(a.is_point_over(10.0, 10.0));
    assert!(!a.is_point_over(60.0, 60.0), "b covers a");
    assert!(global_el(&t, "x").is_point_over(10.0, 10.0));
    assert!(!global_el(&t, "x").is_point_over(30.0, 30.0), "later sibling y covers x");
    assert!(global_el(&t, "y").is_really_visible());
    UIManager::remove_element(a.clone());
    UIManager::update_before_fade_out();
    assert!(!global_el(&t, "y").is_really_visible());
}
