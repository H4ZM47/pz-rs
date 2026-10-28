//! `zombie.characters.action`: `ActionGroup` (the `media/actiongroups/<name>`
//! folders), `ActionState`, `ActionTransition`, the conditions
//! (`CharacterVariableCondition`, `EventOccurred`, `EventNotOccurred`,
//! `LuaCall`), `ActionStateContainer`, `ActionContextEvents` and
//! `ActionContext`.
//!
//! Action states are identified by [`StateHandle`] (the group and the
//! state's index in it), which compares like the original's object
//! identity.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::vars::{equals_ignore_case, is_null_or_whitespace, try_parse_boolean, try_parse_float};
use pz_anim::xml::{self, Element};
use pz_anim::{AnimationVariableHandle, AnimationVariableReference, AnimationVariableSource, MapVariableSource, VariableValue};

// ---- conditions ------------------------------------------------------------

/// `CharacterVariableCondition.Operator`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

/// A parsed operand (`Object` in the original).
#[derive(Clone, Debug)]
pub enum CondValue {
    Str(String),
    Int(i32),
    Float(f32),
    Bool(bool),
    /// `CharacterVariableLookup`.
    Lookup(Rc<AnimationVariableReference>),
}

/// `IActionCondition`.
#[derive(Clone, Debug)]
pub enum Condition {
    /// `CharacterVariableCondition`.
    Variable { op: Operator, lhs: CondValue, rhs: CondValue },
    /// `EventOccurred` (the name lowercased).
    EventOccurred(String),
    /// `EventNotOccurred`.
    EventNotOccurred(String),
    /// `LuaCall`: never passes.
    LuaCall,
}

/// `CharacterVariableCondition.parseValue(value, parseForCharacterVariableLookup)`.
pub fn parse_value(value: &str, lookup: bool) -> CondValue {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return CondValue::Str(value.to_string());
    }
    let first = chars[0];
    if first == '-' || first == '+' || first.is_ascii_digit() {
        let mut int_val: i32 = 0;
        if first.is_ascii_digit() {
            int_val = first as i32 - 48;
        }
        let mut read_pos = 1;
        while read_pos < chars.len() {
            let chr = chars[read_pos];
            if chr.is_ascii_digit() {
                int_val = int_val.wrapping_mul(10).wrapping_add(chr as i32).wrapping_sub(48);
                read_pos += 1;
                continue;
            }
            if chr == ',' {
                read_pos += 1;
                continue;
            }
            if chr == '.' {
                read_pos += 1;
                break;
            }
            return CondValue::Str(value.to_string());
        }
        if read_pos == chars.len() {
            return CondValue::Int(int_val);
        }
        let mut float_val = int_val as f32;
        let mut divisor = 10.0f32;
        while read_pos < chars.len() {
            let chr = chars[read_pos];
            if chr.is_ascii_digit() {
                float_val += (chr as i32 - 48) as f32 / divisor;
                divisor *= 10.0;
            } else if chr != ',' {
                return CondValue::Str(value.to_string());
            }
            read_pos += 1;
        }
        if first == '-' {
            float_val *= -1.0;
        }
        return CondValue::Float(float_val);
    }
    if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes") {
        return CondValue::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") || value.eq_ignore_ascii_case("no") {
        return CondValue::Bool(false);
    }
    if lookup {
        if first == '\'' || first == '"' {
            let mut sb = String::new();
            let mut read_pos = 1;
            while read_pos < chars.len() {
                let c = chars[read_pos];
                match c {
                    '"' | '\'' if c == first => return CondValue::Str(sb),
                    '\\' => sb.push(chars[read_pos]),
                    _ => sb.push(c),
                }
                read_pos += 1;
            }
            return CondValue::Str(sb);
        }
        return CondValue::Lookup(Rc::new(AnimationVariableReference::from_raw_variable_name(value)));
    }
    CondValue::Str(value.to_string())
}

/// `Float.compare`.
fn java_float_compare(a: f32, b: f32) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    let ab = if a.is_nan() { 0x7fc0_0000 } else { a.to_bits() as i32 };
    let bb = if b.is_nan() { 0x7fc0_0000 } else { b.to_bits() as i32 };
    ab.cmp(&bb)
}

/// `String.compareTo` (UTF-16 code units).
fn java_string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

impl Condition {
    /// `IActionCondition.createInstance(conditionNode)`.
    pub fn create(node: &Element) -> Option<Condition> {
        match node.tag.as_str() {
            "isTrue" | "isFalse" => {
                let lhs = CondValue::Lookup(Rc::new(AnimationVariableReference::from_raw_variable_name(
                    pz_anim::vars::java_trim(&node.text_content()),
                )));
                Some(Condition::Variable { op: Operator::Equal, lhs, rhs: CondValue::Bool(node.tag == "isTrue") })
            }
            "compare" => {
                let op = match pz_anim::vars::java_trim(node.attr_or_empty("op")) {
                    "=" | "==" => Operator::Equal,
                    "!=" | "<>" => Operator::NotEqual,
                    "<" => Operator::Less,
                    ">" => Operator::Greater,
                    "<=" => Operator::LessEqual,
                    ">=" => Operator::GreaterEqual,
                    _ => return None,
                };
                Some(Self::compare_values(op, node))
            }
            "gtr" => Some(Self::compare_values(Operator::Greater, node)),
            "less" => Some(Self::compare_values(Operator::Less, node)),
            "equals" => Some(Self::compare_values(Operator::Equal, node)),
            "notEquals" => Some(Self::compare_values(Operator::NotEqual, node)),
            "lessEqual" => Some(Self::compare_values(Operator::LessEqual, node)),
            "gtrEqual" => Some(Self::compare_values(Operator::GreaterEqual, node)),
            "eventOccurred" => Some(Condition::EventOccurred(node.text_content().to_lowercase())),
            "eventNotOccurred" => Some(Condition::EventNotOccurred(node.text_content().to_lowercase())),
            "lua" => Some(Condition::LuaCall),
            _ => None,
        }
    }

