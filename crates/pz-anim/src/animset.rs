//! The AnimSet data: `AnimationSet` → `AnimState` (one directory) →
//! `AnimNode` (one XML file) with its `AnimCondition`s, `AnimTransition`s,
//! `AnimEvent`s, 2D blend points/triangles (`Anim2DBlend`,
//! `Anim2DBlendTriangle`, `Anim2DBlendPicker`) and bone masks.
//!
//! Field parsing follows what JAXB does with the original classes: unknown
//! elements are ignored, the last occurrence of a scalar wins, lists
//! collect every occurrence, and numbers/booleans are trimmed.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::interp::{Interpolator, LerpType};
use crate::math::pzmath;
use crate::vars::{
    equals_ignore_case, is_null_or_whitespace, java_parse_float, java_trim, AnimationVariableReference, AnimationVariableSource, VariableReferenceRc,
};
use crate::xml::{self, Element};
use crate::{AnimBoneWeight, BoneAxis};

// ---- scalar parsing (JAXB) ----

fn parse_f32(e: &Element) -> Option<f32> {
    let t = e.text_content();
    match java_trim(&t) {
        "INF" => Some(f32::INFINITY),
        "-INF" => Some(f32::NEG_INFINITY),
        s => java_parse_float(s),
    }
}

fn parse_i32(e: &Element) -> Option<i32> {
    java_trim(&e.text_content()).trim_start_matches('+').parse().ok()
}

/// JAXB's `xs:boolean`: "true" or "1" (trimmed) is true, anything else
/// false (the game data's `False` reads as false).
fn parse_bool(e: &Element) -> Option<bool> {
    Some(matches!(java_trim(&e.text_content()), "true" | "1"))
}

fn set_f32(dst: &mut f32, e: &Element) {
    if let Some(v) = parse_f32(e) {
        *dst = v;
    }
}

fn set_bool(dst: &mut bool, e: &Element) {
    if let Some(v) = parse_bool(e) {
        *dst = v;
    }
}

fn parse_axis(e: &Element) -> Option<BoneAxis> {
    match java_trim(&e.text_content()) {
        "Y" => Some(BoneAxis::Y),
        "Z" => Some(BoneAxis::Z),
        _ => None,
    }
}

// ---- conditions ----

/// `AnimCondition.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimConditionType {
    String,
    StrNeq,
    Bool,
    Equ,
    Neq,
    Less,
    Gtr,
    AbsLess,
    AbsGtr,
    Or,
}

impl AnimConditionType {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "STRING" => Self::String,
            "STRNEQ" => Self::StrNeq,
            "BOOL" => Self::Bool,
            "EQU" => Self::Equ,
            "NEQ" => Self::Neq,
            "LESS" => Self::Less,
            "GTR" => Self::Gtr,
            "ABSLESS" => Self::AbsLess,
            "ABSGTR" => Self::AbsGtr,
            "OR" => Self::Or,
            _ => return None,
        })
    }

    fn is_float(self) -> bool {
        matches!(self, Self::Equ | Self::Neq | Self::Less | Self::Gtr | Self::AbsLess | Self::AbsGtr)
    }
}

/// `AnimCondition`.
#[derive(Clone, Debug)]
pub struct AnimCondition {
    pub name: String,
    pub ty: AnimConditionType,
    pub value: String,
    pub float_value: f32,
    pub bool_value: bool,
    pub string_value: String,
    variable_reference: Option<VariableReferenceRc>,
}

impl Default for AnimCondition {
    fn default() -> Self {
        AnimCondition {
            name: String::new(),
            ty: AnimConditionType::String,
            value: String::new(),
            float_value: 0.0,
            bool_value: false,
            string_value: String::new(),
            variable_reference: None,
        }
    }
}

impl AnimCondition {
    fn from_element(e: &Element) -> AnimCondition {
        let mut c = AnimCondition::default();
        for f in e.elements() {
            match f.tag.as_str() {
                "m_Name" => c.name = f.text_content(),
                // An unknown type is left as the default (JAXB would leave
                // null and the original would then fail to load the node).
                "m_Type" => {
                    if let Some(t) = AnimConditionType::parse(java_trim(&f.text_content())) {
                        c.ty = t;
                    }
                }
                "m_Value" => c.value = f.text_content(),
                "m_FloatValue" => set_f32(&mut c.float_value, f),
                "m_BoolValue" => set_bool(&mut c.bool_value, f),
                "m_StringValue" => c.string_value = f.text_content(),
                _ => {}
            }
        }
        c
    }

    /// `parse(fromNode, toNode)`.
    pub fn parse(&mut self, from_node: &str, to_node: Option<&str>) {
        self.parse_value();
        self.variable_reference = Some(Rc::new(AnimationVariableReference::from_raw_variable_name(&self.name)));
        if self.is_type_string() {
            if self.string_value.contains("$this") {
                self.string_value = self.string_value.replace("$this", from_node);
            }
            if self.string_value.contains("$source") {
                self.string_value = self.string_value.replace("$source", from_node);
            }
            if self.string_value.contains("$target") {
                if let Some(t) = to_node {
                    self.string_value = self.string_value.replace("$target", t);
                }
            }
        }
    }

    /// `parseValue`.
    pub fn parse_value(&mut self) {
        if self.value.is_empty() {
            return;
        }
        match self.ty {
            AnimConditionType::String | AnimConditionType::StrNeq => self.string_value = self.value.clone(),
            AnimConditionType::Bool => self.bool_value = crate::vars::try_parse_boolean(&self.value),
            t if t.is_float() => self.float_value = if is_null_or_whitespace(&self.value) { 0.0 } else { java_parse_float(&self.value).unwrap_or(0.0) },
            _ => {}
        }
    }

