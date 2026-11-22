//! `zombie.characters.Safety` over the character's own `safety` field.
//! See [`pz_characters::character::safety`] for what is not ported.
use super::{canonical, character_value, chr_of, Handle, LuaGameCharacterRef};
use crate::expose::{bool_arg, expose_class, num_arg, with_this};
use pz_characters::character::safety::Safety;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::rc::Rc;

pub const SAFETY_CLASS: &str = "zombie.characters.Safety";

/// The `safety` of a character.
pub struct SafetyObject(LuaGameCharacterRef);

impl LuaObject for SafetyObject {
    fn class_name(&self) -> &str {
        SAFETY_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `IsoGameCharacter.getSafety()`: one Lua object per character.
pub(super) fn get_safety(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let chr = chr_of(c)?;
    let v = canonical((Handle::Safety, Rc::as_ptr(&chr) as usize, 0), || {
        Value::Object(Rc::new(SafetyObject(chr.clone())))
    });
    Ok(c.push(v))
}

fn owner(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<SafetyObject, _>(c, |s| s.0.clone())
}

fn with<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Safety) -> R) -> LuaResult<R> {
    let chr = owner(c)?;
    let mut chr = chr
        .try_borrow_mut()
        .map_err(|_| LuaError::runtime("IsoGameCharacter is in use"))?;
    Ok(f(&mut chr.safety))
}

pub(super) fn register(thread: &mut Thread) {
    let b = expose_class(thread, SAFETY_CLASS, None);
    b.method("getCharacter", |c| {
        let chr = owner(c)?;
        Ok(c.push(character_value(&chr)))
    });
    b.method("isEnabled", |c| {
        let v = with(c, |s| s.enabled)?;
        Ok(c.push(v))
    });
    b.method("setEnabled", |c| {
        let v = bool_arg(c, 1);
        with(c, |s| s.enabled = v)?;
        Ok(0)
    });
    b.method("isLast", |c| {
        let v = with(c, |s| s.last)?;
        Ok(c.push(v))
    });
    b.method("setLast", |c| {
        let v = bool_arg(c, 1);
        with(c, |s| s.last = v)?;
        Ok(0)
    });
    b.method("getCooldown", |c| {
        let v = with(c, |s| s.cooldown)?;
        Ok(c.push(Value::from(f64::from(v))))
    });
    b.method("setCooldown", |c| {
        let v = num_arg(c, 1) as f32;
        with(c, |s| s.cooldown = v)?;
        Ok(0)
    });
    b.method("getToggle", |c| {
        let v = with(c, |s| s.toggle)?;
        Ok(c.push(Value::from(f64::from(v))))
    });
    b.method("setToggle", |c| {
        let v = num_arg(c, 1) as f32;
        with(c, |s| s.toggle = v)?;
        Ok(0)
    });
    // copyFrom(other): enabled, last, cooldown and toggle.
    b.method("copyFrom", |c| {
        let other = match c.arg(1) {
            Value::Object(o) => match o.as_any().downcast_ref::<SafetyObject>() {
                Some(s) => s.0.clone(),
                None => return Err(LuaError::runtime(format!("Expected Safety but got {}", o.class_name()))),
            },
            Value::Nil => {
                return Err(LuaError::Runtime {
                    message: "Cannot read field \"enabled\" because \"other\" is null".into(),
                    class: "java.lang.NullPointerException",
                })
            }
            _ => return Err(LuaError::runtime("Expected Safety")),
        };
        let copy = other
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoGameCharacter is in use"))?
            .safety;
        with(c, |s| *s = copy)?;
        Ok(0)
    });
    b.method("getDescription", |c| {
        let v = with(c, |s| s.description())?;
        Ok(c.push(Value::from(v)))
    });
}
