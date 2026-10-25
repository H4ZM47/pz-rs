use super::*;

/// `java.util.Random`.
struct JRandom(i64);

impl JRandom {
    fn new(seed: i64) -> Self {
        JRandom((seed ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound.count_ones() == 1 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits - val + (bound - 1) >= 0 {
                return val;
            }
        }
    }
    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }
}

struct Env {
    net: NetMode,
    stamp: i64,
    mult: f32,
    parent: ParentKind,
    rng: JRandom,
    draws: Vec<String>,
    mains: MainsPower,
    lists: Vec<(String, Vec<(i32, String)>)>,
    media: Vec<Vec<MediaLine>>,
    sound_source: bool,
    iso: Option<IsoObjectPos>,
    listener_dsq: f32,
    deaf: Option<bool>,
    events: Vec<DeviceEvent>,
    audio: Option<Audio>,
}

#[derive(Default)]
struct Audio {
    next_id: i64,
    emitters: u64,
    log: Vec<String>,
    playing: Vec<(EmitterId, String, i64)>,
}

impl DeviceAudio for Audio {
    fn get_free_emitter(&mut self, x: f32, y: f32, z: i32) -> EmitterId {
        self.emitters += 1;
        self.log.push(format!("new {x} {y} {z}"));
        self.emitters
    }
    fn return_emitter(&mut self, e: EmitterId) {
        self.log.push(format!("return {e}"));
        self.playing.retain(|p| p.0 != e);
    }
    fn set_pos(&mut self, _e: EmitterId, _x: f32, _y: f32, _z: i32) {}
    fn play_sound(&mut self, e: EmitterId, name: &str) -> i64 {
        self.play_sound_impl(e, name)
    }
    fn play_sound_impl(&mut self, e: EmitterId, name: &str) -> i64 {
        self.next_id += 1;
        self.log.push(format!("play {name}"));
        self.playing.push((e, name.to_string(), self.next_id));
        self.next_id
    }
    fn is_using_parameter(&mut self, _e: EmitterId, _s: i64, _n: &str) -> bool {
        false
    }
    fn set_volume(&mut self, _e: EmitterId, _s: i64, _v: f32) {}
    fn stop_sound(&mut self, _e: EmitterId, s: i64) {
        self.log.push(format!("stop {s}"));
        self.playing.retain(|p| p.2 != s);
    }
    fn stop_or_trigger_sound(&mut self, e: EmitterId, s: i64) {
        self.stop_sound(e, s);
    }
    fn stop_or_trigger_sound_by_name(&mut self, _e: EmitterId, n: &str) {
        self.playing.retain(|p| p.1 != n);
    }
    fn is_playing(&mut self, e: EmitterId, name: &str) -> bool {
        self.playing.iter().any(|p| p.0 == e && p.1 == name)
    }
    fn tick(&mut self, _e: EmitterId) {}
}

impl Env {
    fn new(parent: ParentKind, seed: i64) -> Self {
        Env {
            net: NetMode::SinglePlayer,
            stamp: 1000,
            mult: 1.0,
            parent,
            rng: JRandom::new(seed),
            draws: Vec::new(),
            mains: MainsPower::Square(Some(SquarePower::default())),
            lists: Vec::new(),
            media: Vec::new(),
            sound_source: true,
            iso: None,
            listener_dsq: f32::MAX,
            deaf: Some(false),
            events: Vec::new(),
            audio: None,
        }
    }
}

