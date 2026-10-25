use super::*;
use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pz-bind-video-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join("media/videos")).unwrap();
    dir
}

fn lua(dir: &std::path::Path) -> LuaManager {
    let fs = Rc::new(RefCell::new(GameFs::new(dir)));
    LuaManager::init(fs).unwrap()
}

fn run(mgr: &mut LuaManager, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), mgr.thread.globals().clone()).unwrap();
    mgr.thread.call(&f, &[]).unwrap()
}

/// A `BIKi` file of `frames` frames, `w` x `h`, without audio; frame i's
/// video data is the single byte `i` (padded to an even size).
fn bik(w: u32, h: u32, frames: u32) -> Vec<u8> {
    let mut out = b"BIKi".to_vec();
    for v in [0, frames, 2, frames, w, h, 30, 1, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut off = 44 + 4 * (frames + 1);
    for i in 0..frames {
        out.extend_from_slice(&(off | u32::from(i == 0)).to_le_bytes());
        off += 2;
    }
    out.extend_from_slice(&off.to_le_bytes());
    for i in 0..frames {
        out.extend_from_slice(&[i as u8, 0]);
    }
    out
}

/// Decodes frame i as flat luma 16 + 40 * i, neutral chroma.
struct Flat(usize, usize, Vec<u8>, Vec<u8>, bool);

impl pz_video::FrameDecoder for Flat {
    fn decode(&mut self, _index: u32, video: &[u8], _key: bool) -> Result<(), String> {
        self.2.fill(16 + 40 * video[0]);
        self.4 = true;
        Ok(())
    }
    fn planes(&self) -> Option<pz_video::yuv::FramePlanes<'_>> {
        let cw = self.0.div_ceil(2);
        self.4.then(|| pz_video::yuv::FramePlanes {
            width: self.0,
            height: self.1,
            y: pz_video::yuv::Plane { data: &self.2, pitch: self.0 },
            cr: pz_video::yuv::Plane { data: &self.3, pitch: cw },
            cb: pz_video::yuv::Plane { data: &self.3, pitch: cw },
            a: None,
        })
    }
}

fn install_flat_decoder() {
    pz_video::with_manager(|m| {
        let start = std::time::Instant::now();
        // Every query is a frame period later, so each RenderFrame is due.
        let ticks = Cell::new(0u32);
        m.set_clock(move || {
            ticks.set(ticks.get() + 1);
            start.elapsed() + std::time::Duration::from_millis(40 * u64::from(ticks.get()))
        });
        m.set_decoders(|h| {
            let (w, hh) = (h.width as usize, h.height as usize);
            Some(Box::new(Flat(w, hh, vec![0; w * hh], vec![128; w.div_ceil(2) * hh.div_ceil(2)], false)))
        });
    });
}

#[test]
fn failed_open_returns_nil_like_the_original() {
    let dir = temp_dir("fail");
    std::fs::write(dir.join("media/videos/broken.bik"), b"not a video").unwrap();
    let mut mgr = lua(&dir);
    let r = run(
        &mut mgr,
        r#"
        local a = getVideo("missing.bik", 64, 32)
        local b = getVideo("broken.bik", 64, 32)
        local c = getVideo("../x.bik", 64, 32)
        local d = VideoTexture.getOrCreate("missing.bik", 64, 32)
        return a == nil, b == nil, c == nil, d == nil, VideoTexture.getWhite ~= nil
    "#,
    );
    assert!(r.iter().all(|v| v.truthy()), "{r:?}");
    // Remembered as failed: no second open.
    let abs = mgr.fs.borrow().media_file("videos/missing.bik").to_string_lossy().into_owned();
    assert!(statics(|s| s.failed_to_load.contains(&abs)));
    assert!(statics(|s| s.lua_textures.is_empty()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn playing_video_uploads_frames() {
    let dir = temp_dir("play");
    std::fs::write(dir.join("media/videos/clip.bik"), bik(4, 2, 3)).unwrap();
    install_flat_decoder();
    let mut mgr = lua(&dir);
    let r = run(
        &mut mgr,
        r#"
        local v = getVideo("clip.bik", 4, 2)
        local again = getVideo("clip.bik", 4, 2)
        v:RenderFrame()
        return v, v == again, v:isValid(), v:getWidth(), v:getHeight(), v:getXEnd(), instanceof(v, "Texture")
    "#,
    );
    assert!(r[1].truthy() && r[2].truthy(), "{r:?}");
    assert_eq!((r[3].as_num(), r[4].as_num(), r[5].as_num()), (Some(4.0), Some(2.0), Some(1.0)));
    assert!(r[6].truthy());
    let v = statics(|s| s.lua_textures.get("clip.bik").cloned()).unwrap();
    let tex = v.texture();
    {
        let tm = textures();
        let tm = tm.borrow();
        let d = tm.data(tex.id.unwrap()).unwrap();
        assert_eq!((d.version, d.width, d.height, d.width_hw), (1, 4, 2, 4));
        assert!(d.min_nearest() && d.mag_nearest());
        // Frame 1 (luma 16) is black.
        let px = &d.image.as_ref().unwrap().data.pixels;
        assert_eq!(px.len(), 32);
        assert!(px.chunks(4).all(|p| p[0] <= 1 && p[3] == 255));
    }
    v.render_frame();
    {
        let tm = textures();
        let tm = tm.borrow();
        let d = tm.data(tex.id.unwrap()).unwrap();
        assert_eq!(d.version, 2);
        // Frame 2: luma 56.
        assert!(d.image.as_ref().unwrap().data.pixels[0] > 40);
    }
    // closeAndDestroy: invalid, destroyed, and a new getOrCreate opens a
    // fresh texture (the native video stays open: closeVideo is empty).
    run(&mut mgr, r#"getVideo("clip.bik", 4, 2):closeAndDestroy()"#);
    assert!(!v.is_valid() && v.destroyed.get());
    let r = run(&mut mgr, r#"local w = getVideo("clip.bik", 4, 2) return w ~= nil and w:isValid()"#);
    assert!(r[0].truthy());
    let w = statics(|s| s.lua_textures.get("clip.bik").cloned()).unwrap();
    assert!(!Rc::ptr_eq(&v, &w));
    release_all_video_textures();
    assert!(!w.is_valid() && w.destroyed.get());
    assert!(statics(|s| s.lua_textures.is_empty() && s.successfully_loaded.is_empty()));
    pz_video::with_manager(|m| *m = pz_video::VideoManager::new());
    std::fs::remove_dir_all(&dir).unwrap();
}
