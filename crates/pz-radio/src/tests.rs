//! Tests on invented radio data. Expected RNG-driven values were checked
//! against the original classes with `java.util.Random` behind `Rand`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::java::string_hash;
use crate::*;

/// `java.util.Random`, the generator the tests put behind `Rand`.
pub(crate) struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    pub(crate) fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1),
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(0x5DEECE66D).wrapping_add(0xB) & ((1 << 48) - 1);
        (self.seed >> (48 - bits)) as i32
    }

    pub(crate) fn next_int(&mut self, bound: i32) -> i32 {
        if bound.isolate_lowest_one() == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
}

fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "pz-radio-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const XML: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<RadioData>
  <RootInfo>
    <FileGUID>test-file</FileGUID>
    <Version>1</Version>
  </RootInfo>
  <Adverts>
    <ScriptEntry ID="ads" name="Ads" loopmin="1" loopmax="1">
      <BroadcastEntry ID="ad1" timestamp="0" endstamp="0" advertCat="none" isSegment="true">
        <LineEntry ID="a1" r="255" g="0" b="0"/>
      </BroadcastEntry>
      <BroadcastEntry ID="ad2" timestamp="0" endstamp="0" advertCat="none" isSegment="true">
        <LineEntry ID="a2" r="0" g="255" b="0"/>
        <LineEntry ID="a3" r="0" g="255" b="0"/>
      </BroadcastEntry>
    </ScriptEntry>
  </Adverts>
  <Channels>
    <ChannelEntry ID="c1" name="Test FM" cat="Radio" freq="93200" startscript="morning">
      <ScriptEntry ID="s1" name="morning" loopmin="1" loopmax="3">
        <ExitOptions>
          <ExitOption script="evening" chance="60" delay="0"/>
          <ExitOption script="morning" chance="40" delay="1"/>
        </ExitOptions>
        <BroadcastEntry ID="b1" timestamp="60" endstamp="120" advertCat="ads" isSegment="false">
          <LineEntry ID="l1" r="255" g="255" b="255" codes="ABC-1"/>
          <LineEntry ID="l2" r="128" g="128" b="128"/>
        </BroadcastEntry>
        <BroadcastEntry ID="b2" timestamp="600" endstamp="700" advertCat="ads" isSegment="false">
          <LineEntry ID="l3" r="255" g="255" b="255"/>
        </BroadcastEntry>
        <BroadcastEntry ID="bad" timestamp="650" endstamp="660" advertCat="none" isSegment="false">
          <LineEntry ID="l4" r="255" g="255" b="255"/>
        </BroadcastEntry>
      </ScriptEntry>
      <ScriptEntry ID="s2" name="evening" loopmin="2" loopmax="2">
        <ExitOptions/>
        <BroadcastEntry ID="b3" timestamp="0" endstamp="30" advertCat="none" isSegment="false">
          <LineEntry ID="timed" r="10" g="20" b="30"/>
        </BroadcastEntry>
      </ScriptEntry>
    </ChannelEntry>
    <ChannelEntry ID="c2" name="Test TV" cat="Television" freq="201" startscript="none"/>
    <ChannelEntry ID="c3" name="Ham" cat="Amateur" freq="91200" startscript="morning">
      <ScriptEntry ID="s3" name="morning" loopmin="1" loopmax="1">
        <BroadcastEntry ID="b4" timestamp="0" endstamp="1440" advertCat="none" isSegment="false">
          <LineEntry ID="l5" r="255" g="255" b="255"/>
        </BroadcastEntry>
      </ScriptEntry>
    </ChannelEntry>
    <ChannelEntry ID="c4" name="Other FM" cat="Radio" freq="89400" startscript="morning"/>
  </Channels>
</RadioData>
"#;

fn text(key: &str) -> String {
    match key {
        "RD_timed" => "${t:2} Short one".to_string(),
        _ => format!("line {key}"),
    }
}

type Heard = (DeviceId, Option<usize>, String, [f32; 3], i32);

struct TestWorld {
    save: Option<PathBuf>,
    apo: i32,
    mode: String,
    players: Vec<PlayerView>,
    devices: Vec<(DeviceId, DeviceView)>,
    heard: Vec<Heard>,
}

impl TestWorld {
    fn new(save: Option<PathBuf>) -> Self {
        TestWorld {
            save,
            apo: 1,
            mode: "Sandbox".into(),
            players: Vec::new(),
            devices: Vec::new(),
            heard: Vec::new(),
        }
    }
}