impl DeviceEnv for Env {
    fn net(&self) -> NetMode {
        self.net
    }
    fn minutes_stamp(&self) -> i64 {
        self.stamp
    }
    fn multiplier(&self) -> f32 {
        self.mult
    }
    fn parent(&self) -> ParentKind {
        self.parent
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        self.draws.push(format!("i{bound}"));
        self.rng.next_int(bound)
    }
    fn next_float(&mut self) -> f32 {
        self.draws.push("f".into());
        self.rng.next_float()
    }
    fn mains_power(&self) -> MainsPower {
        self.mains
    }
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.lists
            .iter()
            .find(|(c, _)| c == category)
            .map(|(_, l)| l.clone())
    }
    fn media_exists(&self, index: i16) -> bool {
        usize::try_from(index).is_ok_and(|i| i < self.media.len())
    }
    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine> {
        self.media
            .get(usize::try_from(index).ok()?)?
            .get(usize::try_from(line).ok()?)
            .cloned()
    }
    fn iso_object(&self) -> Option<IsoObjectPos> {
        self.iso
    }
    fn closest_listener_distance_sq(&self, _x: f32, _y: f32, _z: f32) -> f32 {
        self.listener_dsq
    }
    fn parent_pos(&self) -> [f32; 3] {
        [10.0, 20.0, 0.0]
    }
    fn has_sound_source(&self) -> bool {
        self.sound_source
    }
    fn item_player_deaf(&self) -> Option<bool> {
        self.deaf
    }
    fn event(&mut self, e: DeviceEvent) {
        self.events.push(e);
    }
    fn audio(&mut self) -> Option<&mut dyn DeviceAudio> {
        self.audio.as_mut().map(|a| a as &mut dyn DeviceAudio)
    }
}

fn radio() -> DeviceData {
    DeviceData {
        is_turned_on: true,
        channel: 93200,
        use_delta: 0.01,
        ..Default::default()
    }
}

#[test]
fn power_drains_per_minute_and_turns_off() {
    let mut env = Env::new(ParentKind::Iso, 1);
    let mut d = radio();
    d.power_delta = 0.05;
    d.update(&mut env, true, false);
    assert_eq!(d.last_minute_stamp, 1000);
    assert_eq!(d.power_delta, 0.05);
    env.stamp = 1003;
    d.update(&mut env, true, false);
    assert!((d.power_delta - 0.02).abs() < 1e-6);
    assert_eq!(d.runtime.listen_cnt, 3);
    env.stamp = 1010;
    d.update(&mut env, true, false);
    assert_eq!(d.power_delta, 0.0);
    assert!(!d.is_turned_on);
    assert_eq!(d.last_minute_stamp, -1);
}

#[test]
fn listening_triggers_every_five_minutes_in_range() {
    let mut env = Env::new(ParentKind::Iso, 1);
    let mut d = radio();
    d.is_battery_powered = false;
    env.mains = MainsPower::Square(Some(SquarePower {
        have_electricity: true,
        ..Default::default()
    }));
    d.update(&mut env, true, true);
    for m in 1..=10 {
        env.stamp = 1000 + m;
        d.update(&mut env, true, true);
    }
    let listens = env
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                DeviceEvent::PlayerListensChannel {
                    channel: 93200,
                    is_tv: false
                }
            )
        })
        .count();
    assert_eq!(listens, 2);
}

#[test]
fn mains_power_rules() {
    let env_with = |p: MainsPower| {
        let mut e = Env::new(ParentKind::Iso, 1);
        e.mains = p;
        e
    };
    let d = DeviceData {
        is_battery_powered: false,
        ..Default::default()
    };
    let sq = |g, e, r| {
        MainsPower::Square(Some(SquarePower {
            grid_power: g,
            have_electricity: e,
            has_room: r,
        }))
    };
    assert!(d.can_be_powered_here(&env_with(sq(false, true, false))));
    assert!(!d.can_be_powered_here(&env_with(sq(true, false, false))));
    assert!(d.can_be_powered_here(&env_with(sq(true, false, true))));
    assert!(!d.can_be_powered_here(&env_with(sq(false, false, true))));
    assert!(!d.can_be_powered_here(&env_with(MainsPower::Square(None))));
    assert!(!d.can_be_powered_here(&env_with(MainsPower::Vehicle {
        uninstalled: true,
        has_power: true
    })));
    assert!(d.can_be_powered_here(&env_with(MainsPower::Vehicle {
        uninstalled: false,
        has_power: true
    })));
}

