use super::*;

#[test]
fn constructor_targets() {
    let s = SearchMode::new();
    assert_eq!(s.fade_time(), 1.0);
    for i in 0..MAX_PLAYERS {
        let m = s.player(i).unwrap();
        assert_eq!(m.plr_index(), i as i32);
        assert!(!m.enabled() && !m.is_shader_enabled());
        let t = |p| (m.param(p).target_exterior(), m.param(p).target_interior(), m.param(p).exterior());
        assert_eq!(t(SearchModeParam::Blur), (1.0, 1.0, 0.0));
        assert_eq!(t(SearchModeParam::Desat), (0.85, 0.85, 0.0));
        assert_eq!(t(SearchModeParam::Radius), (4.0, 4.0, 0.0));
        assert_eq!(t(SearchModeParam::Darkness), (0.0, 0.0, 0.0));
        assert_eq!(t(SearchModeParam::GradientWidth), (4.0, 4.0, 0.0));
        assert_eq!((m.radius.min(), m.radius.max(), m.radius.stepsize()), (0.0, 50.0, 1.0));
        assert_eq!((m.gradient_width.min(), m.gradient_width.max()), (0.0, 20.0));
        assert_eq!((m.blur.max(), m.blur.stepsize()), (1.0, 0.01));
    }
    assert!(s.player(4).is_none());
}

#[test]
fn fade_in_scales_targets_then_equalises() {
    let mut s = SearchMode::new();
    s.set_enabled(0, true).unwrap();
    assert_eq!(s.player(0).unwrap().fade_state(), (0.0, true, false));
    s.update(0.25);
    let m = s.player(0).unwrap();
    assert!(m.is_shader_enabled());
    assert_eq!(m.blur.exterior(), 0.25);
    assert_eq!(m.radius.interior(), 1.0);
    assert_eq!(m.desat.exterior(), 0.25 * 0.85);
    // The gradient width equalises: 0 -> 4 by a hundredth.
    assert_eq!(m.gradient_width.exterior(), 0.04);
    // The other players stay off: values reset to zero.
    assert_eq!(s.player(1).unwrap().blur.exterior(), 0.0);
    for _ in 0..3 {
        s.update(0.25);
    }
    let m = s.player(0).unwrap();
    assert_eq!(m.fade_state(), (1.0, false, false));
    assert_eq!(m.radius.exterior(), 4.0);
    // Enabled and faded in: equalise toward changed targets.
    s.player_mut(0).unwrap().radius.set_targets(10.0, 2.0);
    s.update(0.25);
    let m = s.player(0).unwrap();
    assert_eq!(m.radius.exterior(), 4.0 + (10.0 - 4.0) * 0.01);
    assert_eq!(m.radius.interior(), 4.0 + (2.0 - 4.0) * 0.01);
}

#[test]
fn fade_out_then_reset() {
    let mut s = SearchMode::new();
    s.set_enabled(2, true).unwrap();
    s.update(5.0);
    assert_eq!(s.player(2).unwrap().fade_state(), (1.0, false, false));
    s.set_enabled(2, false).unwrap();
    assert_eq!(s.player(2).unwrap().fade_state(), (1.0, false, true));
    s.update(0.5);
    let m = s.player(2).unwrap();
    assert!(m.is_shader_enabled() && !m.enabled());
    assert_eq!(m.blur.exterior(), 0.5);
    s.update(0.5);
    assert_eq!(s.player(2).unwrap().fade_state(), (0.0, false, false));
    assert!(!s.player(2).unwrap().is_shader_enabled());
    // Off: blur/desat/radius/darkness reset, the targets stay.
    s.player_mut(2).unwrap().blur.set_exterior(0.7);
    s.update(0.1);
    let m = s.player(2).unwrap();
    assert_eq!(m.blur.exterior(), 0.0);
    assert_eq!(m.blur.target_exterior(), 1.0);
}

#[test]
fn set_enabled_is_edge_triggered_and_override_freezes() {
    let mut s = SearchMode::new();
    s.set_enabled(0, false).unwrap();
    assert!(!s.player(0).unwrap().is_shader_enabled());
    s.set_enabled(0, true).unwrap();
    s.update(0.5);
    // Setting it on again does not restart the fade.
    s.set_enabled(0, true).unwrap();
    assert_eq!(s.player(0).unwrap().fade_state(), (0.5, true, false));
    // Off mid-fade fades out from where it is.
    s.set_enabled(0, false).unwrap();
    assert_eq!(s.player(0).unwrap().fade_state(), (0.5, false, true));
    s.player_mut(0).unwrap().override_ = true;
    s.update(0.5);
    assert_eq!(s.player(0).unwrap().fade_state(), (0.5, false, true));
    assert!(s.set_enabled(4, true).is_none());
}

