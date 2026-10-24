//! `CoroutineLib`.

use super::*;

pub fn register(env: &TableRef) {
    let co = TableRef::new();
    co.set_str("__index", co.clone());
    reg(&co, "create", |c| {
        let f = arg(c, 1, "create")?;
        let Value::Closure(cl) = f else { return Err(LuaError::runtime("argument must be a lua function")) };
        let r = c.thread.create_coroutine(cl);
        Ok(c.push(Value::Coroutine(r)))
    });
    reg(&co, "resume", |c| {
        let Value::Coroutine(t) = arg(c, 1, "resume")? else {
            return Err(LuaError::runtime("argument must be a coroutine"));
        };
        let args: Vec<Value> = (1..c.nargs()).map(|i| c.arg(i)).collect();
        match c.thread.resume(&t, &args)? {
            Ok(vals) => {
                c.push(true);
                for v in &vals {
                    c.push(v.clone());
                }
                Ok(1 + vals.len())
            }
            Err((msg, trace)) => {
                c.push(false);
                c.push(msg);
                c.push(trace);
                Ok(3)
            }
        }
    });
    reg(&co, "yield", |c| {
        let vals: Vec<Value> = (0..c.nargs()).map(|i| c.arg(i)).collect();
        c.thread.yield_values(vals)?;
        Ok(0)
    });
    reg(&co, "status", |c| {
        let Value::Coroutine(t) = arg(c, 1, "status")? else {
            return Err(LuaError::runtime("argument must be a coroutine"));
        };
        let s = c.thread.coroutine_status(&t);
        Ok(c.push(s))
    });
    reg(&co, "running", |c| {
        let r = c.thread.current_coroutine().map(Value::Coroutine).unwrap_or_default();
        Ok(c.push(r))
    });
    env.set_str("coroutine", co);
}