#[test]
fn static_sounds_draw_bzzt_then_next_delay() {
    let mut env = Env::new(ParentKind::Iso, 42);
    let mut d = radio();
    d.runtime.next_static_sound = 0.5;
    d.update_simple(&mut env);
    // getRandomBzztFzzt, then setNextStaticSound; the sound counter is
    // already out, so the world sound follows in the same update.
    assert_eq!(env.draws, vec!["i4", "f", "i300"]);
    let mut check = JRandom::new(42);
    let s = STATIC_SOUNDS[check.next_int(4) as usize];
    let next = 250.0 + check.next_float() * 1250.0;
    let wait = 300 + check.next_int(300);
    assert_eq!(d.runtime.next_static_sound, next);
    assert_eq!(d.runtime.sound_counter, wait as f32);
    assert!(!d.runtime.do_trigger_world_sound);
    assert_eq!(
        env.events,
        vec![
            DeviceEvent::ChatLine {
                target: ChatTarget::Device,
                text: s.to_string(),
                rgb: [1.0; 3],
                range: d.device_volume_range(),
            },
            DeviceEvent::WorldSound {
                repeating: true,
                radius: 5 + (15.0f32 * 0.75) as i32,
                volume: 100,
                stress: true,
            }
        ]
    );
    assert!(d.runtime.sound_counter >= 300.0);
}

#[test]
fn received_signal_sets_counters_and_world_sound() {
    let mut env = Env::new(ParentKind::Item, 3);
    env.sound_source = true;
    let mut d = radio();
    d.device_volume = 0.4;
    d.add_device_text(&mut env, "Hello", [1.0, 0.5, 0.0], None, Some("RAD"), 7);
    assert_eq!(d.runtime.last_recorded_distance, 7);
    assert_eq!(d.runtime.signal_counter, 300.0);
    assert_eq!(d.runtime.sound_counter, 120.0);
    assert!(d.is_receiving_signal());
    assert_eq!(env.draws, vec!["f"]);
    assert_eq!(
        env.events,
        vec![
            DeviceEvent::WorldSound {
                repeating: false,
                radius: 5 + (15.0f32 * 0.4) as i32,
                volume: 40,
                stress: false,
            },
            DeviceEvent::SayRadio {
                text: "Hello".into(),
                rgb: [1.0, 0.5, 0.0],
                range: 5 + (15.0f32 * 0.4) as i32,
                channel: 93200,
            },
            DeviceEvent::OnDeviceText {
                guid: None,
                codes: "RAD".into(),
                pos: None,
                text: "Hello".into(),
            },
        ]
    );
    // Headphones on an item: no world sound.
    let mut env = Env::new(ParentKind::Item, 3);
    let mut d = radio();
    d.headphone_type = 0;
    d.do_receive_signal(&mut env, 2);
    assert!(env.events.is_empty());
    assert_eq!(d.runtime.sound_counter, 0.0);
}

#[test]
fn player_variant_routes_chat() {
    let mut env = Env::new(ParentKind::Item, 3);
    let mut d = radio();
    let p = Listener {
        local: true,
        deaf: false,
        equipped_parent: false,
    };
    d.add_device_text_for_player(&mut env, Some(p), "<fzzt>", [1.0; 3], None, None, -1);
    assert_eq!(
        env.events,
        vec![DeviceEvent::ShowStaticRadioSound("<fzzt>".into())]
    );
    assert!(env.draws.is_empty());
    env.events.clear();
    let p = Listener {
        equipped_parent: true,
        ..p
    };
    d.add_device_text_for_player(&mut env, Some(p), "Hi", [1.0; 3], Some("g"), Some("c"), 3);
    assert!(matches!(
        env.events[..],
        [
            DeviceEvent::WorldSound { .. },
            DeviceEvent::ChatLine {
                target: ChatTarget::Player,
                ..
            },
            DeviceEvent::ShowRadioMessage { channel: 93200, .. },
            DeviceEvent::OnDeviceText { pos: None, .. },
        ]
    ));
}

