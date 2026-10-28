//! State machine, variables and event routing on invented states and
//! action groups.

use std::rc::Rc;

use pz_anim::{AnimEvent, AnimEventKind, AnimEventRecord, AnimationVariableReference, AnimationVariableSource, VariableValue};

use super::action::{ActionGroup, ActionGroups};
use super::*;
use crate::character::tests::{fixture, player, Recorder};
use crate::character::{GameCharacter, SimEvent};

/// A state that logs its calls into a parameter.
struct Logger(&'static str);

const LOG: Param<Vec<String>> = Param::new(StateId("t.Log"), "log", Vec::new);

fn log<I: 'static>(c: &mut GameCharacter<I>, s: String) {
    let mut v = LOG.get(&mut c.state.params);
    v.push(s);
    LOG.set(&mut c.state.params, v);
}

impl<I: 'static> State<I> for Logger {
    fn id(&self) -> StateId {
        StateId(self.0)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        log(owner, format!("enter {}", StateId(self.0).simple_name()));
    }
    fn execute(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        log(owner, format!("execute {}", StateId(self.0).simple_name()));
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        log(owner, format!("exit {}", StateId(self.0).simple_name()));
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        log(owner, format!("event {} {}", StateId(self.0).simple_name(), e.event.event_name));
    }
}

static A: Logger = Logger("t.A");
static B: Logger = Logger("t.B");
static S1: Logger = Logger("t.S1");
static S2: Logger = Logger("t.S2");

struct HostFaultState { enter: bool }
impl State<()> for HostFaultState {
    fn id(&self) -> StateId { StateId("t.HostFault") }
    fn enter(&self, owner: &mut GameCharacter<()>, cx: &mut StateCx<'_, ()>) {
        if self.enter {
            log(owner, "fault enter".into());
            cx.record_host_error(pz_inventory::floor_drop::RuntimeHostError::Borrowed("state enter placement"));
        }
    }
    fn execute(&self, owner: &mut GameCharacter<()>, cx: &mut StateCx<'_, ()>) {
        log(owner, "fault execute".into());
        let nested = cx.reborrow();
        nested.record_host_error(pz_inventory::floor_drop::RuntimeHostError::Borrowed("state execute placement"));
        nested.record_host_error(pz_inventory::floor_drop::RuntimeHostError::InvalidHostState("later fault"));
    }
}
static FAULT_EXECUTE: HostFaultState = HostFaultState { enter: false };
static FAULT_ENTER: HostFaultState = HostFaultState { enter: true };

#[test]
fn immediate_shared_host_fault_stops_state_loop_and_preserves_first_error_domain() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let mut world = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    c.change_state_with(Some(&FAULT_EXECUTE), &[Some(&S1), Some(&S2)], false, &mut cx);
    take_log(&mut c);
    c.state_machine_update(&mut cx);
    assert_eq!(take_log(&mut c), ["fault execute"]);
    assert_eq!(cx.state_result().map_err(|error| error.host().cloned().expect("original Host domain")), Err(pz_inventory::floor_drop::RuntimeHostError::Borrowed("state execute placement")));
    c.state_machine_update(&mut cx);
    c.state_anim_event(-1, &record("later event", -1, None), &mut cx);
    c.change_state(Some(&A), &mut cx);
    assert!(take_log(&mut c).is_empty());
    assert_eq!(c.current_state_id(), Some(StateId("t.HostFault")));
    assert!(cx.has_host_error());
}

#[test]
fn fault_in_state_enter_stops_substate_publication_and_later_state_change_event() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let mut world = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    c.change_state_with(Some(&FAULT_ENTER), &[Some(&S1)], false, &mut cx);
    assert_eq!(take_log(&mut c), ["fault enter"]);
    assert_eq!(c.current_state_id(), Some(StateId("t.HostFault")), "completed root assignment remains");
    assert_eq!(c.state_machine().sub_state_count(), 0);
    assert!(ctx.events.borrow().is_empty());
    assert!(cx.state_result().map_err(|error| error.host().cloned().expect("original Host domain")).is_err());
}