    pub fn is_type_string(&self) -> bool {
        matches!(self.ty, AnimConditionType::String | AnimConditionType::StrNeq)
    }

    /// `check(varSource)`.
    pub fn check(&self, vars: &dyn AnimationVariableSource) -> bool {
        use AnimConditionType as T;
        if self.ty == T::Or {
            return false;
        }
        let slot = self.variable_reference.as_ref().and_then(|r| r.get_variable(vars));
        let Some(slot) = slot else {
            return match self.ty {
                T::String => equals_ignore_case(&self.string_value, ""),
                T::StrNeq => !equals_ignore_case(&self.string_value, ""),
                T::Bool => !self.bool_value,
                _ => false,
            };
        };
        match self.ty {
            T::Equ => self.float_value == slot.as_float(),
            T::Neq => self.float_value != slot.as_float(),
            T::Less => slot.as_float() < self.float_value,
            T::Gtr => slot.as_float() > self.float_value,
            T::AbsLess => pzmath::abs(slot.as_float()) < self.float_value,
            T::AbsGtr => pzmath::abs(slot.as_float()) > self.float_value,
            T::Bool => slot.as_bool() == self.bool_value,
            T::String => equals_ignore_case(&self.string_value, &slot.as_string()),
            T::StrNeq => !equals_ignore_case(&self.string_value, &slot.as_string()),
            T::Or => false,
        }
    }

    /// `AnimCondition.pass`: AND within a group, `OR` separates groups.
    pub fn pass(vars: &dyn AnimationVariableSource, conditions: &[AnimCondition]) -> bool {
        let mut valid = true;
        for c in conditions {
            if c.ty == AnimConditionType::Or {
                if valid {
                    break;
                }
                valid = true;
                continue;
            }
            valid = valid && c.check(vars);
        }
        valid
    }
}

// ---- events ----

/// `AnimEvent.AnimEventTime`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnimEventTime {
    #[default]
    Percentage,
    Start,
    End,
    /// An `m_Time` value that is not one of the `@XmlEnumValue`s
    /// (`Percentage`, `Start`, `End`): JAXB leaves null and the event never
    /// fires.
    Invalid,
}

/// The `AnimEvent` subclasses.
#[derive(Clone, Debug, Default)]
pub enum AnimEventKind {
    #[default]
    Plain,
    /// `AnimEventSetVariable`.
    SetVariable { reference: VariableReferenceRc, value: String },
    /// `AnimEventFlagWhileAlive`.
    FlagWhileAlive { reference: VariableReferenceRc, flag_value: bool },
}

/// `AnimEvent`.
#[derive(Clone, Debug, Default)]
pub struct AnimEvent {
    pub event_name: String,
    pub time: AnimEventTime,
    pub time_pc: f32,
    pub parameter_value: Option<String>,
    pub kind: AnimEventKind,
    /// `parentAnimNode.name`.
    pub parent_anim_node: Option<String>,
}

impl AnimEvent {
    pub fn named(name: &str, time: AnimEventTime, time_pc: f32) -> AnimEvent {
        AnimEvent { event_name: name.into(), time, time_pc, ..Default::default() }
    }

    fn from_element(e: &Element) -> AnimEvent {
        let mut ev = AnimEvent::default();
        for f in e.elements() {
            match f.tag.as_str() {
                "m_EventName" => ev.event_name = f.text_content(),
                "m_Time" => {
                    ev.time = match java_trim(&f.text_content()) {
                        "Start" => AnimEventTime::Start,
                        "End" => AnimEventTime::End,
                        "Percentage" => AnimEventTime::Percentage,
                        _ => AnimEventTime::Invalid,
                    }
                }
                "m_TimePc" => set_f32(&mut ev.time_pc, f),
                "m_ParameterValue" => ev.parameter_value = Some(f.text_content()),
                _ => {}
            }
        }
        let split = |s: &str| -> (String, Option<String>) {
            // String.split("=") drops trailing empty strings.
            let mut parts: Vec<&str> = s.split('=').collect();
            while parts.len() > 1 && parts.last() == Some(&"") {
                parts.pop();
            }
            if parts.len() == 2 {
                (parts[0].to_string(), Some(parts[1].to_string()))
            } else {
                (s.to_string(), None)
            }
        };
        let param = ev.parameter_value.clone().unwrap_or_default();
        if ev.event_name.eq_ignore_ascii_case("SetVariable") {
            let (name, value) = split(&param);
            ev.kind = AnimEventKind::SetVariable {
                reference: Rc::new(AnimationVariableReference::from_raw_variable_name(&name)),
                value: value.unwrap_or_default(),
            };
        } else if ev.event_name.eq_ignore_ascii_case("FlagWhileAlive") {
            let (name, value) = split(&param);
            ev.kind = AnimEventKind::FlagWhileAlive {
                reference: Rc::new(AnimationVariableReference::from_raw_variable_name(&name)),
                flag_value: value.map(|v| v.eq_ignore_ascii_case("true")).unwrap_or(true),
            };
        }
        ev
    }
}

// ---- 2D blends ----

/// `Anim2DBlend`.
#[derive(Clone, Debug, PartialEq)]
pub struct Anim2DBlend {
    pub anim_name: String,
    pub pos_x: f32,
    pub pos_y: f32,
    pub speed_scale: f32,
    pub reference_id: Option<String>,
}

impl Anim2DBlend {
    fn from_element(e: &Element) -> Anim2DBlend {
        let mut b = Anim2DBlend { anim_name: String::new(), pos_x: 0.0, pos_y: 0.0, speed_scale: 1.0, reference_id: e.attr("referenceID").map(|s| java_trim(s).to_string()) };
        for f in e.elements() {
            match f.tag.as_str() {
                "m_AnimName" => b.anim_name = f.text_content(),
                "m_XPos" => set_f32(&mut b.pos_x, f),
                "m_YPos" => set_f32(&mut b.pos_y, f),
                "m_SpeedScale" => set_f32(&mut b.speed_scale, f),
                _ => {}
            }
        }
        b
    }
}