#[test]
fn media_plays_lines_then_stops() {
    let mut env = Env::new(ParentKind::Iso, 9);
    env.media.push(vec![
        MediaLine {
            text: "Short".into(),
            rgb: [1.0; 3],
            guid: Some("G1".into()),
            codes: None,
        },
        MediaLine {
            text: "x".repeat(100),
            rgb: [0.5; 3],
            guid: Some("G2".into()),
            codes: Some("BOR-1".into()),
        },
    ]);
    let mut d = DeviceData {
        is_turned_on: true,
        is_battery_powered: false,
        is_television: true,
        media_type: 1,
        media_index: 0,
        ..Default::default()
    };
    env.mains = MainsPower::Square(Some(SquarePower {
        have_electricity: true,
        ..Default::default()
    }));
    d.start_play_media(&mut env);
    assert!(d.is_playing_media());
    assert_eq!(d.runtime.line_counter, 150.0);
    // televisionMediaSwitch: two bzzt draws, the second shown, grey; the
    // static line does not count as a signal.
    assert_eq!(env.draws, vec!["i4", "i4"]);
    assert!(matches!(
        &env.events[..],
        [DeviceEvent::ChatLine {
            rgb: [0.5, 0.5, 0.5],
            ..
        }]
    ));
    env.events.clear();
    env.draws.clear();
    env.mult = 200.0;
    d.runtime.next_static_sound = 1e9;
    d.update_media_playing(&mut env);
    assert_eq!(d.runtime.media_line_index, 1);
    assert_eq!(d.runtime.line_counter, 90.0);
    // The first line is a signal (doReceiveSignal's next-static draw).
    assert_eq!(env.draws, vec!["f"]);
    d.update_media_playing(&mut env);
    assert_eq!(d.runtime.line_counter, 300.0);
    assert!(env.events.iter().any(|e| matches!(
        e,
        DeviceEvent::OnDeviceText {
            pos: Some([10.0, 20.0, 0.0]),
            ..
        }
    )));
    env.mult = 1000.0;
    d.update_media_playing(&mut env);
    // No third line: StopPlayMedia, then the stop counter runs out.
    assert!(d.runtime.is_stopping_media);
    assert!(d.is_playing_media());
    d.update_media_playing(&mut env);
    assert!(!d.is_playing_media());
    assert!(!d.runtime.is_stopping_media);
}

#[test]
fn channel_change_zaps_and_restarts_loop() {
    let mut env = Env::new(ParentKind::Vehicle, 5);
    env.iso = Some(IsoObjectPos {
        x: 3.0,
        y: 4.0,
        z: 0.0,
        character: false,
    });
    env.listener_dsq = 10.0;
    env.audio = Some(Audio::default());
    env.mains = MainsPower::Vehicle {
        uninstalled: false,
        has_power: true,
    };
    let mut d = radio();
    d.is_battery_powered = false;
    d.update(&mut env, true, false);
    let a = env.audio.as_ref().unwrap();
    // Vehicles have no half-tile offset.
    assert_eq!(a.log, vec!["new 3 4 0", "play VehicleRadioStatic"]);
    assert_eq!(d.runtime.radio_loop_sound, 1);
    d.set_channel(&mut env, 98000, true);
    let a = env.audio.as_ref().unwrap();
    assert_eq!(&a.log[2..], ["play VehicleRadioZap", "stop 1"]);
    assert_eq!(d.runtime.radio_loop_sound, 0);
    assert!(env.events.contains(&DeviceEvent::PlayerListensChannel {
        channel: 98000,
        is_tv: false
    }));
    // Out of range: ignored.
    d.set_channel(&mut env, 100, true);
    assert_eq!(d.channel, 98000);
    // Far from the listener: the emitter is given back.
    env.listener_dsq = 300.0;
    d.update(&mut env, true, false);
    assert_eq!(d.runtime.emitter, None);
    assert_eq!(env.audio.as_ref().unwrap().log.last().unwrap(), "return 1");
}