fn take_log(c: &mut GameCharacter<()>) -> Vec<String> {
    LOG.remove(&mut c.state.params).unwrap_or_default()
}

fn record(name: &str, layer: i32, state: Option<&str>) -> AnimEventRecord {
    AnimEventRecord {
        state_name: state.map(str::to_string),
        layer_depth: layer,
        track: None,
        event: Rc::new(AnimEvent::named(name, pz_anim::AnimEventTime::Percentage, 0.5)),
    }
}

#[test]
fn change_state_enters_exits_and_reverts() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let mut world = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    c.change_state(Some(&A), &mut cx);
    c.change_state_with(Some(&B), &[Some(&S1), None, Some(&S2)], false, &mut cx);
    assert_eq!(take_log(&mut c), ["enter A", "exit A", "enter B", "enter S1", "enter S2"]);
    assert!(c.is_current_state(&S1) && c.is_current_state(&B) && !c.is_current_state(&A));
    assert_eq!(c.previous_state_name(), Some("A"));
    c.state_machine_update(&mut cx);
    assert_eq!(take_log(&mut c), ["execute B", "execute S1", "execute S2"]);
    // S1 leaves, its slot is reused by A.
    c.change_state_with(Some(&B), &[Some(&S2), Some(&A)], false, &mut cx);
    assert_eq!(take_log(&mut c), ["enter A", "exit S1"]);
    assert_eq!(c.state_machine().sub_state_count(), 3);
    // Restart re-enters the root only.
    c.change_state_with(Some(&B), &[Some(&S2), Some(&A)], true, &mut cx);
    assert_eq!(take_log(&mut c), ["enter B"]);
    // A sub state reverts by leaving; the root reverts to the previous.
    c.revert_to_previous_state(&A, &mut cx);
    c.revert_to_previous_state(&B, &mut cx);
    assert_eq!(take_log(&mut c), ["exit A", "exit B", "enter A"]);
    // Anim events: layer 0 the root only, -1 everyone, 1 the sub states.
    c.state_anim_event(0, &record("E0", 0, None), &mut cx);
    c.state_anim_event(-1, &record("E1", -1, None), &mut cx);
    c.state_anim_event(1, &record("E2", 1, None), &mut cx);
    assert_eq!(take_log(&mut c), ["event A E0", "event A E1", "event S2 E1", "event S2 E2"]);
    // Locked: no change.
    c.set_state_machine_locked(true);
    c.change_state(Some(&B), &mut cx);
    assert_eq!(c.current_state_name(), Some("A"));
    c.set_state_machine_locked(false);
    c.set_default_state(&mut cx);
    assert_eq!(c.current_state_name(), Some("GenericDefaultState"));
    let n = ctx.events.borrow().iter().filter(|e| e.starts_with("AiStateChange")).count();
    assert_eq!(n, 4);
}

