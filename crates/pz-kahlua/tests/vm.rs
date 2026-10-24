//! End-to-end VM behaviour, written against Kahlua's semantics.

use pz_kahlua::{stdlib, TableRef, Thread, Value};
use std::cell::RefCell;
use std::rc::Rc;

fn run(src: &str) -> (Vec<Value>, String) {
    let mut t = Thread::new(TableRef::new());
    stdlib::install(&mut t, None).unwrap();
    let out = Rc::new(RefCell::new(String::new()));
    let o = out.clone();
    t.out = Box::new(move |s| {
        o.borrow_mut().push_str(s);
        o.borrow_mut().push('\n');
    });
    // The game's stdlib.lua aliases these; tests cannot ship that file.
    let prelude = stdlib::load_string("pairs = table.pairs; ipairs = table.ipairs", None, t.globals().clone()).unwrap();
    t.call(&prelude, &[]).unwrap();
    let f = stdlib::load_string(src, Some("test"), t.globals().clone()).expect("compile");
    let r = t.call(&f, &[]).unwrap_or_else(|e| panic!("runtime error: {e}"));
    let s = out.borrow().clone();
    (r, s)
}

fn printed(src: &str) -> String {
    run(src).1
}

#[test]
fn arithmetic_and_formatting() {
    assert_eq!(printed("print(1+2, 7/2, 2^10, -7 % 3, 7 % -3, 10 % 0)"), "3\t3.5\t1024\t-1\t1\tnan\n");
    assert_eq!(printed("print(1e15, 0.1, 100000000000000, 1/0, -1/0)"), "1.0E15\t0.1\t1.0E14\tinf\t-inf\n");
    assert_eq!(printed("print('10' + 5, 'a' .. 1 .. 2.5)"), "15\ta12.5\n");
}

#[test]
fn closures_upvalues_and_varargs() {
    let src = r#"
        local function counter()
            local n = 0
            return function() n = n + 1; return n end
        end
        local c = counter(); c(); c()
        local function va(...) return select('#', ...), ... end
        local fns = {}
        for i = 1, 3 do fns[i] = function() return i end end
        print(c(), va(1, nil, 3), fns[1]() + fns[3]())
    "#;
    assert_eq!(printed(src), "3\t3\t4\n");
}

#[test]
fn tables_keep_insertion_order() {
    let src = r#"
        local t = {}
        t.b = 1; t.a = 2; t[3] = 'x'; t.c = 3; t.a = 4
        local s = ''
        for k, v in pairs(t) do s = s .. tostring(k) .. '=' .. tostring(v) .. ';' end
        -- Kahlua's border search over 2*size finds 3 here, not 5.
        print(s, #{1, 2, 3, nil, 5})
    "#;
    assert_eq!(printed(src), "b=1;a=4;3=x;c=3;\t3\n");
}

#[test]
fn class_style_inheritance() {
    let src = r#"
        Base = {}
        Base.__index = Base
        function Base:derive(type)
            local o = {}
            setmetatable(o, self)
            self.__index = self
            o.Type = type
            return o
        end
        function Base:new() local o = {}; setmetatable(o, self); self.__index = self; return o end
        function Base:name() return 'base:' .. self.Type end
        Child = Base:derive('Child')
        function Child:extra() return 'extra' end
        local c = Child:new()
        print(c:name(), c:extra(), rawget(c, 'name') ~= nil)
    "#;
    // Kahlua's rawget falls through to the metatable, so rawget finds `name`.
    assert_eq!(printed(src), "base:Child\textra\ttrue\n");
}

#[test]
fn errors_and_pcall() {
    let src = r#"
        local ok, msg = pcall(function() error('boom') end)
        local ok2, msg2 = pcall(function() local x = nil; return x.y end)
        local ok3, msg3 = pcall(function() error({code = 7}) end)
        print(ok, msg, ok2, msg2, msg3.code)
    "#;
    assert_eq!(
        printed(src),
        "false\tboom\tfalse\tattempted index: y of non-table: null java.lang.RuntimeException\t7\n"
    );
}

#[test]
fn string_methods_through_metatable() {
    assert_eq!(printed("local s = 'abc'; print(#s, s.x == nil)"), "3\ttrue\n");
}

#[test]
fn coroutines() {
    let src = r#"
        local co = coroutine.create(function(a, b)
            local c = coroutine.yield(a + b)
            local d, e = coroutine.yield(c * 2)
            return d + e
        end)
        print(coroutine.resume(co, 1, 2))
        print(coroutine.resume(co, 10))
        print(coroutine.resume(co, 3, 4))
        print(coroutine.status(co), pcall(coroutine.resume, co))
    "#;
    assert_eq!(
        printed(src),
        "true\t3\ntrue\t20\ntrue\t7\ndead\tfalse\tjava.lang.RuntimeException Can not resume coroutine that is in status: dead in resume java.lang.RuntimeException\tat resume\n\tjava.lang.RuntimeException: java.lang.RuntimeException Can not resume coroutine that is in status: dead in resume\n"
    );
}

#[test]
fn while_repeat_and_numeric_for() {
    let src = r#"
        local s, i = 0, 0
        while true do i = i + 1; if i > 5 then break end; s = s + i end
        repeat i = i - 1 until i <= 0
        for j = 10, 1, -3 do s = s + j end
        print(s, i)
    "#;
    assert_eq!(printed(src), "37\t0\n");
}

#[test]
fn tostring_of_tables_lists_contents() {
    let (r, _) = run("local t = {x = 1}; return tostring(t)");
    let s = r[0].as_str().unwrap().to_string();
    assert!(s.starts_with("table 0x") && s.ends_with("{ x=1 }"), "{s}");
}