/// `Anim2DBlendTriangle`: indices into the node's `blends2d`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anim2DBlendTriangle {
    pub node1: usize,
    pub node2: usize,
    pub node3: usize,
}

impl Anim2DBlendTriangle {
    fn sign(p1x: f32, p1y: f32, p2x: f32, p2y: f32, p3x: f32, p3y: f32) -> f64 {
        ((p1x - p3x) * (p2y - p3y) - (p2x - p3x) * (p1y - p3y)) as f64
    }

    #[allow(clippy::too_many_arguments)]
    pub fn point_in_triangle(px: f32, py: f32, v1x: f32, v1y: f32, v2x: f32, v2y: f32, v3x: f32, v3y: f32) -> bool {
        let b1 = Self::sign(px, py, v1x, v1y, v2x, v2y) < 0.0;
        let b2 = Self::sign(px, py, v2x, v2y, v3x, v3y) < 0.0;
        let b3 = Self::sign(px, py, v3x, v3y, v1x, v1y) < 0.0;
        b1 == b2 && b2 == b3
    }

    pub fn contains(&self, blends: &[Anim2DBlend], x: f32, y: f32) -> bool {
        let (a, b, c) = (&blends[self.node1], &blends[self.node2], &blends[self.node3]);
        Self::point_in_triangle(x, y, a.pos_x, a.pos_y, b.pos_x, b.pos_y, c.pos_x, c.pos_y)
    }
}

/// `Anim2DBlendPicker.PickResults`: blend indices and weights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PickResults {
    pub num_nodes: usize,
    pub node1: Option<usize>,
    pub node2: Option<usize>,
    pub node3: Option<usize>,
    pub scale1: f32,
    pub scale2: f32,
    pub scale3: f32,
}

/// `Anim2DBlendPicker`.
#[derive(Clone, Debug, Default)]
pub struct Anim2DBlendPicker {
    tris: Vec<Anim2DBlendTriangle>,
    /// Hull blend indices, sorted around the centre.
    hull: Vec<usize>,
    center: (i32, i32),
}

impl Anim2DBlendPicker {
    /// `SetPickTriangles`.
    pub fn new(tris: Vec<Anim2DBlendTriangle>, blends: &[Anim2DBlend]) -> Anim2DBlendPicker {
        // Edges (ordered by position) used by exactly one triangle form the
        // hull.
        let edge = |a: usize, b: usize| -> (usize, usize) {
            let (pa, pb) = (&blends[a], &blends[b]);
            let swap = if pa.pos_x != pb.pos_x { pa.pos_x > pb.pos_x } else { pa.pos_y > pb.pos_y };
            if swap {
                (b, a)
            } else {
                (a, b)
            }
        };
        let mut edges: Vec<((usize, usize), u32)> = Vec::new();
        for t in &tris {
            for e in [edge(t.node1, t.node2), edge(t.node2, t.node3), edge(t.node3, t.node1)] {
                match edges.iter_mut().find(|(k, _)| *k == e) {
                    Some((_, c)) => *c += 1,
                    None => edges.push((e, 1)),
                }
            }
        }
        // The original collects the hull from a HashSet keyed by identity
        // hash, so points that tie in the comparator (collinear hulls, such
        // as the player's Idle blend) come out in an arbitrary order; here
        // they keep first-seen edge order.
        let mut hull: Vec<usize> = Vec::new();
        for ((a, b), c) in &edges {
            if *c == 1 {
                for n in [*a, *b] {
                    if !hull.contains(&n) {
                        hull.push(n);
                    }
                }
            }
        }
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        for &n in &hull {
            cx += blends[n].pos_x;
            cy += blends[n].pos_y;
        }
        cx /= hull.len() as f32;
        cy /= hull.len() as f32;
        let center = ((cx * 1000.0) as i32, (cy * 1000.0) as i32);
        let mut p = Anim2DBlendPicker { tris, hull: Vec::new(), center };
        let pts: Vec<(f32, f32)> = hull.iter().map(|&n| (blends[n].pos_x, blends[n].pos_y)).collect();
        let mut order: Vec<usize> = (0..hull.len()).collect();
        order.sort_by(|&a, &b| p.compare(pts[a], pts[b]));
        p.hull = order.into_iter().map(|i| hull[i]).collect();
        p
    }

    fn is_less_than(&self, o1: (f32, f32), o2: (f32, f32)) -> bool {
        let v1x = (o1.0 * 1000.0) as i32 - self.center.0;
        let v1y = (o1.1 * 1000.0) as i32 - self.center.1;
        let v2x = (o2.0 * 1000.0) as i32 - self.center.0;
        let v2y = (o2.1 * 1000.0) as i32 - self.center.1;
        if v1y == 0 && v1x > 0 {
            return true;
        }
        if v2y == 0 && v2x > 0 {
            return false;
        }
        if v1y > 0 && v2y < 0 {
            return true;
        }
        if v1y < 0 && v2y > 0 {
            return false;
        }
        v1x.wrapping_mul(v2y).wrapping_sub(v1y.wrapping_mul(v2x)) > 0
    }

