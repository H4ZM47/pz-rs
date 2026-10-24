//! `zombie.scripting.objects.TimedActionScript` with `ActionSoundTime`.
//!
//! The `Metabolics` and `BodyPartType` constants it names are game data read
//! from the jar ([`crate::registry::builtin::BuiltinTables`]); the script
//! holds their constant names.

use super::{parse_float, parse_int};
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

/// `zombie.scripting.objects.ActionSoundTime`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionSoundTime {
    ActionStart,
    AnimationEvent,
    AnimationStart,
}

impl ActionSoundTime {
    pub const ALL: &'static [ActionSoundTime] =
        &[ActionSoundTime::ActionStart, ActionSoundTime::AnimationEvent, ActionSoundTime::AnimationStart];

    /// `toString`: the script id.
    pub fn id(self) -> &'static str {
        match self {
            ActionSoundTime::ActionStart => "action_start",
            ActionSoundTime::AnimationEvent => "animation_event",
            ActionSoundTime::AnimationStart => "animation_start",
        }
    }

    /// `fromValue`: exact, case-sensitive id.
    pub fn from_value(value: &str) -> ScriptResult<ActionSoundTime> {
        Self::ALL.iter().copied().find(|t| t.id() == value).ok_or_else(|| {
            ScriptError::new(format!("java.lang.IllegalArgumentException: No enum constant for value: {value}"))
        })
    }
}

/// The original's field initializer, `Metabolics.Default`.
const DEFAULT_METABOLICS: &str = "Default";

/// Ids of the perks `PerkFactory.Perks` creates statically, which is what
/// `Perks.FromString` finds at script load. Perks added later from Lua are
/// not known here. TODO(PerkFactory)

#[derive(Debug)]
pub struct TimedActionScript {
    pub base: BaseScriptObject,
    /// A `Metabolics` constant name.
    metabolics: String,
    time: i32,
    face_object: bool,
    prop1: Option<String>,
    prop2: Option<String>,
    action_anim: Option<String>,
    anim_var_key: Option<String>,
    anim_var_val: Option<String>,
    sound: Option<String>,
    sound_time: ActionSoundTime,
    completion_sound: Option<String>,
    muscle_strain_factor: f32,
    /// `BodyPartType` constant names.
    muscle_strain_parts: Vec<String>,
    /// Perk id; `None` is Java's `null`.
    muscle_strain_skill: Option<String>,
    cant_sit: bool,
}

impl TimedActionScript {
    pub fn new() -> Self {
        TimedActionScript {
            base: BaseScriptObject::new(ScriptType::TimedAction),
            metabolics: DEFAULT_METABOLICS.to_string(),
            time: -1,
            face_object: false,
            prop1: None,
            prop2: None,
            action_anim: None,
            anim_var_key: None,
            anim_var_val: None,
            sound: None,
            sound_time: ActionSoundTime::ActionStart,
            completion_sound: None,
            muscle_strain_factor: 0.0,
            muscle_strain_parts: Vec::new(),
            muscle_strain_skill: None,
            cant_sit: false,
        }
    }

    /// `getName`.
    pub fn name(&self) -> &str {
        self.base.script_object_name()
    }

    /// `getFullType`.
    pub fn full_type(&self) -> ScriptResult<String> {
        self.base.full_type()
    }

    /// `getMetabolics`: the constant's name.
    pub fn metabolics(&self) -> &str {
        &self.metabolics
    }

    /// `getTime`.
    pub fn time(&self) -> i32 {
        self.time
    }

    /// `isFaceObject`.
    pub fn is_face_object(&self) -> bool {
        self.face_object
    }

    /// `isCantSit`.
    pub fn is_cant_sit(&self) -> bool {
        self.cant_sit
    }

    /// `getProp1`.
    pub fn prop1(&self) -> Option<&str> {
        self.prop1.as_deref()
    }

    /// `getProp2`.
    pub fn prop2(&self) -> Option<&str> {
        self.prop2.as_deref()
    }

    /// `getActionAnim`.
    pub fn action_anim(&self) -> Option<&str> {
        self.action_anim.as_deref()
    }

