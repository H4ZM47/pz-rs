//! The item's entity components in its tooltip: the `AttributeContainer`
//! (`Sharpness`, `HeadCondition`, `TimesHeadRepaired` and the attribute
//! rows), the `FluidContainer` (`getUiName` for the item's name and its
//! rows) and `Durability`, each `DoTooltip(tooltipUI, layout)`.

use super::{fluid_text, tr, trc, Cx, Row, Tip, LABEL, WHITE};
use crate::bind_items::{item_env, ItemRef};
use pz_entity::components::attributes::AttributeValue;
use pz_entity::fluids::{FluidContainer, FluidInstance, FluidRef};
use pz_entity::{Component, EntityContext, Lookup};
use pz_inventory::InventoryItem;
use pz_scripts::objects::definitions::{FluidDefinitionScript, PoisonEffect};
use pz_scripts::ScriptType;
use pz_ui::tooltip::Layout;
use std::cell::RefCell;
use std::rc::Rc;

// ---- Attributes -----------------------------------------------------------

fn attr(it: &InventoryItem, name: &str) -> Option<AttributeValue> {
    let e = item_env();
    it.get_attribute(e.entity()?, name)
}

fn int_of(v: &AttributeValue) -> i32 {
    match v {
        AttributeValue::Int(i) => *i,
        AttributeValue::Short(i) => *i as i32,
        AttributeValue::Byte(i) => *i as i32,
        AttributeValue::Long(i) => *i as i32,
        AttributeValue::Float(f) => *f as i32,
        AttributeValue::Double(f) => *f as i32,
        _ => 0,
    }
}

fn float_of(v: &AttributeValue) -> f32 {
    match v {
        AttributeValue::Float(f) => *f,
        AttributeValue::Double(f) => *f as f32,
        other => int_of(other) as f32,
    }
}

/// `hasSharpness()`.
pub fn has_sharpness(it: &InventoryItem) -> bool {
    attr(it, "Sharpness").is_some()
}

/// `hasHeadCondition()`.
pub fn has_head_condition(it: &InventoryItem) -> bool {
    attr(it, "HeadCondition").is_some()
}

/// `hasTimesHeadRepaired()`.
pub fn has_times_head_repaired(it: &InventoryItem) -> bool {
    attr(it, "TimesHeadRepaired").is_some()
}

/// `getTimesHeadRepaired()`.
pub fn times_head_repaired(it: &InventoryItem) -> i32 {
    match attr(it, "TimesHeadRepaired") {
        Some(v) => int_of(&v),
        None => it.have_been_repaired,
    }
}

/// `getHeadCondition()`.
pub fn head_condition(it: &InventoryItem) -> i32 {
    attr(it, "HeadCondition").map_or(0, |v| int_of(&v))
}

/// `getHeadConditionMax()`.
pub fn head_condition_max(it: &InventoryItem) -> i32 {
    if !has_head_condition(it) {
        return 0;
    }
    match attr(it, "HeadConditionMax") {
        Some(v) => int_of(&v),
        None => it.condition_max,
    }
}

/// `getMaxSharpness()`.
fn max_sharpness(it: &InventoryItem) -> f32 {
    if has_head_condition(it) {
        return head_condition(it) as f32 / head_condition_max(it) as f32;
    }
    it.condition as f32 / it.condition_max as f32
}

/// The `Sharpness` attribute's value as stored.
pub fn raw_sharpness(it: &InventoryItem) -> Option<f32> {
    attr(it, "Sharpness").map(|v| float_of(&v))
}

/// `getSharpness()` without its write-back.
pub fn sharpness(it: &InventoryItem) -> f32 {
    match attr(it, "Sharpness") {
        Some(v) => float_of(&v).min(max_sharpness(it)),
        None => 0.0,
    }
}