impl RadioWorld for TestWorld {
    fn hour(&self) -> i32 {
        9
    }
    fn nights_survived(&self) -> i32 {
        3
    }
    fn multiplier(&self) -> f32 {
        0.8
    }
    fn time_since_apo(&self) -> i32 {
        self.apo
    }
    fn core_game_mode(&self) -> String {
        self.mode.clone()
    }
    fn current_save_dir(&self) -> Option<PathBuf> {
        self.save.clone()
    }
    fn num_players(&self) -> usize {
        self.players.len()
    }
    fn player(&self, index: usize) -> Option<PlayerView> {
        self.players.get(index).cloned()
    }
    fn device(&self, id: DeviceId) -> Option<DeviceView> {
        self.devices.iter().find(|(d, _)| *d == id).map(|(_, v)| *v)
    }
    fn add_device_text(
        &mut self,
        device: DeviceId,
        listener: Option<usize>,
        text: &str,
        rgb: [f32; 3],
        _guid: Option<&str>,
        _codes: Option<&str>,
        distance: i32,
    ) {
        self.heard
            .push((device, listener, text.to_string(), rgb, distance));
    }
}

fn game_dir(xml: &str) -> PathBuf {
    let d = temp_dir();
    std::fs::create_dir_all(d.join("media/radio")).unwrap();
    std::fs::write(d.join("media/radio/RadioData.xml"), xml).unwrap();
    d
}

fn source(dir: &Path) -> RadioSource {
    RadioSource {
        media_dir: dir.join("media"),
        language: "EN".into(),
        default_language: "EN".into(),
        available_languages: vec!["EN".into()],
        ..Default::default()
    }
}

fn init_radio(seed: i64, world: &mut TestWorld, dir: &Path, version: i32) -> ZomboidRadio {
    let r = RefCell::new(JavaRandom::new(seed));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    let mut radio = ZomboidRadio::new(9);
    radio
        .init(version, &source(dir), &text, world, &mut rng)
        .unwrap();
    radio
}

#[test]
fn loads_channels_scripts_and_lines() {
    let dir = game_dir(XML);
    let mut world = TestWorld::new(None);
    let radio = init_radio(7, &mut world, &dir, -1);
    let sm = radio.script_manager().unwrap();
    let freqs: Vec<i32> = sm.channels().keys().copied().collect();
    assert_eq!(freqs, vec![93200, 201, 91200, 89400]);
    let fm = &sm.channels()[&93200];
    assert_eq!(fm.name(), "Test FM");
    assert_eq!(fm.category(), ChannelCategory::Radio);
    assert!(fm.louisville_obfuscate(), "93200 is a Louisville channel");
    assert!(fm.radio_data().unwrap().vanilla);
    let morning = fm.radio_script("morning").unwrap();
    // The overlapping broadcast is refused.
    assert_eq!(morning.broadcasts().len(), 2);
    assert_eq!(morning.exit_options().len(), 2);
    let b1 = sm.broadcasts.get(morning.broadcasts()[0]);
    assert_eq!(b1.lines()[0].text(), "line RD_l1");
    assert_eq!(b1.lines()[0].effects(), "ABC-1");
    assert_eq!(b1.lines()[1].r(), 128.0 / 255.0);
    let evening = fm.radio_script("evening").unwrap();
    let timed = &sm.broadcasts.get(evening.broadcasts()[0]).lines()[0];
    assert_eq!(timed.text(), "[cdt=2.0]Short one");
    assert!(timed.is_custom_air_time());
    // An unknown start script leaves the channel without a script.
    assert!(sm.channels()[&201].current_script().is_none());
    assert!(sm.channels()[&201].is_tv());
    assert_eq!(fm.current_script_name(), Some("morning"));
}

#[test]
fn channel_lists_follow_hash_map_order() {
    use pz_inventory::device::ChannelLists;
    let dir = game_dir(XML);
    let mut world = TestWorld::new(None);
    let radio = init_radio(7, &mut world, &dir, -1);
    let radio_list = radio.channel_list("Radio").unwrap();
    // 89400 and 93200 in a 16-bucket table: (h ^ h >>> 16) & 15.
    let mut expected = vec![93200, 89400];
    expected.sort_by_key(|f: &i32| {
        let h = *f as u32;
        (h ^ (h >> 16)) & 15
    });
    assert_eq!(
        radio_list.iter().map(|(f, _)| *f).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        radio.channel_list("Television").unwrap(),
        vec![(201, "Test TV".to_string())]
    );
    assert!(radio.channel_list("Military").is_none());
    let lists: &dyn ChannelLists = &radio;
    assert_eq!(lists.channel_list("Amateur").unwrap()[0].1, "Ham");
    let cats: Vec<String> = radio
        .full_channel_list()
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    let mut by_hash = cats.clone();
    by_hash.sort_by_key(|c| {
        let h = string_hash(c) as u32;
        (h ^ (h >> 16)) & 15
    });
    assert_eq!(cats, by_hash);
}