#[test]
fn equalise_snaps_within_a_thousandth() {
    let mut f = SearchModeFloat::new(0.0, 1.0, 0.01);
    f.set(0.9995, 1.0, 0.5, 1.0);
    f.equalise();
    assert_eq!(f.exterior(), 1.0);
    assert_eq!(f.interior(), 0.5 + 0.5 * 0.01);
    f.reset_all();
    assert_eq!((f.exterior(), f.target_exterior(), f.interior(), f.target_interior()), (0.0, 0.0, 0.0, 0.0));
    f.set_all(0.3);
    assert_eq!((f.exterior(), f.target_exterior(), f.interior(), f.target_interior()), (0.3, 0.3, 0.3, 0.3));
}

#[test]
fn shader_picks_exterior_or_interior() {
    let mut s = SearchMode::new();
    let m = s.player_mut(1).unwrap();
    m.radius.set(6.0, 0.0, 3.0, 0.0);
    m.darkness.set(0.2, 0.0, 0.4, 0.0);
    let m = s.player(1).unwrap();
    assert_eq!(m.shader(true).radius, 6.0);
    assert_eq!(m.shader(false).radius, 3.0);
    assert_eq!(m.shader(false).darkness, 0.4);
    assert!(!m.shader(true).enabled);
}

#[test]
fn odd_fade_times_do_not_panic() {
    let mut s = SearchMode::new();
    s.set_fade_time(-1.0);
    s.set_enabled(0, true).unwrap();
    s.update(0.5);
    // clamp(0.5, 0, -1) is -1 (below-min check first fails, above-max wins).
    assert_eq!(s.player(0).unwrap().fade_state().0, -1.0);
    s.set_fade_time(0.0);
    s.update(0.5);
}

/// Replays a log of the Java oracle (actions, `GameTime.getTimeDelta()`,
/// and every player's state after each `SearchMode.update()`, floats as
/// raw bits) and compares frame by frame. The logs are not committed: set
/// `PZ_SEARCH_MODE_ORACLE` to a comma-separated list of log files.
#[test]
#[ignore]
fn java_oracle_replay() {
    let Ok(files) = std::env::var("PZ_SEARCH_MODE_ORACLE") else { return };
    let bits = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
    let param = |s: &str| SearchModeParam::ALL[s.parse::<usize>().unwrap()];
    for file in files.split(',') {
        let text = std::fs::read_to_string(file).unwrap();
        let mut s = SearchMode::new();
        let (mut frames, mut states) = (0, 0);
        for (ln, line) in text.lines().enumerate() {
            let w: Vec<&str> = line.split(' ').collect();
            let p = || w[1].parse::<usize>().unwrap();
            match w[0] {
                "enable" => {
                    s.set_enabled(p(), w[2] == "true").unwrap();
                }
                "targets" => s.player_mut(p()).unwrap().param_mut(param(w[2])).set_targets(bits(w[3]), bits(w[4])),
                "setext" => s.player_mut(p()).unwrap().param_mut(param(w[2])).set_exterior(bits(w[3])),
                "resetall" => s.player_mut(p()).unwrap().param_mut(param(w[2])).reset_all(),
                "fade" => s.set_fade_time(bits(w[1])),
                "override" => s.player_mut(p()).unwrap().override_ = w[2] == "true",
                "update" => {
                    s.update(bits(w[1]));
                    frames += 1;
                }
                "state" => {
                    let m = s.player(p()).unwrap();
                    let (timer, fade_in, fade_out) = m.fade_state();
                    let mut got = vec![
                        m.enabled().to_string(),
                        fade_in.to_string(),
                        fade_out.to_string(),
                        m.is_shader_enabled().to_string(),
                        format!("{:x}", timer.to_bits()),
                    ];
                    for q in SearchModeParam::ALL {
                        let f = m.param(q);
                        got.push(format!("{:x}", f.exterior().to_bits()));
                        got.push(format!("{:x}", f.interior().to_bits()));
                    }
                    assert_eq!(got, w[2..].to_vec(), "{file}:{} (frame {frames})", ln + 1);
                    states += 1;
                }
                other => panic!("{file}:{}: unknown line {other}", ln + 1),
            }
        }
        eprintln!("{file}: {frames} frames, {states} player states match");
    }
}