    fn compare_values(op: Operator, node: &Element) -> Condition {
        let lhs = parse_value(pz_anim::vars::java_trim(node.attr_or_empty("a")), true);
        let rhs = parse_value(pz_anim::vars::java_trim(node.attr_or_empty("b")), false);
        Condition::Variable { op, lhs, rhs }
    }

    fn resolve(v: &CondValue, owner: &dyn AnimationVariableSource) -> Option<CondValue> {
        match v {
            CondValue::Lookup(r) => r.get_variable(owner).map(|s| parse_value(&s.as_string(), false)),
            other => Some(other.clone()),
        }
    }

    fn resolve_compare_to(op: Operator, r: Ordering) -> bool {
        match op {
            Operator::Equal => r == Ordering::Equal,
            Operator::NotEqual => r != Ordering::Equal,
            Operator::Less => r == Ordering::Less,
            Operator::LessEqual => r != Ordering::Greater,
            Operator::Greater => r == Ordering::Greater,
            Operator::GreaterEqual => r != Ordering::Less,
        }
    }

    /// `passes(context, state)`.
    pub fn passes(&self, events: &ActionContextEvents, owner: &dyn AnimationVariableSource, state_name: &str) -> bool {
        match self {
            Condition::EventOccurred(e) => events.contains(e, Some(state_name), true),
            Condition::EventNotOccurred(e) => !events.contains(e, Some(state_name), true),
            Condition::LuaCall => false,
            Condition::Variable { op, lhs, rhs } => Self::passes_variable(*op, lhs, rhs, owner),
        }
    }

    fn passes_variable(op: Operator, lhs_v: &CondValue, rhs_v: &CondValue, owner: &dyn AnimationVariableSource) -> bool {
        let lhs = Self::resolve(lhs_v, owner);
        let rhs = Self::resolve(rhs_v, owner);
        if let (None, Some(CondValue::Str(s))) = (&lhs, &rhs) {
            if s.is_empty() {
                if op == Operator::Equal {
                    return true;
                }
                if op == Operator::NotEqual {
                    return false;
                }
            }
        }
        let (Some(lhs), Some(rhs)) = (lhs, rhs) else { return false };
        let cmp = |o: Ordering| Self::resolve_compare_to(op, o);
        match (&lhs, &rhs) {
            (CondValue::Str(a), CondValue::Str(b)) => return cmp(java_string_compare(a, b)),
            (CondValue::Int(a), CondValue::Int(b)) => return cmp(a.cmp(b)),
            (CondValue::Float(a), CondValue::Float(b)) => return cmp(java_float_compare(*a, *b)),
            (CondValue::Bool(a), CondValue::Bool(b)) => return cmp(a.cmp(b)),
            _ => {}
        }
        let num = |v: &CondValue| match v {
            CondValue::Int(i) => Some((true, *i as f32, *i)),
            CondValue::Float(f) => Some((false, *f, *f as i32)),
            _ => None,
        };
        let (Some((lhs_int, lf, li)), Some((rhs_int, rf, ri))) = (num(&lhs), num(&rhs)) else { return false };
        let lhs_lookup = matches!(lhs_v, CondValue::Lookup(_));
        let rhs_lookup = matches!(rhs_v, CondValue::Lookup(_));
        if lhs_lookup == rhs_lookup {
            return cmp(java_float_compare(lf, rf));
        }
        if lhs_lookup {
            if !rhs_int {
                return cmp(java_float_compare(lf, rf));
            }
            return cmp(li.cmp(&ri));
        }
        if !lhs_int {
            return cmp(java_float_compare(lf, rf));
        }
        cmp(li.cmp(&ri))
    }
}

// ---- transitions and states ---------------------------------------------------

/// `ActionTransition`.
#[derive(Clone, Debug, Default)]
pub struct ActionTransition {
    pub transition_to: Option<String>,
    pub as_substate: bool,
    pub transition_out: bool,
    pub force_parent: bool,
    pub condition_priority: i32,
    pub conditions: Vec<Condition>,
}

impl ActionTransition {
    /// `ActionTransition.parse(root, srcInfo, transitions)`: `None` when the
    /// root is neither `<transitions>` nor `<transition>`.
    pub fn parse(root: &Element) -> Option<Vec<ActionTransition>> {
        match root.tag.as_str() {
            "transitions" => Some(root.elements().filter(|c| c.tag == "transition").map(Self::load).collect()),
            "transition" => Some(vec![Self::load(root)]),
            _ => None,
        }
    }

    fn load(e: &Element) -> ActionTransition {
        let mut t = ActionTransition::default();
        for child in e.elements() {
            let s = child.tag.as_str();
            if s.eq_ignore_ascii_case("transitionTo") {
                t.transition_to = Some(child.text_content());
            } else if s.eq_ignore_ascii_case("transitionOut") {
                t.transition_out = try_parse_boolean(&child.text_content());
            } else if s.eq_ignore_ascii_case("forceParent") {
                t.force_parent = try_parse_boolean(&child.text_content());
            } else if s.eq_ignore_ascii_case("asSubstate") {
                t.as_substate = try_parse_boolean(&child.text_content());
            } else if s.eq_ignore_ascii_case("conditionPriority") {
                t.condition_priority = try_parse_int(&child.text_content(), 0);
            } else if s.eq_ignore_ascii_case("conditions") {
                t.conditions.extend(child.elements().filter_map(Condition::create));
            }
        }
        t
    }

