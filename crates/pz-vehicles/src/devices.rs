//! The vehicle part side of a radio's [`DeviceEnv`]: a vehicle part
//! parent, powered by the vehicle battery while installed, at the
//! vehicle's position.

use pz_inventory::device_sim::{
    DeviceAudio, DeviceEnv, DeviceEvent, IsoObjectPos, MainsPower, MediaLine, NetMode, ParentKind,
};

/// A world [`DeviceEnv`] with the answers of `VehiclePart`.
pub struct PartDeviceEnv<'a> {
    pub inner: &'a mut dyn DeviceEnv,
    /// `isInventoryItemUninstalled()`.
    pub uninstalled: bool,
    /// `hasDevicePower()`: `vehicle.getBatteryCharge() > 0`.
    pub has_power: bool,
    /// The vehicle's `getX/Y/Z()`.
    pub pos: [f32; 3],
}

impl DeviceEnv for PartDeviceEnv<'_> {
    fn net(&self) -> NetMode {
        self.inner.net()
    }
    fn minutes_stamp(&self) -> i64 {
        self.inner.minutes_stamp()
    }
    fn multiplier(&self) -> f32 {
        self.inner.multiplier()
    }
    fn parent(&self) -> ParentKind {
        ParentKind::Vehicle
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        self.inner.next_int(bound)
    }
    fn next_float(&mut self) -> f32 {
        self.inner.next_float()
    }
    fn mains_power(&self) -> MainsPower {
        MainsPower::Vehicle {
            uninstalled: self.uninstalled,
            has_power: self.has_power,
        }
    }
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.inner.channel_list(category)
    }
    fn media_exists(&self, index: i16) -> bool {
        self.inner.media_exists(index)
    }
    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine> {
        self.inner.media_line(index, line)
    }
    /// `getIsoObject()` is the vehicle.
    fn iso_object(&self) -> Option<IsoObjectPos> {
        Some(IsoObjectPos {
            x: self.pos[0],
            y: self.pos[1],
            z: self.pos[2],
            character: false,
        })
    }
    fn closest_listener_distance_sq(&self, x: f32, y: f32, z: f32) -> f32 {
        self.inner.closest_listener_distance_sq(x, y, z)
    }
    fn parent_pos(&self) -> [f32; 3] {
        self.pos
    }
    fn has_sound_source(&self) -> bool {
        true
    }
    fn in_room(&self) -> bool {
        false
    }
    fn item_player_deaf(&self) -> Option<bool> {
        None
    }
    fn event(&mut self, e: DeviceEvent) {
        self.inner.event(e)
    }
    fn audio(&mut self) -> Option<&mut dyn DeviceAudio> {
        self.inner.audio()
    }
}
