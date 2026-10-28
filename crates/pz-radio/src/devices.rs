//! The glue between `ZomboidRadio` and the device simulation of
//! `pz_inventory::device_sim`: what the radio reads from a device
//! ([`device_view`]), a [`DeviceEnv`] that answers the radio's questions
//! (channel lists, recorded media) from a [`ZomboidRadio`]
//! ([`WithRadio`]), and the delivery of a line the radio distributes
//! ([`deliver_line`]).

use pz_inventory::device::DeviceData;
use pz_inventory::device_sim::{
    DeviceAudio, DeviceEnv, DeviceEvent, IsoObjectPos, Listener, MainsPower, MediaLine, NetMode,
    ParentKind,
};

use crate::radio::{DeviceView, RadioWorld, ZomboidRadio};
use crate::{RadioText, RecordedMedia};

/// The [`DeviceView`] of a device at `pos`; `has_player_in_range` is the
/// parent's `HasPlayerInRange()` (false for radio items).
pub fn device_view(d: &DeviceData, pos: [f32; 3], has_player_in_range: bool) -> DeviceView {
    DeviceView {
        turned_on: d.is_turned_on,
        two_way: d.two_way,
        portable: d.is_portable,
        television: d.is_television,
        mic_muted: d.mic_is_muted,
        mic_range: d.mic_range,
        transmit_range: d.transmit_range,
        channel: d.channel,
        volume: d.device_volume,
        playing_media: d.is_playing_media(),
        no_transmit: d.no_transmit,
        x: pos[0],
        y: pos[1],
        z: pos[2],
        has_player_in_range,
    }
}

/// A line of `getMediaDataFromIndex(index)`.
pub fn media_line(
    media: &RecordedMedia,
    text: &dyn RadioText,
    index: i16,
    line: i32,
) -> Option<MediaLine> {
    let l = media.media_data_from_index(index)?.line(line)?;
    Some(MediaLine {
        text: l.translated_text(text),
        rgb: [l.r(), l.g(), l.b()],
        guid: Some(l.text_guid().to_string()),
        codes: l.codes().map(str::to_string),
    })
}

/// `WaveSignalDevice.AddDeviceText` for a line the radio sends to a
/// device ([`RadioWorld::add_device_text`]): with a listener the
/// interface's default method, without one the parent's own.
#[allow(clippy::too_many_arguments)]
pub fn deliver_line(
    d: &mut DeviceData,
    env: &mut dyn DeviceEnv,
    listener: Option<Option<Listener>>,
    text: &str,
    rgb: [f32; 3],
    guid: Option<&str>,
    codes: Option<&str>,
    distance: i32,
) {
    match listener {
        Some(p) => d.add_device_text_for_player(env, p, text, rgb, guid, codes, distance),
        None => d.add_device_text(env, text, rgb, guid, codes, distance),
    }
}

/// A [`DeviceEnv`] that takes the channel lists and recorded media from a
/// [`ZomboidRadio`] and everything else from `base`. The radio is only
/// read; `PlayerListensChannel` events are kept in `listens` for
/// [`WithRadio::apply_listens`] once the radio can be borrowed mutably.
pub struct WithRadio<'a> {
    pub base: &'a mut dyn DeviceEnv,
    pub radio: &'a ZomboidRadio,
    pub text: &'a dyn RadioText,
    /// `(channel, isTv)` of each `PlayerListensChannel(channel, true, isTv)`.
    pub listens: Vec<(i32, bool)>,
}

impl<'a> WithRadio<'a> {
    pub fn new(
        base: &'a mut dyn DeviceEnv,
        radio: &'a ZomboidRadio,
        text: &'a dyn RadioText,
    ) -> Self {
        WithRadio {
            base,
            radio,
            text,
            listens: Vec::new(),
        }
    }

    /// Sends the collected `PlayerListensChannel` calls to the radio.
    pub fn apply_listens(
        listens: &[(i32, bool)],
        radio: &mut ZomboidRadio,
        world: &mut dyn RadioWorld,
    ) {
        for &(channel, is_tv) in listens {
            radio.player_listens_channel(channel, true, is_tv, world);
        }
    }
}

