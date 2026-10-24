//! Differential testing against the original Kahlua.
//!
//! `differential write <dir>` writes the corpus as `<dir>/NNN.lua`;
//! run the original interpreter over that directory locally (it writes
//! `NNN.lua.out`), then `differential compare <dir>` runs the same scripts
//! here and reports mismatches. Oracle outputs never enter the repository.

use pz_kahlua::{stdlib, TableRef, Thread};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

const CORPUS: &[&str] = &[
    "print(1, 1.5, -0.0, 1e15, 1e-5, 123456789, 2^53, 1/3, 100/3, -7%3, 7%-3, 5.5%2)",
    "print(10 == '10', '10' + 1, '0x10' + 0, ' 5 ' * 2, 'abc' < 'abd', 'Z' < 'a')",
    "print(tostring(nil), tostring(true), type(print), type({}), type('x'), type(2))",
    "print(tonumber('12'), tonumber('1e2'), tonumber('abc'), tonumber('ff', 16), tonumber('777', 8), tonumber('z', 36), tonumber('2.5f'), tonumber('5d'))",
    "print(select('#'), select('#', nil, nil), select(2, 'a', 'b', 'c'), select(-1, 'a'))",
    "local t = {} t[1]=1 t[2]=2 t[4]=4 print(#t) t[3]=3 print(#t) t[8]=8 print(#t)",
    "print(#{1,2,3,nil,5}, #{nil,nil,3}, #{n=1})",
    "local t = {c=1, a=2, b=3, 10, 20} local s='' for k,v in pairs(t) do s=s..tostring(k)..'='..tostring(v)..',' end print(s)",
    "local t = {} for i=1,5 do t['k'..i]=i end t.k2=nil t.k6=6 t.k2=22 local s='' for k,v in pairs(t) do s=s..k..'='..v..',' end print(s)",
    "print(math.floor(-2.5), math.ceil(-2.5), math.abs(-3), math.max(1,5,3), math.min(4,2,8), math.fmod(-7,3), math.fmod(7,-3), math.huge, -math.huge, math.pi)",
    "print(math.modf(3.7), math.modf(-3.7), math.frexp(8), math.ldexp(1, 4), math.sqrt(2), math.pow(2, 0.5), math.log(10), math.log10(1000), math.exp(1))",
    "print(pcall(error, 'msg'), pcall(error, {}), pcall(error), pcall(function() return nil + 1 end))",
    "print(pcall(function() local t = nil; return t.x end))",
    "print(pcall(function() return {} .. 'x' end))",
    "print(pcall(function() return #nil end))",
    "print(pcall(function() undefined_fn() end))",
    "print(pcall(function() return {} < {} end))",
    "local mt = {__index = function(t, k) return k .. '!' end} local t = setmetatable({}, mt) print(t.foo, rawget(t, 'foo'))",
    "local base = {x = 1} local t = setmetatable({}, base) print(t.x, rawget(t, 'x'))",
    "local mt = {__add = function(a, b) return 'added' end, __concat = function(a, b) return 'cat' end, __eq = function() return true end} local a, b = setmetatable({}, mt), setmetatable({}, mt) print(a + 1, 1 + a, a .. 'x', 'x' .. a, a == b)",
    "local mt = {__lt = function(a, b) return true end} local a, b = setmetatable({}, mt), setmetatable({}, mt) print(a < b, a <= b, a > b)",
    "local mt = {__call = function(self, x) return 'called ' .. tostring(x) end} local t = setmetatable({}, mt) print(t(5))",
    "local mt = {__newindex = function(t, k, v) rawset(t, k, v * 2) end} local t = setmetatable({}, mt) t.a = 2 print(t.a)",
    "local mt = {__tostring = function() return 'custom' end} print(tostring(setmetatable({}, mt)))",
    "print(unpack({1, 2, 3})) print(unpack({1, 2, 3}, 2)) print(unpack({1, 2, 3}, 2, 3)) print(unpack({}))",
    "local t = {1,2,3} table.insert(t, 4) table.insert(t, 1, 0) print(table.concat(t, ',')) print(table.remove(t), table.remove(t, 1), table.concat(t, '-'))",
    "local t = {3,1,2} table.sort(t) print(table.concat(t, ',')) table.sort(t, function(a,b) return a > b end) print(table.concat(t, ','))",
    "print(table.concat({1, 2.5, 'x'}, ', ', 2, 3), table.concat({}), table.isempty({}), table.isempty({1}))",
    "local co = coroutine.create(function(a) local b = coroutine.yield(a * 2) return b + 1 end) print(coroutine.resume(co, 5)) print(coroutine.status(co)) print(coroutine.resume(co, 10)) print(coroutine.status(co))",
    "local f = coroutine.wrap(function() for i = 1, 3 do coroutine.yield(i) end end) print(f(), f(), f())",
    "print(coroutine.running())",
    "local function f(...) local a, b = ... return select('#', ...), a, b, ... end print(f()) print(f(1)) print(f(1, 2, 3))",
    "local t = {} for i = 10, 1, -2.5 do t[#t+1] = i end print(table.concat(t, ' ')) for i = 1, 0 do print('never') end",
    "local s = 0 for i = 0.1, 1, 0.1 do s = s + 1 end print(s)",
    "local a, b, c = (function() return 1, 2, 3 end)() print(a, b, c) print((function() return 1, 2 end)())",
    "local x = 10 local function g() x = x + 1 return x end print(g(), g(), x)",
    "local fs = {} for i = 1, 3 do local j = i fs[i] = function() return j end end print(fs[1](), fs[2](), fs[3]())",
    "print(rawequal('a', 'a'), rawequal({}, {}), rawequal(1, 1.0), rawlen and 1)",
    "local t = setmetatable({}, {__metatable = 'locked'}) print(getmetatable(t), pcall(setmetatable, t, {}))",
    "print(getmetatable('abc') == string, ('x'):rep(3), ('abc'):upper())",
    "local t = {} t[1.0] = 'a' t[2] = 'b' print(t[1], t[2.0], #t)",
    "print(2^-1, 2^0.5, (-8)^(1/3), 0/0 ~= 0/0, 1e308 * 10, -1e308 * 10)",
    "print(string.format('%d %5d %-5d| %05d %x %X %o', 42, 42, 42, 42, 255, 255, 8))",
    "print(string.format('%f %.2f %10.3f %-10.1f| %e %.3E %g %g %g', 3.14159, 3.14159, 3.14159, 3.14159, 12345.678, 0.000123, 0.0001, 100000, 1e20))",
    "print(string.format('%s %10s %-10s| %q %c%c', 'hi', 'hi', 'hi', 'a\\nb\"c', 72, 105))",
    "print(string.format('%5.1s| %.3s %%', 'abc', 'abcdef'), pcall(string.format, '%y', 1))",
    "print(string.find('hello world', 'o w'), string.find('hello', 'l+'), string.find('hello', 'xyz'), string.find('a.b', '.', 1, true))",
    "print(string.match('key=value', '(%w+)=(%w+)'), string.match('  trim  ', '^%s*(.-)%s*$'), string.match('2024-01-05', '(%d+)-(%d+)-(%d+)'))",
    "print(string.gsub('hello world', 'o', '0'), string.gsub('abc', '%w', '%0%0'), string.gsub('hello', '', '-'), string.gsub('abc', 'b', {b = 'B'}), string.gsub('abc', '%w', function(c) return c:upper() end))",
    "print(string.gsub('hello world', '(%w+)', '<%1>', 1), string.find('THE (quick) fox', '%((%a+)%)'), string.match('[[x]]', '%[(.-)%]'))",
    "for w in string.gmatch('one two three', '%a+') do io = nil print(w) end",
    "print(string.sub('hello', 2, 4), string.sub('hello', -3), string.sub('hello', 0), string.sub('hello', 10), string.sub('hello', 2), string.byte('A'), string.byte('abc', 1, 3), string.char(72, 105))",
    "print(string.len('héllo'), #'héllo', string.upper('héllo'), string.reverse('abc'), string.rep('ab', 3))",
    "print(string.trim and string.trim('  x  '), string.split and #string.split('a,b,c', ','), string.contains and string.contains('hello', 'ell'))",
    "print(string.find('abc', '[a-b]+'), string.find('abc', '[^a]+'), string.match('x=1, y=2', 'y=(%d)'), string.find('f(x)', '%b()'))",
    "print(string.format('%d', 3.7), string.format('%d', -3.7), string.format('%5.2f%%', 99.555), string.format('%.0f %.0f', 0.5, 1.5))",
    "print(os.time() > 0, type(os.date()), os.date('!%Y-%m-%d %H:%M:%S', 0), os.difftime(10, 4))",
    "local t = os.date('!*t', 86400 * 365) print(t.year, t.month, t.day, t.hour, t.min, t.sec, t.wday, t.yday)",
    "print(loadstring('return 1 + 1')(), loadstring('syntax error here'))",
    "print(pcall(loadstring, nil))",
    "local f = loadstring('local a, b = ... return a * b') print(f(6, 7))",
    "print(next)",
    "print(string.format('[%s]', tostring(1e100)), tostring(-1e-10), tostring(123456.789), tostring(0.1 + 0.2))",
    "local t = {} t[-0] = 'neg' print(t[0])",
    "print(math.floor(2^31 + 0.5), string.format('%d', 2^40), string.format('%x', -1))",
];

