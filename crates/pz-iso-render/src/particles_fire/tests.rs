use super::*;
use pz_core::random::Rand;

fn cam() -> ParticlesCamera {
    ParticlesCamera {
        off_x: 1000.0,
        off_y: 500.0,
        offscreen_width: 1024,
        offscreen_height: 768,
    }
}

#[test]
fn the_array_reuses_holes_and_compacts() {
    let mut a = ParticlesArray::default();
    for i in 0..5 {
        assert_eq!(a.add(i), Some(i as usize));
    }
    assert!(a.delete(1) && a.delete(3));
    assert!(!a.delete(3) && !a.delete(9) && !a.delete(-1));
    assert_eq!(a.count(), 3);
    // From the last slot filled (0): the first hole.
    assert_eq!(a.add(10), Some(1));
    a.defragment();
    assert!(!a.need_to_update());
    assert_eq!(a.items, vec![Some(0), Some(10), Some(2), Some(4), None]);
    // Holes only at the end: the last item moves into the first hole, and
    // the original's loop leaves a gap before it.
    let mut b = ParticlesArray::default();
    for i in 0..4 {
        b.add(i);
    }
    b.delete(2);
    b.delete(3);
    b.defragment();
    assert_eq!(b.items, vec![Some(0), None, Some(1), None]);
    assert_eq!(b.add(7), Some(3));
}

#[test]
fn zones_fill_with_rand_in_the_originals_order() {
    let mut p = ParticlesFire::new(&cam());
    // cx = 1000 + 512, cy = 500 + 384.
    assert_eq!(
        p.zones[0],
        Zone {
            kind: ZoneType::Rectangle,
            intensity: 10,
            current_particles: 0,
            x0: 1482.0,
            y0: 874.0,
            x1: 1542.0,
            y1: 894.0,
            r: 0.0
        }
    );
    assert_eq!(p.zones[4].kind, ZoneType::Line);
    // Equal x: the ends swap.
    assert_eq!(
        (p.zones[4].x0, p.zones[4].y0, p.zones[4].y1),
        (1162.0, 584.0, 684.0)
    );
    let r = Rand::new(77);
    p.process(&r);
    // ceil(0.1 * intensity) each: 1 + 1 + 4 + 6 + 1.
    assert_eq!(p.particles.count(), 13);
    let t = Rand::new(77);
    let first = p.particles.items[0].unwrap();
    let (x, y) = (t.next_float(1482.0, 1542.0), t.next_float(874.0, 894.0));
    let (vx, vy, id) = (
        t.next_float(-3.0, 3.0),
        t.next_float(1.0, 5.0),
        t.next_float(-1e6, 1e6),
    );
    assert_eq!(
        first,
        Particle {
            id,
            x,
            y,
            t_shift: 0.0,
            vx,
            vy,
            zone: 0
        }
    );
    let circle = p.particles.items[1].unwrap();
    let (a, rr) = (
        t.next_float(0.0, std::f32::consts::PI * 2.0),
        t.next_float(0.0, 50.0),
    );
    assert_eq!(circle.x, (1312.0f64 + rr as f64 * (a as f64).cos()) as f32);
    // Topping up converges on each zone's intensity.
    for _ in 0..200 {
        p.process(&r);
    }
    assert_eq!(p.particles.count(), 130);
    assert!(p.zones.iter().all(|z| z.current_particles == z.intensity));
}

#[test]
fn uniforms_match_the_originals_buffers() {
    let mut p = ParticlesFire::new(&cam());
    p.advance(&cam(), 2.0);
    assert_eq!(p.time, 0.0166 * 2.0);
    let m = p.mvp;
    // offX..offX+w maps to -1..1, offY+h..offY to -1..1.
    let ndc = |x: f32, y: f32| [m[0][0] * x + m[3][0], m[1][1] * y + m[3][1]];
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
    assert!(near(ndc(1000.0, 500.0), [-1.0, 1.0]));
    assert!(near(ndc(2024.0, 1268.0), [1.0, -1.0]));
    assert_eq!(m[3][2], 0.0);
    p.update_particle_params(
        Wind {
            angle_intensity: 0.25,
            intensity: 0.5,
        },
        &Rand::new(1),
    );
    let wx = (1.5f64).sin() as f32 * 0.5;
    let wy = (1.5f64).cos() as f32 * 0.5;
    assert_eq!((p.wind_x, p.wind_y), (wx, wy));
    assert_eq!(
        p.parameters_fire(),
        [wx, 0.0, 0.0, wy, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    );
    // FireParam[0][0] is windX and FireParam[0][1] windY.
    let fp = p.fire_param();
    assert_eq!((fp[0][0], fp[0][1], fp[1][0]), (wx, wy, 0.0));
    assert_eq!(p.data.len(), 13);
    assert_eq!(p.data[2][3], 2.0 / 13.0);
}

#[test]
fn render_draws_flames_then_smoke_with_the_previous_wind() {
    let mut p = ParticlesFire::new(&cam());
    let r = Rand::new(5);
    let wind = Wind {
        angle_intensity: 0.1,
        intensity: 1.0,
    };
    let out = p.render(&cam(), 1.0, wind, None, None, &r);
    let [DrawCmd::Particles(fire), DrawCmd::Particles(smoke)] = &out[..] else {
        panic!("{out:?}")
    };
    assert_eq!(
        (fire.program, fire.blend_func),
        (ParticleProgram::Fire, [gl::SRC_ALPHA, gl::ONE])
    );
    assert_eq!(
        (smoke.program, smoke.blend_func),
        (
            ParticleProgram::Smoke,
            [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA]
        )
    );
    // The flames' uniforms were set before this frame's wind.
    assert_eq!(fire.param[0][0], 0.0);
    assert_eq!(smoke.param[0][0], ((0.1f32 * 6.0) as f64).sin() as f32);
    // Two processes a frame.
    assert_eq!((fire.particles.len(), smoke.particles.len()), (13, 26));
    assert_eq!(fire.viewport, (1024, 768));
}