#[test]
fn emergency_broadcast_uses_its_own_talk_sound() {
    let mut env = Env::new(ParentKind::Iso, 5);
    env.lists
        .push(("Emergency".into(), vec![(93200, "Emergency".into())]));
    env.iso = Some(IsoObjectPos {
        x: 3.0,
        y: 4.0,
        z: 1.0,
        character: false,
    });
    env.listener_dsq = 0.0;
    env.audio = Some(Audio::default());
    let mut d = radio();
    d.runtime.signal_counter = 100.0;
    d.runtime.next_static_sound = 1e9;
    assert!(d.is_emergency_broadcast(&env));
    d.update_emitter(&mut env);
    let a = env.audio.as_ref().unwrap();
    assert_eq!(a.log, vec!["new 3.5 4.5 1", "play BroadcastEmergency"]);
}

#[test]
fn tv_screen_and_light() {
    let mut env = Env::new(ParentKind::Iso, 11);
    env.mains = MainsPower::Square(Some(SquarePower {
        have_electricity: true,
        ..Default::default()
    }));
    let mut d = DeviceData {
        is_turned_on: true,
        is_battery_powered: false,
        is_television: true,
        ..Default::default()
    };
    d.runtime.next_static_sound = 1e9;
    let tv = TvUpdate {
        screen_sprites: 4,
        sprite_changed: false,
    };
    d.update_wave_signal(&mut env, Some((0.0, 0.0)), Some(tv), false);
    assert!(d.runtime.wave.has_player_in_range);
    assert_eq!(d.runtime.wave.current_screen, TvScreen::Test);
    assert_eq!(d.runtime.wave.overlay, Some(0));
    // Light: added, then the first update draws interval, intensity, gb.
    assert_eq!(env.draws, vec!["f", "f", "f"]);
    assert!(matches!(env.events[0], DeviceEvent::AddLight { radius: 4 }));
    assert!(env
        .events
        .iter()
        .any(|e| matches!(e, DeviceEvent::TvOverlay(Some(0)))));
    d.runtime.signal_counter = 300.0;
    d.update_wave_signal(&mut env, None, Some(tv), false);
    assert_eq!(d.runtime.wave.current_screen, TvScreen::Alternate);
    assert_eq!(d.runtime.wave.overlay, Some(1));
    assert!(!d.runtime.wave.has_player_in_range);
    d.set_is_turned_on(&mut env, false);
    env.events.clear();
    d.update_wave_signal(&mut env, None, Some(tv), false);
    assert_eq!(d.runtime.wave.overlay, None);
    assert!(env.events.contains(&DeviceEvent::RemoveLight));
    assert!(env.events.contains(&DeviceEvent::TvOverlay(None)));
}

#[test]
fn vehicle_part_any_living_player_is_in_range() {
    let mut env = Env::new(ParentKind::Vehicle, 1);
    env.mains = MainsPower::Vehicle {
        uninstalled: false,
        has_power: true,
    };
    let mut d = radio();
    d.is_battery_powered = false;
    d.runtime.next_static_sound = 1e9;
    let far = [None, Some((5000.0, -5000.0))];
    assert!(update_vehicle_part_device(
        &mut d, &mut env, false, false, &far
    ));
    assert!(!update_vehicle_part_device(
        &mut d,
        &mut env,
        false,
        false,
        &[None]
    ));
    // Uninstalled: turned off.
    assert!(!update_vehicle_part_device(
        &mut d, &mut env, true, false, &far
    ));
    assert!(!d.is_turned_on);
    assert!(env.events.contains(&DeviceEvent::UpdateGenerator));
}