fn run_ours(src: &str, name: &str, stdlib_lua: Option<&str>) -> String {
    let mut t = Thread::new(TableRef::new());
    let out = Rc::new(RefCell::new(String::new()));
    let o = out.clone();
    t.out = Box::new(move |s| {
        o.borrow_mut().push_str(s);
        o.borrow_mut().push('\n');
    });
    t.log = Box::new(|_| {});
    if let Err(e) = stdlib::install(&mut t, stdlib_lua) {
        return format!("STDLIB ERROR: {e}\n");
    }
    match stdlib::load_string(src, Some(name), t.globals().clone()) {
        Ok(f) => {
            if let Err((msg, _)) = t.pcall(&f, &[]) {
                let m = pz_kahlua::exec::java_value_of(&msg);
                out.borrow_mut().push_str(&format!("ERROR: {m}\n"));
            }
        }
        Err(e) => out.borrow_mut().push_str(&format!("ERROR: {e}\n")),
    }
    let s = out.borrow().clone();
    s
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("write|compare");
    let dir = args.next().expect("dir");
    let dir = Path::new(&dir);
    match mode.as_str() {
        "write" => {
            std::fs::create_dir_all(dir).unwrap();
            for (i, src) in CORPUS.iter().enumerate() {
                std::fs::write(dir.join(format!("{i:03}.lua")), src).unwrap();
            }
            println!("wrote {} scripts", CORPUS.len());
        }
        "compare" => {
            let stdlib_lua = args.next().map(|p| std::fs::read_to_string(p).expect("stdlib.lua"));
            let (mut same, mut diff) = (0, 0);
            for (i, src) in CORPUS.iter().enumerate() {
                let name = format!("{i:03}.lua");
                let Ok(theirs) = std::fs::read_to_string(dir.join(format!("{name}.out"))) else { continue };
                let ours = run_ours(src, &name, stdlib_lua.as_deref());
                if ours == theirs {
                    same += 1;
                } else {
                    diff += 1;
                    println!("--- {name}: {src}\n  original: {theirs:?}\n  ours:     {ours:?}");
                }
            }
            println!("{same} match, {diff} differ");
        }
        _ => panic!("unknown mode"),
    }
}
