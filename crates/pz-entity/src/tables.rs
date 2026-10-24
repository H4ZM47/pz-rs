//! Built-in registries the original defines in code, read from the
//! installed `projectzomboid.jar` at runtime (never shipped): the attribute
//! types `Attribute.<clinit>` registers and the fluids `Fluid.<clinit>`
//! creates for `FluidType` constants.

use std::collections::BTreeMap;

use pz_classfile::{Jar, Value};

use crate::context::AttributeKind;

const ATTRIBUTE: &str = "zombie/entity/components/attributes/Attribute";
const ATTRIBUTE_TYPE: &str = "zombie/entity/components/attributes/AttributeType$";
const FLUID: &str = "zombie/entity/components/fluids/Fluid";
const COLORS: &str = "zombie/core/Colors";
const COLOR: &str = "zombie/core/Color";
const FLUID_TYPE: &str = "zombie/entity/components/fluids/FluidType";

/// One registered `AttributeType`.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeTypeInfo {
    pub id: i16,
    pub name: String,
    pub kind: AttributeKind,
    /// The constructor's initial value for numeric, string and bool kinds.
    pub initial: Option<InitialValue>,
}

/// A constant constructor argument.
#[derive(Clone, Debug, PartialEq)]
pub enum InitialValue {
    Int(i64),
    Float(f64),
    Str(String),
}

/// `Attribute`'s registry (`attributeTypeIdMap`).
#[derive(Clone, Debug, Default)]
pub struct AttributeTypes {
    by_id: BTreeMap<i16, AttributeTypeInfo>,
}

impl AttributeTypes {
    /// The types `Attribute.<clinit>` passes to `registerType`.
    pub fn from_jar(jar: &Jar) -> pz_classfile::Result<AttributeTypes> {
        let cf = pz_classfile::load_class(jar, ATTRIBUTE)?;
        let mut out = AttributeTypes::default();
        for init in pz_classfile::static_inits(&cf)? {
            let Value::Call { name, args, .. } = &init.value else {
                continue;
            };
            if name != "registerType" {
                continue;
            }
            let Some(Value::New { class, args }) = args.first() else {
                continue;
            };
            let Some(kind) = class
                .strip_prefix(ATTRIBUTE_TYPE)
                .and_then(AttributeKind::from_class_suffix)
            else {
                continue;
            };
            let (Some(Value::Int(id)), Some(Value::Str(n))) = (args.first(), args.get(1)) else {
                continue;
            };
            let initial = match (kind, args.get(2)) {
                (
                    AttributeKind::Enum | AttributeKind::EnumSet | AttributeKind::EnumStringSet,
                    _,
                ) => None,
                (_, Some(Value::Int(v))) => Some(InitialValue::Int(*v)),
                (_, Some(Value::Float(v))) => Some(InitialValue::Float(*v)),
                (_, Some(Value::Str(v))) => Some(InitialValue::Str(v.clone())),
                _ => None,
            };
            out.insert(AttributeTypeInfo {
                id: *id as i16,
                name: n.clone(),
                kind,
                initial,
            });
        }
        if out.by_id.is_empty() {
            return Err(pz_classfile::Error::Missing(
                "Attribute.registerType calls".into(),
            ));
        }
        Ok(out)
    }

    pub fn insert(&mut self, info: AttributeTypeInfo) {
        self.by_id.insert(info.id, info);
    }

    pub fn get(&self, id: i16) -> Option<&AttributeTypeInfo> {
        self.by_id.get(&id)
    }

    pub fn kind(&self, id: i16) -> Option<AttributeKind> {
        self.get(id).map(|t| t.kind)
    }

    pub fn by_name(&self, name: &str) -> Option<&AttributeTypeInfo> {
        self.by_id
            .values()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn iter(&self) -> impl Iterator<Item = &AttributeTypeInfo> {
        self.by_id.values()
    }
}

/// The `FluidType` constant names `Fluid.<clinit>` creates a `Fluid` for
/// (`fluidEnumMap`), in creation order.
pub fn enum_fluid_names(jar: &Jar) -> pz_classfile::Result<Vec<String>> {
    let cf = pz_classfile::load_class(jar, FLUID)?;
    let mut out = Vec::new();
    for init in pz_classfile::static_inits(&cf)? {
        let Value::Call { name, args, .. } = &init.value else {
            continue;
        };
        if name != "addFluid" {
            continue;
        }
        if let Some(Value::Static { class, name }) = args.first() {
            if class == FLUID_TYPE {
                out.push(name.clone());
            }
        }
    }
    if out.is_empty() {
        return Err(pz_classfile::Error::Missing("Fluid.addFluid calls".into()));
    }
    Ok(out)
}

/// `Colors.colorMap`: the colours `Colors.<clinit>` passes to `addColor`
/// and `AddGameColor` (not the colour-blind set), keyed by lower-case name,
/// as `r, g, b, a`.
pub fn named_colors(
    jar: &Jar,
) -> pz_classfile::Result<std::collections::HashMap<String, [f32; 4]>> {
    let cf = pz_classfile::load_class(jar, COLORS)?;
    let mut out = std::collections::HashMap::new();
    for init in pz_classfile::static_inits(&cf)? {
        let Value::Call { name, args, .. } = &init.value else {
            continue;
        };
        if name != "addColor" && name != "AddGameColor" {
            continue;
        }
        let (Some(Value::Str(n)), Some(Value::New { class, args: c })) =
            (args.first(), args.get(1))
        else {
            continue;
        };
        if class != COLOR {
            continue;
        }
        let f: Vec<f32> = c
            .iter()
            .filter_map(|v| match v {
                Value::Float(f) => Some(*f as f32),
                Value::Int(i) => Some(*i as f32),
                _ => None,
            })
            .collect();
        let rgba = match f.len() {
            3 => [f[0], f[1], f[2], 1.0],
            4 => [f[0], f[1], f[2], f[3]],
            _ => continue,
        };
        out.insert(n.to_lowercase(), rgba);
    }
    if out.is_empty() {
        return Err(pz_classfile::Error::Missing("Colors.addColor calls".into()));
    }
    Ok(out)
}