#[test]
fn state_round_trip_and_clone() {
    let mut d = radio();
    d.presets.add_preset("A", 1);
    d.presets.add_preset("B", 2);
    d.device_volume = 0.25;
    let mut w = crate::io::Writer::new();
    d.save_state(&mut w);
    let bytes = w.buf;
    let mut e = DeviceData::default();
    e.presets.add_preset("X", 9);
    e.presets.add_preset("Y", 8);
    e.presets.add_preset("Z", 7);
    e.load_state(&mut crate::io::Reader::new(&bytes)).unwrap();
    assert_eq!(e.presets.presets, d.presets.presets);
    assert!(e.is_turned_on);
    assert_eq!(e.channel, 93200);
    assert_eq!(e.device_volume, 0.25);
    d.runtime.emitter = Some(3);
    d.runtime.radio_loop_sound = 5;
    let c = d.get_clone();
    assert_eq!(c.runtime.emitter, None);
    assert_eq!(c.runtime.radio_loop_sound, 5);
    assert_eq!(c.presets, d.presets);
}

#[test]
fn battery_headphones_media_items() {
    let mut env = Env::new(ParentKind::Item, 1);
    let mut d = radio();
    d.power_delta = 0.7;
    assert_eq!(d.take_battery(&mut env), Some(0.7));
    assert!(!d.has_battery);
    assert_eq!(d.power_delta, 0.0);
    assert!(!d.add_battery(&mut env, "Base.Nope", 0.5));
    assert!(d.add_battery(&mut env, "Base.Battery", 0.5));
    assert_eq!(d.power_delta, 0.5);
    assert!(d.add_headphones(&mut env, "Base.Earbuds", true));
    assert_eq!(d.headphone_type, 1);
    assert!(!d.add_headphones(&mut env, "Base.Headphones", true));
    assert_eq!(d.take_headphones(&mut env), Some("Base.Earbuds"));
    d.media_type = 0;
    assert!(d.add_media_item(&mut env, "Base.Disc", true, 0, 12));
    assert_eq!(d.media_index, 12);
    assert_eq!(
        d.remove_media_item(&mut env),
        Some((Some("Base.Disc".into()), 12))
    );
    assert_eq!(d.media_index, -1);
}

#[test]
fn transmits_only_in_their_mode() {
    let mut env = Env::new(ParentKind::Iso, 1);
    env.net = NetMode::Client;
    let mut d = radio();
    d.set_device_volume_env(&mut env, 2.0);
    assert_eq!(d.device_volume, 1.0);
    assert_eq!(
        env.events,
        vec![DeviceEvent::Transmit {
            state: DeviceState::Volume,
            server: false
        }]
    );
    env.events.clear();
    env.net = NetMode::Server;
    d.set_device_volume_env(&mut env, 0.5);
    assert!(env.events.is_empty());
    d.take_battery(&mut env);
    assert_eq!(
        env.events,
        vec![DeviceEvent::Transmit {
            state: DeviceState::Battery,
            server: true
        }]
    );
}