    /// `passes(context, state)`.
    pub fn passes(&self, events: &ActionContextEvents, owner: &dyn AnimationVariableSource, state_name: &str) -> bool {
        self.conditions.iter().all(|c| c.passes(events, owner, state_name))
    }
}

/// `PZMath.tryParseInt(s, default)`.
fn try_parse_int(s: &str, default: i32) -> i32 {
    if is_null_or_whitespace(s) {
        return default;
    }
    pz_anim::vars::java_trim(s).parse().unwrap_or(default)
}

/// `ActionState`.
#[derive(Clone, Debug)]
pub struct ActionState {
    /// The lowercased folder name.
    pub name: String,
    pub transitions: Vec<ActionTransition>,
    pub tags: Option<Vec<String>>,
    pub child_tags: Option<Vec<String>>,
    pub is_grappler_state: bool,
    /// `stateVariables` (read-only slots).
    pub state_variables: MapVariableSource,
}

impl ActionState {
    pub fn new(name: &str) -> Self {
        ActionState {
            name: name.to_string(),
            transitions: Vec::new(),
            tags: None,
            child_tags: None,
            is_grappler_state: false,
            state_variables: MapVariableSource::new(),
        }
    }

    /// `canHaveSubStates()`.
    pub fn can_have_sub_states(&self) -> bool {
        self.child_tags.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// `canBeSubstate()`.
    pub fn can_be_substate(&self) -> bool {
        self.tags.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// `ActionState.tagsOverlap(parentTags, childTags)`.
    pub fn tags_overlap(parent: Option<&[String]>, child: Option<&[String]>) -> bool {
        let (Some(p), Some(c)) = (parent, child) else { return false };
        if p.is_empty() || c.is_empty() {
            return false;
        }
        p.iter().any(|pt| c.iter().any(|ct| equals_ignore_case(pt, ct)))
    }

    /// `parse(file)` on an already parsed root.
    pub fn parse_root(&mut self, root: &Element) {
        if root.tag == "ActionState" {
            for child in root.elements() {
                if child.tag.eq_ignore_ascii_case("isGrapplerState") {
                    self.is_grappler_state = try_parse_boolean(&child.text_content());
                }
                if child.tag.eq_ignore_ascii_case("variables") {
                    self.parse_state_variables(child);
                }
            }
            return;
        }
        if let Some(mut loaded) = ActionTransition::parse(root) {
            loaded.retain(|t| t.transition_to.as_deref() != Some(self.name.as_str()));
            self.transitions.extend(loaded);
            return;
        }
        let tags = |root: &Element| root.elements().filter(|c| c.tag == "tag").map(|c| c.text_content()).collect::<Vec<_>>();
        if root.tag == "tags" {
            self.tags.get_or_insert_with(Vec::new).extend(tags(root));
        } else if root.tag == "childTags" {
            self.child_tags.get_or_insert_with(Vec::new).extend(tags(root));
        }
    }

    fn parse_state_variables(&mut self, root: &Element) {
        for child in root.elements() {
            if !child.tag.eq_ignore_ascii_case("var") {
                continue;
            }
            let key = child.attr_or_empty("key");
            if is_null_or_whitespace(key) {
                continue;
            }
            let ty = child.attr_or_empty("type");
            let value = child.text_content();
            if is_null_or_whitespace(ty) || ty.eq_ignore_ascii_case("string") {
                self.state_variables.set_variable(key, VariableValue::String(value));
            } else if ty.eq_ignore_ascii_case("bool") {
                self.state_variables.set_variable(key, VariableValue::Bool(try_parse_boolean(&value)));
            } else if ty.eq_ignore_ascii_case("float") {
                self.state_variables.set_variable(key, VariableValue::Float(try_parse_float(&value, 0.0)));
            } else if ty.eq_ignore_ascii_case("Enum") {
                if is_null_or_whitespace(child.attr_or_empty("enumType")) {
                    continue;
                }
                // TODO(enums): `AnimationVariableEnumParser.tryParse` checks
                // the value against the enum class; the slot holds the name.
                self.state_variables.set_variable(key, VariableValue::String(pz_anim::vars::java_trim(&value).to_string()));
            }
        }
        let keys: Vec<String> = self.state_variables.game_variables().map(|(k, _)| k.to_string()).collect();
        for k in keys {
            self.state_variables.set_read_only(&k, true);
        }
    }

    /// `sortTransitions()`: by descending priority, then descending
    /// condition count (stable).
    pub fn sort_transitions(&mut self) {
        self.transitions.sort_by(|l, r| {
            if r.condition_priority != l.condition_priority {
                return (r.condition_priority - l.condition_priority).cmp(&0);
            }
            (r.conditions.len() as i32 - l.conditions.len() as i32).cmp(&0)
        });
    }

    /// `hasStateVariables()`.
    pub fn has_state_variables(&self) -> bool {
        !self.state_variables.is_empty()
    }
}

/// `Collator.getInstance(Locale.ENGLISH)` on file names: `-` is
/// ignorable, `_` sorts before `.`, digits, then letters (case-insensitive);
/// case breaks ties (lower first), then the ignorable characters.
pub fn collator_compare(a: &str, b: &str) -> Ordering {
    fn key(c: char) -> (u32, u32) {
        match c {
            '_' => (0, 0),
            ',' => (2, 0),
            ';' => (3, 0),
            ':' => (4, 0),
            '!' => (5, 0),
            '?' => (6, 0),
            '.' => (7, 0),
            '\'' => (8, 0),
            '"' => (9, 0),
            '(' => (10, 0),
            ')' => (11, 0),
            '0'..='9' => (100 + c as u32 - '0' as u32, 0),
            c if c.is_ascii_alphabetic() => (200 + c.to_ascii_lowercase() as u32, c.is_ascii_uppercase() as u32),
            c => (50 + c as u32, 0),
        }
    }
    let ka: Vec<(u32, u32)> = a.chars().filter(|&c| c != '-').map(key).collect();
    let kb: Vec<(u32, u32)> = b.chars().filter(|&c| c != '-').map(key).collect();
    let pa: Vec<u32> = ka.iter().map(|k| k.0).collect();
    let pb: Vec<u32> = kb.iter().map(|k| k.0).collect();
    pa.cmp(&pb)
        .then_with(|| {
            let ta: Vec<u32> = ka.iter().map(|k| k.1).collect();
            let tb: Vec<u32> = kb.iter().map(|k| k.1).collect();
            ta.cmp(&tb)
        })
        .then_with(|| a.chars().filter(|&c| c == '-').count().cmp(&b.chars().filter(|&c| c == '-').count()))
        .then_with(|| a.cmp(b))
}

/// `ActionGroup`.
#[derive(Debug, Default)]
pub struct ActionGroup {
    pub name: String,
    pub initial_state_name: Option<String>,
    pub states: Vec<ActionState>,
    lookup: HashMap<String, usize>,
}

impl ActionGroup {
    pub fn new(name: &str) -> Self {
        ActionGroup { name: name.to_lowercase(), ..Default::default() }
    }

    /// `findState(name)`.
    pub fn find_state(&self, name: &str) -> Option<usize> {
        self.lookup.get(&name.to_lowercase()).copied()
    }

    /// `getOrCreate(name)`.
    pub fn get_or_create(&mut self, name: &str) -> usize {
        let name = name.to_lowercase();
        if let Some(i) = self.find_state(&name) {
            return i;
        }
        self.states.push(ActionState::new(&name));
        let i = self.states.len() - 1;
        self.lookup.insert(name, i);
        i
    }

    /// `getInitialState()` / `getDefaultState()`.
    pub fn initial_state(&self) -> Option<usize> {
        let mut s = None;
        if let Some(n) = &self.initial_state_name {
            s = self.find_state(n);
        }
        if s.is_none() && !self.states.is_empty() {
            s = Some(0);
        }
        s
    }

    /// `loadGroupData`: the `<initial>` of `<actiongroup>`.
    pub fn load_group_data(&mut self, root: &Element) {
        if root.tag != "actiongroup" {
            return;
        }
        for child in root.elements() {
            if child.tag == "initial" {
                self.initial_state_name = Some(pz_anim::vars::java_trim(&child.text_content()).to_string());
            }
        }
    }

    /// Adds one state folder's parsed files (in file order) and sorts its
    /// transitions (`ActionState.load`).
    pub fn load_state(&mut self, folder: &str, roots: &[Element]) {
        let i = self.get_or_create(folder);
        for r in roots {
            self.states[i].parse_root(r);
        }
        self.states[i].sort_transitions();
    }

    /// `ActionGroup.load()` from `<media>/actiongroups/<name>`: state
    /// folders in name order, files in `Collator` order. Files that fail
    /// to parse are skipped (the original logs them).
    pub fn load_from_media(media: &Path, name: &str) -> ActionGroup {
        let mut g = ActionGroup::new(name);
        let dir = media.join("actiongroups").join(g.name.clone());
        let dir = xml::resolve_case_insensitive(&dir).unwrap_or(dir);
        let group_file = dir.join("actionGroup.xml");
        if let Ok(root) = xml::parse_xml_file(&group_file) {
            let mut root = root;
            root.normalize();
            g.load_group_data(&root);
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { return g };
        let mut folders: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            let fname = folder.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase().ends_with(".xml")))
                        .collect()
                })
                .unwrap_or_default();
            files.sort_by(|a, b| {
                collator_compare(
                    a.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                    b.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                )
            });
            let roots: Vec<Element> = files.iter().filter_map(|f| xml::parse_xml_file(f).ok()).collect();
            g.load_state(&fname, &roots);
        }
        g
    }

    /// A group from in-memory XML: `(folder, [(file name, text)])`, files
    /// sorted like the original.
    pub fn from_xml(name: &str, group_xml: Option<&str>, folders: &[(&str, &[(&str, &str)])]) -> ActionGroup {
        let mut g = ActionGroup::new(name);
        if let Some(Ok(mut root)) = group_xml.map(xml::parse) {
            root.normalize();
            g.load_group_data(&root);
        }
        for (folder, files) in folders {
            let mut files: Vec<&(&str, &str)> = files.iter().collect();
            files.sort_by(|a, b| collator_compare(a.0, b.0));
            let roots: Vec<Element> = files.iter().filter_map(|(_, t)| xml::parse(t).ok()).collect();
            g.load_state(folder, &roots);
        }
        g
    }
}

