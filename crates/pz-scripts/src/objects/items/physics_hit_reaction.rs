//! `zombie.scripting.objects.PhysicsHitReactionScript` and
//! `PhysicsHitReaction`: ragdoll impulses per ammo type or physics object.

use std::rc::Rc;

use super::keys::AmmoType;
use super::{at, parse_float};
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::{Block, BlockElement};
use crate::registry::RegistryKey;
use crate::script_type::ScriptType;

/// `RagdollBodyPart.BODYPART_COUNT.ordinal()`: the number of ragdoll body
/// parts, the length of the impulse arrays.
pub const BODYPART_COUNT: usize = 11;

/// `zombie.scripting.objects.PhysicsHitReaction`.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsHitReaction {
    pub use_impulse_override: bool,
    pub ammo_type: Option<Rc<RegistryKey>>,
    pub physics_object: Option<String>,
    pub override_forward_impulse: f32,
    pub override_upward_impulse: f32,
    pub impulse: [f32; BODYPART_COUNT],
    pub upward_impulse: [f32; BODYPART_COUNT],
}

impl Default for PhysicsHitReaction {
    fn default() -> Self {
        PhysicsHitReaction {
            use_impulse_override: false,
            ammo_type: None,
            physics_object: None,
            override_forward_impulse: 80.0,
            override_upward_impulse: 40.0,
            impulse: [0.0; BODYPART_COUNT],
            upward_impulse: [0.0; BODYPART_COUNT],
        }
    }
}

/// Stores `value` at `index` (post-incremented) with Java's array check.
fn store(arr: &mut [f32; BODYPART_COUNT], index: &mut usize, value: f32) -> ScriptResult {
    let i = *index;
    *index += 1;
    let slot = arr.get_mut(i).ok_or_else(|| {
        ScriptError::new(format!("java.lang.ArrayIndexOutOfBoundsException: Index {i} out of bounds for length {BODYPART_COUNT}"))
    })?;
    *slot = value;
    Ok(())
}

fn same_ammo(a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.location == b.location,
        (None, None) => true,
        _ => false,
    }
}

/// `PhysicsHitReactionScript.getImpulse(RagdollBodyPart, String)`.
pub fn impulse_for_object(
    sm: &ScriptManager,
    body_part: usize,
    physics_object: Option<&str>,
) -> f32 {
    let st = sm.statics.items.borrow();
    match st
        .physics_hit_reaction_list
        .iter()
        .find(|r| r.physics_object.as_deref() == physics_object)
    {
        Some(r) if r.use_impulse_override => r.override_forward_impulse,
        Some(r) => r.impulse[body_part],
        None => 80.0,
    }
}

/// `PhysicsHitReactionScript.getImpulse(RagdollBodyPart, AmmoType)`.
pub fn impulse_for_ammo(
    sm: &ScriptManager,
    body_part: usize,
    ammo: Option<&Rc<RegistryKey>>,
) -> f32 {
    let st = sm.statics.items.borrow();
    match st
        .physics_hit_reaction_list
        .iter()
        .find(|r| same_ammo(r.ammo_type.as_ref(), ammo))
    {
        Some(r) if r.use_impulse_override => r.override_forward_impulse,
        Some(r) => r.impulse[body_part],
        None => 80.0,
    }
}

/// `PhysicsHitReactionScript.getUpwardImpulse(RagdollBodyPart, String)`.
pub fn upward_impulse_for_object(
    sm: &ScriptManager,
    body_part: usize,
    physics_object: Option<&str>,
) -> f32 {
    let st = sm.statics.items.borrow();
    match st
        .physics_hit_reaction_list
        .iter()
        .find(|r| r.physics_object.as_deref() == physics_object)
    {
        Some(r) if r.use_impulse_override => r.override_upward_impulse,
        Some(r) => r.upward_impulse[body_part],
        None => 40.0,
    }
}

/// `PhysicsHitReactionScript.getUpwardImpulse(RagdollBodyPart, AmmoType)`.
pub fn upward_impulse_for_ammo(
    sm: &ScriptManager,
    body_part: usize,
    ammo: Option<&Rc<RegistryKey>>,
) -> f32 {
    let st = sm.statics.items.borrow();
    match st
        .physics_hit_reaction_list
        .iter()
        .find(|r| same_ammo(r.ammo_type.as_ref(), ammo))
    {
        Some(r) if r.use_impulse_override => r.override_upward_impulse,
        Some(r) => r.upward_impulse[body_part],
        None => 40.0,
    }
}

/// `zombie.scripting.objects.PhysicsHitReactionScript`. Each script adds
/// one reaction to the shared list in this group's statics.
#[derive(Debug)]
pub struct PhysicsHitReactionScript {
    pub base: BaseScriptObject,
}

impl Default for PhysicsHitReactionScript {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsHitReactionScript {
    pub fn new() -> Self {
        PhysicsHitReactionScript {
            base: BaseScriptObject::new(ScriptType::PhysicsHitReaction),
        }
    }

    /// `loadPhysicsHitReaction`: the block id names an ammo type, or else
    /// an item (the physics object).
    fn load_reaction(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        let id = block.id.as_deref().unwrap_or("");
        let mut reaction = PhysicsHitReaction {
            ammo_type: AmmoType::get_id(sm, id)?,
            ..Default::default()
        };
        if reaction.ammo_type.is_none() {
            let module = self
                .base
                .module()
                .ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?;
            reaction.physics_object = sm.resolve_item_type(&module, id);
        }
        let mut impulse_index = 0;
        let mut up_impulse_index = 0;
        for element in &block.elements {
            let BlockElement::Value(i) = element else {
                continue;
            };
            let ss = java::split_char(&block.values[*i].string, '=');
            let k = java::trim(at(&ss, 0)?);
            let v = java::trim(at(&ss, 1)?);
            if k.contains("impulse_") {
                store(&mut reaction.impulse, &mut impulse_index, parse_float(v)?)?;
            } else if k.contains("upwardImpulse_") {
                store(
                    &mut reaction.upward_impulse,
                    &mut up_impulse_index,
                    parse_float(v)?,
                )?;
            }
        }
        sm.statics
            .items
            .borrow_mut()
            .physics_hit_reaction_list
            .push(reaction);
        Ok(())
    }
}

impl ScriptObject for PhysicsHitReactionScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.PhysicsHitReactionScript"
    }

    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        sm.statics
            .items
            .borrow_mut()
            .physics_hit_reaction_original_filename = sm.current_file_name();
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        self.load_reaction(sm, block)
    }
}
