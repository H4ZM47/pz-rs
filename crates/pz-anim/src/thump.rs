//! `zombie.characters.ThumpAnimations`: how many `ThumpFrame` events the
//! playing thump animation crosses in a time span, read from the root
//! layer's live node instead of the dispatched events (`ThumpState.execute`
//! counts thumps this way).

use crate::animator::{AdvancedAnimator, LiveAnimNode};
use crate::animset::AnimEventTime;
use crate::player::AnimationPlayer;
use crate::track::AnimationTrack;
use crate::vars::equals_ignore_case;

/// `ThumpAnimations.EVENT_NAME`.
pub const EVENT_NAME: &str = "ThumpFrame";

/// `ThumpAnimations.MAX_THUMP_TIMES`: the size of the original's scratch
/// array. A node with more `ThumpFrame` events throws there
/// (`ArrayIndexOutOfBoundsException`); here the extra events count.
pub const MAX_THUMP_TIMES: usize = 10;

/// `getLiveAnimNode(chr)`: the first live node of the root layer whose
/// source node has a `ThumpFrame` event.
pub fn live_anim_node(animator: &AdvancedAnimator) -> Option<&LiveAnimNode> {
    animator
        .root_layer()
        .live_nodes()
        .iter()
        .filter_map(|&id| animator.node(id))
        .find(|n| {
            n.source
                .events
                .iter()
                .any(|e| equals_ignore_case(&e.event_name, EVENT_NAME))
        })
}

/// `getAnimationTrack(liveAnimNode)`: the node's first playing main track.
pub fn node_animation_track<'a>(
    node: &LiveAnimNode,
    player: &'a AnimationPlayer,
) -> Option<&'a AnimationTrack> {
    node.main_animation_tracks()
        .iter()
        .filter_map(|&t| player.track(t))
        .find(|t| t.is_playing)
}

/// `getAnimationTrack(chr)`.
pub fn animation_track<'a>(
    animator: &AdvancedAnimator,
    player: &'a AnimationPlayer,
) -> Option<&'a AnimationTrack> {
    node_animation_track(live_anim_node(animator)?, player)
}

/// `getDuration(chr)`: the thump track's duration, -1 without one.
pub fn duration(animator: &AdvancedAnimator, player: &AnimationPlayer) -> f32 {
    animation_track(animator, player).map_or(-1.0, AnimationTrack::get_duration)
}

/// `getThumpTimes(liveAnimNode, times)`: the node's `ThumpFrame` times as
/// fractions of the animation (`timePc`, or 0 for `Start` and 1 for `End`
/// and an unset time), in event order.
pub fn thump_times(node: &LiveAnimNode) -> Vec<f32> {
    node.source
        .events
        .iter()
        .filter(|e| equals_ignore_case(&e.event_name, EVENT_NAME))
        .map(|e| match e.time {
            AnimEventTime::Percentage => e.time_pc,
            AnimEventTime::Start => 0.0,
            _ => 1.0,
        })
        .collect()
}

/// The count for one set of thump times and a duration: for each time,
/// how many times `start..end` (track seconds, accumulated across loops)
/// crosses `time * duration` modulo the duration. Java's
/// `(int) Math.floor(float)` saturates (NaN gives 0) and the int sums
/// wrap.
pub fn count_between(times: &[f32], duration: f32, start_time: f32, end_time: f32) -> i32 {
    let mut result = 0i32;
    for &t in times {
        let s = (((start_time - t * duration) / duration) as f64).floor() as i32;
        let e = (((end_time - t * duration) / duration) as f64).floor() as i32;
        result = result.wrapping_add(e.wrapping_sub(s));
    }
    result
}

/// `countThumpEventsBetween(chr, startTime, endTime)`.
pub fn count_thump_events_between(
    animator: &AdvancedAnimator,
    player: &AnimationPlayer,
    start_time: f32,
    end_time: f32,
) -> i32 {
    let Some(node) = live_anim_node(animator) else {
        return 0;
    };
    let Some(track) = node_animation_track(node, player) else {
        return 0;
    };
    let times = thump_times(node);
    if times.is_empty() {
        return 0;
    }
    count_between(&times, track.get_duration(), start_time, end_time)
}

/// The thump count `ThumpState.execute` takes for this frame: when the
/// animation updates this frame (`isAnimationUpdatingThisFrame()`) and a
/// thump track plays, the events between the track's accumulated time and
/// that time plus `animation_time_delta * speedDelta`
/// (`getAnimationTimeDelta()` is `GameTime.getTimeDelta() *
/// animationTimeScale`); 0 otherwise. Read before the animation advances.
pub fn thumps_this_frame(
    animator: &AdvancedAnimator,
    player: &AnimationPlayer,
    animation_updating_this_frame: bool,
    animation_time_delta: f32,
) -> i32 {
    if !animation_updating_this_frame {
        return 0;
    }
    let Some(track) = animation_track(animator, player) else {
        return 0;
    };
    let previous = track.accumulated_time_value();
    let current = previous + animation_time_delta * track.speed_delta();
    count_thump_events_between(animator, player, previous, current)
}

#[cfg(test)]
mod tests {
    use super::count_between;

    #[test]
    fn crossings_of_each_thump_time() {
        // DoorClaw-like: events at 23% and 73% of a 2 s clip (0.46 s, 1.46 s).
        let times = [0.23, 0.73];
        assert_eq!(count_between(&times, 2.0, 0.0, 0.4), 0);
        assert_eq!(count_between(&times, 2.0, 0.4, 0.5), 1);
        assert_eq!(count_between(&times, 2.0, 0.5, 1.4), 0);
        assert_eq!(count_between(&times, 2.0, 1.4, 1.5), 1);
        // Both, and the next loop's first: 0.46, 1.46, 2.46.
        assert_eq!(count_between(&times, 2.0, 0.0, 2.5), 3);
        // The start of the span is exclusive, the end inclusive:
        // floor((0.46 - 0.46) / 2) = 0, floor((0.5 - 0.46) / 2) = 0.
        assert_eq!(count_between(&[0.25], 2.0, 0.5, 0.6), 0);
        assert_eq!(count_between(&[0.25], 2.0, 0.4, 0.5), 1);
        // An `End` event (1.0) fires as the loop wraps.
        assert_eq!(count_between(&[1.0], 1.0, 0.9, 1.1), 1);
        // A whole loop in one step crosses every event once.
        assert_eq!(count_between(&times, 1.0, 0.0, 1.0), 2);
        // Zero duration: NaN / infinities saturate as Java's d2i does;
        // the wrapping int maths of the original.
        assert_eq!(count_between(&[0.5], 0.0, 0.0, 0.0), 0);
        assert_eq!(
            count_between(&[0.5], 0.0, 0.0, 1.0),
            i32::MAX.wrapping_sub(0)
        );
        assert_eq!(
            count_between(&[0.5], 0.0, -1.0, 1.0),
            i32::MAX.wrapping_sub(i32::MIN)
        );
    }
}
