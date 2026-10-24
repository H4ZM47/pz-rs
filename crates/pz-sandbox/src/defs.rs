//! The built-in sandbox option list, read at runtime from the installed
//! `projectzomboid.jar`.
//!
//! The original declares every option as an instance field initialised by
//! `this.newXOption(name, ...)`, sometimes chained with `setTranslation` /
//! `setValueTranslation`, in `SandboxOptions.<init>` and in the
//! constructors of its inner classes (`Basement`, `Map`, `ZombieLore`, ...).
//! The definitions are game data, so instead of copying them this module
//! symbolically executes those constructors and records each option in the
//! order the original adds it.

use pz_classfile::class::Constant;
use pz_classfile::static_init::descriptor_args;
use pz_classfile::{load_class, ClassFile, Error, Jar, Result};

pub const SANDBOX_CLASS: &str = "zombie/SandboxOptions";

/// Constructor arguments of one option, as `SandboxOptions.newXOption`
/// receives them.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionKind {
    Boolean {
        default: bool,
    },
    Integer {
        min: i32,
        max: i32,
        default: i32,
    },
    Double {
        min: f64,
        max: f64,
        default: f64,
    },
    Enum {
        num_values: i32,
        default: i32,
    },
    /// `newEnumOption(name, Class, default)`: a Java enum's constants, the
    /// default being one of them.
    StrongEnum {
        constants: Vec<String>,
        default: String,
    },
    String {
        default: String,
        max_length: i32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionDef {
    pub name: String,
    pub kind: OptionKind,
    pub translation: Option<String>,
    pub value_translation: Option<String>,
}

/// A value on the evaluator's stack; also the argument type of
/// [`ctor_calls`].
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    This,
    Outer,
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    /// A class literal (`Foo.class`).
    Class(String),
    /// `getstatic owner.name`.
    Static {
        class: String,
        name: String,
    },
    /// The result of a call the evaluator does not model.
    Call {
        class: String,
        name: String,
        args: Vec<Arg>,
    },
    Opt(usize),
    /// An object recorded by [`ctor_calls`] (index into its calls).
    Ctor(usize),
    New(String),
    Unknown,
}

use Arg as V;

