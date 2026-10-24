//! `RandomLib`: `newrandom()` objects wrapping `java.util.Random`.

use super::*;
use crate::value::LuaObject;
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

/// `java.util.Random` (48-bit LCG).
pub struct JavaRandom {
    seed: Cell<i64>,
}

const MULT: i64 = 0x5DEECE66D;
const MASK: i64 = (1 << 48) - 1;

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        JavaRandom { seed: Cell::new((seed ^ MULT) & MASK) }
    }
    pub fn set_seed(&self, seed: i64) {
        self.seed.set((seed ^ MULT) & MASK);
    }
    fn next(&self, bits: u32) -> i32 {
        let s = (self.seed.get().wrapping_mul(MULT).wrapping_add(0xB)) & MASK;
        self.seed.set(s);
        (s >> (48 - bits)) as i32
    }
    pub fn next_int(&self, bound: i32) -> Option<i32> {
        if bound <= 0 {
            return None;
        }
        if bound & -bound == bound {
            return Some(((bound as i64 * self.next(31) as i64) >> 31) as i32);
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return Some(val);
            }
        }
    }
    pub fn next_double(&self) -> f64 {
        let hi = (self.next(26) as i64) << 27;
        (hi + self.next(27) as i64) as f64 * (1.0 / (1i64 << 53) as f64)
    }
}

impl LuaObject for JavaRandom {
    fn class_name(&self) -> &str {
        "java.util.Random"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn get_random(c: &NativeCall<'_>, name: &str) -> LuaResult<Rc<dyn LuaObject>> {
    let o = arg(c, 1, name)?;
    match o {
        Value::Object(obj) if obj.as_any().is::<JavaRandom>() => Ok(obj),
        _ => Err(LuaError::runtime(format!("First argument to {name} must be an object of type random."))),
    }
}

/// Java `Object.hashCode()` for the values scripts pass as seeds.
fn java_hash(v: &Value) -> i32 {
    match v {
        Value::Nil => 0,
        Value::Num(d) => {
            let bits = if d.is_nan() { 0x7ff8_0000_0000_0000u64 } else { d.to_bits() };
            (bits ^ (bits >> 32)) as i32
        }
        Value::Bool(b) => if *b { 1231 } else { 1237 },
        Value::Str(s) => s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32)),
        other => crate::exec::identity_hash(other) as i32,
    }
}

pub fn register(thread: &mut Thread, env: &TableRef) {
    let t = TableRef::new();
    reg(&t, "random", |c| {
        let r = get_random(c, "random")?;
        let r = r.as_any().downcast_ref::<JavaRandom>().unwrap();
        let min = opt_number_arg(c, 2);
        let max = opt_number_arg(c, 3);
        let Some(min) = min else { return Ok(c.push(r.next_double())) };
        let mut m = min as i32;
        let n = match max {
            None => {
                let n = m;
                m = 1;
                n
            }
            Some(x) => x as i32,
        };
        let bound = n.wrapping_sub(m).wrapping_add(1);
        let v = r.next_int(bound).ok_or_else(|| LuaError::Runtime {
            message: "bound must be positive".into(),
            class: "java.lang.IllegalArgumentException",
        })?;
        Ok(c.push(m.wrapping_add(v) as f64))
    });
    reg(&t, "seed", |c| {
        let r = get_random(c, "seed")?;
        let r = r.as_any().downcast_ref::<JavaRandom>().unwrap();
        r.set_seed(java_hash(&c.arg(1)) as i64);
        Ok(0)
    });
    t.set_str("__index", t.clone());
    thread.class_metatables.set_str("java.util.Random", t);
    reg(env, "newrandom", |c| {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        Ok(c.push(Value::Object(Rc::new(JavaRandom::new(seed ^ 0x5DEECE66D)))))
    });
}