impl DeviceEnv for WithRadio<'_> {
    fn net(&self) -> NetMode {
        self.base.net()
    }
    fn minutes_stamp(&self) -> i64 {
        self.base.minutes_stamp()
    }
    fn multiplier(&self) -> f32 {
        self.base.multiplier()
    }
    fn parent(&self) -> ParentKind {
        self.base.parent()
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        self.base.next_int(bound)
    }
    fn next_float(&mut self) -> f32 {
        self.base.next_float()
    }
    fn mains_power(&self) -> MainsPower {
        self.base.mains_power()
    }
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.radio.channel_list(category)
    }
    fn media_exists(&self, index: i16) -> bool {
        self.radio
            .recorded_media()
            .media_data_from_index(index)
            .is_some()
    }
    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine> {
        media_line(self.radio.recorded_media(), self.text, index, line)
    }
    fn iso_object(&self) -> Option<IsoObjectPos> {
        self.base.iso_object()
    }
    fn closest_listener_distance_sq(&self, x: f32, y: f32, z: f32) -> f32 {
        self.base.closest_listener_distance_sq(x, y, z)
    }
    fn parent_pos(&self) -> [f32; 3] {
        self.base.parent_pos()
    }
    fn has_sound_source(&self) -> bool {
        self.base.has_sound_source()
    }
    fn in_room(&self) -> bool {
        self.base.in_room()
    }
    fn item_player_deaf(&self) -> Option<bool> {
        self.base.item_player_deaf()
    }
    fn event(&mut self, e: DeviceEvent) {
        if let DeviceEvent::PlayerListensChannel { channel, is_tv } = e {
            self.listens.push((channel, is_tv));
        }
        self.base.event(e)
    }
    fn audio(&mut self) -> Option<&mut dyn DeviceAudio> {
        self.base.audio()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_inventory::device_sim::DeviceEvent;

    struct Base {
        events: Vec<DeviceEvent>,
        draws: i32,
    }

    impl DeviceEnv for Base {
        fn minutes_stamp(&self) -> i64 {
            0
        }
        fn multiplier(&self) -> f32 {
            100.0
        }
        fn parent(&self) -> ParentKind {
            ParentKind::Iso
        }
        fn next_int(&mut self, _bound: i32) -> i32 {
            self.draws += 1;
            1
        }
        fn next_float(&mut self) -> f32 {
            self.draws += 1;
            0.5
        }
        fn mains_power(&self) -> MainsPower {
            MainsPower::Square(None)
        }
        fn channel_list(&self, _c: &str) -> Option<Vec<(i32, String)>> {
            panic!("asked the base for channels")
        }
        fn media_exists(&self, _i: i16) -> bool {
            panic!("asked the base for media")
        }
        fn media_line(&self, _i: i16, _l: i32) -> Option<MediaLine> {
            panic!("asked the base for media")
        }
        fn iso_object(&self) -> Option<IsoObjectPos> {
            None
        }
        fn closest_listener_distance_sq(&self, _x: f32, _y: f32, _z: f32) -> f32 {
            f32::MAX
        }
        fn parent_pos(&self) -> [f32; 3] {
            [1.0, 2.0, 0.0]
        }
        fn event(&mut self, e: DeviceEvent) {
            self.events.push(e);
        }
    }

    #[test]
    fn radio_answers_channels_and_media() {
        let mut radio = ZomboidRadio::new(9);
        radio.add_channel_name("Emergency", 93200, "Emergency", false);
        let m = radio
            .recorded_media_mut()
            .register("CDs", "cd-1", "Disc", 0)
            .unwrap();
        m.add_line("RM_line1", 0.0, 0.0, 0.0, None);
        m.add_line("RM_line2", 0.2, 0.3, 0.4, Some("BOR+1"));
        let text = |k: &str| format!("<{k}>");
        let mut base = Base {
            events: Vec::new(),
            draws: 0,
        };
        let mut env = WithRadio::new(&mut base, &radio, &text);
        let mut d = DeviceData {
            is_turned_on: true,
            channel: 93200,
            media_index: 0,
            media_type: 0,
            ..Default::default()
        };
        assert!(d.is_emergency_broadcast(&env));
        d.trigger_player_listening(&mut env);
        assert_eq!(env.listens, vec![(93200, false)]);
        assert_eq!(
            env.media_line(0, 1),
            Some(MediaLine {
                text: "<RM_line2>".into(),
                rgb: [0.2, 0.3, 0.4],
                guid: Some("RM_line2".into()),
                codes: Some("BOR+1".into()),
            })
        );
        // Black lines are white.
        assert_eq!(env.media_line(0, 0).unwrap().rgb, [1.0; 3]);
        d.start_play_media(&mut env);
        assert!(d.is_playing_media());
        // 150 ticks before the first line, then 90 (the minimum).
        for _ in 0..3 {
            d.update_media_playing(&mut env);
        }
        assert_eq!(d.runtime.media_line_index, 2);
        let lines: Vec<_> = base
            .events
            .iter()
            .filter_map(|e| match e {
                DeviceEvent::ChatLine { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["<RM_line1>", "<RM_line2>"]);
        assert!(base.events.iter().any(|e| matches!(
            e,
            DeviceEvent::OnDeviceText { codes, pos: Some([1.0, 2.0, 0.0]), .. } if codes == "BOR+1"
        )));
        let view = device_view(&d, [1.0, 2.0, 0.0], true);
        assert!(view.playing_media && view.turned_on && view.has_player_in_range);
    }
}