fn actual_attribute_ref(
    item: &ItemRef,
    context: &dyn EntityContext,
    name: &str,
) -> Result<Option<pz_entity::runtime_attributes::AttributeRef>, pz_entity::RuntimeError> {
    use pz_entity::{ComponentType, RuntimeError};
    let component = {
        let item = item
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("InventoryItem"))?;
        item.get_attributes_ref()?
    };
    let Some(component) = component else {
        return Ok(None);
    };
    let type_ref = match context.attribute_type_by_name_ref(name) {
        Lookup::Found(type_ref) => type_ref,
        Lookup::Missing | Lookup::Unknown => {
            return Err(RuntimeError::UnsupportedRuntime(
                "native attribute type registry is unavailable",
            ))
        }
    };
    let body = component
        .try_borrow()
        .map_err(|_| RuntimeError::BorrowedComponent {
            index: None,
            kind: ComponentType::Attributes,
        })?;
    let attributes = body
        .attributes()
        .ok_or(RuntimeError::InvalidBody(ComponentType::Attributes))?;
    Ok(attributes.get_ref_by_type(&type_ref)?.cloned())
}

fn actual_attribute_value(
    attribute: &pz_entity::runtime_attributes::AttributeRef,
) -> Result<AttributeValue, pz_entity::RuntimeError> {
    attribute
        .try_borrow()
        .map(|instance| instance.value_snapshot())
        .map_err(|_| pz_entity::RuntimeError::BorrowedTransient("AttributeInstance"))
}

fn actual_max_sharpness(
    item: &ItemRef,
    context: &dyn EntityContext,
) -> Result<f32, pz_entity::RuntimeError> {
    use pz_entity::RuntimeError;
    if actual_attribute_ref(item, context, "Sharpness")?.is_none() {
        return Ok(1.0);
    }
    if let Some(head) = actual_attribute_ref(item, context, "HeadCondition")? {
        let head = int_of(&actual_attribute_value(&head)?);
        let maximum = match actual_attribute_ref(item, context, "HeadConditionMax")? {
            Some(maximum) => int_of(&actual_attribute_value(&maximum)?),
            None => {
                item.try_borrow()
                    .map_err(|_| RuntimeError::BorrowedTransient("InventoryItem"))?
                    .condition_max
            }
        };
        Ok(head as f32 / maximum as f32)
    } else {
        let item = item
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("InventoryItem"))?;
        Ok(item.condition as f32 / item.condition_max as f32)
    }
}

fn actual_set_sharpness(
    item: &ItemRef,
    context: &dyn EntityContext,
    mut value: f32,
    notices: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
) -> Result<(), pz_entity::RuntimeError> {
    use pz_entity::RuntimeError;
    if actual_attribute_ref(item, context, "Sharpness")?.is_none() {
        return Ok(());
    }
    let maximum = actual_max_sharpness(item, context)?;
    if value > maximum {
        value = maximum;
    }
    if value < 0.0 {
        value = 0.0;
    }
    if value > 1.0 {
        value = 1.0;
    }
    let value = pz_core::config::float_to_string(value);
    let instance = actual_attribute_ref(item, context, "Sharpness")?.ok_or_else(|| {
        RuntimeError::Codec(pz_entity::EntityError("NullPointerException".into()))
    })?;
    // Native uses the existing instance's script-string setter, including its
    // actual type validation. It does not get-or-add or replace the instance.
    instance
        .try_borrow_mut()
        .map_err(|_| RuntimeError::BorrowedTransient("AttributeInstance"))?
        .set_value_from_script_string(&value, notices)?;
    Ok(())
}

fn actual_get_sharpness(
    item: &ItemRef,
    context: &dyn EntityContext,
    notices: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
) -> Result<f32, pz_entity::RuntimeError> {
    use pz_entity::RuntimeError;
    let Some(instance) = actual_attribute_ref(item, context, "Sharpness")? else {
        return Ok(0.0);
    };
    let sharpness = float_of(&actual_attribute_value(&instance)?);
    let maximum = actual_max_sharpness(item, context)?;
    // fcmpl/ifle skips the native write for either NaN operand and equal
    // signed zeros. Only the ordered greater-than relation enters it.
    if sharpness > maximum {
        // applyMaxSharpness checks presence and reads max again; the setter
        // then performs its own third max read before the ordered clamps.
        if actual_attribute_ref(item, context, "Sharpness")?.is_some() {
            let maximum = actual_max_sharpness(item, context)?;
            actual_set_sharpness(item, context, maximum, notices)?;
        }
    }
    let instance = actual_attribute_ref(item, context, "Sharpness")?.ok_or_else(|| {
        RuntimeError::Codec(pz_entity::EntityError("NullPointerException".into()))
    })?;
    Ok(float_of(&actual_attribute_value(&instance)?))
}