#[test]
fn variables_read_callbacks_action_states_and_the_map() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    // Callback slots.
    c.set_var_str("hitreaction", "Shot");
    assert_eq!(c.vars.hit_reaction, "Shot");
    assert!(c.variable_boolean("hashitreaction"));
    c.set_var_str("bumptype", "stagger");
    assert!(c.vars.bump_staggered && c.variable_boolean("bumped"));
    c.set_var_float("MoveDelta", 0.5);
    assert_eq!(c.variable_float("movedelta", 9.0), 0.5);
    c.clear_var("MoveDelta");
    assert_eq!(c.vars.move_delta, 1.0);
    // Read-only slots ignore writes.
    c.set_var_bool("bdead", true);
    assert!(!c.variable_boolean("bdead"));
    // The player's TurnDelta replaces the character's.
    c.set_var_float("TurnDelta", 3.0);
    assert_eq!((c.vars.player_turn_delta, c.vars.turn_delta_normal), (3.0, 1.0));
    // Strings and floats convert like the slots.
    c.set_var_str("bonfloor", "true");
    assert!(c.vars.on_floor && c.is_prone());
    assert_eq!(c.variable_string("bonfloor"), "true");
    assert_eq!(c.variable_string("MoveDelta"), "1.0");
    // Plain slots take the first value's type.
    c.set_var_float("Speed", 2.0);
    c.set_var_str("Speed", "x");
    assert_eq!(c.variable_float("Speed", 7.0), 0.0);
    assert!(!c.contains_variable("Nothing") && c.contains_variable("speed"));
    assert!(c.is_variable("Weapon", ""));
    // Action-state variables come first.
    let idle: &[(&str, &str)] =
        &[("idle.xml", "<ActionState><variables><var key=\"MoveDelta\" type=\"float\">0.25</var></variables></ActionState>")];
    let g = ActionGroups::default().insert(ActionGroup::from_xml("g", None, &[("idle", idle)]));
    c.set_action_group(g);
    assert_eq!(c.variable_float("MoveDelta", 0.0), 0.25);
    // `$sub.name` references to missing sub sources read nothing.
    assert!(AnimationVariableReference::from_raw_variable_name("$GrappledTarget.x").get_variable(&c).is_none());
    c.clear_variables();
    assert_eq!(c.variable_float("Speed", 7.0), 0.0);
    assert_eq!(c.vars.hit_reaction, "");
}

#[test]
fn action_context_drives_ai_states_and_events() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let mut world = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    let idle: &[(&str, &str)] = &[
        ("childTags.xml", "<childTags><tag>ext</tag></childTags>"),
        ("to_hit.xml", "<transition><transitionTo>hitreaction</transitionTo><conditions><isTrue>hashitreaction</isTrue></conditions></transition>"),
        ("to_ext.xml", "<transition><transitionTo>ext</transitionTo><asSubstate>true</asSubstate><conditions><eventOccurred>EventExt</eventOccurred></conditions></transition>"),
    ];
    let hit: &[(&str, &str)] = &[(
        "out.xml",
        "<transition><transitionOut>true</transitionOut><conditions><eventOccurred>ActiveAnimFinished</eventOccurred></conditions></transition>",
    )];
    let ext: &[(&str, &str)] = &[
        ("tags.xml", "<tags><tag>ext</tag></tags>"),
        ("out.xml", "<transition><transitionOut>true</transitionOut><conditions><eventOccurred>ExtDone</eventOccurred></conditions></transition>"),
    ];
    let g = ActionGroups::default().insert(ActionGroup::from_xml(
        "player",
        Some("<actiongroup><initial>idle</initial></actiongroup>"),
        &[("idle", idle), ("hitreaction", hit), ("ext", ext)],
    ));
    c.register_ai_state("idle", &A);
    c.register_ai_state("hitreaction", &B);
    c.register_ai_state("ext", &S1);
    c.set_action_group(g);
    c.update_action_context(&mut cx);
    assert_eq!(c.current_state_name(), Some("A"));
    c.report_event("EventExt");
    c.update_action_context(&mut cx);
    assert!(c.is_current_state(&S1));
    // An anim event of the ext layer ends it.
    c.on_anim_event(&record("ExtDone", 1, Some("ext")), &mut cx);
    c.update_action_context(&mut cx);
    assert!(!c.is_current_state(&S1));
    c.vars.hit_reaction = "Bite".into();
    c.update_action_context(&mut cx);
    assert_eq!(c.current_state_name(), Some("B"));
    assert_eq!(c.variable_string("previousState"), "idle");
    c.on_anim_event(&record("ActiveAnimFinished", 0, Some("hitreaction")), &mut cx);
    c.update_action_context(&mut cx);
    assert_eq!(c.current_state_name(), Some("A"));
    assert_eq!(
        take_log(&mut c),
        [
            "enter A",
            "enter S1",
            "event S1 ExtDone",
            "exit S1",
            "exit A",
            "enter B",
            "event B ActiveAnimFinished",
            "exit B",
            "enter A"
        ]
    );
}

