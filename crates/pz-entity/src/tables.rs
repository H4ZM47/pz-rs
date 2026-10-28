//! Built-in registries the original defines in code, read from the
//! installed `projectzomboid.jar` at runtime (never shipped): the attribute
//! types `Attribute.<clinit>` registers and the fluids `Fluid.<clinit>`
//! creates for `FluidType` constants.

use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::ops::Deref;
use std::rc::Rc;

use pz_classfile::{Jar, Value};

use crate::context::{AttributeKind, Lookup};

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
    /// `isHiddenUI()`: constructed with `Attribute.UI.Display.Hidden`.
    pub hidden_ui: bool,
}

/// A constant constructor argument.
#[derive(Clone, Debug, PartialEq)]
pub enum InitialValue {
    Int(i64),
    Float(f64),
    Str(String),
}

/// The actual registered type object. Instances retain this allocation;
/// detached AttributeTypeInfo values are explicit presentation metadata.
pub type AttributeTypeRef = Rc<RefCell<AttributeTypeDefinition>>;

#[derive(Clone, Debug, PartialEq)]
pub struct NumericAttributeBounds {
    pub min: InitialValue,
    pub max: InitialValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumericAttributeValidation {
    pub requires_validation: bool,
    pub bounds: Option<NumericAttributeBounds>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributeTypeDefinition {
    info: AttributeTypeInfo,
    pub read_only: bool,
    /// Unknown imported metadata is distinct from native requires=false.
    pub numeric_validation: Lookup<NumericAttributeValidation>,
}

impl Deref for AttributeTypeDefinition {
    type Target = AttributeTypeInfo;
    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

/// `Attribute`'s registry (`attributeTypeIdMap`).
#[derive(Clone, Debug, Default)]
pub struct AttributeTypes {
    by_id: BTreeMap<i16, AttributeTypeRef>,
    by_name: HashMap<String, AttributeTypeRef>,
}

impl AttributeTypes {
    /// The types `Attribute.<clinit>` passes to `registerType`.
    pub fn from_jar(jar: &Jar) -> pz_classfile::Result<AttributeTypes> {
        let cf = pz_classfile::load_class(jar, ATTRIBUTE)?;
        let mut out = AttributeTypes::default();
        let effects = pz_classfile::method_effects(&cf, "<clinit>")?;
        let mut fields = HashMap::new();
        for effect in effects {
            if let pz_classfile::Effect::Call(Value::Call { name, args, .. }) = &effect {
                if name == "setBounds" {
                    apply_static_attribute_bounds(&fields, args)?;
                }
            }
            let pz_classfile::Effect::PutStatic(init) = effect else {
                continue;
            };
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
            let hidden_ui = args.iter().any(|a| {
                matches!(a, Value::Static { class, name }
                    if class.ends_with("Attribute$UI$Display") && name == "Hidden")
            });
            let numeric_validation = match kind {
                AttributeKind::Long
                | AttributeKind::Int
                | AttributeKind::Short
                | AttributeKind::Byte
                | AttributeKind::Double
                | AttributeKind::Float => Lookup::Found(NumericAttributeValidation {
                    requires_validation: false,
                    bounds: None,
                }),
                _ => Lookup::Unknown,
            };
            // Numeric/String/Bool overloads forward the fourth boolean as
            // readOnly. The three-argument overload supplies false.
            let read_only = matches!(args.get(3), Some(Value::Int(value)) if *value != 0);
            let registered = out.insert_with_validation(
                AttributeTypeInfo {
                    id: *id as i16,
                    name: n.clone(),
                    kind,
                    initial,
                    hidden_ui,
                },
                read_only,
                numeric_validation,
            );
            fields.insert(init.field, registered);
        }
        if out.by_id.is_empty() {
            return Err(pz_classfile::Error::Missing(
                "Attribute.registerType calls".into(),
            ));
        }
        Ok(out)
    }

    pub fn insert(&mut self, info: AttributeTypeInfo) {
        self.insert_with_validation(info, false, Lookup::Unknown);
    }

    /// Explicit registry construction; metadata imports do not invent numeric
    /// validation. Native construction supplies the decoded class state.
    pub fn insert_with_validation(
        &mut self,
        info: AttributeTypeInfo,
        read_only: bool,
        numeric_validation: Lookup<NumericAttributeValidation>,
    ) -> AttributeTypeRef {
        let id = info.id;
        let name = info.name.to_ascii_lowercase();
        let definition = Rc::new(RefCell::new(AttributeTypeDefinition {
            info,
            read_only,
            numeric_validation,
        }));
        if let Some(old) = self.by_id.insert(id, definition.clone()) {
            self.by_name.retain(|_, value| !Rc::ptr_eq(value, &old));
        }
        self.by_name.insert(name, definition.clone());
        definition
    }

    pub fn get_ref(&self, id: i16) -> Option<AttributeTypeRef> {
        self.by_id.get(&id).cloned()
    }

    pub fn by_name_ref(&self, name: &str) -> Option<AttributeTypeRef> {
        self.by_name.get(&name.to_ascii_lowercase()).cloned()
    }

    pub fn get(&self, id: i16) -> Option<Ref<'_, AttributeTypeInfo>> {
        self.by_id
            .get(&id)
            .map(|value| Ref::map(value.borrow(), |definition| &definition.info))
    }

    pub fn kind(&self, id: i16) -> Option<AttributeKind> {
        self.get(id).map(|t| t.kind)
    }

    pub fn by_name(&self, name: &str) -> Option<Ref<'_, AttributeTypeInfo>> {
        self.by_name
            .get(&name.to_ascii_lowercase())
            .map(|value| Ref::map(value.borrow(), |definition| &definition.info))
    }

    pub fn iter(&self) -> impl Iterator<Item = Ref<'_, AttributeTypeInfo>> {
        self.by_id
            .values()
            .map(|value| Ref::map(value.borrow(), |definition| &definition.info))
    }
}

