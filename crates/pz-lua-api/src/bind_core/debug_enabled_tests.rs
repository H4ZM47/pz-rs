use super::*;

#[test]
fn is_debug_enabled_reads_the_live_core_debug_flag() {
    let core = Rc::new(RefCell::new(Core::new()));
    let mut thread = Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    register(&mut thread, None, core.clone());
    let read = |thread: &mut Thread| {
        let f = pz_kahlua::stdlib::load_string("return isDebugEnabled()", Some("test"), thread.globals().clone()).unwrap();
        thread.pcall(&f, &[]).map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e)).unwrap()[0].clone()
    };
    core.borrow_mut().debug = false;
    assert!(matches!(read(&mut thread), Value::Bool(false)));
    core.borrow_mut().debug = true;
    assert!(matches!(read(&mut thread), Value::Bool(true)));
}