/// Length of the instruction at `pc` (JVMS §6.5).
fn insn_len(code: &[u8], pc: usize) -> Result<usize> {
    let op = code[pc];
    Ok(match op {
        0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
        0x11
        | 0x13
        | 0x14
        | 0x84
        | 0x99..=0xa8
        | 0xb2..=0xb8
        | 0xbb
        | 0xbd
        | 0xc0
        | 0xc1
        | 0xc6
        | 0xc7 => 3,
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
                code.get(at..at + 4)
                    .map(|s| i32::from_be_bytes(s.try_into().unwrap()))
                    .ok_or(Error::Truncated)
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

struct Reader<'a> {
    jar: &'a Jar,
    out: Vec<OptionDef>,
    /// Record `new <class>(this, ...)` for classes with this prefix.
    ctor_prefix: Option<String>,
    ctors: Vec<(String, Vec<Arg>)>,
    /// Field name -> recorded constructor stored into it.
    fields: std::collections::HashMap<String, usize>,
    /// String arguments of `ArrayList.remove(String)` calls.
    removals: Vec<String>,
}

impl<'a> Reader<'a> {
    fn new(jar: &'a Jar, ctor_prefix: Option<String>) -> Self {
        Reader {
            jar,
            out: Vec::new(),
            ctor_prefix,
            ctors: Vec::new(),
            fields: Default::default(),
            removals: Vec::new(),
        }
    }

    /// Runs `class.<init>`; `inner` is true for an inner class whose first
    /// constructor argument is the outer `SandboxOptions`.
    fn run_init(&mut self, cf: &ClassFile, inner: bool) -> Result<()> {
        let code = cf
            .method("<init>")
            .and_then(|m| m.code.as_ref())
            .ok_or_else(|| Error::Missing(format!("{}.<init>", cf.this_class)))?;
        let u16_at = |pc: usize| -> Result<u16> {
            code.get(pc..pc + 2)
                .map(|s| u16::from_be_bytes([s[0], s[1]]))
                .ok_or(Error::Truncated)
        };
        let mut stack: Vec<V> = Vec::new();
        let pop = |s: &mut Vec<V>| s.pop().unwrap_or(V::Unknown);
        let mut pc = 0;
        while pc < code.len() {
            let op = code[pc];
            match op {
                0x01 => stack.push(V::Null),
                0x02..=0x08 => stack.push(V::Int(op as i64 - 3)),
                0x09 | 0x0a => stack.push(V::Int(op as i64 - 9)),
                0x0b..=0x0d => stack.push(V::Float((op - 0x0b) as f64)),
                0x0e | 0x0f => stack.push(V::Float((op - 0x0e) as f64)),
                0x10 => stack.push(V::Int(code[pc + 1] as i8 as i64)),
                0x11 => stack.push(V::Int(u16_at(pc + 1)? as i16 as i64)),
                0x12..=0x14 => {
                    let idx = if op == 0x12 {
                        code[pc + 1] as u16
                    } else {
                        u16_at(pc + 1)?
                    };
                    stack.push(match cf.constant(idx)? {
                        Constant::String(s) => V::Str(cf.utf8(*s)?.to_string()),
                        Constant::Integer(v) => V::Int(*v as i64),
                        Constant::Long(v) => V::Int(*v),
                        Constant::Float(v) => V::Float(*v as f64),
                        Constant::Double(v) => V::Float(*v),
                        Constant::Class(_) => V::Class(cf.class_name(idx)?.to_string()),
                        _ => V::Unknown,
                    });
                }
                // aload_0 / aload_1
                0x2a => stack.push(V::This),
                0x2b => stack.push(if inner { V::Outer } else { V::Unknown }),
                0x57 => {
                    pop(&mut stack);
                }
                0x59 => {
                    let top = stack.last().cloned().unwrap_or(V::Unknown);
                    stack.push(top);
                }
                0xbb => stack.push(V::New(cf.class_name(u16_at(pc + 1)?)?.to_string())),
                0xb2 => {
                    let r = cf.member_ref(u16_at(pc + 1)?)?;
                    stack.push(V::Static {
                        class: r.class,
                        name: r.name,
                    });
                }
                // getfield: `this.this$0` in an inner class is the outer object.
                0xb4 => {
                    let r = cf.member_ref(u16_at(pc + 1)?)?;
                    let obj = pop(&mut stack);
                    stack.push(if inner && obj == V::This && r.name == "this$0" {
                        V::Outer
                    } else if obj == V::This && self.fields.contains_key(&r.name) {
                        V::Ctor(self.fields[&r.name])
                    } else {
                        V::Unknown
                    });
                }
                // putfield
                0xb5 => {
                    let r = cf.member_ref(u16_at(pc + 1)?)?;
                    let value = pop(&mut stack);
                    let obj = pop(&mut stack);
                    if let (V::This, V::Ctor(i)) = (obj, value) {
                        self.fields.insert(r.name, i);
                    }
                }
                0xb6..=0xb9 => {
                    let r = cf.member_ref(u16_at(pc + 1)?)?;
                    let (n, returns) = descriptor_args(&r.descriptor);
                    let mut args: Vec<V> = (0..n).map(|_| pop(&mut stack)).collect();
                    args.reverse();
                    if op == 0xb7 && r.name == "<init>" {
                        let target = pop(&mut stack);
                        if let V::New(class) = &target {
                            let mut recorded = None;
                            if let Some(prefix) = &self.ctor_prefix {
                                if class.starts_with(prefix.as_str())
                                    && args.first() == Some(&V::This)
                                {
                                    self.ctors.push((class.clone(), args[1..].to_vec()));
                                    recorded = Some(self.ctors.len() - 1);
                                }
                            }
                            // `new Inner(this)`: run the inner constructor now.
                            if class.starts_with(&format!("{SANDBOX_CLASS}$"))
                                && args.first() == Some(&V::This)
                            {
                                let inner_cf = load_class(self.jar, class)?;
                                self.run_init(&inner_cf, true)?;
                            }
                            // The `dup`ed reference stays as the new object.
                            if let Some(top) = stack.last_mut() {
                                if *top == target {
                                    *top = recorded.map_or_else(|| V::New(class.clone()), V::Ctor);
                                }
                            }
                        }
                        pc += insn_len(code, pc)?;
                        continue;
                    }
                    let recv = if op != 0xb8 {
                        pop(&mut stack)
                    } else {
                        V::Unknown
                    };
                    let result = self.call(&recv, &r.class, &r.name, &args)?;
                    if returns {
                        stack.push(result);
                    }
                }
                0xc0 => {}
                0xb1 => break,
                _ => stack.clear(),
            }
            pc += insn_len(code, pc)?;
        }
        Ok(())
    }

    fn call(&mut self, recv: &V, class: &str, name: &str, args: &[V]) -> Result<V> {
        let owner = matches!(recv, V::This | V::Outer) && class == SANDBOX_CLASS;
        let s = |v: &V| match v {
            V::Str(s) => Some(s.clone()),
            _ => None,
        };
        let i = |v: &V| match v {
            V::Int(n) => Some(*n),
            _ => None,
        };
        let f = |v: &V| match v {
            V::Float(x) => Some(*x),
            V::Int(n) => Some(*n as f64),
            _ => None,
        };
        let bad = || Error::Unsupported(format!("SandboxOptions.{name} with arguments {args:?}"));
        if owner && name.starts_with("new") && name.ends_with("Option") {
            let opt_name = args.first().and_then(s).ok_or_else(bad)?;
            let kind = match (name, args.len()) {
                ("newBooleanOption", 2) => OptionKind::Boolean {
                    default: i(&args[1]).ok_or_else(bad)? != 0,
                },
                ("newIntegerOption", 4) => OptionKind::Integer {
                    min: i(&args[1]).ok_or_else(bad)? as i32,
                    max: i(&args[2]).ok_or_else(bad)? as i32,
                    default: i(&args[3]).ok_or_else(bad)? as i32,
                },
                ("newDoubleOption", 4) => OptionKind::Double {
                    min: f(&args[1]).ok_or_else(bad)?,
                    max: f(&args[2]).ok_or_else(bad)?,
                    default: f(&args[3]).ok_or_else(bad)?,
                },
                ("newEnumOption", 3) => match (&args[1], &args[2]) {
                    (V::Class(enum_class), V::Static { name: default, .. }) => {
                        let constants = pz_classfile::enum_constants(self.jar, enum_class)?;
                        OptionKind::StrongEnum {
                            constants,
                            default: default.clone(),
                        }
                    }
                    (a, b) => OptionKind::Enum {
                        num_values: i(a).ok_or_else(bad)? as i32,
                        default: i(b).ok_or_else(bad)? as i32,
                    },
                },
                ("newStringOption", 3) => OptionKind::String {
                    default: s(&args[1]).ok_or_else(bad)?,
                    max_length: i(&args[2]).ok_or_else(bad)? as i32,
                },
                _ => return Err(bad()),
            };
            self.out.push(OptionDef {
                name: opt_name,
                kind,
                translation: None,
                value_translation: None,
            });
            return Ok(V::Opt(self.out.len() - 1));
        }
        if let V::Ctor(idx) = recv {
            // `option.getName()`: the option name is the first argument.
            if name == "getName" {
                return Ok(self.ctors[*idx].1.first().cloned().unwrap_or(V::Unknown));
            }
        }
        if class == "java/util/ArrayList" && name == "remove" {
            if let Some(V::Str(s)) = args.first() {
                self.removals.push(s.clone());
            }
        }
        if let V::Opt(idx) = recv {
            match name {
                "setTranslation" => self.out[*idx].translation = args.first().and_then(s),
                "setValueTranslation" => {
                    self.out[*idx].value_translation = args.first().and_then(s)
                }
                _ => {}
            }
            return Ok(V::Opt(*idx));
        }
        let mut all = Vec::with_capacity(args.len() + 1);
        if !matches!(recv, V::Unknown) {
            all.push(recv.clone());
        }
        all.extend_from_slice(args);
        Ok(V::Call {
            class: class.to_string(),
            name: name.to_string(),
            args: all,
        })
    }
}

/// The built-in options of `zombie.SandboxOptions`, in the order the
/// original's constructor adds them.
pub fn builtin_options(jar: &Jar) -> Result<Vec<OptionDef>> {
    let cf = load_class(jar, SANDBOX_CLASS)?;
    let mut r = Reader::new(jar, None);
    r.run_init(&cf, false)?;
    Ok(r.out)
}

/// Every `new <C>(this, args...)` that `class.<init>` performs for a class
/// `C` whose name starts with `prefix`, in execution order, with the
/// arguments after `this` (how `ServerOptions` declares its options).
pub fn ctor_calls(jar: &Jar, class: &str, prefix: &str) -> Result<CtorScan> {
    let cf = load_class(jar, class)?;
    let mut r = Reader::new(jar, Some(prefix.to_string()));
    r.run_init(&cf, false)?;
    Ok(CtorScan {
        calls: r.ctors,
        list_removals: r.removals,
    })
}

/// What [`ctor_calls`] found in a constructor.
#[derive(Clone, Debug, Default)]
pub struct CtorScan {
    /// `(class, arguments after this)` for each recorded `new`.
    pub calls: Vec<(String, Vec<Arg>)>,
    /// Strings passed to `ArrayList.remove`, in order (option names when the
    /// constructor removes `this.option.getName()`).
    pub list_removals: Vec<String>,
}