fn numeric_constant(value: &Value) -> Option<InitialValue> {
    match value {
        Value::Int(value) => Some(InitialValue::Int(*value)),
        Value::Float(value) => Some(InitialValue::Float(*value)),
        Value::Call {
            class, name, args, ..
        } if class.starts_with("java/lang/") && name == "valueOf" && args.len() == 1 => {
            numeric_constant(&args[0])
        }
        _ => None,
    }
}

fn apply_static_attribute_bounds(
    fields: &HashMap<String, AttributeTypeRef>,
    args: &[Value],
) -> pz_classfile::Result<()> {
    let [Value::Static { class, name }, min, max] = args else {
        return Err(pz_classfile::Error::Missing(
            "Attribute setBounds receiver/arguments".into(),
        ));
    };
    if class != ATTRIBUTE {
        return Err(pz_classfile::Error::Missing(
            "Attribute setBounds static owner".into(),
        ));
    }
    let definition = fields
        .get(name)
        .ok_or_else(|| pz_classfile::Error::Missing(format!("Attribute setBounds type {name}")))?;
    let min = numeric_constant(min).ok_or_else(|| {
        pz_classfile::Error::Missing(format!("Attribute setBounds minimum {name}"))
    })?;
    let max = numeric_constant(max).ok_or_else(|| {
        pz_classfile::Error::Missing(format!("Attribute setBounds maximum {name}"))
    })?;
    let mut definition = definition.borrow_mut();
    let Lookup::Found(validation) = &mut definition.numeric_validation else {
        return Err(pz_classfile::Error::Missing(format!(
            "Attribute numeric constructor {name}"
        )));
    };
    validation.requires_validation = true;
    // Native setBounds allocates NumericVars only once. Later calls retain
    // those same bounds; native class initialization uses valid constants.
    if validation.bounds.is_none() {
        validation.bounds = Some(NumericAttributeBounds { min, max });
    }
    Ok(())
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

#[cfg(test)]
#[path = "tables/attribute_tests.rs"]
mod attribute_tests;
