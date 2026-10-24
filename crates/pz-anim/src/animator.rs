//! `AdvancedAnimator`, `AnimLayer`, `LiveAnimNode` and `SubLayerSlot`: the
//! AnimSet state machine.
//!
//! Each frame the character calls [`AdvancedAnimator::update`], then
//! [`AnimationPlayer::update`], then
//! [`AdvancedAnimator::dispatch_track_events`] (or [`AdvancedAnimator::animate`]
//! for all three). The original runs the track listeners synchronously
//! inside the player update; they only touch animator state and variables,
//! which the player does not read in between, so dispatching them right
//! after gives the same results and event order.
//!
//! Fired events are collected in [`AdvancedAnimator::events`] for the
//! character (the original's `IAnimEventCallback`); `FlagWhileAlive`
//! events are handled here as in the original.

use std::rc::Rc;

use crate::animset::{AnimCondition, AnimEvent, AnimEventKind, AnimEventTime, AnimNode, AnimState, AnimTransition, AnimationSet};
use crate::math::pzmath;
use crate::player::{AnimationPlayer, LayerBinding, StartAnimTrackParameters};
use crate::track::{TrackEventKind, TrackId, TrackListener};
use crate::vars::{equals_ignore_case, AnimationVariableHandle, AnimationVariableSource, VariableReferenceRc, VariableValue};
use crate::BoneAxis;

/// `zombie.core.random.Rand` as the animator uses it.
pub trait AnimRandom {
    /// `Rand.Next(float min, float max)`.
    fn next_f32(&mut self, min: f32, max: f32) -> f32;
    /// `Rand.Next(int max)`: `0..max`.
    fn next_int(&mut self, max: i32) -> i32;
}

/// A small deterministic generator (xorshift). Not the game's RNG stream.
#[derive(Clone, Debug)]
pub struct XorShiftRandom(pub u64);

impl Default for XorShiftRandom {
    fn default() -> Self {
        XorShiftRandom(0x9E37_79B9_7F4A_7C15)
    }
}

impl XorShiftRandom {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
}

impl AnimRandom for XorShiftRandom {
    fn next_f32(&mut self, min: f32, max: f32) -> f32 {
        if max <= min {
            return min;
        }
        let f = (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
        min + f * (max - min)
    }

    fn next_int(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        (self.next_u32() % max as u32) as i32
    }
}

/// One fired event (`IAnimEventCallback.OnAnimEvent(sender, track, event)`).
#[derive(Clone, Debug)]
pub struct AnimEventRecord {
    /// The sending layer's current state name (`None` for global events or
    /// a stateless layer).
    pub state_name: Option<String>,
    /// The sending layer's depth (`-1` for global events).
    pub layer_depth: i32,
    pub track: Option<TrackId>,
    pub event: Rc<AnimEvent>,
}

/// `LocalAnimEvent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalAnimEvent {
    ActiveAnimLooped,
    ActiveNonLoopedAnimFadeOut,
    ActiveAnimFinishing,
    ActiveNonLoopedAnimFinished,
    NoAnimConditionsPass,
}

impl LocalAnimEvent {
    pub fn anim_event(self) -> AnimEvent {
        match self {
            LocalAnimEvent::ActiveAnimLooped => AnimEvent::named("ActiveAnimLooped", AnimEventTime::Percentage, 1.0),
            LocalAnimEvent::ActiveNonLoopedAnimFadeOut => AnimEvent::named("NonLoopedAnimFadeOut", AnimEventTime::Percentage, 1.0),
            LocalAnimEvent::ActiveAnimFinishing => AnimEvent::named("ActiveAnimFinishing", AnimEventTime::End, 0.0),
            LocalAnimEvent::ActiveNonLoopedAnimFinished => AnimEvent::named("ActiveAnimFinished", AnimEventTime::End, 0.0),
            LocalAnimEvent::NoAnimConditionsPass => AnimEvent::named("NoAnimConditionsPass", AnimEventTime::End, 0.0),
        }
    }
}

/// A transition of some node's source, by index.
#[derive(Clone, Debug)]
struct TransitionRef {
    node: Rc<AnimNode>,
    index: usize,
}

impl TransitionRef {
    fn get(&self) -> &AnimTransition {
        &self.node.transitions[self.index]
    }
}

#[derive(Clone, Debug, Default)]
struct TransitionIn {
    time: f32,
    transitioned_from: Option<String>,
    active: bool,
    track: Option<TrackId>,
    data: Option<TransitionRef>,
    weight: f32,
    raw_weight: f32,
    blending_in: bool,
    blending_out: bool,
}

/// `LiveAnimNode`.
#[derive(Debug)]
pub struct LiveAnimNode {
    pub id: u32,
    pub source: Rc<AnimNode>,
    layer: u32,
    active: bool,
    was_active: bool,
    pub transitioning_out: bool,
    weight: f32,
    raw_weight: f32,
    is_new: bool,
    pub is_blend_field: bool,
    running_ragdoll_track: Option<TrackId>,
    transition_in: TransitionIn,
    animation_tracks: Vec<TrackId>,
    ragdoll_tracks: Vec<TrackId>,
    pub node_anim_time: f32,
    pub prev_node_anim_time: f32,
    blending_in: bool,
    blending_out: bool,
    transition_out: Option<TransitionRef>,
    while_alive_flags: Vec<(VariableReferenceRc, bool)>,
    cached_random_anim: String,
}