#[test]
fn adverts_and_loops_draw_the_rng_in_order() {
    let dir = game_dir(XML);
    let mut world = TestWorld::new(None);
    let radio = init_radio(7, &mut world, &dir, -1);
    let sm = radio.script_manager().unwrap();
    let fm = &sm.channels()[&93200];
    // Replays the draws the original makes while loading: per broadcast
    // of "morning" (advert chance, side, pick), then the loop count.
    let mut r = JavaRandom::new(7);
    let mut segs = Vec::new();
    for _ in 0..2 {
        if r.next_int(101) < 75 {
            let pre = r.next_int(101) < 50;
            let pick = r.next_int(2);
            segs.push(Some((pre, pick)));
        } else {
            segs.push(None);
        }
    }
    let loops = r.next_int(2) + 1;
    assert_eq!(fm.current_script_max_loops(), loops);
    let morning = fm.radio_script("morning").unwrap();
    for (i, s) in segs.into_iter().enumerate() {
        let b = sm.broadcasts.get(morning.broadcasts()[i]);
        let got = match (b.pre_segment(), b.post_segment()) {
            (Some(p), None) => Some((true, p)),
            (None, Some(p)) => Some((false, p)),
            (None, None) => None,
            _ => unreachable!(),
        };
        let got = got.map(|(pre, id)| (pre, i32::from(sm.broadcasts.get(id).id() != "ad1")));
        assert_eq!(got, s);
    }
}

fn run_days(
    radio: &mut ZomboidRadio,
    world: &mut TestWorld,
    rng: &mut dyn FnMut(i32) -> i32,
    days: i32,
) -> Vec<String> {
    let mut log = Vec::new();
    for d in 0..days {
        for h in 0..24 {
            for m in (0..60).step_by(10) {
                radio.update_scripts(h, m, world, rng);
                radio.player_listens_channel(93200, true, false, world);
                for _ in 0..3 {
                    radio.update(world, rng);
                }
                if let Some(c) = radio.script_manager().unwrap().channels().get(&93200) {
                    log.push(format!(
                        "{d} {h} {m} {} {:?} {}",
                        c.last_aired_line(),
                        c.current_script_name(),
                        c.current_script_loop()
                    ));
                }
            }
        }
    }
    log
}

#[test]
fn schedule_save_and_reload() {
    let dir = game_dir(XML);
    let save = temp_dir();
    let mut world = TestWorld::new(Some(save.clone()));
    let mut radio = init_radio(3, &mut world, &dir, -1);
    let r = RefCell::new(JavaRandom::new(99));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    let mut log = run_days(&mut radio, &mut world, &mut rng, 1);
    // The first midnight after 09:00 counts a day.
    assert_eq!(radio.days_since_start(), 1);
    let file = save.join("radio/data").join(RADIO_SAVE_FILE);
    let saved = std::fs::read_to_string(&file).unwrap();
    assert_eq!(Some(saved.clone()), radio.save_content());
    let nl = crate::java::line_separator();
    assert!(saved.starts_with(&format!(
        "DaysSinceStart = 1{nl}LvObfuscation = false{nl}93200,"
    )));
    assert_eq!(saved.lines().count(), 6);
    assert!(save.join(crate::media::MEDIA_SAVE_FILE).exists());

    // A loaded game restores the same state while the scripts run.
    let mut world2 = TestWorld::new(Some(save.clone()));
    let radio2 = init_radio(5, &mut world2, &dir, 195);
    assert_eq!(radio2.days_since_start(), 1);
    assert_eq!(radio2.save_content().unwrap(), saved);

    log.extend(run_days(&mut radio, &mut world, &mut rng, 5));
    assert!(log.iter().any(|l| l.contains("line RD_l1")));
    assert!(log.iter().any(|l| l.contains("Some(\"evening\")")));
    // Once a channel's last script has ended, the loop counters and the
    // last broadcast are saved but not restored (no script to restore them
    // into), as in the original.
    let ended = radio.save_content().unwrap();
    assert!(ended.contains(&format!("{nl}93200,2,2,none,-1,b3,-1{nl}")));
}