    fn compare(&self, a: (f32, f32), b: (f32, f32)) -> std::cmp::Ordering {
        if self.is_less_than(a, b) {
            std::cmp::Ordering::Less
        } else if self.is_less_than(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }

    fn project_point_to_line(x: f32, y: f32, sx: f32, sy: f32, ex: f32, ey: f32) -> f32 {
        let (lpx, lpy) = (x - sx, y - sy);
        let (lex, ley) = (ex - sx, ey - sy);
        (lex * lpx + ley * lpy) / (lex * lex + ley * ley)
    }

    /// `Pick(x, y)`.
    pub fn pick(&self, blends: &[Anim2DBlend], x: f32, y: f32) -> PickResults {
        let mut r = PickResults::default();
        for t in &self.tris {
            if !t.contains(blends, x, y) {
                continue;
            }
            let (n1, n2, n3) = (&blends[t.node1], &blends[t.node2], &blends[t.node3]);
            let (v1x, v1y, v2x, v2y, v3x, v3y) = (n1.pos_x, n1.pos_y, n2.pos_x, n2.pos_y, n3.pos_x, n3.pos_y);
            r.num_nodes = 3;
            r.node1 = Some(t.node1);
            r.node2 = Some(t.node2);
            r.node3 = Some(t.node3);
            let den = (v2y - v3y) * (v1x - v3x) + (v3x - v2x) * (v1y - v3y);
            r.scale1 = ((v2y - v3y) * (x - v3x) + (v3x - v2x) * (y - v3y)) / den;
            r.scale2 = ((v3y - v1y) * (x - v3x) + (v1x - v3x) * (y - v3y)) / den;
            r.scale3 = 1.0 - r.scale1 - r.scale2;
            return r;
        }
        if self.hull.is_empty() {
            return r;
        }
        let x = x * 1.1;
        let y = y * 1.1;
        // LowerBoundIdx (upper bound by the comparator).
        let (mut lo, mut hi) = (0usize, self.hull.len());
        while lo != hi {
            let mid = (lo + hi) / 2;
            let m = &blends[self.hull[mid]];
            if self.compare((x, y), (m.pos_x, m.pos_y)) == std::cmp::Ordering::Less {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let mut idx = lo;
        if idx == self.hull.len() {
            idx = 0;
        }
        let idx2 = if idx > 0 { idx - 1 } else { self.hull.len() - 1 };
        let (a, b) = (self.hull[idx], self.hull[idx2]);
        let s = Self::project_point_to_line(x, y, blends[a].pos_x, blends[a].pos_y, blends[b].pos_x, blends[b].pos_y);
        if s < 0.0 {
            r.num_nodes = 1;
            r.node1 = Some(a);
            r.scale1 = 1.0;
        } else if s > 1.0 {
            r.num_nodes = 1;
            r.node1 = Some(b);
            r.scale1 = 1.0;
        } else {
            r.num_nodes = 2;
            r.node1 = Some(a);
            r.node2 = Some(b);
            r.scale1 = 1.0 - s;
            r.scale2 = s;
        }
        r
    }
}

// ---- transitions ----

/// `AnimTransition`.
#[derive(Debug)]
pub struct AnimTransition {
    pub source: Option<String>,
    pub target: Option<String>,
    pub anim_name: Option<String>,
    pub deferred_bone_name: Option<String>,
    pub deferred_bone_axis: BoneAxis,
    pub use_defered_rotation: bool,
    pub use_deferred_movement: bool,
    pub deferred_rotation_scale: f32,
    pub sync_adjust_time: f32,
    pub blend_in_time: f32,
    pub blend_out_time: f32,
    pub speed_scale: f32,
    parsed: Cell<bool>,
    pub conditions: RefCell<Vec<AnimCondition>>,
}

impl AnimTransition {
    fn from_element(e: &Element) -> AnimTransition {
        let mut t = AnimTransition {
            source: None,
            target: None,
            anim_name: None,
            deferred_bone_name: None,
            deferred_bone_axis: BoneAxis::Y,
            use_defered_rotation: false,
            use_deferred_movement: true,
            deferred_rotation_scale: 1.0,
            sync_adjust_time: 0.0,
            blend_in_time: f32::INFINITY,
            blend_out_time: f32::INFINITY,
            speed_scale: f32::INFINITY,
            parsed: Cell::new(false),
            conditions: RefCell::new(Vec::new()),
        };
        let mut conds = Vec::new();
        for f in e.elements() {
            match f.tag.as_str() {
                "m_Source" => t.source = Some(f.text_content()),
                "m_Target" => t.target = Some(f.text_content()),
                "m_AnimName" => t.anim_name = Some(f.text_content()),
                "m_DeferredBoneName" => t.deferred_bone_name = Some(f.text_content()),
                "m_deferredBoneAxis" => t.deferred_bone_axis = parse_axis(f).unwrap_or(t.deferred_bone_axis),
                "m_useDeferedRotation" => set_bool(&mut t.use_defered_rotation, f),
                "m_useDeferredMovement" => set_bool(&mut t.use_deferred_movement, f),
                "m_deferredRotationScale" => set_f32(&mut t.deferred_rotation_scale, f),
                "m_SyncAdjustTime" => set_f32(&mut t.sync_adjust_time, f),
                "m_blendInTime" => set_f32(&mut t.blend_in_time, f),
                "m_blendOutTime" => set_f32(&mut t.blend_out_time, f),
                "m_speedScale" => set_f32(&mut t.speed_scale, f),
                "m_Conditions" => conds.push(AnimCondition::from_element(f)),
                _ => {}
            }
        }
        *t.conditions.get_mut() = conds;
        t
    }

    /// `parse(fromNode, toNode)`: once.
    pub fn parse(&self, from_node: &str, to_node: &str) {
        if self.parsed.get() {
            return;
        }
        for c in self.conditions.borrow_mut().iter_mut() {
            c.parse(from_node, Some(to_node));
        }
        self.parsed.set(true);
    }

    pub fn has_anim(&self) -> bool {
        !self.anim_name.as_deref().is_none_or(is_null_or_whitespace)
    }
}

/// `GrappleOffsetBehaviour`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GrappleOffsetBehaviour {
    None,
    #[default]
    Grappled,
    GrappledTweenOutToNone,
    Grappler,
    NoneTweenInGrappler,
}

// ---- nodes ----

static NEXT_STATE_ID: AtomicU64 = AtomicU64::new(1);

/// `AnimNode`.
#[derive(Debug)]
pub struct AnimNode {
    pub name: String,
    is_idle_anim: bool,
    pub priority: i32,
    pub condition_priority: i32,
    pub anim_name: String,
    pub alternate_anims: Vec<String>,
    pub matching_grappled_anim_node: String,
    pub grapple_offset_forward: f32,
    pub grapple_offset_yaw: f32,
    pub grapple_tween_in_time: f32,
    pub grappler_offset_behaviour: GrappleOffsetBehaviour,
    pub is_ragdoll: bool,
    pub chance_to_ragdoll: f32,
    pub ragdoll_start_time_min: f32,
    pub ragdoll_start_time_max: f32,
    pub ragdoll_max_time: f32,
    pub deferred_bone_name: String,
    pub deferred_bone_axis: BoneAxis,
    pub use_defered_rotation: bool,
    pub use_deferred_movement: bool,
    pub deferred_rotation_scale: f32,
    pub is_looped: bool,
    pub blend_time: f32,
    pub blend_out_time: f32,
    pub blend_curve: Option<Rc<Interpolator>>,
    pub stop_anim_on_exit: bool,
    pub early_transition_out: bool,
    /// `m_SpeedScale` (a number, or a variable name).
    pub speed_scale: RefCell<String>,
    pub speed_scale_variable: Option<String>,
    pub speed_scale_random_multiplier_min: f32,
    pub speed_scale_random_multiplier_max: f32,
    speed_scale_f: f32,
    pub random_advance_fraction: f32,
    pub max_torso_twist: f32,
    pub scalar: String,
    pub scalar2: String,
    pub is_anim_reverse: bool,
    pub sync_tracking_enabled: bool,
    pub track_time_to_variable: Option<String>,
    pub blends2d: Vec<Anim2DBlend>,
    pub conditions: Vec<AnimCondition>,
    pub events: Vec<Rc<AnimEvent>>,
    pub blend_tris: Vec<Anim2DBlendTriangle>,
    pub transitions: Vec<AnimTransition>,
    pub sub_state_bone_weights: Vec<AnimBoneWeight>,
    pub blend2d_picker: Option<Anim2DBlendPicker>,
    /// `parentState`: the id of the [`AnimState`] that holds this node.
    pub parent_state: Cell<u64>,
    transition_out: Option<usize>,
}

impl AnimNode {
    /// `AnimNode.Parse(file)`.
    pub fn parse_file(path: &Path) -> Result<AnimNode, xml::XmlError> {
        let root = xml::parse_xml_file(path)?;
        Ok(AnimNode::from_element(&root))
    }

    /// The unmarshalling plus the post-processing of `AnimNode.Parse`.
    pub fn from_element(root: &Element) -> AnimNode {
        let mut n = AnimNode {
            name: String::new(),
            is_idle_anim: false,
            priority: 5,
            condition_priority: 0,
            anim_name: String::new(),
            alternate_anims: Vec::new(),
            matching_grappled_anim_node: String::new(),
            grapple_offset_forward: 0.0,
            grapple_offset_yaw: 0.0,
            grapple_tween_in_time: 0.25,
            grappler_offset_behaviour: GrappleOffsetBehaviour::Grappled,
            is_ragdoll: false,
            chance_to_ragdoll: 1.0,
            ragdoll_start_time_min: 0.0,
            ragdoll_start_time_max: 0.0,
            ragdoll_max_time: 5.0,
            deferred_bone_name: "Translation_Data".into(),
            deferred_bone_axis: BoneAxis::Y,
            use_defered_rotation: false,
            use_deferred_movement: true,
            deferred_rotation_scale: 1.0,
            is_looped: true,
            blend_time: 0.0,
            blend_out_time: -1.0,
            blend_curve: None,
            stop_anim_on_exit: false,
            early_transition_out: false,
            speed_scale: RefCell::new("1.00".into()),
            speed_scale_variable: None,
            speed_scale_random_multiplier_min: 1.0,
            speed_scale_random_multiplier_max: 1.0,
            speed_scale_f: f32::INFINITY,
            random_advance_fraction: 0.0,
            max_torso_twist: 15.0,
            scalar: String::new(),
            scalar2: String::new(),
            is_anim_reverse: false,
            sync_tracking_enabled: true,
            track_time_to_variable: None,
            blends2d: Vec::new(),
            conditions: Vec::new(),
            events: Vec::new(),
            blend_tris: Vec::new(),
            transitions: Vec::new(),
            sub_state_bone_weights: Vec::new(),
            blend2d_picker: None,
            parent_state: Cell::new(0),
            transition_out: None,
        };
        let mut raw_tris: Vec<[String; 3]> = Vec::new();
        let mut events = Vec::new();
        for f in root.elements() {
            let t = || f.text_content();
            match f.tag.as_str() {
                "m_Name" => n.name = t(),
                "m_Priority" => n.priority = parse_i32(f).unwrap_or(n.priority),
                "m_ConditionPriority" => n.condition_priority = parse_i32(f).unwrap_or(n.condition_priority),
                "m_AnimName" => n.anim_name = t(),
                "m_AlternateAnims" => n.alternate_anims.push(t()),
                "m_MatchingGrappledAnimNode" => n.matching_grappled_anim_node = t(),
                "m_GrappleOffsetForward" => set_f32(&mut n.grapple_offset_forward, f),
                "m_GrappleOffsetYaw" => set_f32(&mut n.grapple_offset_yaw, f),
                "m_GrappleTweenInTime" => set_f32(&mut n.grapple_tween_in_time, f),
                "m_GrapplerOffsetBehaviour" => {
                    n.grappler_offset_behaviour = match java_trim(&t()) {
                        "None" => GrappleOffsetBehaviour::None,
                        "Grappled" => GrappleOffsetBehaviour::Grappled,
                        "Grappled_TweenOutToNone" => GrappleOffsetBehaviour::GrappledTweenOutToNone,
                        "Grappler" => GrappleOffsetBehaviour::Grappler,
                        "None_TweenInGrappler" => GrappleOffsetBehaviour::NoneTweenInGrappler,
                        _ => n.grappler_offset_behaviour,
                    }
                }
                "m_isRagdoll" => set_bool(&mut n.is_ragdoll, f),
                "m_chanceToRagdoll" => set_f32(&mut n.chance_to_ragdoll, f),
                "m_ragdollStartTimeMin" => set_f32(&mut n.ragdoll_start_time_min, f),
                "m_ragdollStartTimeMax" => set_f32(&mut n.ragdoll_start_time_max, f),
                "m_ragdollMaxTime" => set_f32(&mut n.ragdoll_max_time, f),
                "m_DeferredBoneName" => n.deferred_bone_name = t(),
                "m_deferredBoneAxis" => n.deferred_bone_axis = parse_axis(f).unwrap_or(n.deferred_bone_axis),
                "m_useDeferedRotation" => set_bool(&mut n.use_defered_rotation, f),
                "m_useDeferredMovement" => set_bool(&mut n.use_deferred_movement, f),
                "m_deferredRotationScale" => set_f32(&mut n.deferred_rotation_scale, f),
                "m_Looped" => set_bool(&mut n.is_looped, f),
                "m_BlendTime" => set_f32(&mut n.blend_time, f),
                "m_BlendOutTime" => set_f32(&mut n.blend_out_time, f),
                "m_BlendCurve" => n.blend_curve = parse_interpolator_slot(f).map(Rc::new).or(n.blend_curve.take()),
                "m_StopAnimOnExit" => set_bool(&mut n.stop_anim_on_exit, f),
                "m_EarlyTransitionOut" => set_bool(&mut n.early_transition_out, f),
                "m_SpeedScale" => *n.speed_scale.get_mut() = t(),
                "m_SpeedScaleVariable" => n.speed_scale_variable = Some(t()),
                "m_SpeedScaleRandomMultiplierMin" => set_f32(&mut n.speed_scale_random_multiplier_min, f),
                "m_SpeedScaleRandomMultiplierMax" => set_f32(&mut n.speed_scale_random_multiplier_max, f),
                "m_randomAdvanceFraction" => set_f32(&mut n.random_advance_fraction, f),
                "m_maxTorsoTwist" => set_f32(&mut n.max_torso_twist, f),
                "m_Scalar" => n.scalar = t(),
                "m_Scalar2" => n.scalar2 = t(),
                "m_AnimReverse" => set_bool(&mut n.is_anim_reverse, f),
                "m_SyncTrackingEnabled" => set_bool(&mut n.sync_tracking_enabled, f),
                "m_TrackTimeToVariable" => n.track_time_to_variable = Some(t()),
                "m_2DBlends" => n.blends2d.push(Anim2DBlend::from_element(f)),
                "m_Conditions" => n.conditions.push(AnimCondition::from_element(f)),
                "m_Events" => events.push(AnimEvent::from_element(f)),
                "m_2DBlendTri" => {
                    let mut tri: [String; 3] = Default::default();
                    for g in f.elements() {
                        match g.tag.as_str() {
                            "node1" => tri[0] = java_trim(&g.text_content()).to_string(),
                            "node2" => tri[1] = java_trim(&g.text_content()).to_string(),
                            "node3" => tri[2] = java_trim(&g.text_content()).to_string(),
                            _ => {}
                        }
                    }
                    raw_tris.push(tri);
                }
                "m_Transitions" => n.transitions.push(AnimTransition::from_element(f)),
                "m_SubStateBoneWeights" => {
                    let mut w = AnimBoneWeight { bone_name: String::new(), weight: 1.0, include_descendants: true };
                    for g in f.elements() {
                        match g.tag.as_str() {
                            "boneName" => w.bone_name = g.text_content(),
                            "weight" => set_f32(&mut w.weight, g),
                            "includeDescendants" => set_bool(&mut w.include_descendants, g),
                            _ => {}
                        }
                    }
                    n.sub_state_bone_weights.push(w);
                }
                _ => {}
            }
        }
        // IDREFs: the last blend with an id wins.
        let mut ids: HashMap<&str, usize> = HashMap::new();
        for (i, b) in n.blends2d.iter().enumerate() {
            if let Some(id) = &b.reference_id {
                ids.insert(id.as_str(), i);
            }
        }
        for tri in &raw_tris {
            if let (Some(&a), Some(&b), Some(&c)) = (ids.get(tri[0].as_str()), ids.get(tri[1].as_str()), ids.get(tri[2].as_str())) {
                n.blend_tris.push(Anim2DBlendTriangle { node1: a, node2: b, node3: c });
            }
        }

        // AnimNode.Parse post-processing.
        n.is_idle_anim = n.name.contains("Idle");
        if !n.blend_tris.is_empty() {
            n.blend2d_picker = Some(Anim2DBlendPicker::new(n.blend_tris.clone(), &n.blends2d));
        }
        if n.is_ragdoll {
            n.conditions.push(AnimCondition {
                ty: AnimConditionType::Bool,
                bool_value: true,
                name: "canRagdoll".into(),
                ..Default::default()
            });
        }
        let name = n.name.clone();
        for c in &mut n.conditions {
            c.parse(&name, None);
        }
        events.sort_by(|a, b| a.time_pc.total_cmp(&b.time_pc));
        n.events = events
            .into_iter()
            .map(|mut e| {
                e.parent_anim_node = Some(name.clone());
                Rc::new(e)
            })
            .collect();
        match java_parse_float(&n.speed_scale.borrow()) {
            Some(v) => n.speed_scale_f = v,
            None => n.speed_scale_variable = Some(n.speed_scale.borrow().clone()),
        }
        if n.sub_state_bone_weights.is_empty() {
            for (b, w) in [("Bip01_Spine1", 0.5), ("Bip01_Neck", 1.0), ("Bip01_BackPack", 1.0), ("Bip01_Prop1", 1.0), ("Bip01_Prop2", 1.0)] {
                n.sub_state_bone_weights.push(AnimBoneWeight::new(b, w));
            }
        }
        n.transition_out = None;
        for (i, t) in n.transitions.iter().enumerate() {
            if t.target.as_deref().is_none_or(is_null_or_whitespace) {
                n.transition_out = Some(i);
            }
        }
        n
    }

    pub fn check_conditions(&self, vars: &dyn AnimationVariableSource) -> bool {
        AnimCondition::pass(vars, &self.conditions)
    }

    /// `getSpeedScale`.
    pub fn get_speed_scale(&self, vars: &dyn AnimationVariableSource) -> f32 {
        if self.speed_scale_f != f32::INFINITY {
            return self.speed_scale_f;
        }
        vars.get_variable_float(&self.speed_scale.borrow(), 1.0)
    }

    pub fn is_idle_anim(&self) -> bool {
        self.is_idle_anim
    }

    /// `findTransitionTo`.
    pub fn find_transition_to(&self, vars: &dyn AnimationVariableSource, to_node: &AnimNode) -> Option<usize> {
        for (i, t) in self.transitions.iter().enumerate() {
            if !equals_ignore_case(t.target.as_deref().unwrap_or(""), &to_node.name) {
                continue;
            }
            t.parse(&self.name, &to_node.name);
            if !AnimCondition::pass(vars, &t.conditions.borrow()) {
                continue;
            }
            return Some(i);
        }
        None
    }

    /// `isAbstract`.
    pub fn is_abstract(&self) -> bool {
        if self.is_ragdoll {
            return false;
        }
        if !is_null_or_whitespace(&self.anim_name) {
            return false;
        }
        if !self.alternate_anims.is_empty() {
            return false;
        }
        self.blends2d.is_empty()
    }

    /// `getBlendOutTime`.
    pub fn get_blend_out_time(&self) -> f32 {
        if let Some(t) = self.transition_out {
            return self.transitions[t].blend_out_time;
        }
        if self.blend_out_time >= 0.0 {
            return self.blend_out_time;
        }
        self.blend_time
    }

    /// `getDeferredBoneName`.
    pub fn get_deferred_bone_name(&self) -> &str {
        if is_null_or_whitespace(&self.deferred_bone_name) {
            "Translation_Data"
        } else {
            &self.deferred_bone_name
        }
    }

    /// `compareSelectionConditions`.
    pub fn compare_selection_conditions(a: &AnimNode, b: &AnimNode) -> i32 {
        if a.is_abstract() != b.is_abstract() {
            return if a.is_abstract() { -1 } else { 1 };
        }
        if a.condition_priority != b.condition_priority {
            return if a.condition_priority < b.condition_priority { -1 } else { 1 };
        }
        if a.conditions.len() != b.conditions.len() {
            return if a.conditions.len() < b.conditions.len() { -1 } else { 1 };
        }
        0
    }

    pub fn is_grappler(&self) -> bool {
        !is_null_or_whitespace(&self.matching_grappled_anim_node)
    }

    /// `getRandomAnim`.
    pub fn get_random_anim(&self, rng: &mut dyn crate::AnimRandom) -> String {
        let empty = self.anim_name.is_empty();
        if empty && self.alternate_anims.is_empty() {
            return String::new();
        }
        if empty {
            return self.alternate_anims[rng.next(self.alternate_anims.len() as i32) as usize].clone();
        }
        if self.alternate_anims.is_empty() {
            return self.anim_name.clone();
        }
        let idx = rng.next(1 + self.alternate_anims.len() as i32);
        if idx == 0 {
            self.anim_name.clone()
        } else {
            self.alternate_anims[rng.next(self.alternate_anims.len() as i32) as usize].clone()
        }
    }
}

fn parse_interpolator_slot(e: &Element) -> Option<Interpolator> {
    let mut out = None;
    for f in e.elements() {
        let attr = |n: &str, d: f32| f.attr(n).and_then(java_parse_float).unwrap_or(d);
        let i = match f.tag.as_str() {
            "EaseInQuad" => Interpolator::EaseInQuad { y0: attr("y0", 0.0), y1: attr("y1", 1.0) },
            "EaseOutInQuad" => Interpolator::EaseOutInQuad { y0: attr("y0", 0.0), y1: attr("y1", 1.0) },
            "EaseOutQuad" => Interpolator::EaseOutQuad { y0: attr("y0", 0.0), y1: attr("y1", 1.0) },
            "Linear" => Interpolator::Linear { y0: attr("y0", 0.0), y1: attr("y1", 1.0) },
            "One" => Interpolator::One,
            "Zero" => Interpolator::Zero,
            "Polyline" => {
                let pts: Vec<(f32, f32)> = f
                    .elements()
                    .filter(|p| p.tag == "m_Points")
                    .map(|p| (p.attr("x").and_then(java_parse_float).unwrap_or(0.0), p.attr("y").and_then(java_parse_float).unwrap_or(0.0)))
                    .collect();
                Interpolator::polyline(&pts)
            }
            "Sequence" => {
                // `out` is declared as an element; the data writes it as an
                // attribute, which JAXB ignores, so those points stay Linear.
                let pts: Vec<(f32, f32, LerpType)> = f
                    .elements()
                    .filter(|p| p.tag == "point")
                    .map(|p| {
                        let ty = p.elements().filter(|o| o.tag == "out").filter_map(|o| LerpType::parse(java_trim(&o.text_content()))).last();
                        (
                            p.attr("x").and_then(java_parse_float).unwrap_or(0.0),
                            p.attr("y").and_then(java_parse_float).unwrap_or(0.0),
                            ty.unwrap_or(LerpType::Linear),
                        )
                    })
                    .collect();
                Interpolator::sequence(&pts)
            }
            _ => continue,
        };
        out = Some(i);
    }
    out
}

// ---- states and sets ----

/// `AnimState`: the nodes of one state directory, ordered for selection.
#[derive(Debug)]
pub struct AnimState {
    pub id: u64,
    pub name: String,
    pub nodes: Vec<Rc<AnimNode>>,
    pub abstract_nodes: Vec<Rc<AnimNode>>,
    pub default_index: i32,
}

impl AnimState {
    pub fn new(name: &str) -> AnimState {
        AnimState { id: NEXT_STATE_ID.fetch_add(1, Ordering::Relaxed), name: name.to_string(), nodes: Vec::new(), abstract_nodes: Vec::new(), default_index: 0 }
    }

    /// `addNode`: abstract nodes aside, others before the first node they
    /// out-rank.
    pub fn add_node(&mut self, node: AnimNode) {
        node.parent_state.set(self.id);
        let node = Rc::new(node);
        if node.is_abstract() {
            self.abstract_nodes.push(node);
            return;
        }
        let at = self
            .nodes
            .iter()
            .position(|n| AnimNode::compare_selection_conditions(&node, n) > 0)
            .unwrap_or(self.nodes.len());
        self.nodes.insert(at, node);
    }

    /// `AnimState.Parse`: every `*.xml` below `dir`, in a pre-order walk
    /// with each directory's entries sorted by name (the original uses
    /// `File.listFiles` order).
    pub fn parse_dir(name: &str, dir: &Path) -> AnimState {
        let mut st = AnimState::new(name);
        let mut files = Vec::new();
        walk_files(dir, &mut files);
        for f in files {
            match AnimNode::parse_file(&f) {
                Ok(n) => st.add_node(n),
                Err(_) => continue,
            }
        }
        st
    }

    /// `getAnimNodes`: the passing nodes of the best selection rank.
    pub fn get_anim_nodes(&self, vars: &dyn AnimationVariableSource, allow_override: bool) -> Vec<Rc<AnimNode>> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        if allow_override && vars.get_variable_boolean("dbgForceAnim") && vars.is_variable("dbgForceAnimStateName", &self.name) {
            let want = vars.get_variable_string("dbgForceAnimNodeName");
            if let Some(n) = self.nodes.iter().find(|n| equals_ignore_case(&n.name, &want)) {
                out.push(n.clone());
            }
            return out;
        }
        let mut best: Option<&Rc<AnimNode>> = None;
        for n in &self.nodes {
            if let Some(b) = best {
                if AnimNode::compare_selection_conditions(b, n) > 0 {
                    break;
                }
            }
            if !n.check_conditions(vars) {
                continue;
            }
            best = Some(n);
            out.push(n.clone());
        }
        out
    }
}

fn walk_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_file() && p.extension().is_some_and(|e| e == "xml") {
            out.push(p.clone());
        }
        if p.is_dir() {
            walk_files(&p, out);
        }
    }
}