/// Fallible native presence: this checks the actual slot without reading a
/// scalar or collapsing a held component/instance into genuine absence.
pub fn has_sharpness_ref(item: &ItemRef) -> pz_entity::runtime_load::HostResult<bool> {
    use pz_entity::runtime_load::RuntimeHostError;
    let env = item_env();
    match env.entity() {
        Some(context) => actual_attribute_ref(item, context, "Sharpness")
            .map(|value| value.is_some())
            .map_err(RuntimeHostError::Storage),
        None => {
            let item = item
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("InventoryItem"))?;
            if item
                .get_attributes_ref()
                .map_err(RuntimeHostError::Storage)?
                .is_none()
            {
                Ok(false)
            } else {
                Err(RuntimeHostError::Unsupported {
                    operation: "native Sharpness attribute registry",
                    kind: Some(pz_entity::ComponentType::Attributes),
                })
            }
        }
    }
}

/// Native getSharpness over the actual item and instance. All guards end
/// before owned diagnostics reach the host logger, on success and failure.
pub fn sharpness_mut(item: &ItemRef) -> pz_entity::runtime_load::HostResult<f32> {
    use pz_entity::runtime_load::RuntimeHostError;
    let env = item_env();
    let Some(context) = env.entity() else {
        // No attribute component is genuine absence even before a context is
        // installed. A represented component without its registry is explicit.
        has_sharpness_ref(item)?;
        return Ok(0.0);
    };
    let mut notices = Vec::new();
    let result =
        actual_get_sharpness(item, context, &mut notices).map_err(RuntimeHostError::Storage);
    pz_inventory::component_host::emit_native_notices(&notices)?;
    result
}

/// `AttributeInstance.stringValue()`.
fn string_value(v: &AttributeValue) -> String {
    match v {
        AttributeValue::Long(i) => i.to_string(),
        AttributeValue::Int(i) => i.to_string(),
        AttributeValue::Short(i) => i.to_string(),
        AttributeValue::Byte(i) => i.to_string(),
        AttributeValue::Double(d) => pz_core::config::double_to_string(*d),
        AttributeValue::Float(f) => super::fstr(*f),
        AttributeValue::String(s) => s.clone(),
        AttributeValue::Bool(b) => b.to_string(),
        // TODO(attributes): enum attribute values print their constants;
        // no item script sets one.
        AttributeValue::Enum(b) => b.to_string(),
        AttributeValue::EnumSet(v) => format!("{v:?}"),
        AttributeValue::EnumStringSet { enums, strings } => format!("{enums:?}{strings:?}"),
    }
}

/// `AttributeType.getNameUI()`: `Attribute_Type_<name>` translated, else
/// the name. (No attribute registered here has a tooltip override.)
fn name_ui(name: &str) -> String {
    let key = format!("Attribute_Type_{name}");
    crate::bind_text::with_translator(|t| t.get_attribute_text(&key, true))
        .flatten()
        .unwrap_or_else(|| name.to_string())
}