#[test]
fn malformed_or_missing_save_falls_back_to_sandbox_days() {
    let dir = game_dir(XML);
    let save = temp_dir();
    let mut world = TestWorld::new(Some(save.clone()));
    world.apo = 3;
    let radio = init_radio(3, &mut world, &dir, 195);
    // No file: (3 - 1) months * 30.5 + nights survived.
    assert_eq!(radio.days_since_start(), 61 + 3);
    // Load cleared every script before finding no file.
    assert!(radio.script_manager().unwrap().channels()[&93200]
        .current_script()
        .is_none());

    std::fs::write(
        save.join("radio/data").join(RADIO_SAVE_FILE),
        "DaysSinceStart = 9\nLvObfuscation = true\n93200,2,3,evening,4,b3,1\n91200,x,1,morning,0,none,-1\n",
    )
    .unwrap();
    let mut world = TestWorld::new(Some(save.clone()));
    world.apo = 3;
    let radio = init_radio(3, &mut world, &dir, 195);
    assert_eq!(radio.days_since_start(), 61 + 3);
    assert!(radio.louisville_obfuscation);
    let sm = radio.script_manager().unwrap();
    let fm = &sm.channels()[&93200];
    assert_eq!(fm.current_script_name(), Some("evening"));
    assert_eq!(
        (fm.current_script_loop(), fm.current_script_max_loops()),
        (2, 3)
    );
    assert_eq!(fm.current_script().unwrap().start_day(), 4);
    let b = fm.airing_broadcast().unwrap();
    assert_eq!(sm.broadcasts.get(b).id(), "b3");
    assert_eq!(sm.broadcasts.get(b).current_line_number(), 1);
}

#[test]
fn initial_infection_start() {
    let xml = XML.replace(
        r#"<ScriptEntry ID="s2" name="evening""#,
        r#"<ScriptEntry ID="s9" name="init_infection" loopmin="1" loopmax="1"><ExitOptions/></ScriptEntry><ScriptEntry ID="s2" name="evening""#,
    );
    let dir = game_dir(&xml);
    let mut world = TestWorld::new(None);
    world.mode = "Initial Infection".into();
    let r = RefCell::new(JavaRandom::new(1));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    let mut radio = ZomboidRadio::new(9);
    // The empty channel 201 has no current script: the original throws.
    assert!(radio
        .init(-1, &source(&dir), &text, &mut world, &mut rng)
        .is_err());
    let fm = &radio.script_manager().unwrap().channels()[&93200];
    assert_eq!(fm.current_script_name(), Some("init_infection"));
    let exits = fm.radio_script("init_infection").unwrap().exit_options();
    assert_eq!(exits[0].script_name, "morning");
    assert_eq!(exits[0].chance, 100);
}

#[test]
fn scramble_matches_the_original() {
    // Outputs of the original's scrambleString with `new Random(42)`
    // behind Rand, calls in this order.
    let r = RefCell::new(JavaRandom::new(42));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    let mut radio = ZomboidRadio::new(0);
    let m = "Hello there, this is a test of the radio";
    let mut got = Vec::new();
    for i in [0, 1, 25] {
        for bb in [false, true] {
            for cs in [None, Some("<X>")] {
                got.push(radio.scramble_string(m, i, bb, cs, &mut rng));
            }
        }
    }
    assert_eq!(got[4], "Hello there, this is a test of the radio ");
    assert_eq!(got[6], "Hello there, <bzzt> is a test of the radio");
    assert_eq!(got[8], "Hello <bzzt> is a test <bzzt> the radio ");
    assert_eq!(got[9], "<X> there, this is a test of the radio ");
    assert_eq!(got[10], "<szzt> there, this <fzzt> test <szzt> the radio");
    assert_eq!(got[11], "Hello <X> this is a <X> of the radio");
    assert!(got[..4].iter().all(|s| s == m));
}