/// `AnimationSet`: the states of `media/AnimSets/<name>`.
#[derive(Debug, Default)]
pub struct AnimationSet {
    pub name: String,
    /// Keyed by lower-case state name.
    pub states: HashMap<String, Rc<AnimState>>,
}

impl AnimationSet {
    /// `AnimationSet.Load`: one state per sub-directory of `dir`.
    pub fn load(name: &str, dir: &Path) -> AnimationSet {
        let mut set = AnimationSet { name: name.to_string(), states: HashMap::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return set };
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let state_name = d.file_name().unwrap().to_string_lossy().into_owned();
            let st = AnimState::parse_dir(&state_name, &d);
            set.states.insert(state_name.to_lowercase(), Rc::new(st));
        }
        set
    }

    /// Loads only the named state directories (useful for tests).
    pub fn load_states(name: &str, dir: &Path, states: &[&str]) -> AnimationSet {
        let mut set = AnimationSet { name: name.to_string(), states: HashMap::new() };
        for s in states {
            let st = AnimState::parse_dir(s, &dir.join(s));
            set.states.insert(s.to_lowercase(), Rc::new(st));
        }
        set
    }

    pub fn add_state(&mut self, state: AnimState) {
        self.states.insert(state.name.to_lowercase(), Rc::new(state));
    }

    /// `GetState`: a fresh empty state when missing (like the original).
    pub fn get_state(&self, name: &str) -> Rc<AnimState> {
        self.states.get(&name.to_lowercase()).cloned().unwrap_or_else(|| Rc::new(AnimState::new("")))
    }

    pub fn contains_state(&self, name: &str) -> bool {
        self.states.contains_key(&name.to_lowercase())
    }
}

/// `AdvancedAnimator.LoadDefaults`: `(MotionScale, RotationScale)` from
/// `media/AnimSets/Defaults.xml`.
pub fn load_defaults(path: &Path) -> Option<(f32, f32)> {
    let root = xml::parse_xml_file(path).ok()?;
    fn find<'a>(e: &'a Element, tag: &str) -> Option<&'a Element> {
        for c in e.elements() {
            if c.tag == tag {
                return Some(c);
            }
            if let Some(f) = find(c, tag) {
                return Some(f);
            }
        }
        None
    }
    let m = java_parse_float(&find(&root, "MotionScale")?.text_content())?;
    let r = java_parse_float(&find(&root, "RotationScale")?.text_content())?;
    Some((m, r))
}