impl LiveAnimNode {
    fn new(id: u32, layer: u32, source: Rc<AnimNode>) -> Self {
        LiveAnimNode {
            id,
            source,
            layer,
            active: false,
            was_active: false,
            transitioning_out: false,
            weight: 0.0,
            raw_weight: 0.0,
            is_new: true,
            is_blend_field: false,
            running_ragdoll_track: None,
            transition_in: TransitionIn::default(),
            animation_tracks: Vec::new(),
            ragdoll_tracks: Vec::new(),
            node_anim_time: 0.0,
            prev_node_anim_time: 0.0,
            blending_in: false,
            blending_out: false,
            transition_out: None,
            while_alive_flags: Vec::new(),
            cached_random_anim: String::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.source.name
    }

    pub fn weight(&self) -> f32 {
        self.weight
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_blending_in(&self) -> bool {
        self.blending_in
    }

    pub fn is_blending_out(&self) -> bool {
        self.blending_out
    }

    pub fn is_transitioning_in(&self) -> bool {
        self.transition_in.active && self.transition_in.track.is_some()
    }

    pub fn transition_in_weight(&self) -> f32 {
        self.transition_in.weight
    }

    pub fn transition_in_track(&self) -> Option<TrackId> {
        self.transition_in.track
    }

    pub fn transition_from(&self) -> Option<&str> {
        self.transition_in.transitioned_from.as_deref()
    }

    pub fn main_animation_tracks(&self) -> &[TrackId] {
        &self.animation_tracks
    }

    fn is_main_anim_active(&self) -> bool {
        !self.is_transitioning_in() || self.transition_in.blending_out
    }

    fn was_activated(&self) -> bool {
        self.active != self.was_active && self.active
    }

    fn was_deactivated(&self) -> bool {
        self.active != self.was_active && self.was_active
    }

    fn is_looped(&self) -> bool {
        self.source.is_looped
    }

    /// `getPlayingTrackCount` / `getPlayingTrackAt`.
    fn playing_tracks(&self) -> Vec<TrackId> {
        let mut v = Vec::new();
        if self.is_main_anim_active() {
            v.extend_from_slice(&self.animation_tracks);
        }
        if let Some(t) = self.running_ragdoll_track {
            v.push(t);
        }
        if self.is_transitioning_in() {
            v.push(self.transition_in.track.unwrap());
        }
        v
    }

    fn anim_name(&self) -> String {
        if !crate::vars::is_null_or_whitespace(&self.cached_random_anim) {
            return self.cached_random_anim.clone();
        }
        self.source.anim_name.clone()
    }

    /// `getBlendInTime`.
    fn blend_in_time(&self) -> f32 {
        let Some(d) = &self.transition_in.data else { return self.source.blend_time };
        let d = d.get();
        if self.transition_in.track.is_some() && d.blend_out_time != f32::INFINITY {
            return d.blend_out_time;
        }
        if self.transition_in.track.is_none() {
            if d.blend_in_time != f32::INFINITY {
                return d.blend_in_time;
            }
            if d.blend_out_time != f32::INFINITY {
                return d.blend_out_time;
            }
        }
        self.source.blend_time
    }

    fn transition_in_blend_in_time(&self) -> f32 {
        match &self.transition_in.data {
            Some(d) if d.get().blend_in_time != f32::INFINITY => d.get().blend_in_time,
            _ => 0.0,
        }
    }

    fn transition_in_blend_out_time(&self) -> f32 {
        self.blend_in_time()
    }

    /// `getBlendOutTime`.
    fn blend_out_time(&self) -> f32 {
        let Some(t) = &self.transition_out else { return self.source.get_blend_out_time() };
        let t = t.get();
        if t.has_anim() && t.blend_in_time != f32::INFINITY {
            return t.blend_in_time;
        }
        if !t.has_anim() {
            if t.blend_out_time != f32::INFINITY {
                return t.blend_out_time;
            }
            if t.blend_in_time != f32::INFINITY {
                return t.blend_in_time;
            }
        }
        self.source.get_blend_out_time()
    }

    fn increment_blend_time(initial_weight: f32, max: f32, dt: f32) -> f32 {
        initial_weight * max + dt
    }
}

/// `AnimLayer`.
#[derive(Debug)]
pub struct AnimLayer {
    pub id: u32,
    parent: Option<u32>,
    state: Option<Rc<AnimState>>,
    current_sync_node: Option<u32>,
    current_sync_track: Option<TrackId>,
    live_nodes: Vec<u32>,
    no_anim_conditions_event_sent: bool,
}

impl AnimLayer {
    fn new(id: u32, parent: Option<u32>) -> Self {
        AnimLayer { id, parent, state: None, current_sync_node: None, current_sync_track: None, live_nodes: Vec::new(), no_anim_conditions_event_sent: false }
    }

    pub fn state(&self) -> Option<&Rc<AnimState>> {
        self.state.as_ref()
    }

    pub fn current_state_name(&self) -> Option<&str> {
        self.state.as_ref().map(|s| s.name.as_str())
    }

    pub fn is_current_state(&self, s: Option<&Rc<AnimState>>) -> bool {
        match (&self.state, s) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn live_nodes(&self) -> &[u32] {
        &self.live_nodes
    }

    pub fn current_sync_track(&self) -> Option<TrackId> {
        self.current_sync_track
    }
}

/// `SubLayerSlot`.
#[derive(Debug)]
struct SubLayerSlot {
    should_be_active: bool,
    desired_layer: i32,
    layer: u32,
    next_state: Option<Rc<AnimState>>,
}

/// `TransitionNodeProxy.TransitionNodeProxyData`.
struct TransitionData {
    new_node: u32,
    layer_out: u32,
    old_node: u32,
    transition: TransitionRef,
}

/// `AdvancedAnimator`.
pub struct AdvancedAnimator {
    pub anim_set: Option<Rc<AnimationSet>>,
    layers: Vec<Option<AnimLayer>>,
    root: u32,
    sub_layers: Vec<SubLayerSlot>,
    root_state: Option<Rc<AnimState>>,
    sub_states: Vec<Rc<AnimState>>,
    nodes: Vec<Option<LiveAnimNode>>,
    set_flag_counters: Vec<(VariableReferenceRc, i32)>,
    /// Events fired since the caller last drained this.
    pub events: Vec<AnimEventRecord>,
    pub rng: Box<dyn AnimRandom>,
}

impl Default for AdvancedAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl AdvancedAnimator {
    /// `new AdvancedAnimator()` + `init(character, null)`.
    pub fn new() -> AdvancedAnimator {
        let mut a = AdvancedAnimator {
            anim_set: None,
            layers: Vec::new(),
            root: 0,
            sub_layers: Vec::new(),
            root_state: None,
            sub_states: Vec::new(),
            nodes: vec![None],
            set_flag_counters: Vec::new(),
            events: Vec::new(),
            rng: Box::new(XorShiftRandom::default()),
        };
        a.root = a.alloc_layer(None);
        a
    }

    pub fn set_anim_set(&mut self, set: Option<Rc<AnimationSet>>) {
        self.anim_set = set;
    }

    fn alloc_layer(&mut self, parent: Option<u32>) -> u32 {
        let id = self.layers.len() as u32;
        self.layers.push(Some(AnimLayer::new(id, parent)));
        id
    }

    pub fn layer(&self, id: u32) -> Option<&AnimLayer> {
        self.layers.get(id as usize).and_then(|l| l.as_ref())
    }

    fn layer_mut(&mut self, id: u32) -> &mut AnimLayer {
        self.layers[id as usize].as_mut().expect("released layer")
    }

    pub fn node(&self, id: u32) -> Option<&LiveAnimNode> {
        self.nodes.get(id as usize).and_then(|n| n.as_ref())
    }

    fn node_mut(&mut self, id: u32) -> &mut LiveAnimNode {
        self.nodes[id as usize].as_mut().expect("released node")
    }

    fn node_exists(&self, id: u32) -> bool {
        self.node(id).is_some()
    }

    pub fn root_layer(&self) -> &AnimLayer {
        self.layer(self.root).unwrap()
    }

    pub fn sub_layer_count(&self) -> usize {
        self.sub_layers.len()
    }

    pub fn sub_layer_at(&self, i: usize) -> &AnimLayer {
        self.layer(self.sub_layers[i].layer).unwrap()
    }

    /// `getCurrentStateName`.
    pub fn current_state_name(&self) -> Option<&str> {
        self.root_layer().current_state_name()
    }

    pub fn contains_state(&self, name: &str) -> bool {
        self.anim_set.as_ref().is_some_and(|s| s.contains_state(name))
    }

    /// `AnimLayer.getDepth`.
    pub fn layer_depth(&self, id: u32) -> i32 {
        match self.layer(id) {
            Some(l) => match l.parent {
                Some(p) => self.layer_depth(p) + 1,
                None => 0,
            },
            None => 0,
        }
    }

    fn binding(&self, layer: u32) -> LayerBinding {
        LayerBinding { id: layer, depth: self.layer_depth(layer), is_sub_layer: self.layer(layer).is_some_and(|l| l.parent.is_some()) }
    }

    fn sync_track_depths(&self, player: &mut AnimationPlayer) {
        let depths: Vec<(TrackId, i32)> = player.tracks().iter().filter_map(|t| t.anim_layer.map(|l| (t.id, self.layer_depth(l)))).collect();
        for (id, d) in depths {
            if let Some(t) = player.track_mut(id) {
                t.layer_depth = d;
            }
        }
    }

    // ---- states ----

    /// `setState(stateName, subStateNames)`.
    pub fn set_state(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, state: &str, sub_states: &[&str]) {
        let Some(set) = self.anim_set.clone() else { return };
        let root = set.get_state(state);
        let subs: Vec<Rc<AnimState>> = sub_states.iter().map(|s| set.get_state(s)).collect();
        self.root_state = Some(root.clone());
        self.sub_states = subs.clone();
        self.set_state_objects(player, vars, root, subs);
    }

    fn slot_is_next_or_current(&self, slot: &SubLayerSlot, state: &Rc<AnimState>) -> bool {
        match &slot.next_state {
            Some(n) => Rc::ptr_eq(n, state),
            None => self.layer(slot.layer).is_some_and(|l| l.is_current_state(Some(state))),
        }
    }

    fn is_current_state(&self, root: &Rc<AnimState>, subs: &[Rc<AnimState>]) -> bool {
        if !self.root_layer().is_current_state(Some(root)) {
            return false;
        }
        let mut max_desired = 0;
        for slot in &self.sub_layers {
            if !slot.should_be_active {
                continue;
            }
            if slot.desired_layer as usize >= subs.len() {
                return false;
            }
            if !self.slot_is_next_or_current(slot, &subs[slot.desired_layer as usize]) {
                return false;
            }
            max_desired = max_desired.max(slot.desired_layer);
        }
        max_desired as usize == subs.len()
    }

    /// `setState(AnimState, List<AnimState>)`.
    pub fn set_state_objects(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, root: Rc<AnimState>, subs: Vec<Rc<AnimState>>) {
        if self.is_current_state(&root, &subs) {
            return;
        }
        if !self.root_layer().is_current_state(Some(&root)) {
            self.layer_transition_to(self.root, Some(root.clone()), false);
        }
        self.clean_up_empty_layers(player, vars);
        for s in &mut self.sub_layers {
            s.should_be_active = false;
        }
        for (i, sub) in subs.iter().enumerate() {
            if let Some(k) = (0..self.sub_layers.len()).find(|&k| self.slot_is_next_or_current(&self.sub_layers[k], sub)) {
                self.sub_layers[k].should_be_active = true;
                self.sub_layers[k].desired_layer = i as i32;
            }
        }
        for (i, sub) in subs.iter().enumerate() {
            if (0..self.sub_layers.len()).any(|k| self.slot_is_next_or_current(&self.sub_layers[k], sub)) {
                continue;
            }
            if let Some(k) = (0..self.sub_layers.len()).rev().find(|&k| !self.sub_layers[k].should_be_active && self.sub_layers[k].desired_layer == i as i32) {
                self.sub_layers[k].next_state = Some(sub.clone());
                self.sub_layers[k].should_be_active = true;
                continue;
            }
            let layer = self.alloc_layer(None);
            self.sub_layers.push(SubLayerSlot { should_be_active: true, desired_layer: i as i32, layer, next_state: Some(sub.clone()) });
        }
        let depths: Vec<i32> = self.sub_layers.iter().map(|s| self.layer_depth(s.layer)).collect();
        let mut order: Vec<usize> = (0..self.sub_layers.len()).collect();
        order.sort_by(|&a, &b| {
            let (sa, sb) = (&self.sub_layers[a], &self.sub_layers[b]);
            sa.desired_layer
                .cmp(&sb.desired_layer)
                .then(sa.should_be_active.cmp(&sb.should_be_active))
                .then(depths[a].cmp(&depths[b]))
        });
        let mut old: Vec<Option<SubLayerSlot>> = self.sub_layers.drain(..).map(Some).collect();
        self.sub_layers = order.into_iter().map(|i| old[i].take().unwrap()).collect();
        self.ensure_layer_parents(player);
        for k in 0..self.sub_layers.len() {
            let (active, layer) = (self.sub_layers[k].should_be_active, self.sub_layers[k].layer);
            if !active {
                self.layer_transition_to(layer, None, false);
                self.sub_layers[k].next_state = None;
            } else if let Some(next) = self.sub_layers[k].next_state.take() {
                self.layer_transition_to(layer, Some(next), false);
            }
        }
        self.clean_up_empty_layers(player, vars);
        self.sync_track_depths(player);
    }

    fn ensure_layer_parents(&mut self, player: &mut AnimationPlayer) {
        let mut parent = self.root;
        for k in 0..self.sub_layers.len() {
            let l = self.sub_layers[k].layer;
            self.set_parent_layer(player, l, parent);
            parent = l;
        }
    }

    /// `AnimLayer.setParentLayer`.
    fn set_parent_layer(&mut self, player: &mut AnimationPlayer, layer: u32, parent: u32) {
        if self.layer_mut(layer).parent == Some(parent) {
            return;
        }
        self.layer_mut(layer).parent = Some(parent);
        let nodes = self.layer_mut(layer).live_nodes.clone();
        for n in nodes {
            self.node_transferred_to_layer(player, n, layer);
        }
    }

    /// `LiveAnimNode.onTransferredToLayer`.
    fn node_transferred_to_layer(&mut self, player: &mut AnimationPlayer, node: u32, layer: u32) {
        self.node_mut(node).layer = layer;
        let binding = self.binding(layer);
        let n = self.node(node).unwrap();
        let weights = n.source.sub_state_bone_weights.clone();
        let tracks: Vec<TrackId> = n.animation_tracks.iter().chain(&n.ragdoll_tracks).copied().collect();
        let skeleton = player.skeleton().clone();
        for t in tracks {
            if let Some(t) = player.track_mut(t) {
                t.set_anim_layer(Some(layer), binding.depth);
                if binding.is_sub_layer {
                    t.set_bone_weights(Some(&weights));
                    t.init_bone_weights(&skeleton);
                } else {
                    t.set_bone_weights(None);
                }
            }
        }
    }

    fn clean_up_empty_layers(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource) {
        let mut i = 0;
        while i < self.sub_layers.len() {
            let s = &self.sub_layers[i];
            let l = self.layer(s.layer).unwrap();
            if s.should_be_active || l.state.is_some() || !l.live_nodes.is_empty() {
                i += 1;
                continue;
            }
            let slot = self.sub_layers.remove(i);
            self.release_layer(player, vars, slot.layer);
        }
        self.ensure_layer_parents(player);
    }

    fn release_layer(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, layer: u32) {
        let nodes = std::mem::take(&mut self.layer_mut(layer).live_nodes);
        for n in nodes {
            self.release_node(player, vars, n);
        }
        self.layers[layer as usize] = None;
    }

    /// `AnimLayer.transitionTo`.
    fn layer_transition_to(&mut self, layer: u32, state: Option<Rc<AnimState>>, force: bool) {
        if !force && self.layer_mut(layer).is_current_state(state.as_ref()) {
            return;
        }
        let l = self.layer_mut(layer);
        l.state = state;
        l.no_anim_conditions_event_sent = false;
        let nodes = l.live_nodes.clone();
        for n in nodes {
            self.node_mut(n).transitioning_out = true;
        }
    }

    /// `reset()`: drops every layer's live nodes and states.
    pub fn reset(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource) {
        let layers: Vec<u32> = std::iter::once(self.root).chain(self.sub_layers.iter().map(|s| s.layer)).collect();
        for l in layers {
            let nodes = std::mem::take(&mut self.layer_mut(l).live_nodes);
            for n in nodes {
                self.release_node(player, vars, n);
            }
            let l = self.layer_mut(l);
            l.no_anim_conditions_event_sent = false;
            l.state = None;
        }
    }

    // ---- update ----

    /// Animator update, player update and listener dispatch for one frame.
    pub fn animate(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, dt: f32) {
        self.update(player, vars, dt);
        player.update(dt, Some(&*vars));
        self.dispatch_track_events(player, vars);
    }

    /// `AdvancedAnimator.update(deltaT)`.
    pub fn update(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, dt: f32) {
        let Some(set) = self.anim_set.clone() else { return };
        if self.root_layer().state.is_none() {
            self.layer_transition_to(self.root, Some(set.get_state("Idle")), true);
        }
        self.update_live_anim_nodes(player, vars, self.root);
        for k in 0..self.sub_layers.len() {
            let l = self.sub_layers[k].layer;
            self.update_live_anim_nodes(player, vars, l);
        }
        self.generate_transition_data(player, vars);
        self.layer_update(player, vars, self.root, dt);
        for k in 0..self.sub_layers.len() {
            let l = self.sub_layers[k].layer;
            self.layer_update(player, vars, l, dt);
        }
        self.clean_up_empty_layers(player, vars);
        self.sync_track_depths(player);
    }

    fn update_live_anim_nodes(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, layer: u32) {
        // removeFadedOutNodes
        let mut i = self.layer_mut(layer).live_nodes.len();
        while i > 0 {
            i -= 1;
            let id = self.layer_mut(layer).live_nodes[i];
            let n = self.node(id).unwrap();
            if n.active || n.is_transitioning_in() && n.transition_in.weight > 0.01 || n.weight > 0.01 {
                continue;
            }
            self.layer_mut(layer).live_nodes.remove(i);
            self.release_node(player, vars, id);
        }
        self.update_node_active_flags(player, vars, layer);
    }

    /// `AnimLayer.updateNodeActiveFlags`.
    fn update_node_active_flags(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, layer: u32) {
        let ids = self.layer_mut(layer).live_nodes.clone();
        for id in ids {
            self.node_mut(id).active = false;
        }
        let Some(state) = self.layer_mut(layer).state.clone() else { return };
        let locked = AnimationVariableHandle::alloc("AnimLocked").is_some_and(|h| vars.get_variable_boolean_handle(&h));
        if locked {
            return;
        }
        let nodes = state.get_anim_nodes(&*vars, player.debug.allow_anim_node_override);
        if nodes.is_empty() {
            if !self.layer_mut(layer).no_anim_conditions_event_sent {
                self.invoke_anim_event_raw(layer, None, None, Rc::new(LocalAnimEvent::NoAnimConditionsPass.anim_event()), vars);
                self.layer_mut(layer).no_anim_conditions_event_sent = true;
            }
        } else {
            self.layer_mut(layer).no_anim_conditions_event_sent = false;
        }
        for n in nodes {
            self.get_or_create_live_node(player, vars, layer, n);
        }
    }

    fn get_or_create_live_node(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, layer: u32, node: Rc<AnimNode>) {
        let found = self.layer(layer).unwrap().live_nodes.iter().copied().find(|&id| {
            let n = self.node(id).unwrap();
            if n.transitioning_out {
                return false;
            }
            Rc::ptr_eq(&n.source, &node) || n.source.parent_state.get() == node.parent_state.get() && n.source.name == node.name
        });
        if let Some(id) = found {
            self.node_mut(id).active = true;
            return;
        }
        let id = self.nodes.len() as u32;
        self.nodes.push(Some(LiveAnimNode::new(id, layer, node)));
        self.start_live_node_tracks(player, vars, id);
        self.node_mut(id).active = true;
        self.layer_mut(layer).live_nodes.push(id);
    }

    /// `AnimLayer.startLiveNodeTracks`.
    fn start_live_node_tracks(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, id: u32) {
        let node = self.node(id).unwrap().source.clone();
        let real = node.get_speed_scale(&*vars);
        let alpha = self.rng.next_f32(0.0, 1.0);
        let mult = pzmath::lerp(node.speed_scale_random_multiplier_min, node.speed_scale_random_multiplier_max, alpha);
        let speed = real * mult;
        let is_ragdoll = node.is_ragdoll;
        let rmin = pzmath::max(node.ragdoll_start_time_min, 0.0);
        let rmax = pzmath::max(node.ragdoll_start_time_max, rmin);
        let ragdoll_start = self.rng.next_f32(rmin, rmax);
        let start_as_ragdoll = is_ragdoll && pzmath::equal(ragdoll_start, 0.0, 0.01);
        let is_blend_field = !start_as_ragdoll && !node.blends2d.is_empty();
        self.node_mut(id).is_blend_field = is_blend_field;
        if is_ragdoll {
            let name = format!("Ragdoll_{}", node.name);
            self.start_track_generic(player, vars, id, &name, false, speed, true, ragdoll_start);
        }
        if start_as_ragdoll {
            return;
        }
        if !is_blend_field {
            self.start_primary_animation_track(player, vars, id, speed);
            return;
        }
        for b in &node.blends2d {
            if equals_ignore_case(&b.anim_name, &node.anim_name) {
                self.start_primary_animation_track(player, vars, id, speed);
            } else {
                self.start_track_generic(player, vars, id, &b.anim_name, false, speed, false, -1.0);
            }
        }
    }

    fn start_primary_animation_track(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, id: u32, speed: f32) {
        let src = self.node(id).unwrap().source.clone();
        let r = src.get_random_anim(self.rng.as_mut());
        self.node_mut(id).cached_random_anim = r;
        let name = self.node(id).unwrap().anim_name();
        self.start_track_generic(player, vars, id, &name, true, speed, false, -1.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn start_track_generic(
        &mut self,
        player: &mut AnimationPlayer,
        _vars: &mut dyn AnimationVariableSource,
        id: u32,
        anim_name: &str,
        is_primary: bool,
        speed: f32,
        is_ragdoll: bool,
        ragdoll_start_time: f32,
    ) {
        let n = self.node(id).unwrap();
        let node = n.source.clone();
        let p = StartAnimTrackParameters {
            anim_name: anim_name.to_string(),
            is_primary,
            is_ragdoll,
            ragdoll_start_time,
            ragdoll_max_time: node.ragdoll_max_time,
            sub_layer_bone_weights: node.sub_state_bone_weights.clone(),
            sync_tracking_enabled: node.sync_tracking_enabled,
            track_time_to_variable: node.track_time_to_variable.clone(),
            speed_scale: speed,
            initial_weight: n.weight,
            is_looped: n.is_looped(),
            is_reversed: node.is_anim_reverse,
            deferred_bone_name: node.get_deferred_bone_name().to_string(),
            deferred_bone_axis: node.deferred_bone_axis,
            use_deferred_movement: node.use_deferred_movement,
            use_deferred_rotation: node.use_defered_rotation,
            deferred_rotation_scale: node.deferred_rotation_scale,
            priority: node.priority,
            matching_grappled_anim_node: node.matching_grappled_anim_node.clone(),
        };
        let binding = self.binding(n.layer);
        let Some(track) = player.play_params(&p, Some(binding)) else { return };
        let t = player.track_mut(track).unwrap();
        t.blend_curve = node.blend_curve.clone();
        t.add_listener(TrackListener::Node(id));
        // LiveAnimNode.addMainTrack
        let looped = node.is_looped;
        let blend_out = self.node(id).unwrap().blend_out_time();
        let t = player.track_mut(track).unwrap();
        if !looped && !node.stop_anim_on_exit && node.early_transition_out && blend_out > 0.0 && blend_out.is_finite() {
            t.early_blend_out_time = blend_out;
            t.trigger_on_non_looped_anim_fade_out_event = true;
        }
        let rag = t.is_ragdoll();
        let n = self.node_mut(id);
        if rag {
            n.ragdoll_tracks.push(track);
        } else {
            n.animation_tracks.push(track);
        }
    }

    // ---- transitions ----

    fn generate_transition_data(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource) {
        let mut new_nodes: Vec<(u32, u32)> = Vec::new();
        let mut out_nodes: Vec<(u32, u32)> = Vec::new();
        let layers: Vec<(u32, bool)> = std::iter::once((self.root, true)).chain(self.sub_layers.iter().map(|s| (s.layer, false))).collect();
        for (layer, is_root) in layers {
            for &id in &self.layer(layer).unwrap().live_nodes {
                let n = self.node(id).unwrap();
                if n.is_new && n.was_activated() {
                    if !new_nodes.iter().any(|&(nid, _)| Rc::ptr_eq(&self.node(nid).unwrap().source, &n.source)) {
                        new_nodes.push((id, layer));
                    }
                    continue;
                }
                if (!n.was_deactivated() || !is_root) && (is_root || !n.transitioning_out) {
                    continue;
                }
                if !out_nodes.iter().any(|&(nid, _)| Rc::ptr_eq(&self.node(nid).unwrap().source, &n.source)) {
                    out_nodes.push((id, layer));
                }
            }
        }
        if new_nodes.is_empty() || out_nodes.is_empty() {
            return;
        }
        let mut found = Vec::new();
        for &(to, _layer_in) in &new_nodes {
            let to_src = self.node(to).unwrap().source.clone();
            for &(from, layer_out) in &out_nodes {
                let from_src = self.node(from).unwrap().source.clone();
                if Rc::ptr_eq(&to_src, &from_src) {
                    continue;
                }
                if let Some(index) = from_src.find_transition_to(&*vars, &to_src) {
                    found.push(TransitionData { new_node: to, layer_out, old_node: from, transition: TransitionRef { node: from_src, index } });
                }
            }
        }
        for t in found {
            let track = self.start_transition_animation(player, vars, &t);
            self.start_transition_in(player, t.new_node, &t.transition, track);
            self.node_mut(t.old_node).transition_out = Some(t.transition.clone());
        }
    }

    /// `AnimLayer.startTransitionAnimation`.
    fn start_transition_animation(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, t: &TransitionData) -> Option<TrackId> {
        let tr = t.transition.get();
        if !tr.has_anim() {
            return None;
        }
        let mut speed = tr.speed_scale;
        if speed == f32::INFINITY {
            speed = self.node(t.new_node).unwrap().source.get_speed_scale(&*vars);
        }
        let old = self.node(t.old_node).unwrap();
        let use_node = tr.deferred_bone_name.as_deref().is_none_or(crate::vars::is_null_or_whitespace);
        let (bone, axis, rot, mov, scale): (String, BoneAxis, bool, bool, f32) = if use_node {
            let s = &old.source;
            (s.get_deferred_bone_name().to_string(), s.deferred_bone_axis, s.use_defered_rotation, s.use_deferred_movement, s.deferred_rotation_scale)
        } else {
            (
                tr.deferred_bone_name.clone().unwrap(),
                tr.deferred_bone_axis,
                tr.use_defered_rotation,
                tr.use_deferred_movement,
                tr.deferred_rotation_scale,
            )
        };
        let p = StartAnimTrackParameters {
            anim_name: tr.anim_name.clone().unwrap_or_default(),
            sub_layer_bone_weights: old.source.sub_state_bone_weights.clone(),
            speed_scale: speed,
            deferred_bone_name: bone,
            deferred_bone_axis: axis,
            use_deferred_rotation: rot,
            use_deferred_movement: mov,
            deferred_rotation_scale: scale,
            priority: old.source.priority,
            ..Default::default()
        };
        let binding = self.binding(t.layer_out);
        player.play_params(&p, Some(binding))
    }

    /// `LiveAnimNode.startTransitionIn`.
    fn start_transition_in(&mut self, player: &mut AnimationPlayer, id: u32, transition: &TransitionRef, track: Option<TrackId>) {
        if self.node(id).unwrap().transition_in.track.is_some() {
            self.stop_transition_in(player, id);
        }
        let from_name = transition.node.name.clone();
        let n = self.node_mut(id);
        n.transition_in = TransitionIn {
            time: 0.0,
            transitioned_from: Some(from_name),
            active: track.is_some(),
            track,
            data: Some(transition.clone()),
            weight: 0.0,
            raw_weight: 0.0,
            blending_in: true,
            blending_out: false,
        };
        if let Some(t) = track {
            if let Some(t) = player.track_mut(t) {
                t.add_listener(TrackListener::Node(id));
            }
        }
        self.set_main_tracks_playing(player, id, false);
    }

    fn stop_transition_in(&mut self, player: &mut AnimationPlayer, id: u32) {
        if let Some(t) = self.node(id).unwrap().transition_in.track {
            self.destroy_track(player, t);
        }
        if let Some(n) = self.nodes[id as usize].as_mut() {
            n.transition_in = TransitionIn::default();
        }
    }

    /// Removes a track from the player and runs the `onTrackDestroyed`
    /// listeners.
    fn destroy_track(&mut self, player: &mut AnimationPlayer, track: TrackId) {
        let Some(listeners) = player.remove_track(track) else { return };
        for l in listeners {
            if let TrackListener::Node(n) = l {
                if self.node_exists(n) {
                    self.node_on_track_destroyed(player, n, track);
                }
            }
        }
    }

    fn node_on_track_destroyed(&mut self, player: &mut AnimationPlayer, id: u32, track: TrackId) {
        let n = self.node_mut(id);
        n.animation_tracks.retain(|&t| t != track);
        n.ragdoll_tracks.retain(|&t| t != track);
        if n.running_ragdoll_track == Some(track) {
            n.running_ragdoll_track = None;
        }
        if n.transition_in.track == Some(track) {
            n.transition_in.track = None;
            n.transition_in.active = false;
            n.transition_in.weight = 0.0;
            self.set_main_tracks_playing(player, id, true);
        }
    }

    fn set_main_tracks_playing(&mut self, player: &mut AnimationPlayer, id: u32, playing: bool) {
        let n = self.node(id).unwrap();
        for &t in &n.animation_tracks {
            if let Some(t) = player.track_mut(t) {
                t.is_playing = playing;
            }
        }
        if let Some(t) = n.running_ragdoll_track.and_then(|t| player.track_mut(t)) {
            t.is_playing = playing;
        }
    }

    /// `LiveAnimNode.release()`: removes its tracks and drops its
    /// while-alive flags.
    fn release_node(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, id: u32) {
        let n = self.node(id).unwrap();
        let mut tracks: Vec<TrackId> = n.animation_tracks.clone();
        tracks.extend_from_slice(&n.ragdoll_tracks);
        tracks.extend(n.running_ragdoll_track);
        tracks.extend(n.transition_in.track);
        for t in tracks {
            self.destroy_track(player, t);
        }
        self.decrement_node_while_alive_flags(vars, id);
        self.nodes[id as usize] = None;
    }

    // ---- per-layer update ----

    /// `AnimLayer.Update` / `updateInternal`.
    fn layer_update(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, layer: u32, dt: f32) {
        let highest = self.highest_live_node(layer);
        {
            let l = self.layer_mut(layer);
            l.current_sync_node = highest;
            l.current_sync_track = None;
        }
        let Some(highest) = highest else { return };
        let ids = self.layer_mut(layer).live_nodes.clone();
        for &id in &ids {
            self.node_update(player, vars, id, dt);
        }
        self.update_maximum_twist(vars, layer);
        let sync = self.find_sync_track(player, layer, highest);
        self.layer_mut(layer).current_sync_track = sync;
        let sync_value = sync.and_then(|s| player.track(s)).map(|t| t.current_time_fraction()).unwrap_or(-1.0);
        let ids = self.layer_mut(layer).live_nodes.clone();
        for id in ids {
            let mut duration = 1.0f32;
            for tid in self.node(id).unwrap().playing_tracks() {
                let Some(t) = player.track_mut(tid) else { continue };
                if !t.is_playing {
                    continue;
                }
                if sync.is_some() && t.sync_tracking_enabled && t.is_looping() && Some(tid) != sync {
                    t.move_current_time_value_to_fraction(sync_value);
                }
                if !t.is_primary {
                    continue;
                }
                duration = t.get_duration();
                let ct = t.current_time_value();
                self.node_mut(id).node_anim_time = ct;
            }
            let node = self.node(id).unwrap();
            let src = node.source.clone();
            if !src.events.is_empty() {
                let pct = node.node_anim_time / duration;
                let prev = node.prev_node_anim_time / duration;
                let looped = node.is_looped();
                for ev in &src.events {
                    if ev.time != AnimEventTime::Percentage {
                        continue;
                    }
                    let pc = ev.time_pc;
                    if prev < pc && pc <= pct {
                        self.invoke_anim_event_raw(layer, Some(id), None, ev.clone(), vars);
                        continue;
                    }
                    if !looped && pct < pc {
                        break;
                    }
                    if !looped || !(prev > pct) {
                        continue;
                    }
                    if prev < pc && pc <= pct + 1.0 {
                        self.invoke_anim_event_raw(layer, Some(id), None, ev.clone(), vars);
                        continue;
                    }
                    if !(prev > pc) || !(pc <= pct) {
                        continue;
                    }
                    self.invoke_anim_event_raw(layer, Some(id), None, ev.clone(), vars);
                }
            }
            let node = self.node(id).unwrap();
            if node.playing_tracks().is_empty() {
                continue;
            }
            let scalar = src.scalar.clone();
            let scalar2 = src.scalar2.clone();
            let tin_w = node.transition_in.weight;
            if let Some(t) = node.transition_in.track.and_then(|t| player.track_mut(t)) {
                t.set_blend_weight(tin_w);
            }
            let node_weight = node.weight;
            let mut remaining = node_weight;
            if let Some(rt) = node.running_ragdoll_track.and_then(|t| player.track_mut(t)) {
                let ws = pzmath::clamp(vars.get_variable_float(&scalar, 1.0), 0.0, 1.0);
                rt.set_blend_weight(node_weight * ws);
                remaining -= rt.blend_field_weight() * node_weight;
            }
            if node.animation_tracks.is_empty() {
                continue;
            }
            if node.is_blend_field {
                let x = vars.get_variable_float(&scalar, 0.0);
                let y = vars.get_variable_float(&scalar2, 0.0);
                self.apply_blend_field(player, id, remaining, x, y);
                continue;
            }
            let ws = pzmath::clamp(vars.get_variable_float(&scalar, 1.0), 0.0, 1.0);
            let main_w = remaining * ws;
            let portion = main_w / node.animation_tracks.len() as f32;
            for &t in &node.animation_tracks {
                if let Some(t) = player.track_mut(t) {
                    t.set_blend_weight(portion);
                }
            }
        }
    }

    fn apply_blend_field(&mut self, player: &mut AnimationPlayer, id: u32, node_weight: f32, x: f32, y: f32) {
        let n = self.node(id).unwrap();
        if n.active {
            let src = &n.source;
            if let Some(picker) = &src.blend2d_picker {
                let r = picker.pick(&src.blends2d, x, y);
                let s1 = if r.scale1.is_nan() { 0.5 } else { r.scale1 };
                let s2 = if r.scale2.is_nan() { 0.5 } else { r.scale2 };
                let s3 = if r.scale3.is_nan() { 0.5 } else { r.scale3 };
                for (i, &tid) in n.animation_tracks.iter().enumerate() {
                    let i = Some(i);
                    let mut w = if i == r.node1 {
                        s1
                    } else if i == r.node2 {
                        s2
                    } else if i == r.node3 {
                        s3
                    } else {
                        0.0
                    };
                    if w < 1.0e-4 {
                        w = 0.0;
                    }
                    w = pzmath::clamp(w, 0.0, 1.0);
                    if let Some(t) = player.track_mut(tid) {
                        t.set_blend_field_weight(w);
                    }
                }
            }
        }
        for &tid in &n.animation_tracks {
            if let Some(t) = player.track_mut(tid) {
                t.set_blend_weight(node_weight);
            }
        }
    }

    fn highest_live_node(&self, layer: u32) -> Option<u32> {
        let nodes = &self.layer(layer).unwrap().live_nodes;
        let mut best = *nodes.first()?;
        for &id in nodes.iter().rev() {
            if self.node(id).unwrap().weight > self.node(best).unwrap().weight {
                best = id;
            }
        }
        Some(best)
    }

    fn find_sync_track(&self, player: &AnimationPlayer, layer: u32, node: u32) -> Option<TrackId> {
        if let Some(p) = self.layer(layer).unwrap().parent {
            if let Some(s) = self.layer(p).and_then(|l| l.current_sync_track) {
                return Some(s);
            }
        }
        let mut sync: Option<TrackId> = None;
        for tid in self.node(node).unwrap().playing_tracks() {
            let Some(t) = player.track(tid) else { continue };
            if !t.sync_tracking_enabled || !t.has_clip() {
                continue;
            }
            if let Some(s) = sync.and_then(|s| player.track(s)) {
                if !(t.blend_weight() > s.blend_weight()) {
                    continue;
                }
            }
            sync = Some(tid);
        }
        sync
    }

    fn update_maximum_twist(&mut self, vars: &mut dyn AnimationVariableSource, layer: u32) {
        let Some(h) = AnimationVariableHandle::alloc("maxTwist") else { return };
        let Some(v) = vars.get_variable(&h) else { return };
        let m = self.maximum_twist(layer, v.as_float());
        vars.set_existing_variable(&h, VariableValue::Float(m));
    }

    /// `AnimLayer.getMaximumTwist`.
    pub fn maximum_twist(&self, layer: u32, prev: f32) -> f32 {
        let l = self.layer(layer).unwrap();
        let max_possible = match l.parent {
            Some(p) => self.maximum_twist(p, prev),
            None => 70.0,
        };
        let mut twist = 0.0f32;
        let mut remaining = 1.0f32;
        for &id in l.live_nodes.iter().rev() {
            let n = self.node(id).unwrap();
            if remaining <= 0.0 {
                break;
            }
            let w = pzmath::clamp(n.weight, 0.0, remaining);
            remaining -= w;
            let node_max = pzmath::clamp(n.source.max_torso_twist, 0.0, max_possible);
            twist += node_max * w;
        }
        if remaining > 0.0 {
            twist += prev * remaining;
        }
        twist
    }

    // ---- live node update ----

    /// `LiveAnimNode.update`.
    fn node_update(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, id: u32, dt: f32) {
        {
            let n = self.node_mut(id);
            n.is_new = false;
            if n.active != n.was_active {
                n.blending_in = n.active;
                n.blending_out = !n.active;
                if n.transition_in.active {
                    n.transition_in.blending_in = n.active;
                    n.transition_in.blending_out = !n.active;
                }
                n.was_active = n.active;
            }
        }
        let was_main = self.node(id).unwrap().is_main_anim_active();
        if self.node(id).unwrap().is_transitioning_in() {
            self.update_transitioning_in(player, vars, id, dt);
        }
        let n = self.node(id).unwrap();
        let main = n.is_main_anim_active();
        if main {
            let p = !(n.blending_out && n.source.stop_anim_on_exit);
            self.set_main_tracks_playing(player, id, p);
        } else {
            self.set_main_tracks_playing(player, id, false);
        }
        if !main {
            return;
        }
        if !was_main && self.node(id).unwrap().is_looped() {
            let rewind = self.main_initial_rewind_time(player, id);
            for &t in &self.node(id).unwrap().animation_tracks {
                if let Some(t) = player.track_mut(t) {
                    t.scaled_rewind(rewind);
                }
            }
        }
        let disable_blends = player.debug.disable_animation_blends;
        let n = self.node_mut(id);
        if n.blending_in {
            let max = n.blend_in_time();
            if max <= 0.0 || disable_blends {
                n.weight = 1.0;
                n.raw_weight = 1.0;
                n.blending_in = false;
            } else {
                let next = LiveAnimNode::increment_blend_time(n.raw_weight, max, dt);
                let raw = pzmath::clamp(next / max, 0.0, 1.0);
                n.raw_weight = raw;
                n.weight = pzmath::lerp_func_ease_out_in_quad(raw);
                n.blending_in = next < max;
            }
        } else if n.blending_out {
            let max = n.blend_out_time();
            if max <= 0.0 || disable_blends {
                n.weight = 0.0;
                n.raw_weight = 0.0;
                n.blending_out = false;
            } else {
                let next = LiveAnimNode::increment_blend_time(1.0 - n.raw_weight, max, dt);
                let raw = pzmath::clamp(1.0 - next / max, 0.0, 1.0);
                n.raw_weight = raw;
                n.weight = pzmath::lerp_func_ease_out_in_quad(raw);
                n.blending_out = next < max;
            }
        }
        n.prev_node_anim_time = n.node_anim_time;
        match n.source.track_time_to_variable.clone().filter(|s| !s.is_empty()) {
            None => n.node_anim_time += dt,
            Some(var) => {
                let target = n.node_anim_time + dt;
                let len = self.max_duration(player, id);
                let t = len * vars.get_variable_float(&var, target / len);
                let n = self.node_mut(id);
                n.node_anim_time = t;
                if n.prev_node_anim_time < n.node_anim_time - dt * 1.1 {
                    n.prev_node_anim_time = n.node_anim_time;
                }
            }
        }
        let n = self.node(id).unwrap();
        if !n.transition_in.active {
            if let Some(tid) = n.transition_in.track {
                if player.track(tid).is_none_or(|t| t.blend_weight() <= 0.0) {
                    self.stop_transition_in(player, id);
                }
            }
        }
        self.update_ragdoll_tracks(player, id);
    }

    fn max_duration(&self, player: &AnimationPlayer, id: u32) -> f32 {
        let n = self.node(id).unwrap();
        let mut m = 0.0f32;
        for &t in n.animation_tracks.iter().chain(&n.ragdoll_tracks) {
            if let Some(t) = player.track(t) {
                m = pzmath::max(t.get_duration(), m);
            }
        }
        m
    }

    /// `getMainInitialRewindTime`.
    fn main_initial_rewind_time(&mut self, player: &AnimationPlayer, id: u32) -> f32 {
        let frac = self.node(id).unwrap().source.random_advance_fraction;
        let mut random_advance = 0.0f32;
        if frac > 0.0 {
            let f = self.rng.next_f32(0.0, frac);
            random_advance = f * self.max_duration(player, id);
        }
        let n = self.node(id).unwrap();
        let Some(d) = &n.transition_in.data else { return 0.0 - random_advance };
        let blend_in = n.transition_in_blend_out_time();
        let sync_adjust = d.get().sync_adjust_time;
        if n.transition_in.track.is_some() {
            return blend_in - sync_adjust;
        }
        blend_in - sync_adjust - random_advance
    }

    /// `LiveAnimNode.updateTransitioningIn`.
    fn update_transitioning_in(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource, id: u32, dt: f32) {
        let tid = self.node(id).unwrap().transition_in.track.unwrap();
        let Some(t) = player.track(tid) else {
            self.stop_transition_in(player, id);
            return;
        };
        let speed = t.speed_delta();
        let duration = t.get_duration();
        let ct = t.current_time_value();
        self.node_mut(id).transition_in.time = ct;
        if ct >= duration || player.debug.disable_animation_blends {
            self.stop_transition_in(player, id);
            return;
        }
        let n = self.node(id).unwrap();
        if !n.transition_in.blending_out {
            let pass = AnimCondition::pass(&*vars, &n.transition_in.data.as_ref().unwrap().get().conditions.borrow());
            if !pass {
                let n = self.node_mut(id);
                n.transition_in.blending_in = false;
                n.transition_in.blending_out = true;
            }
        }
        let n = self.node_mut(id);
        let blend_out = n.transition_in_blend_out_time() * speed;
        if n.transition_in.time >= duration - blend_out {
            n.transition_in.blending_in = false;
            n.transition_in.blending_out = true;
        }
        if n.transition_in.blending_in {
            let max = n.transition_in_blend_in_time() * speed;
            let next = LiveAnimNode::increment_blend_time(n.transition_in.raw_weight, max, dt * speed);
            let raw = pzmath::clamp(next / max, 0.0, 1.0);
            n.transition_in.raw_weight = raw;
            n.transition_in.weight = pzmath::lerp_func_ease_out_in_quad(raw);
            n.transition_in.blending_in = next < max;
            n.transition_in.active = next < duration;
        }
        if n.transition_in.blending_out {
            let max = n.transition_in_blend_out_time() * speed;
            let next = LiveAnimNode::increment_blend_time(1.0 - n.transition_in.raw_weight, max, dt * speed);
            let raw = pzmath::clamp(1.0 - next / max, 0.0, 1.0);
            n.transition_in.raw_weight = raw;
            n.transition_in.weight = pzmath::lerp_func_ease_out_in_quad(raw);
            n.transition_in.blending_out = next < max;
            n.transition_in.active = n.transition_in.blending_out;
        }
    }

    fn update_ragdoll_tracks(&mut self, player: &mut AnimationPlayer, id: u32) {
        let n = self.node(id).unwrap();
        if n.ragdoll_tracks.is_empty() {
            return;
        }
        if n.running_ragdoll_track.is_none() {
            let mut found: Option<(TrackId, f32)> = None;
            for &tid in &n.ragdoll_tracks {
                let Some(t) = player.track(tid) else { continue };
                if t.ragdoll_start_time > n.node_anim_time {
                    continue;
                }
                match found {
                    None => found = Some((tid, t.ragdoll_start_time)),
                    Some((_, s)) if t.ragdoll_start_time < s => found = Some((tid, t.ragdoll_start_time)),
                    _ => {}
                }
            }
            if let Some((tid, _)) = found {
                self.node_mut(id).running_ragdoll_track = Some(tid);
                if let Some(t) = player.track_mut(tid) {
                    t.set_blend_field_weight(0.0);
                }
            }
        }
        let n = self.node(id).unwrap();
        let Some(rt) = n.running_ragdoll_track else { return };
        let Some(t) = player.track_mut(rt) else { return };
        if n.animation_tracks.is_empty() {
            t.set_blend_field_weight(1.0);
            return;
        }
        if t.blend_field_weight() == 1.0 {
            return;
        }
        let diff = n.node_anim_time - t.ragdoll_start_time;
        let bt = n.source.blend_time;
        let w = if diff > 0.0 && diff <= bt {
            diff / bt
        } else if diff > bt {
            1.0
        } else {
            0.0
        };
        let cur = t.blend_field_weight();
        t.set_blend_field_weight(pzmath::max(cur, w));
    }

    // ---- events ----

    /// Dispatches the listener callbacks the last player update queued.
    pub fn dispatch_track_events(&mut self, player: &mut AnimationPlayer, vars: &mut dyn AnimationVariableSource) {
        let events = std::mem::take(&mut player.events);
        let allow_early = player.debug.allow_early_transition_out;
        for e in events {
            for l in &e.listeners {
                match *l {
                    TrackListener::Layer(layer) => {
                        if self.layer(layer).is_none() {
                            continue;
                        }
                        match e.kind {
                            TrackEventKind::AnimStarted => {}
                            TrackEventKind::LoopedAnim => self.layer_local_event(layer, e.track, LocalAnimEvent::ActiveAnimLooped, false, vars),
                            TrackEventKind::NonLoopedAnimFadeOut => {
                                self.layer_local_event(layer, e.track, LocalAnimEvent::ActiveAnimFinishing, true, vars);
                                self.layer_local_event(layer, e.track, LocalAnimEvent::ActiveNonLoopedAnimFadeOut, true, vars);
                            }
                            TrackEventKind::NonLoopedAnimFinished => {
                                self.layer_local_event(layer, e.track, LocalAnimEvent::ActiveAnimFinishing, false, vars);
                                self.layer_local_event(layer, e.track, LocalAnimEvent::ActiveNonLoopedAnimFinished, true, vars);
                            }
                        }
                    }
                    TrackListener::Node(id) => {
                        if !self.node_exists(id) {
                            continue;
                        }
                        match e.kind {
                            TrackEventKind::AnimStarted => self.node_time_event(id, e.track, AnimEventTime::Start, vars),
                            TrackEventKind::LoopedAnim => {
                                if !self.node(id).unwrap().transitioning_out {
                                    self.node_time_event(id, e.track, AnimEventTime::End, vars);
                                }
                            }
                            TrackEventKind::NonLoopedAnimFadeOut => {
                                if allow_early {
                                    self.node_time_event(id, e.track, AnimEventTime::End, vars);
                                    self.node_mut(id).transitioning_out = true;
                                    self.decrement_node_while_alive_flags(vars, id);
                                }
                            }
                            TrackEventKind::NonLoopedAnimFinished => {
                                if !self.node(id).unwrap().transitioning_out {
                                    self.node_time_event(id, e.track, AnimEventTime::End, vars);
                                    self.decrement_node_while_alive_flags(vars, id);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.sync_track_depths(player);
    }

    /// `AnimLayer.invokeAnimEvent(track, LocalAnimEvent, includeTransitioningOut)`.
    fn layer_local_event(&mut self, layer: u32, track: TrackId, ev: LocalAnimEvent, include_out: bool, vars: &mut dyn AnimationVariableSource) {
        let l = self.layer(layer).unwrap();
        let state_id = l.state.as_ref().map(|s| s.id);
        let target = l.live_nodes.iter().copied().find(|&id| {
            let n = self.node(id).unwrap();
            !(n.transitioning_out && !include_out) && Some(n.source.parent_state.get()) == state_id && n.animation_tracks.contains(&track)
        });
        if let Some(id) = target {
            self.invoke_anim_event_raw(layer, Some(id), Some(track), Rc::new(ev.anim_event()), vars);
        }
    }

    /// `LiveAnimNode.invokeAnimTimeEvent`.
    fn node_time_event(&mut self, id: u32, track: TrackId, time: AnimEventTime, vars: &mut dyn AnimationVariableSource) {
        let n = self.node(id).unwrap();
        let layer = n.layer;
        let events: Vec<Rc<AnimEvent>> = n.source.events.iter().filter(|e| e.time == time).cloned().collect();
        for e in events {
            if self.layer(layer).is_none() {
                break;
            }
            self.invoke_anim_event_raw(layer, Some(id), Some(track), e, vars);
        }
    }

    /// `AnimLayer.invokeAnimEvent(node, track, event)`.
    fn invoke_anim_event_raw(&mut self, layer: u32, node: Option<u32>, track: Option<TrackId>, ev: Rc<AnimEvent>, vars: &mut dyn AnimationVariableSource) {
        if let (AnimEventKind::FlagWhileAlive { reference, flag_value }, Some(id)) = (&ev.kind, node) {
            let n = self.node_mut(id);
            if !n.while_alive_flags.iter().any(|(r, _)| Rc::ptr_eq(r, reference)) {
                n.while_alive_flags.push((reference.clone(), *flag_value));
                self.increment_while_alive_flag(vars, reference, *flag_value);
            }
        }
        let l = self.layer(layer).unwrap();
        self.events.push(AnimEventRecord {
            state_name: l.current_state_name().map(str::to_string),
            layer_depth: self.layer_depth(layer),
            track,
            event: ev,
        });
    }

    /// `invokeGlobalAnimEvent`.
    pub fn invoke_global_anim_event(&mut self, name: &str) {
        self.events.push(AnimEventRecord {
            state_name: None,
            layer_depth: -1,
            track: None,
            event: Rc::new(AnimEvent::named(name, AnimEventTime::End, 0.0)),
        });
    }

    fn counter_index(&mut self, reference: &VariableReferenceRc) -> usize {
        match self.set_flag_counters.iter().position(|(r, _)| r.equals(reference)) {
            Some(i) => i,
            None => {
                self.set_flag_counters.push((reference.clone(), 0));
                self.set_flag_counters.len() - 1
            }
        }
    }

    fn increment_while_alive_flag(&mut self, vars: &mut dyn AnimationVariableSource, reference: &VariableReferenceRc, value: bool) {
        let i = self.counter_index(reference);
        self.set_flag_counters[i].1 += 1;
        let c = self.set_flag_counters[i].1;
        reference.set_variable(vars, VariableValue::Bool(if c > 0 { value } else { !value }));
    }

    fn decrement_while_alive_flag(&mut self, vars: &mut dyn AnimationVariableSource, reference: &VariableReferenceRc, value: bool) {
        let Some(i) = self.set_flag_counters.iter().position(|(r, _)| r.equals(reference)) else { return };
        if self.set_flag_counters[i].1 > 0 {
            self.set_flag_counters[i].1 -= 1;
        }
        let c = self.set_flag_counters[i].1;
        reference.set_variable(vars, VariableValue::Bool(if c > 0 { value } else { !value }));
    }

    fn decrement_node_while_alive_flags(&mut self, vars: &mut dyn AnimationVariableSource, id: u32) {
        let flags = std::mem::take(&mut self.node_mut(id).while_alive_flags);
        for (r, v) in flags {
            self.decrement_while_alive_flag(vars, &r, v);
        }
    }

    // ---- queries ----

    /// `containsAnyIdleNodes`.
    pub fn contains_any_idle_nodes(&self) -> bool {
        let is_idle = |id: &u32| self.node(*id).is_some_and(|n| n.active && n.source.is_idle_anim());
        let mut idle = self.root_layer().live_nodes.iter().any(is_idle);
        for k in 0..self.sub_layers.len() {
            let l = self.sub_layer_at(k);
            for id in &l.live_nodes {
                idle = is_idle(id);
                if !idle {
                    break;
                }
            }
        }
        idle
    }

    /// `updateSpeedScale(variable, newSpeed)`.
    pub fn update_speed_scale(&mut self, player: &mut AnimationPlayer, variable: &str, new_speed: f32) {
        let ids = self.root_layer().live_nodes.clone();
        for id in ids {
            let n = self.node(id).unwrap();
            if !n.active || n.source.speed_scale_variable.as_deref() != Some(variable) {
                continue;
            }
            *n.source.speed_scale.borrow_mut() = crate::vars::java_float_to_string(new_speed);
            for &t in &n.animation_tracks {
                if let Some(t) = player.track_mut(t) {
                    t.set_speed_delta(new_speed);
                }
            }
        }
    }

    /// The main tracks of the root layer's active live nodes.
    pub fn root_layer_tracks(&self) -> Vec<TrackId> {
        self.root_layer()
            .live_nodes
            .iter()
            .filter_map(|&id| self.node(id))
            .filter(|n| n.active)
            .flat_map(|n| n.animation_tracks.iter().copied())
            .collect()
    }

    /// The live nodes of a layer.
    pub fn live_nodes(&self, layer: u32) -> Vec<&LiveAnimNode> {
        self.layer(layer).map(|l| l.live_nodes.iter().filter_map(|&id| self.node(id)).collect()).unwrap_or_default()
    }
}
