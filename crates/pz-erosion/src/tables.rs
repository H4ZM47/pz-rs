//! Game-code tables the erosion system needs, read at runtime from the
//! installed game's `projectzomboid.jar`. None of these values are part of
//! this repository.
//!
//! Static tables (`Noise2D.perm`, `ErosionMain.soilTable`,
//! `ErosionData.Square.rands`) come from the classes' static initializers
//! through `pz-classfile`. Instance field initializers (the categories'
//! `soilRef` tables and tree / plant / bush definitions, the
//! `ErosionConfig` defaults, `ErosionSeason.names`) are read by symbolically
//! executing the constructors with [`instance_inits`].

use std::sync::Arc;

use pz_classfile::class::{ClassFile, Constant};
use pz_classfile::static_init::descriptor_args;
use pz_classfile::{load_class, static_value, Jar, Value};

/// A value on the constructor evaluator's stack.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    This,
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    New { class: String, args: Vec<Ev> },
    Array(Vec<Ev>),
    Unknown,
}

impl Ev {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Ev::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Ev::Float(v) => Some(*v),
            Ev::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Ev::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Ev]> {
        match self {
            Ev::Array(a) => Some(a),
            _ => None,
        }
    }

    /// An `int[]` literal.
    pub fn int_array(&self) -> Option<Vec<i32>> {
        self.as_array()?
            .iter()
            .map(|v| v.as_int().map(|i| i as i32))
            .collect()
    }

    /// An `int[][]` literal.
    pub fn int_array2(&self) -> Option<Vec<Vec<i32>>> {
        self.as_array()?.iter().map(Ev::int_array).collect()
    }
}

