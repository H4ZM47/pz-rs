//! `MathLib` (J2SE variant). Transcendental functions use the `libm`
//! (fdlibm-derived) implementations, which match Java's `Math` results.
//! Kahlua has no `math.random`; the game provides
//! `ZombRand` and friends instead.

use super::*;

fn is_negative(v: f64) -> bool {
    (v.to_bits() as i64) < 0
}

macro_rules! unary {
    ($t:expr, $name:literal, $f:expr) => {
        reg_named($t, $name, concat!("math.", $name), |c| {
            lua_assert(c.nargs() >= 1, "Not enough arguments")?;
            let x = number_arg(c, 1, $name)?;
            let f: fn(f64) -> f64 = $f;
            Ok(c.push(f(x)))
        });
    };
}

pub fn register(env: &TableRef) {
    let math = TableRef::new();
    env.set_str("math", math.clone());
    math.set_str("pi", std::f64::consts::PI);
    math.set_str("huge", f64::INFINITY);
    let m = &math;
    unary!(m, "abs", f64::abs);
    unary!(m, "acos", libm::acos);
    unary!(m, "asin", libm::asin);
    unary!(m, "atan", f64::atan);
    reg_named(m, "atan2", "math.atan2", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let y = number_arg(c, 1, "atan2")?;
        let x = number_arg(c, 2, "atan2")?;
        Ok(c.push(libm::atan2(y, x)))
    });
    unary!(m, "ceil", f64::ceil);
    unary!(m, "cos", f64::cos);
    unary!(m, "cosh", libm::cosh);
    // Math.toDegrees / toRadians use these exact constant factors.
    unary!(m, "deg", |x| x * (180.0 / std::f64::consts::PI));
    unary!(m, "exp", libm::exp);
    unary!(m, "floor", f64::floor);
    reg_named(m, "fmod", "math.fmod", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let mut v1 = number_arg(c, 1, "fmod")?;
        let mut v2 = number_arg(c, 2, "fmod")?;
        let res = if v1.is_infinite() || v1.is_nan() {
            f64::NAN
        } else if v2.is_infinite() {
            v1
        } else {
            v2 = v2.abs();
            let negate = is_negative(v1);
            if negate {
                v1 = -v1;
            }
            let r = v1 - (v1 / v2).floor() * v2;
            if negate {
                -r
            } else {
                r
            }
        };
        Ok(c.push(res))
    });
    reg_named(m, "frexp", "math.frexp", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let x = number_arg(c, 1, "frexp")?;
        let (mm, e) = if x.is_infinite() || x.is_nan() {
            (x, 0.0)
        } else {
            let e = (libm::log(x) * (1.0 / std::f64::consts::LN_2)).ceil();
            let div = 1i32.wrapping_shl(e as i32 as u32);
            (x / div as f64, e)
        };
        c.push(mm);
        c.push(e);
        Ok(2)
    });
    reg_named(m, "ldexp", "math.ldexp", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let mm = number_arg(c, 1, "ldexp")?;
        let de = number_arg(c, 2, "ldexp")?;
        let tmp = mm + de;
        let r = if tmp.is_infinite() || tmp.is_nan() { mm } else { mm * 1i32.wrapping_shl(de as i32 as u32) as f64 };
        Ok(c.push(r))
    });
    unary!(m, "log", libm::log);
    unary!(m, "log10", libm::log10);
    reg_named(m, "modf", "math.modf", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let mut x = number_arg(c, 1, "modf")?;
        let negate = is_negative(x);
        if negate {
            x = -x;
        }
        let mut ip = x.floor();
        let mut fp = if ip.is_infinite() { 0.0 } else { x - ip };
        if negate {
            ip = -ip;
            fp = -fp;
        }
        c.push(ip);
        c.push(fp);
        Ok(2)
    });
    reg_named(m, "pow", "math.pow", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let x = number_arg(c, 1, "pow")?;
        let y = number_arg(c, 2, "pow")?;
        Ok(c.push(libm::pow(x, y)))
    });
    unary!(m, "rad", |x| x * (std::f64::consts::PI / 180.0));
    unary!(m, "sin", f64::sin);
    unary!(m, "sinh", libm::sinh);
    unary!(m, "sqrt", f64::sqrt);
    unary!(m, "tan", f64::tan);
    unary!(m, "tanh", libm::tanh);
}