/// `AttributeContainer.DoTooltip`: the visible attributes by UI name
/// (every attribute the game registers is hidden; the debug option lists
/// them). Numeric bars need the types' bounds, which no registered type
/// sets (TODO(attributes): `getDisplayAsBarUnit`).
fn attributes_tooltip(cx: &Cx<'_>, it: &InventoryItem, layout: &mut Layout) {
    let e = item_env();
    let Some(ctx) = e.entity() else {
        return;
    };
    let Some(comps) = it.decode_components(ctx) else {
        return;
    };
    let Some(a) = comps.list.iter().find_map(|c| match c {
        Component::Attributes(a) => Some(a),
        _ => None,
    }) else {
        return;
    };
    if a.attributes.is_empty() {
        return;
    }
    let info = |id: i16| ctx.attribute_type(id);
    let row = |layout: &mut Layout, name: &str, v: &AttributeValue| {
        layout
            .add_item()
            .label(&format!("{}:", name_ui(name)), LABEL)
            .value(&string_value(v), WHITE);
    };
    let mut visible: Vec<(String, &AttributeValue)> = Vec::new();
    let mut hidden: Vec<(String, &AttributeValue)> = Vec::new();
    for x in &a.attributes {
        let Some(t) = info(x.id) else {
            continue;
        };
        if t.hidden_ui {
            hidden.push((t.name.clone(), &x.value));
        } else {
            visible.push((t.name.clone(), &x.value));
        }
    }
    let blue = super::cornflower_blue(cx.t);
    if cx.opts.tooltip_attributes {
        layout
            .add_item()
            .label("[Debug Begin Attributes]", [blue[0], blue[1], blue[2], 1.0]);
    }
    visible.sort_by(|a, b| java_compare(&name_ui(&a.0), &name_ui(&b.0)));
    for (n, v) in &visible {
        row(layout, n, v);
    }
    if cx.opts.tooltip_attributes {
        if !hidden.is_empty() {
            layout.add_item().label(
                "[Debug Hidden Attributes]",
                [blue[0], blue[1], blue[2], 1.0],
            );
            for (n, v) in &hidden {
                row(layout, n, v);
            }
        }
        layout
            .add_item()
            .label("[Debug End Attributes]", [blue[0], blue[1], blue[2], 1.0]);
    }
}

/// `String.compareTo` (UTF-16 code units).
fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

// ---- Blood of bags --------------------------------------------------------

/// `InventoryContainer.getBloodLevel()`: the visual's blood over the
/// covered parts.
pub fn container_blood_level(it: &InventoryItem) -> f32 {
    let e = item_env();
    let Some(assets) = e.context().assets() else {
        return 0.0;
    };
    let parts = it.get_covered_parts(assets).unwrap_or_default();
    let Some(v) = &it.visual else {
        return 0.0;
    };
    parts.iter().fold(0.0f32, |a, &p| a + v.get_blood(p))
}

// ---- Fluids ---------------------------------------------------------------

/// What the tooltip reads of a `FluidInstance`'s `Fluid`.
struct FluidView {
    /// The fluid's name (`FluidType` name or modded type string); `None`
    /// for a fluid the registry does not know.
    name: Option<String>,
    translated: String,
    poison: Option<PoisonEffect>,
    color: [f32; 3],
    amount: f32,
}

fn definition(name: &str) -> Option<Rc<RefCell<FluidDefinitionScript>>> {
    let sm = crate::bind_registry::shared_script_manager();
    // `Fluid.script`: the last definition registered for the fluid.
    sm.get_all::<FluidDefinitionScript>(ScriptType::FluidDefinition)
        .into_iter()
        .rev()
        .find(|d| {
            let d = d.borrow();
            match d.fluid_type_string() {
                Some(s) if d.fluid_type().name() == "Modded" => s == name,
                _ => d.fluid_type().name() == name,
            }
        })
}

fn view(fi: &FluidInstance, ctx: &dyn EntityContext) -> FluidView {
    let info = match &fi.fluid {
        Some(FluidRef::Type(id)) => ctx.fluid_by_id(*id),
        Some(FluidRef::Modded(n)) => ctx.fluid_by_name(n),
        None => Lookup::Missing,
    };
    let name = match (&info, &fi.fluid) {
        (Lookup::Found(f), _) => Some(f.name.clone()),
        (_, Some(FluidRef::Modded(n))) => Some(n.clone()),
        _ => None,
    };
    let def = name.as_deref().and_then(definition);
    let (translated, poison) = match &def {
        // FluidDefinitionScript loads `displayName` through
        // Translator.getFluidText.
        Some(d) => {
            let d = d.borrow();
            (
                fluid_text(d.display_name()),
                d.has_poison().then(|| d.poison_max_effect()),
            )
        }
        None => ("<unknown_fluid>".to_string(), None),
    };
    FluidView {
        name,
        translated,
        poison,
        color: FluidContainer::instance_color(fi, ctx),
        amount: fi.amount,
    }
}

