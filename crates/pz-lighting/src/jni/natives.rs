//! The native calls the Java side makes, as a trait so the same Java-side
//! logic can drive [`LightingSession`] or another implementation (for
//! example the original library in a parity harness).

use crate::{Character, LightingError, LightingSession, Transmission};

type R<T = ()> = Result<T, LightingError>;

/// The subset of `LightingJNI`'s natives the Java side calls.
#[allow(clippy::too_many_arguments)]
pub trait LightingNatives {
    fn state_begin_update(&mut self, p: i32, x: i32, y: i32, w: i32, h: i32) -> R;
    fn player_set(&mut self, c: Character) -> R;
    fn state_end_frame(
        &mut self,
        rmod: f32,
        gmod: f32,
        bmod: f32,
        ambient: f32,
        night: f32,
        view_dist: f32,
        view_dist_max: f32,
        cache_cleared: bool,
        light_source_update: f32,
        sky: i32,
    ) -> R;
    fn state_end_update(&mut self) -> R;
    fn state_update_counter(&mut self, p: i32) -> R<i32>;
    fn chunk_lighting_done(&mut self, wx: i32, wy: i32) -> R<bool>;
    fn chunk_begin_update(&mut self, wx: i32, wy: i32, min: i32, max: i32) -> R;
    fn chunk_end_update(&mut self) -> R;
    fn chunk_level_begin_update(&mut self, z: i32) -> R;
    fn chunk_level_end_update(&mut self) -> R;
    fn square_set_null(&mut self, x: i32, y: i32, z: i32) -> R;
    fn square_begin_update(&mut self, x: i32, y: i32, z: i32) -> R;
    fn square_set(&mut self, vu: i32, up: bool, down: bool, elevated: bool, vm: i32, building: i64, room: i64, level: i32, open_air: bool) -> R;
    fn square_set_light_transmission(&mut self, t: [Transmission; 4]) -> R;
    fn square_add_curtain(&mut self, wnes: i32, open: bool) -> R;
    fn square_add_door(&mut self, north: bool, open: bool, trans: bool) -> R;
    fn square_add_thumpable(&mut self, north: bool, open: bool, is_door: bool, door_trans: bool) -> R;
    fn square_add_window(&mut self, north: bool, open: bool, opaque: bool) -> R;
    fn square_end_update(&mut self) -> R;
    fn add_light(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, r: f32, g: f32, b: f32, building: i32, active: bool) -> R;
    fn add_temp_light(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, r: f32, g: f32, b: f32, life: i32) -> R;
    fn remove_light(&mut self, id: i32) -> R;
    fn set_light_active(&mut self, id: i32, active: bool) -> R;
    fn set_light_color(&mut self, id: i32, r: f32, g: f32, b: f32) -> R;
    fn add_room_light(&mut self, id: i32, building: i64, room: i64, x: i32, y: i32, z: i32, w: i32, h: i32, active: bool) -> R;
    fn remove_room_light(&mut self, id: i32) -> R;
    fn set_room_light_active(&mut self, id: i32, active: bool) -> R;
    fn update_torch(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        r: f32,
        g: f32,
        b: f32,
        angle_x: f32,
        angle_y: f32,
        dist: f32,
        strength: f32,
        cone: bool,
        dot: f32,
        focusing: i32,
    ) -> R;
    fn remove_torch(&mut self, id: i32) -> R;
    fn get_square_dirty(&mut self, p: i32, x: i32, y: i32, z: i32) -> R<bool>;
    /// `getSquareLighting` into a 49-int buffer (the Java `lightInts`).
    fn get_square_lighting(&mut self, p: i32, x: i32, y: i32, z: i32, out: &mut [i32; 49]) -> R<bool>;
    fn get_visible_room_count(&mut self, p: i32) -> R<i32>;
    fn get_visible_rooms(&mut self, p: i32, out: &mut [i64]) -> R<i32>;
}