#[test]
fn character_anim_events_set_variables_and_flags() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let mut world = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    let mut ev = AnimEvent::named("SetVariable", pz_anim::AnimEventTime::Percentage, 0.0);
    ev.kind = AnimEventKind::SetVariable {
        reference: Rc::new(AnimationVariableReference::from_raw_variable_name("bKnockedDown")),
        value: "TRUE".into(),
    };
    let r = AnimEventRecord { state_name: None, layer_depth: 0, track: None, event: Rc::new(ev) };
    c.on_anim_event(&r, &mut cx);
    assert!(c.vars.knocked_down);
    let mut set_on_floor = record("SetOnFloor", 0, None);
    Rc::get_mut(&mut set_on_floor.event).unwrap().parameter_value = Some("true".into());
    c.on_anim_event(&set_on_floor, &mut cx);
    assert!(c.vars.on_floor);
    c.on_anim_event(&record("IsAlmostUp", 0, None), &mut cx);
    assert!(!c.vars.on_floor && !c.vars.knocked_down);
    c.set_forward_direction(0.0, 2.0);
    c.on_anim_event(&record("TurnAround", 0, None), &mut cx);
    assert_eq!((c.vars.forward_x, c.vars.forward_y), (-0.0, -1.0));
    assert_eq!(c.vars.dir, pz_world::square::IsoDirections::N);
    assert!(c.state.action_context.has_event_occurred("turnaround", None));
    c.on_anim_event(&record("KilledByAttacker", 0, None), &mut cx);
    assert!(c.is_dead() && c.vars.on_kill_done);
    assert!(ctx.events.borrow().iter().any(|e| e.starts_with("Killed")));
    let _ = SimEvent::Footstep(String::new());
    let _ = VariableValue::Bool(true);
}

#[test]
fn registry_and_directions() {
    let mut r: StateRegistry<()> = StateRegistry::with_ported();
    let idle = r.get_or_opaque("zombie/ai/states/IdleState");
    assert_eq!(idle.name(), "IdleState");
    let fishing = r.get_or_opaque("zombie.ai.states.FishingState");
    assert!(std::ptr::addr_eq(fishing, r.get_or_opaque("zombie.ai.states.FishingState")));
    assert_eq!(StateId("zombie.ai.states.animals.AnimalIdleState").simple_name(), "AnimalIdleState");
    use pz_world::square::IsoDirections as D;
    let dirs: Vec<D> = [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0), (-1.0, -1.0), (0.0, -1.0), (1.0, -1.0)]
        .iter()
        .map(|&(x, y)| vars::iso_direction_from_angle(x, y))
        .collect();
    assert_eq!(dirs, [D::E, D::SE, D::S, D::SW, D::W, D::NW, D::N, D::NE]);
    assert_eq!(vars::fall_severity(0.0), "None");
    assert_eq!(vars::fall_severity(vars::iso_impact_speed_from_height(2.0)), "Hard");
}

#[test]
fn player_ai_states_follow_the_action_group() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let mut world = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    let mut reg = StateRegistry::with_ported();
    c.initialize_player_states(&mut reg, false);
    let idle: &[(&str, &str)] =
        &[("to_kd.xml", "<transition><transitionTo>knockeddown</transitionTo><conditions><isTrue>bKnockedDown</isTrue></conditions></transition>")];
    let g = ActionGroups::default().insert(ActionGroup::from_xml("player", None, &[("idle", idle), ("knockeddown", &[])]));
    c.set_action_group(g);
    c.update_action_context(&mut cx);
    assert_eq!(c.current_state_name(), Some("IdleState"));
    c.vars.knocked_down = true;
    c.update_action_context(&mut cx);
    assert_eq!(c.current_state_name(), Some("PlayerKnockedDown"));
    assert!(c.vars.block_movement && c.vars.block_turning);
    c.initialize_player_states(&mut reg, true);
    assert!(c.try_get_ai_state("knockeddown").is_none());
    assert_eq!(c.try_get_ai_state("melee").map(|s| s.name()), Some("SwipeStatePlayer"));
}
