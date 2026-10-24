//! `IsoGameCharacter.OnAnimEvent` and the listeners
//! `registerAnimEventCallbacks` adds to the character's broadcaster.

use pz_anim::{AnimEventKind, AnimEventRecord};

use super::StateCx;
use crate::character::{GameCharacter, SimEvent};

/// `Boolean.parseBoolean`.
fn parse_boolean(s: Option<&str>) -> bool {
    s.is_some_and(|s| s.eq_ignore_ascii_case("true"))
}

impl<I: 'static> GameCharacter<I> {
    /// `OnAnimEvent(layer, track, event)`: the character's own listeners,
    /// `actionContext.reportEvent(stateName, eventName)`, then
    /// `stateMachine.stateAnimEvent(layerDepth, ...)`.
    pub fn on_anim_event(&mut self, event: &AnimEventRecord, cx: &mut StateCx<'_, I>) {
        if pz_anim::vars::is_null_or_whitespace(&event.event.event_name) {
            return;
        }
        self.character_anim_event(event, cx);
        let state = event.state_name.clone();
        self.state.action_context.report_event(state.as_deref(), &event.event.event_name);
        self.state_anim_event(event.layer_depth, event, cx);
    }

    /// The character's `AnimEventBroadcaster` (`animEvent(this, ...)`).
    fn character_anim_event(&mut self, event: &AnimEventRecord, cx: &mut StateCx<'_, I>) {
        let e = &event.event;
        let name = e.event_name.as_str();
        let param = e.parameter_value.as_deref();
        let is = |n: &str| name.eq_ignore_ascii_case(n);
        if is("SetVariable") {
            if let AnimEventKind::SetVariable { reference, value } = &e.kind {
                reference.set_variable(self, pz_anim::VariableValue::String(value.clone()));
            }
        } else if is("ClearVariable") {
            let r = pz_anim::AnimationVariableReference::from_raw_variable_name(param.unwrap_or(""));
            r.clear_variable(self);
        } else if is("PlaySound") {
            cx.ctx.event(SimEvent::PlaySound(param.unwrap_or("").to_string()));
        } else if is("PlaySoundNoBlend") {
            // TODO(anim): skipped while the layer blends in or out.
            cx.ctx.event(SimEvent::PlaySound(param.unwrap_or("").to_string()));
        } else if is("Footstep") {
            cx.ctx.event(SimEvent::Footstep(param.unwrap_or("").to_string()));
        } else if is("DamageWhileInTrees") {
            // TODO(world): damageWhileInTrees needs the clothing and holes.
        } else if is("TurnAround") || is("TurnAround_FlipSkeleton") {
            // TODO(anim): the flip also adjusts the track's bone pose.
            let (x, y) = (-self.vars.forward_x, -self.vars.forward_y);
            self.set_forward_direction(x, y);
        } else if is("FallOnFront") {
            self.environment.fall_on_front = parse_boolean(param);
        } else if is("SetOnFloor") {
            self.vars.on_floor = parse_boolean(param);
        } else if is("SetKnockedDown") {
            self.vars.knocked_down = parse_boolean(param);
        } else if is("IsAlmostUp") {
            self.vars.on_floor = false;
            self.vars.knocked_down = false;
            self.sit_on_ground = false;
        } else if is("KilledByAttacker") {
            cx.ctx.event(SimEvent::Kill { attacker: self.vars.attacked_by });
            self.kill(cx);
        }
    }
}