impl LightingNatives for LightingSession {
    fn state_begin_update(&mut self, p: i32, x: i32, y: i32, w: i32, h: i32) -> R {
        LightingSession::state_begin_update(self, p, x, y, w, h)
    }
    fn player_set(&mut self, c: Character) -> R {
        LightingSession::player_set(self, c)
    }
    fn state_end_frame(&mut self, rm: f32, gm: f32, bm: f32, am: f32, ni: f32, vd: f32, vdm: f32, cc: bool, lsu: f32, sky: i32) -> R {
        LightingSession::state_end_frame(self, rm, gm, bm, am, ni, vd, vdm, cc, lsu, sky)
    }
    fn state_end_update(&mut self) -> R {
        LightingSession::state_end_update(self)
    }
    fn state_update_counter(&mut self, p: i32) -> R<i32> {
        LightingSession::state_update_counter(self, p)
    }
    fn chunk_lighting_done(&mut self, wx: i32, wy: i32) -> R<bool> {
        LightingSession::chunk_lighting_done(self, wx, wy)
    }
    fn chunk_begin_update(&mut self, wx: i32, wy: i32, min: i32, max: i32) -> R {
        LightingSession::chunk_begin_update(self, wx, wy, min, max)
    }
    fn chunk_end_update(&mut self) -> R {
        LightingSession::chunk_end_update(self)
    }
    fn chunk_level_begin_update(&mut self, z: i32) -> R {
        LightingSession::chunk_level_begin_update(self, z)
    }
    fn chunk_level_end_update(&mut self) -> R {
        LightingSession::chunk_level_end_update(self)
    }
    fn square_set_null(&mut self, x: i32, y: i32, z: i32) -> R {
        LightingSession::square_set_null(self, x, y, z)
    }
    fn square_begin_update(&mut self, x: i32, y: i32, z: i32) -> R {
        LightingSession::square_begin_update(self, x, y, z)
    }
    fn square_set(&mut self, vu: i32, up: bool, down: bool, el: bool, vm: i32, b: i64, r: i64, lv: i32, oa: bool) -> R {
        LightingSession::square_set(self, vu, up, down, el, vm, b, r, lv, oa)
    }
    fn square_set_light_transmission(&mut self, t: [Transmission; 4]) -> R {
        LightingSession::square_set_light_transmission(self, t)
    }
    fn square_add_curtain(&mut self, wnes: i32, open: bool) -> R {
        LightingSession::square_add_curtain(self, wnes, open)
    }
    fn square_add_door(&mut self, north: bool, open: bool, trans: bool) -> R {
        LightingSession::square_add_door(self, north, open, trans)
    }
    fn square_add_thumpable(&mut self, north: bool, open: bool, is_door: bool, door_trans: bool) -> R {
        LightingSession::square_add_thumpable(self, north, open, is_door, door_trans)
    }
    fn square_add_window(&mut self, north: bool, open: bool, opaque: bool) -> R {
        LightingSession::square_add_window(self, north, open, opaque)
    }
    fn square_end_update(&mut self) -> R {
        LightingSession::square_end_update(self)
    }
    fn add_light(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, r: f32, g: f32, b: f32, building: i32, active: bool) -> R {
        LightingSession::add_light(self, id, x, y, z, radius, r, g, b, building, active)
    }
    fn add_temp_light(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, r: f32, g: f32, b: f32, life: i32) -> R {
        LightingSession::add_temp_light(self, id, x, y, z, radius, r, g, b, life);
        Ok(())
    }
    fn remove_light(&mut self, id: i32) -> R {
        LightingSession::remove_light(self, id)
    }
    fn set_light_active(&mut self, id: i32, active: bool) -> R {
        LightingSession::set_light_active(self, id, active)
    }
    fn set_light_color(&mut self, id: i32, r: f32, g: f32, b: f32) -> R {
        LightingSession::set_light_color(self, id, r, g, b)
    }
    fn add_room_light(&mut self, id: i32, b: i64, r: i64, x: i32, y: i32, z: i32, w: i32, h: i32, a: bool) -> R {
        LightingSession::add_room_light(self, id, b, r, x, y, z, w, h, a)
    }
    fn remove_room_light(&mut self, id: i32) -> R {
        LightingSession::remove_room_light(self, id)
    }
    fn set_room_light_active(&mut self, id: i32, active: bool) -> R {
        LightingSession::set_room_light_active(self, id, active)
    }
    fn update_torch(&mut self, id: i32, x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, ax: f32, ay: f32, d: f32, s: f32, c: bool, dot: f32, f: i32) -> R {
        LightingSession::update_torch(self, id, x, y, z, r, g, b, ax, ay, d, s, c, dot, f);
        Ok(())
    }
    fn remove_torch(&mut self, id: i32) -> R {
        LightingSession::remove_torch(self, id);
        Ok(())
    }
    fn get_square_dirty(&mut self, p: i32, x: i32, y: i32, z: i32) -> R<bool> {
        LightingSession::get_square_dirty(self, p, x, y, z)
    }
    fn get_square_lighting(&mut self, p: i32, x: i32, y: i32, z: i32, out: &mut [i32; 49]) -> R<bool> {
        LightingSession::get_square_lighting_into(self, p, x, y, z, out)
    }
    fn get_visible_room_count(&mut self, p: i32) -> R<i32> {
        LightingSession::get_visible_room_count(self, p)
    }
    fn get_visible_rooms(&mut self, p: i32, out: &mut [i64]) -> R<i32> {
        LightingSession::get_visible_rooms(self, p, out)
    }
}