/// Length of the instruction at `pc` (JVMS §6.5).
fn insn_len(code: &[u8], pc: usize) -> Option<usize> {
    let op = code[pc];
    Some(match op {
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
            let rd = |at: usize| {
                code.get(at..at + 4)
                    .map(|s| i32::from_be_bytes(s.try_into().unwrap()))
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

/// Symbolically executes `class`'s first constructor and returns the value
/// each `putfield this.<field>` stores, in execution order. Straight-line
/// code only; anything not modelled becomes [`Ev::Unknown`].
pub fn instance_inits(class: &ClassFile) -> Vec<(String, Ev)> {
    let Some(code) = class.method("<init>").and_then(|m| m.code.as_ref()) else {
        return Vec::new();
    };
    let u16_at = |pc: usize| {
        code.get(pc..pc + 2)
            .map(|s| u16::from_be_bytes([s[0], s[1]]))
            .unwrap_or(0)
    };
    let mut out = Vec::new();
    let mut stack: Vec<Ev> = Vec::new();
    let pop = |s: &mut Vec<Ev>| s.pop().unwrap_or(Ev::Unknown);
    let mut pc = 0;
    while pc < code.len() {
        let op = code[pc];
        match op {
            0x01 => stack.push(Ev::Null),
            0x02..=0x08 => stack.push(Ev::Int(op as i64 - 3)),
            0x09 | 0x0a => stack.push(Ev::Int(op as i64 - 9)),
            0x0b..=0x0d => stack.push(Ev::Float((op - 0x0b) as f64)),
            0x0e | 0x0f => stack.push(Ev::Float((op - 0x0e) as f64)),
            0x10 => stack.push(Ev::Int(code[pc + 1] as i8 as i64)),
            0x11 => stack.push(Ev::Int(u16_at(pc + 1) as i16 as i64)),
            0x12..=0x14 => {
                let idx = if op == 0x12 {
                    code[pc + 1] as u16
                } else {
                    u16_at(pc + 1)
                };
                stack.push(match class.constant(idx) {
                    Ok(Constant::String(s)) => class
                        .utf8(*s)
                        .map(|s| Ev::Str(s.to_string()))
                        .unwrap_or(Ev::Unknown),
                    Ok(Constant::Integer(v)) => Ev::Int(*v as i64),
                    Ok(Constant::Long(v)) => Ev::Int(*v),
                    Ok(Constant::Float(v)) => Ev::Float(*v as f64),
                    Ok(Constant::Double(v)) => Ev::Float(*v),
                    _ => Ev::Unknown,
                });
            }
            0x2a => stack.push(Ev::This),
            0x15..=0x19 | 0x1a..=0x29 | 0x2b..=0x2d => stack.push(Ev::Unknown),
            0x36..=0x3a | 0x3b..=0x4e | 0x57 => {
                pop(&mut stack);
            }
            0x59 => {
                let top = stack.last().cloned().unwrap_or(Ev::Unknown);
                stack.push(top);
            }
            0xbb => {
                let c = class
                    .class_name(u16_at(pc + 1))
                    .map(str::to_string)
                    .unwrap_or_default();
                stack.push(Ev::New {
                    class: c,
                    args: Vec::new(),
                });
            }
            0xb2 => stack.push(Ev::Unknown),
            0xb4 => {
                pop(&mut stack);
                stack.push(Ev::Unknown);
            }
            0xb3 => {
                pop(&mut stack);
            }
            0xb5 => {
                let field = class
                    .member_ref(u16_at(pc + 1))
                    .map(|r| r.name)
                    .unwrap_or_default();
                let value = pop(&mut stack);
                if pop(&mut stack) == Ev::This {
                    out.push((field, value));
                }
            }
            0xb6..=0xb9 => {
                let Ok(r) = class.member_ref(u16_at(pc + 1)) else {
                    stack.clear();
                    pc += insn_len(code, pc).unwrap_or(1);
                    continue;
                };
                let (n, returns) = descriptor_args(&r.descriptor);
                let mut args: Vec<Ev> = (0..n).map(|_| pop(&mut stack)).collect();
                args.reverse();
                if op == 0xb7 && r.name == "<init>" {
                    pop(&mut stack);
                    if let Some(Ev::New { class, args: a }) = stack.last_mut() {
                        if *class == r.class && a.is_empty() {
                            *a = args;
                        }
                    }
                } else {
                    if op != 0xb8 {
                        pop(&mut stack);
                    }
                    if returns {
                        stack.push(Ev::Unknown);
                    }
                }
            }
            0xbc | 0xbd => {
                let len = match pop(&mut stack) {
                    Ev::Int(n) if (0..=1 << 20).contains(&n) => n as usize,
                    _ => 0,
                };
                stack.push(Ev::Array(vec![Ev::Unknown; len]));
            }
            0x4f..=0x56 => {
                let value = pop(&mut stack);
                let index = pop(&mut stack);
                pop(&mut stack);
                if let (Some(Ev::Array(a)), Ev::Int(i)) = (stack.last_mut(), index) {
                    if let Some(slot) = usize::try_from(i).ok().and_then(|i| a.get_mut(i)) {
                        *slot = value;
                    }
                }
            }
            0xc0 => {}
            0xb1 => break,
            _ => stack.clear(),
        }
        pc += insn_len(code, pc).unwrap_or(1);
    }
    out
}

fn value_to_ints(v: &Value) -> Option<Vec<i32>> {
    match v {
        Value::Array(a) => a
            .iter()
            .map(|x| match x {
                Value::Int(i) => Some(*i as i32),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// `NatureTrees.TreeInit`.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeInit {
    pub name: String,
    pub tileset_name: String,
    pub jumbo_tileset_name: String,
    pub jumbo_xl_tileset_name: String,
    pub jumbo_xxl_tileset_name: String,
    pub evergreen: bool,
}

impl TreeInit {
    /// `new TreeInit(name, tilesetName, seasonal)`.
    pub fn new(name: &str, tileset_name: &str, evergreen: bool) -> Self {
        TreeInit {
            name: name.to_string(),
            tileset_name: tileset_name.to_string(),
            jumbo_tileset_name: tileset_name.replace("_1", "JUMBO_1"),
            jumbo_xl_tileset_name: tileset_name.replace("_1", "JUMBOXL_1"),
            jumbo_xxl_tileset_name: tileset_name.replace("_1", "JUMBOXXL_1"),
            evergreen,
        }
    }
}

/// `NaturePlants.PlantInit`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlantInit {
    pub name: String,
    pub has_flower: bool,
    pub bloomstart: f32,
    pub bloomend: f32,
}

/// `NatureBush.BushInit`.
#[derive(Clone, Debug, PartialEq)]
pub struct BushInit {
    pub name: String,
    pub bloomstart: f32,
    pub bloomend: f32,
    pub has_flower: bool,
}

/// Every game-code table the erosion port reads from the jar.
#[derive(Clone, Debug)]
pub struct ErosionTables {
    /// `Noise2D.perm`.
    pub noise_perm: Arc<[i32]>,
    /// `ErosionMain.soilTable`.
    pub soil_table: Vec<Vec<i32>>,
    /// `ErosionData.Square.rands`.
    pub square_rands: Vec<i32>,
    pub trees: Vec<TreeInit>,
    pub trees_soil_ref: Vec<Vec<i32>>,
    pub plants: Vec<PlantInit>,
    pub plants_soil_ref: Vec<Vec<i32>>,
    pub bush: Vec<BushInit>,
    pub bush_soil_ref: Vec<Vec<i32>>,
    /// `Flowerbed.tileId`.
    pub flowerbed_tile_ids: Vec<i32>,
    /// `ErosionSeason.names`.
    pub season_names: Vec<String>,
    /// Field initializers of `ErosionConfig.Seeds/Time/Debug/Season`, by
    /// `"<section>.<field>"` (e.g. `seeds.seedMain0`).
    pub config_defaults: Vec<(String, Ev)>,
}

const ER: &str = "zombie/erosion/";

fn missing(what: &str) -> String {
    format!("erosion table {what} not found in projectzomboid.jar")
}

fn field<'a>(inits: &'a [(String, Ev)], name: &str) -> Option<&'a Ev> {
    inits.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn ctor_args<'a>(v: &'a Ev, class: &str) -> Option<Vec<&'a [Ev]>> {
    v.as_array()?
        .iter()
        .map(|e| match e {
            Ev::New { class: c, args } if c == class => Some(args.as_slice()),
            _ => None,
        })
        .collect()
}

impl ErosionTables {
    /// Reads the tables from an opened `projectzomboid.jar`.
    pub fn from_jar(jar: &Jar) -> Result<Self, String> {
        let e = |x: pz_classfile::Error| x.to_string();
        let statics = |class: &str, f: &str| -> Result<Vec<i32>, String> {
            static_value(jar, class, f)
                .map_err(e)?
                .as_ref()
                .and_then(value_to_ints)
                .ok_or_else(|| missing(&format!("{class}.{f}")))
        };
        let noise_perm: Arc<[i32]> = statics(&format!("{ER}utils/Noise2D"), "perm")?.into();
        if noise_perm.len() < 256 {
            return Err(missing("Noise2D.perm"));
        }
        let soil_table = match static_value(jar, &format!("{ER}ErosionMain"), "soilTable")
            .map_err(e)?
        {
            Some(Value::Array(rows)) => rows.iter().map(value_to_ints).collect::<Option<Vec<_>>>(),
            _ => None,
        }
        .ok_or_else(|| missing("ErosionMain.soilTable"))?;
        let square_rands = statics(&format!("{ER}ErosionData$Square"), "rands")?;
        if square_rands.len() < 8100 {
            return Err(missing("ErosionData.Square.rands"));
        }

        let inits = |class: &str| -> Result<Vec<(String, Ev)>, String> {
            Ok(instance_inits(
                &load_class(jar, &format!("{ER}{class}")).map_err(e)?,
            ))
        };
        let soil_ref = |inits: &[(String, Ev)], class: &str| {
            field(inits, "soilRef")
                .and_then(Ev::int_array2)
                .ok_or_else(|| missing(&format!("{class}.soilRef")))
        };

        let t = inits("categories/NatureTrees")?;
        let trees_soil_ref = soil_ref(&t, "NatureTrees")?;
        let trees = field(&t, "trees")
            .and_then(|v| ctor_args(v, &format!("{ER}categories/NatureTrees$TreeInit")))
            .and_then(|list| {
                list.into_iter()
                    .map(|a| {
                        Some(TreeInit::new(
                            a.first()?.as_str()?,
                            a.get(1)?.as_str()?,
                            a.get(2)?.as_int()? != 0,
                        ))
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| missing("NatureTrees.trees"))?;

        let p = inits("categories/NaturePlants")?;
        let plants_soil_ref = soil_ref(&p, "NaturePlants")?;
        let plants = field(&p, "plants")
            .and_then(|v| ctor_args(v, &format!("{ER}categories/NaturePlants$PlantInit")))
            .and_then(|list| {
                list.into_iter()
                    .map(|a| {
                        Some(PlantInit {
                            name: a.first()?.as_str()?.to_string(),
                            has_flower: a.get(1)?.as_int()? != 0,
                            bloomstart: a.get(2)?.as_float()? as f32,
                            bloomend: a.get(3)?.as_float()? as f32,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| missing("NaturePlants.plants"))?;

        let b = inits("categories/NatureBush")?;
        let bush_soil_ref = soil_ref(&b, "NatureBush")?;
        let bush = field(&b, "bush")
            .and_then(|v| ctor_args(v, &format!("{ER}categories/NatureBush$BushInit")))
            .and_then(|list| {
                list.into_iter()
                    .map(|a| {
                        // BushInit(outer, name, bloomstart, bloomend, hasFlower)
                        Some(BushInit {
                            name: a.get(1)?.as_str()?.to_string(),
                            bloomstart: a.get(2)?.as_float()? as f32,
                            bloomend: a.get(3)?.as_float()? as f32,
                            has_flower: a.get(4)?.as_int()? != 0,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| missing("NatureBush.bush"))?;

        let f = inits("categories/Flowerbed")?;
        let flowerbed_tile_ids = field(&f, "tileId")
            .and_then(Ev::int_array)
            .ok_or_else(|| missing("Flowerbed.tileId"))?;

        let s = inits("season/ErosionSeason")?;
        let season_names = field(&s, "names")
            .and_then(|v| {
                v.as_array()?
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| missing("ErosionSeason.names"))?;

        let mut config_defaults = Vec::new();
        for (section, class) in [
            ("seeds", "Seeds"),
            ("time", "Time"),
            ("debug", "Debug"),
            ("season", "Season"),
        ] {
            for (name, v) in inits(&format!("ErosionConfig${class}"))? {
                config_defaults.push((format!("{section}.{name}"), v));
            }
        }

        Ok(ErosionTables {
            noise_perm,
            soil_table,
            square_rands,
            trees,
            trees_soil_ref,
            plants,
            plants_soil_ref,
            bush,
            bush_soil_ref,
            flowerbed_tile_ids,
            season_names,
            config_defaults,
        })
    }

    /// Opens `projectzomboid.jar` and reads the tables.
    pub fn from_jar_path(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        Self::from_jar(&Jar::open(path).map_err(|e| e.to_string())?)
    }

    /// Default of a config field (`"season.noon"`), if the constructor
    /// assigns one (fields it leaves alone default to 0).
    pub fn config_default(&self, key: &str) -> Option<&Ev> {
        self.config_defaults
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}