    /// `getAnimVarKey`.
    pub fn anim_var_key(&self) -> Option<&str> {
        self.anim_var_key.as_deref()
    }

    /// `getAnimVarVal`.
    pub fn anim_var_val(&self) -> Option<&str> {
        self.anim_var_val.as_deref()
    }

    /// `getSound`.
    pub fn sound(&self) -> Option<&str> {
        self.sound.as_deref()
    }

    /// `getSoundTime`.
    pub fn sound_time(&self) -> ActionSoundTime {
        self.sound_time
    }

    /// `getCompletionSound`.
    pub fn completion_sound(&self) -> Option<&str> {
        self.completion_sound.as_deref()
    }

    pub fn muscle_strain_factor(&self) -> f32 {
        self.muscle_strain_factor
    }

    /// The `BodyPartType` constant names.
    pub fn muscle_strain_parts(&self) -> &[String] {
        &self.muscle_strain_parts
    }

    pub fn muscle_strain_skill(&self) -> Option<&str> {
        self.muscle_strain_skill.as_deref()
    }

    /// `hasMuscleStrain`. Applying it (`applyMuscleStrain`) is character
    /// logic. TODO(IsoGameCharacter)
    pub fn has_muscle_strain(&self) -> bool {
        !self.muscle_strain_parts.is_empty() && self.muscle_strain_factor > 0.0
    }
}

impl Default for TimedActionScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for TimedActionScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.TimedActionScript"
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "metabolics") {
                self.metabolics = sm.builtin_tables.borrow().metabolics_value_of(val)?;
            } else if java::eq_ignore_case(key, "time") {
                self.time = parse_int(val)?;
            } else if java::eq_ignore_case(key, "faceObject") {
                self.face_object = java::parse_bool(val);
            } else if java::eq_ignore_case(key, "cantSit") {
                self.cant_sit = java::parse_bool(val);
            } else if java::eq_ignore_case(key, "prop1") {
                self.prop1 = Some(val.to_string());
            } else if java::eq_ignore_case(key, "prop2") {
                self.prop2 = Some(val.to_string());
            } else if java::eq_ignore_case(key, "actionAnim") {
                self.action_anim = Some(val.to_string());
            } else if java::eq_ignore_case(key, "animVarKey") {
                self.anim_var_key = Some(val.to_string());
            } else if java::eq_ignore_case(key, "animVarVal") {
                self.anim_var_val = Some(val.to_string());
            } else if java::eq_ignore_case(key, "sound") {
                self.sound = Some(val.to_string());
            } else if java::eq_ignore_case(key, "soundTime") {
                self.sound_time = ActionSoundTime::from_value(val)?;
            } else if java::eq_ignore_case(key, "completionSound") {
                self.completion_sound = Some(val.to_string());
            } else if java::eq_ignore_case(key, "muscleStrainFactor") {
                self.muscle_strain_factor = parse_float(val)?;
            } else if java::eq_ignore_case(key, "muscleStrainSkill") {
                // Perks.FromString is an exact HashMap lookup defaulting to MAX.
                if sm.builtin_tables.borrow().perk_from_string(val) == "MAX" {
                    // The original formats the script with Object.toString.
                    sm.warn(&format!("Unknown skill \"{val}\" in timedaction script \"{}\"", self.class_name()));
                    continue;
                }
                self.muscle_strain_skill = Some(val.to_string());
            } else if java::eq_ignore_case(key, "muscleStrainParts") {
                let tables = sm.builtin_tables.borrow();
                self.muscle_strain_parts =
                    java::split_char(val, ';').into_iter().map(|s| tables.body_part_from_string(java::trim(s))).collect();
            }
        }
        Ok(())
    }

    /// `PreReload`. The muscle strain fields are not reset.
    fn pre_reload(&mut self) {
        self.metabolics = DEFAULT_METABOLICS.to_string();
        self.time = -1;
        self.face_object = false;
        self.cant_sit = false;
        self.prop1 = None;
        self.prop2 = None;
        self.action_anim = None;
        self.anim_var_key = None;
        self.anim_var_val = None;
        self.sound = None;
        self.sound_time = ActionSoundTime::ActionStart;
        self.completion_sound = None;
    }
}