/// The loaded groups (`ActionGroup.s_actionGroupMap`), by lowercase name.
#[derive(Default)]
pub struct ActionGroups {
    media: Option<PathBuf>,
    groups: HashMap<String, Rc<ActionGroup>>,
}

impl ActionGroups {
    /// Groups loaded from `<media>/actiongroups`.
    pub fn from_media(media: impl Into<PathBuf>) -> Self {
        ActionGroups { media: Some(media.into()), groups: HashMap::new() }
    }

    /// Adds a group built elsewhere (tests, mods).
    pub fn insert(&mut self, g: ActionGroup) -> Rc<ActionGroup> {
        let g = Rc::new(g);
        self.groups.insert(g.name.clone(), g.clone());
        g
    }

    /// `ActionGroup.getActionGroup(name)`: loaded once; an unknown name is
    /// an empty group.
    pub fn get(&mut self, name: &str) -> Rc<ActionGroup> {
        let key = name.to_lowercase();
        if let Some(g) = self.groups.get(&key) {
            return g.clone();
        }
        let g = match &self.media {
            Some(m) => ActionGroup::load_from_media(m, &key),
            None => ActionGroup::new(&key),
        };
        self.insert(g)
    }
}

// ---- the context ------------------------------------------------------------

/// An action state of a loaded group (compared by identity).
#[derive(Clone, Debug)]
pub struct StateHandle {
    pub group: Rc<ActionGroup>,
    pub index: usize,
}