fn is_tainted_water(v: &FluidView) -> bool {
    v.name.as_deref() == Some("TaintedWater")
}

/// `getPoisonEffect()`: the strongest `PoisonInfo.maxEffect`.
fn poison_effect(views: &[FluidView]) -> PoisonEffect {
    let mut e = PoisonEffect::None;
    for v in views {
        if let Some(p) = v.poison {
            if p.level() > e.level() {
                e = p;
            }
        }
    }
    e
}

/// `DecimalFormat(pattern).format(v)` for `#`, `#.#` and `#.##`:
/// half-even, no trailing zeros, no leading zero before the point.
fn decimal_format(v: f32, decimals: usize) -> String {
    // DecimalFormat works on the double's shortest decimal digits.
    let repr = format!("{}", v as f64);
    let neg = repr.starts_with('-');
    let digits = repr.trim_start_matches('-');
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    let mut all: Vec<u8> = int.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
    let mut point = int.len();
    let keep = point + decimals;
    if all.len() > keep {
        let first = all[keep];
        let tail_nonzero = all[keep + 1..].iter().any(|&x| x != 0);
        let last_odd = keep > 0 && all[keep - 1] % 2 == 1;
        let up = first > 5 || (first == 5 && (tail_nonzero || last_odd));
        all.truncate(keep);
        if up {
            let mut i = keep;
            loop {
                if i == 0 {
                    all.insert(0, 1);
                    point += 1;
                    break;
                }
                i -= 1;
                if all[i] == 9 {
                    all[i] = 0;
                } else {
                    all[i] += 1;
                    break;
                }
            }
        }
    }
    let text = |d: &[u8]| d.iter().map(|d| (d + b'0') as char).collect::<String>();
    let mut int = text(&all[..point]).trim_start_matches('0').to_string();
    if int.is_empty() {
        int.push('0');
    }
    let frac = text(&all[point..]).trim_end_matches('0').to_string();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&int);
    if !frac.is_empty() {
        s.push('.');
        s.push_str(&frac);
    }
    s
}

/// `FluidUtil.getFractionFormatted(numerator, denominator)`.
fn fraction_formatted(n: f32, d: f32) -> String {
    let amount = n.max(d);
    let liter = tr("IGUI_Liter");
    if amount >= 1000.0 {
        return format!("{} / {}{liter}", decimal_format(n, 0), decimal_format(d, 0));
    }
    if amount >= 10.0 {
        return format!("{} / {}{liter}", decimal_format(n, 1), decimal_format(d, 1));
    }
    if amount >= 1.0 {
        return format!("{} / {}{liter}", decimal_format(n, 2), decimal_format(d, 2));
    }
    let ml = tr("IGUI_Milliliter");
    format!(
        "{} / {}{ml}",
        (n * 1000.0 + 0.5).floor() as i32,
        (d * 1000.0 + 0.5).floor() as i32
    )
}

