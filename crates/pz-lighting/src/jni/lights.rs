//! `LightingJNI.checkLights`: keeps the library's lights, room lights and
//! torches in step with the world's.

use std::collections::HashMap;

use super::input::{LightSource, RoomLightSource, TorchInfo};
use super::natives::LightingNatives;
use crate::LightingError;

/// Torch id of a player's `i`-th active light item (`playerIndex * 4 + i +
/// 1`; online players use `onlineId + 1` as the index).
pub fn player_torch_id(player_index: i32, item: i32) -> i32 {
    player_index * 4 + item + 1
}

/// Torch id of a vehicle's `j`-th light (`4096 + vehicleId * 10 + j`).
pub fn vehicle_torch_id(vehicle_id: i32, light: i32) -> i32 {
    4096 + vehicle_id * 10 + light
}

/// What one [`LightSync::check_lights`] call changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckLightsResult {
    /// Set `GameTime.lightSourceUpdate = 100` (a light or room light was
    /// added or removed, or a room light toggled).
    pub light_source_update: bool,
    /// Temporary lights that were handed to the library; Java removes them
    /// from the lamppost list, so the world must drop them too.
    pub consumed_temp_lights: Vec<u64>,
    /// Lamppost lights that left the list (dead or out of bounds); the
    /// world drops them.
    pub removed_lights: Vec<u64>,
    /// Room lights that went out of bounds; the world drops them.
    pub removed_room_lights: Vec<u64>,
}

#[derive(Clone, Copy, Debug)]
struct LightMirror {
    id: i32,
    rgb: [f32; 3],
    active: bool,
}

#[derive(Clone, Copy, Debug)]
struct RoomMirror {
    id: i32,
    active: bool,
}

/// The Java side's light bookkeeping (`IsoLightSource.id/rJni/gJni/bJni/
/// activeJni`, `JNILights`, `IsoRoomLight.id/activeJni`, the torch list).
#[derive(Clone, Debug)]
pub struct LightSync {
    next_light_id: i32,
    next_room_light_id: i32,
    /// `JNILights`, in insertion order.
    jni_lights: Vec<u64>,
    lights: HashMap<u64, LightMirror>,
    room_lights: HashMap<u64, RoomMirror>,
    torches: Vec<i32>,
}

impl Default for LightSync {
    fn default() -> Self {
        LightSync {
            next_light_id: 1,
            next_room_light_id: 1,
            jni_lights: Vec::new(),
            lights: HashMap::new(),
            room_lights: HashMap::new(),
            torches: Vec::new(),
        }
    }
}

fn boost(c: f32) -> f32 {
    (c * 2.0).clamp(0.0, 1.0)
}

impl LightSync {
    pub fn new() -> LightSync {
        LightSync::default()
    }

    /// The library id a world light was given, if it was sent.
    pub fn light_id(&self, key: u64) -> Option<i32> {
        self.lights.get(&key).map(|l| l.id)
    }

    /// The library id a world room light was given, if it was sent.
    pub fn room_light_id(&self, key: u64) -> Option<i32> {
        self.room_lights.get(&key).map(|l| l.id)
    }

    /// `LightingJNI.checkLights()`. `lights` is
    /// `IsoCell.getLamppostPositions()`, `room_lights` is
    /// `IsoCell.roomLights` and `torches` every active torch this frame
    /// (players' light items, then vehicle lights), all in list order.
    /// `lock_fps` is `PerformanceSettings.getLockFPS()` (temporary light
    /// lifetimes are scaled from 30 fps frames).
    pub fn check_lights(
        &mut self,
        n: &mut impl LightingNatives,
        lights: &[LightSource],
        room_lights: &[RoomLightSource],
        torches: &[TorchInfo],
        lock_fps: i32,
    ) -> Result<CheckLightsResult, LightingError> {
        let mut out = CheckLightsResult::default();
        let mut listed = std::collections::HashSet::new();
        for l in lights {
            if !l.alive {
                out.removed_lights.push(l.key);
                if let Some(m) = self.lights.remove(&l.key) {
                    self.jni_lights.retain(|&k| k != l.key);
                    n.remove_light(m.id)?;
                    out.light_source_update = true;
                }
                continue;
            }
            match self.lights.get_mut(&l.key) {
                None => {
                    let id = self.next_light_id;
                    self.next_light_id += 1;
                    if l.life != -1 {
                        let frames = ((l.life * lock_fps) as f32 / 30.0) as i32;
                        n.add_temp_light(id, l.x, l.y, l.z + 32, l.radius, l.r, l.g, l.b, frames)?;
                        out.consumed_temp_lights.push(l.key);
                        continue;
                    }
                    listed.insert(l.key);
                    self.lights.insert(l.key, LightMirror { id, rgb: [l.r, l.g, l.b], active: l.active });
                    self.jni_lights.push(l.key);
                    n.add_light(
                        id,
                        l.x,
                        l.y,
                        l.z + 32,
                        l.radius.min(20),
                        boost(l.r),
                        boost(l.g),
                        boost(l.b),
                        l.local_to_building.unwrap_or(-1),
                        l.active,
                    )?;
                }
                Some(m) => {
                    listed.insert(l.key);
                    if m.rgb != [l.r, l.g, l.b] {
                        m.rgb = [l.r, l.g, l.b];
                        n.set_light_color(m.id, boost(l.r), boost(l.g), boost(l.b))?;
                    }
                    if m.active != l.active {
                        m.active = l.active;
                        n.set_light_active(m.id, l.active)?;
                    }
                }
            }
        }
        // `JNILights` entries no longer in the lamppost list.
        let stale: Vec<u64> = self.jni_lights.iter().copied().filter(|k| !listed.contains(k)).collect();
        for k in stale {
            self.jni_lights.retain(|&x| x != k);
            if let Some(m) = self.lights.remove(&k) {
                n.remove_light(m.id)?;
            }
        }
        for rl in room_lights {
            if !rl.in_bounds {
                out.removed_room_lights.push(rl.key);
                if let Some(m) = self.room_lights.remove(&rl.key) {
                    n.remove_room_light(m.id)?;
                    out.light_source_update = true;
                }
                continue;
            }
            match self.room_lights.get_mut(&rl.key) {
                None => {
                    let id = 100000 + self.next_room_light_id;
                    self.next_room_light_id += 1;
                    n.add_room_light(id, rl.building_id, rl.room_id, rl.x, rl.y, rl.z + 32, rl.width, rl.height, rl.active)?;
                    self.room_lights.insert(rl.key, RoomMirror { id, active: rl.active });
                    out.light_source_update = true;
                }
                Some(m) => {
                    if m.active != rl.active {
                        n.set_room_light_active(m.id, rl.active)?;
                        m.active = rl.active;
                        out.light_source_update = true;
                    }
                }
            }
        }
        let mut active = Vec::new();
        for t in torches {
            n.update_torch(t.id, t.x, t.y, t.z + 32.0, t.r, t.g, t.b, t.angle_x, t.angle_y, t.dist, t.strength, t.cone, t.dot, t.focusing)?;
            if !active.contains(&t.id) {
                active.push(t.id);
            }
        }
        for &id in &self.torches {
            if !active.contains(&id) {
                n.remove_torch(id)?;
            }
        }
        self.torches = active;
        Ok(out)
    }

    /// `LightingJNI.stop()` bookkeeping: forget every light (call after
    /// destroying the library).
    pub fn reset(&mut self) {
        *self = LightSync::default();
    }
}