impl PartialEq for StateHandle {
    fn eq(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.group, &o.group) && self.index == o.index
    }
}

impl StateHandle {
    pub fn state(&self) -> &ActionState {
        &self.group.states[self.index]
    }
    pub fn name(&self) -> &str {
        &self.state().name
    }
    fn can_have_sub_state(&self, child: &StateHandle) -> bool {
        if self == child {
            return false;
        }
        ActionState::tags_overlap(self.state().child_tags.as_deref(), child.state().tags.as_deref())
    }
}

/// `ActionStateContainer`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionStateContainer {
    pub current: Option<StateHandle>,
    pub children: Vec<StateHandle>,
    /// `transitionUsedForThisState`: the transition's state and index.
    pub transition_used: Option<(StateHandle, usize)>,
}

impl ActionStateContainer {
    fn transition_used_is_out(&self) -> bool {
        self.transition_used.as_ref().is_some_and(|(s, i)| s.state().transitions[*i].transition_out)
    }

    fn supports_child(&self, child: &StateHandle) -> bool {
        self.current.as_ref().is_some_and(|c| c.can_have_sub_state(child))
    }

    /// `hasChildState`.
    pub fn has_child(&self, child: &StateHandle) -> bool {
        self.children.iter().any(|c| c == child)
    }

    /// `setCurrentState(next, transitionUsed)`.
    pub fn set_current_state(&mut self, next: StateHandle, used: Option<(StateHandle, usize)>) -> bool {
        if self.current.as_ref() == Some(&next) {
            return false;
        }
        self.current = Some(next.clone());
        self.transition_used = used;
        self.children.retain(|c| next.can_have_sub_state(c));
        true
    }

    fn try_insert_child_state(&mut self, next: StateHandle) -> bool {
        if self.has_child(&next) || !self.supports_child(&next) {
            return false;
        }
        let mut insert_at = None;
        for (i, lower) in self.children.iter().enumerate() {
            if next.can_have_sub_state(lower) {
                insert_at = Some(i);
                break;
            }
            if !lower.can_have_sub_state(&next) {
                return false;
            }
        }
        match insert_at {
            Some(i) => self.children.insert(i, next),
            None => self.children.push(next),
        }
        true
    }

    /// `subStatesEqual`.
    pub fn sub_states_equal(&self, o: &ActionStateContainer) -> bool {
        self.children == o.children
    }

    /// `equalTo`.
    pub fn equal_to(&self, o: &ActionStateContainer) -> bool {
        self.current == o.current && self.sub_states_equal(o)
    }

    /// `getVariable(handle)`: the children last to first, then the root.
    pub fn get_variable(&self, handle: &AnimationVariableHandle) -> Option<VariableValue> {
        for c in self.children.iter().rev() {
            if let Some(v) = c.state().state_variables.get_variable(handle) {
                return Some(v);
            }
        }
        self.current.as_ref()?.state().state_variables.get_variable(handle)
    }

    /// `hasStateVariables`.
    pub fn has_state_variables(&self) -> bool {
        self.children.iter().any(|c| c.state().has_state_variables())
            || self.current.as_ref().is_some_and(|c| c.state().has_state_variables())
    }
}

/// `ActionContextEvents`: the events reported since the last
/// `postUpdate`, newest first.
#[derive(Clone, Debug, Default)]
pub struct ActionContextEvents {
    events: Vec<(String, Option<String>)>,
}

impl ActionContextEvents {
    /// `add(name, state)`.
    pub fn add(&mut self, name: &str, state: Option<&str>) {
        if self.contains(name, state, false) {
            return;
        }
        self.events.insert(0, (name.to_string(), state.map(str::to_string)));
    }

