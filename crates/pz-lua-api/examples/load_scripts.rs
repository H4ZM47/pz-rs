//! Run `LuaManager.init` + `LoadDirBase()` against an installed game and
//! summarise script errors. Usage: load_scripts <game-java-dir>

use std::collections::BTreeMap;
use std::rc::Rc;

fn main() {
    let root = std::env::args().nth(1).expect("game directory containing media/");
    let fs = Rc::new(pz_fs::GameFs::new(root));
    let t = std::time::Instant::now();
    let mut mgr = pz_lua_api::LuaManager::init(fs).expect("init");
    mgr.thread.log = Box::new(|_| {});
    mgr.thread.out = Box::new(|_| {});
    let missing: Rc<std::cell::RefCell<BTreeMap<String, usize>>> = Rc::default();
    let m2 = missing.clone();
    mgr.thread.on_missing_global = Some(Box::new(move |n| *m2.borrow_mut().entry(n.to_string()).or_default() += 1));
    mgr.load_all_client();
    let mut mv: Vec<_> = missing.borrow().clone().into_iter().collect();
    mv.sort_by(|a, b| b.1.cmp(&a.1));
    println!("missing globals during load ({}):", mv.len());
    for (n, c) in mv.iter().take(60) {
        print!("{n}({c}) ");
    }
    println!();
    let errors = mgr.loader.borrow().errors.clone();
    println!("loaded in {:?}; {} files failed", t.elapsed(), errors.len());
    // Group by first line's message after the file name.
    let mut causes: BTreeMap<String, usize> = BTreeMap::new();
    for e in &errors {
        let first = e.lines().next().unwrap_or("");
        let msg = first.split_once(": ").map(|x| x.1).unwrap_or(first).to_string();
        *causes.entry(msg).or_default() += 1;
    }
    let mut v: Vec<_> = causes.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (m, n) in v.iter().take(40) {
        println!("{n:4}  {m}");
    }
}