/// `FluidContainer.DoTooltip(tooltipUI, layout)` (the `Core.debug`
/// property rows are TODO(fluids): `SealedFluidProperties` sums).
fn fluid_tooltip(cx: &Cx<'_>, fc: &FluidContainer, ctx: &dyn EntityContext, layout: &mut Layout) {
    let amount = fc.get_amount();
    let cap = fc.get_capacity();
    layout
        .add_item()
        .label(&format!("{}:", fluid_text("Fluid_Amount")), LABEL)
        .value(&fraction_formatted(amount, cap), WHITE);
    if amount == 0.0 {
        return;
    }
    let views: Vec<FluidView> = fc.fluids.iter().map(|f| view(f, ctx)).collect();
    if views.len() == 1 {
        let v = &views[0];
        layout
            .add_item()
            .label(&format!("{}:", v.translated), LABEL)
            .progress(v.amount / cap, [v.color[0], v.color[1], v.color[2], 1.0]);
    } else if views.len() == 2 && views[0].translated == views[1].translated {
        let v = &views[0];
        layout
            .add_item()
            .label(&format!("{}:", v.translated), LABEL)
            .progress(
                (v.amount + views[1].amount) / cap,
                [v.color[0], v.color[1], v.color[2], 1.0],
            );
    } else {
        let c = fc.get_color(ctx);
        layout
            .add_item()
            .label(&format!("{}:", fluid_text("Fluid_Mixture")), LABEL)
            .progress(amount / cap, [c[0], c[1], c[2], 1.0]);
        layout
            .add_item()
            .label(&format!("{}:", fluid_text("Fluid_Fluids")), LABEL);
        let mut sorted: Vec<&FluidView> = views.iter().collect();
        sorted.sort_by(|a, b| java_compare(&a.translated, &b.translated));
        let mut i = 0;
        while i < sorted.len() {
            let fi = sorted[i];
            let mut combined = fi.amount;
            while i + 1 < sorted.len()
                && if cx.tainted_text {
                    sorted[i + 1].name == fi.name
                } else {
                    sorted[i + 1].translated == fi.translated
                }
            {
                i += 1;
                combined += sorted[i].amount;
            }
            layout
                .add_item()
                .label(&format!("{}:", fi.translated), [0.7, 0.7, 0.4, 1.0])
                .progress(
                    combined / amount,
                    [fi.color[0], fi.color[1], fi.color[2], 1.0],
                );
            i += 1;
        }
    }
    let other_poison = views
        .iter()
        .any(|v| !is_tainted_water(v) && v.poison.is_some());
    let poisonous = views.iter().any(|v| v.poison.is_some());
    let tainted = views.iter().any(is_tainted_water);
    if poisonous
        && poison_effect(&views) != PoisonEffect::None
        && (other_poison || !tainted || cx.tainted_text)
    {
        layout.add_item().label(
            &tr("Tooltip_tainted"),
            [cx.bad[0], cx.bad[1], cx.bad[2], 1.0],
        );
    }
}

/// `Durability.DoTooltip`.
fn durability_tooltip(d: &pz_entity::components::basic::Durability, layout: &mut Layout) {
    layout
        .add_item()
        .label("Material:", LABEL)
        .value(&format!("{:?}", d.material), WHITE);
    layout
        .add_item()
        .label("Hit Points:", LABEL)
        .value(&super::fstr(d.current_hit_points), WHITE);
}

/// The component rows: attributes, the fluid container, (an item in an
/// inventory has no world item) and durability.
pub fn do_tooltip(cx: &mut Cx<'_>, item: &ItemRef, _tip: &Tip<'_>, layout: &mut Layout) {
    let it = item.borrow();
    attributes_tooltip(cx, &it, layout);
    let e = item_env();
    let Some(ctx) = e.entity() else {
        return;
    };
    let Some(comps) = it.decode_components(ctx) else {
        return;
    };
    for c in &comps.list {
        if let Component::FluidContainer(fc) = c {
            fluid_tooltip(cx, fc, ctx, layout);
        }
    }
    for c in &comps.list {
        if let Component::Durability(d) = c {
            durability_tooltip(d, layout);
        }
    }
    let _ = trc;
}

#[cfg(test)]
#[path = "components/attributes_tests.rs"]
mod attributes_tests;

#[cfg(test)]
mod tests {
    use super::decimal_format;

    #[test]
    fn decimal_formats() {
        assert_eq!(decimal_format(0.5, 2), "0.5");
        assert_eq!(decimal_format(1.0, 2), "1");
        assert_eq!(decimal_format(1.125, 2), "1.12");
        assert_eq!(decimal_format(1.135, 2), "1.13");
        assert_eq!(decimal_format(0.0, 1), "0");
        assert_eq!(decimal_format(9.99, 1), "10");
        assert_eq!(decimal_format(1500.4, 0), "1500");
        assert_eq!(decimal_format(2.5, 0), "2");
    }
}