#[test]
fn devices_hear_broadcasts_and_speech() {
    let dir = game_dir(XML);
    let mut world = TestWorld::new(None);
    let mut radio = init_radio(3, &mut world, &dir, -1);
    let tv = DeviceId(1);
    let ham = DeviceId(2);
    world.devices.push((
        tv,
        DeviceView {
            turned_on: true,
            television: true,
            channel: 201,
            volume: 1.0,
            has_player_in_range: true,
            ..Default::default()
        },
    ));
    world.devices.push((
        ham,
        DeviceView {
            turned_on: true,
            two_way: true,
            channel: 91200,
            volume: 1.0,
            mic_range: 5,
            transmit_range: 1000,
            x: 10.0,
            y: 10.0,
            has_player_in_range: true,
            ..Default::default()
        },
    ));
    radio.register_device(tv, false);
    radio.register_device(ham, true);
    assert_eq!(radio.broadcast_devices(), &[ham]);
    let r = RefCell::new(JavaRandom::new(4));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    // The ham radio near a player starts the amateur broadcast.
    radio.update_scripts(10, 0, &mut world, &mut rng);
    for _ in 0..3 {
        radio.update(&mut world, &mut rng);
    }
    assert_eq!(world.heard.len(), 1);
    assert_eq!(world.heard[0].0, ham);
    assert_eq!(world.heard[0].2, "line RD_l5");

    // A player speaking next to the ham radio: the transmission starts at
    // the radio itself, which does not hear its own square.
    world.heard.clear();
    world.players.push(PlayerView {
        x: 11.0,
        y: 10.0,
        last_spoken_line: Some("anyone out there".into()),
        speak_colour: [1.0, 0.0, 0.0],
        equipped_radio: None,
    });
    radio.update(&mut world, &mut rng);
    assert!(world.heard.is_empty());
    // A receiver far away hears it, greyed and scrambled by range.
    world.devices.push((
        DeviceId(3),
        DeviceView {
            turned_on: true,
            channel: 91200,
            volume: 1.0,
            x: 990.0,
            y: 100.0,
            ..Default::default()
        },
    ));
    radio.register_device(DeviceId(3), false);
    world.players[0].last_spoken_line = Some("come in please".into());
    radio.update(&mut world, &mut rng);
    assert_eq!(world.heard.len(), 1);
    let (id, listener, _, rgb, dist) = &world.heard[0];
    assert_eq!((*id, *listener), (DeviceId(3), None));
    assert_eq!(*dist, 984);
    assert_eq!(*rgb, [0.5, 0.5, 0.5]);
}

#[test]
fn recorded_media_save_round_trip() {
    let save = temp_dir();
    let r = RefCell::new(JavaRandom::new(11));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    let mut rm = RecordedMedia::new();
    rm.init(Some(&save), &mut |m| {
        for i in 0..9 {
            let cat = ["CDs", "Retail-VHS", "Home-VHS"][i % 3];
            m.register(cat, &format!("m{i}"), &format!("Item{i}"), (i % 3) as i32);
        }
    });
    for _ in 0..30 {
        rm.random_from_category("Home-VHS", &mut rng);
    }
    let spawned = rm.home_vhs_spawned();
    assert!(!spawned.is_empty());
    rm.save(Some(&save));
    let bytes = std::fs::read(save.join(crate::media::MEDIA_SAVE_FILE)).unwrap();
    assert_eq!(&bytes[..8], &[0, 0, 0, 2, 0, 0, 0, 9]);
    let mut rm2 = RecordedMedia::new();
    rm2.init(Some(&save), &mut |m| {
        m.register("CDs", "m4", "x", 0);
    });
    // m4 keeps its saved index.
    assert_eq!(rm2.media_data("m4").unwrap().index(), 4);
    assert_eq!(rm2.home_vhs_spawned(), spawned);
    assert!(rm2.random_from_category("CDs", &mut rng).is_some());
}

/// The game's own radio data (needs `PZ_GAME_DIR`): it loads, every
/// channel is listed and categorised, and after a few days of schedule a
/// reload reproduces the save.
#[test]
#[ignore]
fn game_radio_data() {
    let Ok(game) = std::env::var("PZ_GAME_DIR") else {
        return;
    };
    let game = PathBuf::from(game);
    let save = temp_dir();
    let src = RadioSource {
        media_dir: game.join("media"),
        language: "EN".into(),
        default_language: "EN".into(),
        available_languages: vec!["EN".into()],
        ..Default::default()
    };
    let mut world = TestWorld::new(Some(save.clone()));
    let r = RefCell::new(JavaRandom::new(7));
    let mut rng = |b: i32| r.borrow_mut().next_int(b);
    let key = |k: &str| k.to_string();
    let mut radio = ZomboidRadio::new(9);
    radio.init(-1, &src, &key, &mut world, &mut rng).unwrap();
    let sm = radio.script_manager().unwrap();
    assert!(!sm.channels().is_empty());
    let listed: usize = radio.full_channel_list().iter().map(|(_, l)| l.len()).sum();
    assert_eq!(listed, sm.channels().len());
    for c in sm.channels().values() {
        assert_ne!(c.category(), ChannelCategory::Undefined);
    }
    run_days(&mut radio, &mut world, &mut rng, 3);
    let saved = radio.save_content().unwrap();
    let mut world2 = TestWorld::new(Some(save.clone()));
    let r2 = RefCell::new(JavaRandom::new(8));
    let mut rng2 = |b: i32| r2.borrow_mut().next_int(b);
    let mut radio2 = ZomboidRadio::new(9);
    radio2
        .init(195, &src, &key, &mut world2, &mut rng2)
        .unwrap();
    assert_eq!(radio2.save_content().unwrap(), saved);
}