/// Replays the local Java oracle runs (`DevOracle`: a `DeviceData` with an
/// `IsoWaveSignal`-like parent driven tick by tick) and compares the
/// output. Run with `PZ_DEVICE_ORACLE=<dir with o*.txt>`.
#[test]
#[ignore]
fn java_oracle_replay() {
    let Ok(dir) = std::env::var("PZ_DEVICE_ORACLE") else {
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('o') && n.ends_with(".txt"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for f in files {
        let java = std::fs::read_to_string(&f).unwrap();
        let rust = replay(&java);
        if rust != java {
            let out = f.with_extension("rust");
            std::fs::write(&out, &rust).unwrap();
            let (j, r): (Vec<_>, Vec<_>) = (java.lines().collect(), rust.lines().collect());
            let i = j
                .iter()
                .zip(&r)
                .position(|(a, b)| a != b)
                .unwrap_or(j.len().min(r.len()));
            panic!(
                "{}: differs at line {}:\n java: {:?}\n rust: {:?}",
                f.display(),
                i + 1,
                j.get(i),
                r.get(i)
            );
        }
    }
}

fn jf(f: f32) -> String {
    // Java's Float.toString for the colours used here.
    format!("{f:?}")
}

fn hexf(f: f32) -> String {
    format!("f{:x}", f.to_bits())
}

fn flush_adds(env: &mut Env, out: &mut String) {
    let mut lines: Vec<String> = Vec::new();
    for e in env.events.drain(..) {
        match e {
            DeviceEvent::ChatLine { text, rgb, .. } => lines.push(format!(
                "add {text}|{}|{}|{}|null",
                jf(rgb[0]),
                jf(rgb[1]),
                jf(rgb[2])
            )),
            DeviceEvent::OnDeviceText { codes, .. } => {
                let l = lines.last_mut().unwrap();
                *l = format!("{}{codes}", l.strip_suffix("null").unwrap());
            }
            _ => {}
        }
    }
    for l in lines {
        out.push_str(&l);
        out.push('\n');
    }
}

fn replay(java: &str) -> String {
    let mut out = String::new();
    let mut lines = java.lines();
    let seed: i64 = lines
        .next()
        .unwrap()
        .strip_prefix("seed ")
        .unwrap()
        .parse()
        .unwrap();
    out.push_str(&format!("seed {seed}\n"));
    let cfg = lines.next().unwrap();
    out.push_str(cfg);
    out.push('\n');
    let c: Vec<i64> = cfg[4..].split(' ').map(|x| x.parse().unwrap()).collect();
    let mut env = Env::new(ParentKind::Iso, seed);
    env.sound_source = false;
    let mut d = DeviceData {
        is_battery_powered: true,
        power_delta: f32::from_bits(c[0] as u32),
        use_delta: f32::from_bits(c[1] as u32),
        device_volume: f32::from_bits(c[2] as u32),
        media_type: c[3] as i8,
        is_turned_on: true,
        channel: 93200,
        ..Default::default()
    };
    let mut media = Vec::new();
    for l in java.lines().filter(|l| l.starts_with("mline ")) {
        let mut p = l[6..].splitn(2, ' ');
        let codes = p.next().unwrap();
        let text = p.next().unwrap_or("").to_string();
        media.push(MediaLine {
            text: text.clone(),
            rgb: [0.5, 0.25, 1.0],
            guid: Some(text),
            codes: (codes != "-").then(|| codes.to_string()),
        });
        out.push_str(l);
        out.push('\n');
    }
    env.media.push(media);
    for l in java.lines().filter(|l| l.starts_with("tick ")) {
        let p: Vec<&str> = l.split(' ').collect();
        env.stamp = p[2].parse().unwrap();
        env.mult = f32::from_bits(p[3].parse::<i64>().unwrap() as u32);
        match p[4] {
            "recv" => d.do_receive_signal(&mut env, p[5].parse().unwrap()),
            "text" => d.add_device_text(
                &mut env,
                "Hello there",
                [1.0; 3],
                None,
                Some("X"),
                p[5].parse().unwrap(),
            ),
            "media" => {
                d.media_index = 0;
                d.runtime.playing_media = Some(0);
                d.runtime.is_playing_media = true;
                d.runtime.media_line_index = 0;
                d.pre_playing_media(&mut env);
            }
            "mpsig" => d.do_receive_mp_signal(12.5),
            _ => {}
        }
        flush_adds(&mut env, &mut out);
        out.push_str(l);
        out.push('\n');
        d.update(&mut env, true, false);
        flush_adds(&mut env, &mut out);
        let r = &d.runtime;
        out.push_str(&format!(
            "state {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}\n",
            d.is_turned_on,
            hexf(d.power_delta),
            d.last_minute_stamp,
            r.listen_cnt,
            hexf(r.next_static_sound),
            hexf(r.signal_counter),
            hexf(r.sound_counter),
            hexf(r.voip_counter),
            r.last_recorded_distance,
            r.do_trigger_world_sound,
            r.is_playing_media,
            r.is_stopping_media,
            hexf(r.line_counter),
            hexf(r.stop_media_counter),
            r.media_line_index
        ));
    }
    out
}
