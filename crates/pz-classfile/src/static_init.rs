//! Symbolic evaluation of straight-line method bodies, such as a class's
//! static initializer (`<clinit>`) or a static `init()` that registers
//! built-in data: which value each `putstatic`/`putfield` stores and which
//! calls run for their effect, as expressions over constants, other static
//! fields, static calls and constructor calls. Anything the evaluator does
//! not model (locals, branches, arithmetic) becomes [`Value::Unknown`].

use crate::class::ClassFile;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    /// `getstatic owner.name`.
    Static { class: String, name: String },
    /// A method call; for instance methods the receiver is `args[0]`.
    Call { class: String, name: String, descriptor: String, args: Vec<Value> },
    /// `new class(args)`.
    New { class: String, args: Vec<Value> },
    /// A new array with the elements stored into it so far.
    Array(Vec<Value>),
    Unknown,
}

impl Value {
    /// The string elements of an array value (`None` if any element is not
    /// a string).
    pub fn as_str_array(&self) -> Option<Vec<&str>> {
        match self {
            Value::Array(a) => a.iter().map(Value::as_str).collect(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// One `putstatic` in the initializer, in execution order.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticInit {
    pub class: String,
    pub field: String,
    pub value: Value,
}

/// One effect of an evaluated method body, in execution order.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `putstatic class.field = value`.
    PutStatic(StaticInit),
    /// `putfield object.field = value` (`class` is the field's owner).
    PutField { object: Value, class: String, field: String, value: Value },
    /// A call whose result is unused: a `void` call, or one whose result is
    /// popped. Always a [`Value::Call`].
    Call(Value),
}

/// Number of argument slots in a method descriptor, and whether it returns
/// a value.
pub fn descriptor_args(desc: &str) -> (usize, bool) {
    let b = desc.as_bytes();
    let mut i = 1;
    let mut n = 0;
    while i < b.len() && b[i] != b')' {
        while b[i] == b'[' {
            i += 1;
        }
        if b[i] == b'L' {
            while b[i] != b';' {
                i += 1;
            }
        }
        i += 1;
        n += 1;
    }
    (n, b.get(i + 1) != Some(&b'V'))
}

/// Length of the instruction at `pc` (JVMS §6.5), including operands.
fn insn_len(code: &[u8], pc: usize) -> Result<usize> {
    let op = code[pc];
    Ok(match op {
        0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
        0x11 | 0x13 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb2..=0xb8 | 0xbb | 0xbd | 0xc0 | 0xc1 | 0xc6 | 0xc7 => 3,
        0xc5 => 4,
        0xb9 | 0xba | 0xc8 | 0xc9 => 5,
        0xc4 => {
            if code.get(pc + 1) == Some(&0x84) {
                6
            } else {
                4
            }
        }
        0xaa | 0xab => {
            let base = (pc + 4) & !3;
            let rd = |at: usize| -> Result<i32> {
                code.get(at..at + 4).map(|s| i32::from_be_bytes(s.try_into().unwrap())).ok_or(Error::Truncated)
            };
            if op == 0xaa {
                let lo = rd(base + 4)?;
                let hi = rd(base + 8)?;
                base + 12 + 4 * (hi - lo + 1) as usize - pc
            } else {
                let n = rd(base + 4)?;
                base + 8 + 8 * n as usize - pc
            }
        }
        _ => 1,
    })
}

/// Evaluates `class`'s `<clinit>`; empty if it has none.
pub fn static_inits(class: &ClassFile) -> Result<Vec<StaticInit>> {
    Ok(method_effects(class, "<clinit>")?
        .into_iter()
        .filter_map(|e| match e {
            Effect::PutStatic(i) => Some(i),
            _ => None,
        })
        .collect())
}

/// Evaluates the body of `class`'s first method named `method`; empty if
/// it has no such method or the method has no code.
pub fn method_effects(class: &ClassFile, method: &str) -> Result<Vec<Effect>> {
    let Some(code) = class.method(method).and_then(|m| m.code.as_ref()) else {
        return Ok(Vec::new());
    };
    let u16_at = |pc: usize| -> Result<u16> {
        code.get(pc..pc + 2).map(|s| u16::from_be_bytes([s[0], s[1]])).ok_or(Error::Truncated)
    };
    let mut out = Vec::new();
    let mut stack: Vec<Value> = Vec::new();
    let pop = |stack: &mut Vec<Value>| stack.pop().unwrap_or(Value::Unknown);
    let mut pc = 0;
    while pc < code.len() {
        let op = code[pc];
        match op {
            0x01 => stack.push(Value::Null),
            0x02..=0x08 => stack.push(Value::Int(op as i64 - 3)),
            0x09 | 0x0a => stack.push(Value::Int(op as i64 - 9)),
            0x0b..=0x0d => stack.push(Value::Float((op - 0x0b) as f64)),
            0x0e | 0x0f => stack.push(Value::Float((op - 0x0e) as f64)),
            0x10 => stack.push(Value::Int(code[pc + 1] as i8 as i64)),
            0x11 => stack.push(Value::Int(u16_at(pc + 1)? as i16 as i64)),
            0x12..=0x14 => {
                let idx = if op == 0x12 { code[pc + 1] as u16 } else { u16_at(pc + 1)? };
                stack.push(match class.constant(idx)? {
                    crate::class::Constant::String(s) => Value::Str(class.utf8(*s)?.to_string()),
                    crate::class::Constant::Integer(v) => Value::Int(*v as i64),
                    crate::class::Constant::Long(v) => Value::Int(*v),
                    crate::class::Constant::Float(v) => Value::Float(*v as f64),
                    crate::class::Constant::Double(v) => Value::Float(*v),
                    _ => Value::Unknown,
                });
            }
            0x57 => {
                if let call @ Value::Call { .. } = pop(&mut stack) {
                    out.push(Effect::Call(call));
                }
            }
            0x59 => {
                let top = stack.last().cloned().unwrap_or(Value::Unknown);
                stack.push(top);
            }
            0xbb => {
                let c = class.class_name(u16_at(pc + 1)?)?.to_string();
                stack.push(Value::New { class: c, args: Vec::new() });
            }
            0xb2 => {
                let r = class.member_ref(u16_at(pc + 1)?)?;
                stack.push(Value::Static { class: r.class, name: r.name });
            }
            0xb3 => {
                let r = class.member_ref(u16_at(pc + 1)?)?;
                let value = pop(&mut stack);
                out.push(Effect::PutStatic(StaticInit { class: r.class, field: r.name, value }));
            }
            0xb5 => {
                let r = class.member_ref(u16_at(pc + 1)?)?;
                let value = pop(&mut stack);
                let object = pop(&mut stack);
                out.push(Effect::PutField { object, class: r.class, field: r.name, value });
            }
            0xb6..=0xb9 => {
                let r = class.member_ref(u16_at(pc + 1)?)?;
                let (n, returns) = descriptor_args(&r.descriptor);
                let mut args: Vec<Value> = (0..n).map(|_| pop(&mut stack)).collect();
                args.reverse();
                if op == 0xb7 && r.name == "<init>" {
                    pop(&mut stack);
                    if let Some(Value::New { class, args: a }) = stack.last_mut() {
                        if *class == r.class && a.is_empty() {
                            *a = args;
                        }
                    }
                } else {
                    if op != 0xb8 {
                        args.insert(0, pop(&mut stack));
                    }
                    let call = Value::Call { class: r.class, name: r.name, descriptor: r.descriptor, args };
                    if returns {
                        stack.push(call);
                    } else {
                        out.push(Effect::Call(call));
                    }
                }
            }
            0xbc | 0xbd => {
                let len = match pop(&mut stack) {
                    Value::Int(n) if (0..=1 << 20).contains(&n) => n as usize,
                    _ => 0,
                };
                stack.push(Value::Array(vec![Value::Unknown; len]));
            }
            // iastore..sastore: the array reference below is a `dup` of the
            // one that stays on the stack, so store into that one.
            0x4f..=0x56 => {
                let value = pop(&mut stack);
                let index = pop(&mut stack);
                pop(&mut stack);
                if let (Some(Value::Array(a)), Value::Int(i)) = (stack.last_mut(), index) {
                    if let Some(slot) = usize::try_from(i).ok().and_then(|i| a.get_mut(i)) {
                        *slot = value;
                    }
                }
            }
            0xc0 => {}
            0xb1 => break,
            _ => stack.clear(),
        }
        pc += insn_len(code, pc)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors() {
        assert_eq!(descriptor_args("()V"), (0, false));
        assert_eq!(descriptor_args("(Ljava/lang/String;)LX;"), (1, true));
        assert_eq!(descriptor_args("(I[JLa/B;[[Lc/D;F)Z"), (5, true));
    }
}