    /// `contains(name, state, agnosticLayer)`.
    pub fn contains(&self, name: &str, state: Option<&str>, agnostic: bool) -> bool {
        self.events.iter().any(|(n, s)| {
            if !equals_ignore_case(n, name) {
                return false;
            }
            match state {
                None => true,
                Some(st) => {
                    s.as_deref().is_some_and(|s| equals_ignore_case(s, st)) || (s.is_none() && st.is_empty())
                        || (agnostic && s.is_none())
                }
            }
        })
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// `clearEvent(name)`.
    pub fn clear_event(&mut self, name: &str) {
        self.events.retain(|(n, _)| !equals_ignore_case(n, name));
    }

    /// The events without a state (`get(events)`), newest first.
    pub fn stateless(&self) -> Vec<&str> {
        self.events.iter().filter(|(_, s)| s.is_none()).map(|(n, _)| n.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// `ActionContext.actionStateHistoryMaxSize`.
const HISTORY_MAX: usize = 6;

/// `ActionContext`.
#[derive(Clone, Debug, Default)]
pub struct ActionContext {
    group: Option<Rc<ActionGroup>>,
    history: Vec<StateHandle>,
    previous: ActionStateContainer,
    container: ActionStateContainer,
    states_changed: bool,
    events: ActionContextEvents,
}

impl ActionContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// `getGroup()`.
    pub fn group(&self) -> Option<&Rc<ActionGroup>> {
        self.group.as_ref()
    }

    /// `setGroup(group)`.
    pub fn set_group(&mut self, group: Rc<ActionGroup>) {
        self.group = Some(group.clone());
        let initial = group.initial_state().map(|i| StateHandle { group, index: i });
        self.set_current_state(initial);
    }

    /// `setCurrentState(next)`.
    pub fn set_current_state(&mut self, next: Option<StateHandle>) {
        let mut c = ActionStateContainer::default();
        if let Some(n) = next {
            c.set_current_state(n, None);
        }
        self.transfer_action_state(c);
    }

    fn handle(&self, index: usize) -> Option<StateHandle> {
        self.group.as_ref().map(|g| StateHandle { group: g.clone(), index })
    }

    fn find(&self, name: &str) -> Option<StateHandle> {
        let g = self.group.as_ref()?;
        g.find_state(name).map(|i| StateHandle { group: g.clone(), index: i })
    }

    fn transition_to(&self, t: &ActionTransition) -> Option<String> {
        if t.transition_out {
            return Some(self.peek_previous_state_name());
        }
        t.transition_to.clone()
    }

    /// `updateInternal`'s evaluation: the next container from the current
    /// one, the transitions and `owner`'s variables (the owner's
    /// `getVariable` sees the current container).
    pub fn evaluate_next(&self, owner: &dyn AnimationVariableSource) -> ActionStateContainer {
        let mut next = self.container.clone();
        let Some(cur) = next.current.clone() else { return next };
        // evaluateCurrentStateTransitions: the first passing root transition.
        for (ti, t) in cur.state().transitions.iter().enumerate() {
            if t.as_substate {
                continue;
            }
            let Some(to) = self.transition_to(t) else { continue };
            if is_null_or_whitespace(&to) {
                continue;
            }
            if !t.passes(&self.events, owner, &cur.state().name) {
                continue;
            }
            let Some(ns) = self.find(&to) else { continue };
            next.set_current_state(ns, Some((cur.clone(), ti)));
            break;
        }
        // The sub-state transitions of the (possibly new) root.
        let root = next.current.clone().expect("set above");
        for t in root.state().transitions.iter() {
            if !t.as_substate {
                continue;
            }
            let Some(to) = self.transition_to(t) else { continue };
            if is_null_or_whitespace(&to) {
                continue;
            }
            if !t.passes(&self.events, owner, &root.state().name) {
                continue;
            }
            let Some(ns) = self.find(&to) else { continue };
            next.try_insert_child_state(ns);
        }
        // evaluateSubStateTransitions.
        let mut i = 0i32;
        while (i as usize) < next.children.len() {
            let sub = next.children[i as usize].clone();
            let mut next_state: Option<StateHandle> = None;
            for t in sub.state().transitions.iter() {
                if !t.passes(&self.events, owner, &sub.state().name) {
                    continue;
                }
                if t.transition_out {
                    next.children.remove(i as usize);
                    i -= 1;
                    break;
                }
                let Some(to) = t.transition_to.as_deref() else { continue };
                if is_null_or_whitespace(to) {
                    continue;
                }
                let Some(ns) = self.find(to) else { continue };
                if next.has_child(&ns) {
                    continue;
                }
                if next.supports_child(&ns) {
                    next.children[i as usize] = ns;
                    break;
                }
                if !t.force_parent {
                    continue;
                }
                next_state = Some(ns);
                break;
            }
            if let Some(ns) = next_state {
                if next.current.as_ref() != Some(&ns) {
                    next.set_current_state(ns, None);
                }
            }
            i += 1;
        }
        next
    }

    /// `transferActionState(next)`.
    pub fn transfer_action_state(&mut self, next: ActionStateContainer) {
        if self.container.equal_to(&next) {
            return;
        }
        self.previous = std::mem::replace(&mut self.container, next);
        let prev_root = self.previous.current.clone();
        let cur_root = self.container.current.clone();
        let used_out = self.container.transition_used_is_out();
        self.update_history(prev_root, cur_root, self.container.transition_used.is_some(), used_out);
        self.states_changed = true;
    }

    fn update_history(&mut self, previous: Option<StateHandle>, current: Option<StateHandle>, used: bool, used_out: bool) {
        let Some(current) = current else { return };
        if previous.as_ref() == Some(&current) {
            return;
        }
        let Some(previous) = previous else {
            self.history.clear();
            return;
        };
        if !self.history.is_empty() && used && used_out {
            if self.history.last() == Some(&current) {
                self.history.pop();
                return;
            }
            self.history.clear();
        }
        self.push_previous_state(previous);
    }

    fn push_previous_state(&mut self, s: StateHandle) {
        if self.history.last() != Some(&s) {
            self.history.push(s);
        }
        while self.history.len() >= HISTORY_MAX {
            self.history.remove(0);
        }
    }

    /// `postUpdateInternal`'s first two steps: clears the events and
    /// returns whether the states changed since the last call (the
    /// `onStateChanged` callbacks run then).
    pub fn post_update(&mut self) -> bool {
        self.events.clear();
        std::mem::take(&mut self.states_changed)
    }

    /// `update()` without the state-changed callbacks: evaluate, transfer,
    /// clear the events. Returns whether the callbacks are due.
    pub fn update(&mut self, owner: &dyn AnimationVariableSource) -> bool {
        let next = self.evaluate_next(owner);
        self.transfer_action_state(next);
        self.post_update()
    }

    /// `peekNextState()`.
    pub fn peek_next_state(&self, owner: &dyn AnimationVariableSource) -> Option<StateHandle> {
        let cur = self.container.current.clone()?;
        let mut result = None;
        for t in cur.state().transitions.iter() {
            let Some(to) = self.transition_to(t) else { continue };
            if is_null_or_whitespace(&to) || !t.passes(&self.events, owner, &cur.state().name) {
                continue;
            }
            let Some(ns) = self.find(&to) else { continue };
            if self.container.has_child(&ns) || (t.as_substate && !self.container.supports_child(&ns)) {
                continue;
            }
            result = Some(ns);
            break;
        }
        for sub in &self.container.children {
            let mut next_state = None;
            for t in sub.state().transitions.iter() {
                if !t.passes(&self.events, owner, &sub.state().name) {
                    continue;
                }
                if t.transition_out {
                    break;
                }
                let Some(to) = self.transition_to(t) else { continue };
                if is_null_or_whitespace(&to) {
                    continue;
                }
                let Some(ns) = self.find(&to) else { continue };
                if self.container.has_child(&ns) {
                    continue;
                }
                if self.container.supports_child(&ns) {
                    break;
                }
                if !t.force_parent {
                    continue;
                }
                next_state = Some(ns);
                break;
            }
            if let Some(ns) = next_state {
                if self.container.current.as_ref() != Some(&ns) {
                    result = Some(ns);
                }
            }
        }
        result
    }

    /// `canTransitionToState(name, allowSubState)`.
    pub fn can_transition_to_state(&self, name: &str, allow_sub_state: bool) -> bool {
        let Some(ns) = self.find(name) else { return false };
        let Some(cur) = &self.container.current else { return false };
        if cur.state().transitions.iter().any(|t| t.transition_to.as_deref().is_some_and(|to| equals_ignore_case(name, to))) {
            return true;
        }
        if !allow_sub_state || !self.container.supports_child(&ns) {
            return false;
        }
        self.container.children.iter().any(|sub| {
            sub.state()
                .transitions
                .iter()
                .any(|t| !t.transition_out && t.transition_to.as_deref().is_some_and(|to| equals_ignore_case(name, to)))
        })
    }

    /// `reportEvent(state, event)`.
    pub fn report_event(&mut self, state: Option<&str>, event: &str) {
        self.events.add(event, state);
    }

    /// `hasEventOccurred(name, state)`.
    pub fn has_event_occurred(&self, name: &str, state: Option<&str>) -> bool {
        self.events.contains(name, state, true)
    }

    /// `clearEvent(name)`.
    pub fn clear_event(&mut self, name: &str) {
        self.events.clear_event(name);
    }

    /// `clearActionContextEvents()`.
    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    /// The reported events.
    pub fn events(&self) -> &ActionContextEvents {
        &self.events
    }

    /// `getCurrentState()`.
    pub fn current_state(&self) -> Option<&StateHandle> {
        self.container.current.as_ref()
    }

    /// `getChildStates()`.
    pub fn child_states(&self) -> &[StateHandle] {
        &self.container.children
    }

    /// The current container.
    pub fn container(&self) -> &ActionStateContainer {
        &self.container
    }

    /// `getCurrentStateName()`: the default state's name without a root.
    pub fn current_state_name(&self) -> String {
        match &self.container.current {
            Some(c) => c.name().to_string(),
            None => self.default_state_name(),
        }
    }

    fn default_state_name(&self) -> String {
        self.group
            .as_ref()
            .and_then(|g| g.initial_state().map(|i| g.states[i].name.clone()))
            .unwrap_or_default()
    }

    /// `peekPreviousState()`.
    pub fn peek_previous_state(&self) -> Option<&StateHandle> {
        self.history.last()
    }

    /// `popPreviousState()`.
    pub fn pop_previous_state(&mut self) -> Option<StateHandle> {
        self.history.pop()
    }

    /// `peekPreviousStateName()`.
    pub fn peek_previous_state_name(&self) -> String {
        match self.history.last() {
            Some(s) => s.name().to_string(),
            None => self.default_state_name(),
        }
    }

    /// The history, oldest first (debug).
    pub fn history(&self) -> Vec<String> {
        self.history.iter().map(|s| s.name().to_string()).collect()
    }

    /// `getVariable(handle)`.
    pub fn get_variable(&self, handle: &AnimationVariableHandle) -> Option<VariableValue> {
        self.container.get_variable(handle)
    }

    /// `hasStateVariables()`.
    pub fn has_state_variables(&self) -> bool {
        self.container.has_state_variables()
    }

    /// The state with `index` in the current group.
    pub fn state_at(&self, index: usize) -> Option<StateHandle> {
        self.handle(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> ActionGroup {
        let idle: &[(&str, &str)] = &[
            ("idle.xml", "<ActionState><variables><var key=\"IdleVar\" type=\"bool\">true</var></variables></ActionState>"),
            ("childTags.xml", "<childTags><tag>ext</tag></childTags>"),
            ("to_run.xml", "<transition><transitionTo>run</transitionTo><conditions><isTrue>isRunning</isTrue></conditions></transition>"),
            ("to_hit.xml", "<transition><transitionTo>hit</transitionTo><conditions><eventOccurred>wasHit</eventOccurred></conditions></transition>"),
            (
                "to_ext.xml",
                "<transition><transitionTo>ext</transitionTo><asSubstate>true</asSubstate><conditions><compare op=\"&gt;\" a=\"Speed\" b=\"1.5\"/></conditions></transition>",
            ),
        ];
        let run: &[(&str, &str)] = &[(
            "transitionOut.xml",
            "<transition><transitionOut>true</transitionOut><conditions><isFalse>isRunning</isFalse></conditions></transition>",
        )];
        let hit: &[(&str, &str)] = &[(
            "out.xml",
            "<transitions><transition><transitionOut>true</transitionOut><conditions><eventOccurred>Done</eventOccurred></conditions></transition></transitions>",
        )];
        let ext: &[(&str, &str)] = &[
            ("tags.xml", "<tags><tag>ext</tag></tags>"),
            ("out.xml", "<transition><transitionOut>true</transitionOut><conditions><equals a=\"Ext\" b=\"\"/></conditions></transition>"),
        ];
        ActionGroup::from_xml(
            "test",
            Some("<actiongroup><initial>Idle</initial></actiongroup>"),
            &[("Idle", idle), ("Run", run), ("hit", hit), ("ext", ext)],
        )
    }

    #[test]
    fn values_parse_like_the_original() {
        assert!(matches!(parse_value("12", false), CondValue::Int(12)));
        assert!(matches!(parse_value("-12", false), CondValue::Int(12)));
        assert!(matches!(parse_value("1,000", false), CondValue::Int(1000)));
        match parse_value("-1.25", false) {
            CondValue::Float(f) => assert_eq!(f, -1.25),
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_value("yes", true), CondValue::Bool(true)));
        assert!(matches!(parse_value("'a b'", true), CondValue::Str(ref s) if s == "a b"));
        assert!(matches!(parse_value("abc", true), CondValue::Lookup(_)));
        assert!(matches!(parse_value("abc", false), CondValue::Str(_)));
        assert!(matches!(parse_value("1x", false), CondValue::Str(_)));
    }

    #[test]
    fn transitions_follow_variables_and_events() {
        let mut g = ActionGroups::default();
        let g = g.insert(group());
        assert_eq!(g.states[0].name, "idle");
        let mut ctx = ActionContext::new();
        ctx.set_group(g.clone());
        assert_eq!(ctx.current_state_name(), "idle");
        let mut vars = MapVariableSource::new();
        assert!(ctx.update(&vars));
        assert!(!ctx.update(&vars));
        assert!(ctx.has_state_variables());
        assert!(ctx.get_variable(&AnimationVariableHandle::alloc("IdleVar").unwrap()).is_some_and(|v| v.as_bool()));
        vars.set_variable("isRunning", VariableValue::Bool(true));
        assert!(ctx.update(&vars));
        assert_eq!(ctx.current_state_name(), "run");
        assert_eq!(ctx.peek_previous_state_name(), "idle");
        vars.set_variable("isRunning", VariableValue::Bool(false));
        ctx.update(&vars);
        assert_eq!(ctx.current_state_name(), "idle");
        assert!(ctx.history().is_empty(), "transitionOut pops the history");
        // A sub state by comparison.
        vars.set_variable("Speed", VariableValue::Float(2.0));
        vars.set_variable("Ext", VariableValue::String("x".into()));
        ctx.update(&vars);
        assert_eq!(ctx.child_states().iter().map(|c| c.name()).collect::<Vec<_>>(), ["ext"]);
        vars.set_variable("Ext", VariableValue::String(String::new()));
        vars.set_variable("Speed", VariableValue::Float(1.0));
        ctx.update(&vars);
        assert!(ctx.child_states().is_empty());
        // An event for the root state's layer.
        ctx.report_event(Some("idle"), "wasHit");
        ctx.update(&vars);
        assert_eq!(ctx.current_state_name(), "hit");
        ctx.report_event(None, "Done");
        ctx.update(&vars);
        assert_eq!(ctx.current_state_name(), "idle");
    }

    #[test]
    fn events_match_states() {
        let mut e = ActionContextEvents::default();
        e.add("A", Some("idle"));
        e.add("a", Some("idle"));
        e.add("B", None);
        assert_eq!(e.stateless(), ["B"]);
        assert!(e.contains("a", Some("IDLE"), true));
        assert!(!e.contains("a", Some("run"), true));
        assert!(e.contains("b", Some("run"), true));
        assert!(!e.contains("b", Some("run"), false));
        assert!(e.contains("a", None, true));
        e.clear_event("A");
        assert!(!e.contains("a", None, true));
    }

    #[test]
    fn collator_orders_file_names() {
        // The order `Collator.getInstance(Locale.ENGLISH)` gives.
        let mut v = vec![
            "to_b.xml", "To_a.xml", "tags.xml", "to-a.xml", "to_A.xml", "t1.xml", "toa.xml", "to.xml", "to_.xml",
            "transitionOut.xml", "to_hitreaction-bite.xml", "to_hitreaction.xml", "to_hitreactionpvp.xml",
            "to_climbFence.xml", "to_climbdownrope.xml",
        ];
        v.sort_by(|a, b| collator_compare(a, b));
        assert_eq!(
            v,
            [
                "t1.xml", "tags.xml", "to_.xml", "to_A.xml", "To_a.xml", "to_b.xml", "to_climbdownrope.xml",
                "to_climbFence.xml", "to_hitreaction.xml", "to_hitreaction-bite.xml", "to_hitreactionpvp.xml", "to.xml",
                "toa.xml", "to-a.xml", "transitionOut.xml",
            ]
        );
    }
}
