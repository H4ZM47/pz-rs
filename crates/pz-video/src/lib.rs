//! Video playback behind `zombie.core.textures.VideoTexture`.
//!
//! The game's natives live in `libbink64` (`BinkManager` / `BinkVideo` over
//! RAD's Bink SDK). [`VideoManager`] is that layer: integer video ids, the
//! JNI methods' semantics, and the frame buffer `getCurrentFrameData`
//! returns. What it does was read from `libbink64` and checked against the
//! game's Bink library run as a black box:
//!
//! - `openVideo` opens the file and returns ids 1, 2, ... (a failed open
//!   returns -1 and uses no id). The frame buffer is
//!   `align16(width * 4) * height + 16` bytes.
//! - `processFrameAsync` starts decoding the current frame; while it runs,
//!   `getCurrentFrameData` still copies the last completed frame.
//! - `isReadyForNewFrame` (`BinkWait() == 0`): the first call after opening
//!   is ready; after that frames are paced at the file's frame rate. A
//!   caller slower than the frame rate gets one frame per ready call
//!   (playback slows down); `shouldSkipFrame` never asks to drop frames.
//! - `nextFrame` advances `FrameNum` and wraps from the last frame to 1;
//!   `isEndOfVideo` is `FrameNum == Frames`.
//! - `closeVideo` does nothing (the native method body is empty).
//!
//! Decoding the Bink 1 video bitstream is not ported yet
//! ([`decoder_for`] has no decoder), so every open fails the way the native
//! `openVideo` fails, and callers fall back exactly as the original does
//! when a video cannot be opened.

pub mod bink;
pub mod yuv;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bink::{BinkFile, Header};

/// A Bink video bitstream decoder.
pub trait FrameDecoder {
    /// Decodes the video data of frame `index` (0-based, `key` from the
    /// frame index) on top of the previous frame.
    fn decode(&mut self, index: u32, video: &[u8], key: bool) -> Result<(), String>;
    /// The last decoded frame.
    fn planes(&self) -> Option<yuv::FramePlanes<'_>>;
}

/// The decoder for a file's video, or `None` when it cannot be decoded.
///
/// Bink 1 (`BIKb`..`BIKk`) decoding is not ported yet.
pub fn decoder_for(_header: &Header) -> Option<Box<dyn FrameDecoder>> {
    None
}

type DecoderFactory = Box<dyn Fn(&Header) -> Option<Box<dyn FrameDecoder>>>;
type Clock = Rc<dyn Fn() -> Duration>;

/// `BinkVideo`: one open file.
struct Video {
    file: BinkFile,
    decoder: Box<dyn FrameDecoder>,
    /// `BINK.FrameNum`, 1-based.
    frame_num: u32,
    /// An async decode of `frame_num` is in flight.
    pending: bool,
    /// When `frame_num` may be shown; `None` before the first wait.
    due: Option<Duration>,
    period: Duration,
    buffer: Vec<u8>,
}

impl Video {
    fn pitch(&self) -> usize {
        (self.file.header.width as usize * 4 + 15) & !15
    }

    /// `BinkDoFrame`: decodes the current frame.
    fn decode_current(&mut self) {
        let i = self.frame_num - 1;
        let key = self.file.index[i as usize].key;
        let Ok(data) = self.file.read_frame(i) else { return };
        let Ok(p) = bink::split_packet(&data, self.file.header.audio.len()) else { return };
        // A corrupt frame keeps what was decoded before.
        let _ = self.decoder.decode(i, p.video, key);
    }

    fn finish_pending(&mut self) {
        if self.pending {
            self.pending = false;
            self.decode_current();
        }
    }
}

/// `BinkManager`: open videos by id.
pub struct VideoManager {
    last_id: i32,
    videos: HashMap<i32, Video>,
    clock: Clock,
    decoders: DecoderFactory,
}

impl Default for VideoManager {
    fn default() -> Self {
        let start = Instant::now();
        VideoManager {
            last_id: 0,
            videos: HashMap::new(),
            clock: Rc::new(move || start.elapsed()),
            decoders: Box::new(decoder_for),
        }
    }
}

impl VideoManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the clock frame pacing reads (default: wall time since the
    /// manager was created). Headless runs use it to step video time.
    pub fn set_clock(&mut self, clock: impl Fn() -> Duration + 'static) {
        self.clock = Rc::new(clock);
    }

    /// Replaces the decoder lookup (tests).
    pub fn set_decoders(&mut self, f: impl Fn(&Header) -> Option<Box<dyn FrameDecoder>> + 'static) {
        self.decoders = Box::new(f);
    }

    fn now(&self) -> Duration {
        (self.clock)()
    }

    /// `openVideo(path)`: the new id, or -1.
    pub fn open_video(&mut self, path: &str) -> i32 {
        match BinkFile::open(Path::new(path)) {
            Ok(f) => self.open_file(f),
            Err(_) => -1,
        }
    }

    /// `openVideo` for an already opened container.
    pub fn open_file(&mut self, file: BinkFile) -> i32 {
        let Some(decoder) = (self.decoders)(&file.header) else { return -1 };
        let h = &file.header;
        let size = ((h.width as usize * 4 + 15) & !15) * h.height as usize + 16;
        let period = Duration::from_secs_f64(h.frame_period());
        self.last_id += 1;
        let id = self.last_id;
        let video = Video { file, decoder, frame_num: 1, pending: false, due: None, period, buffer: vec![0; size] };
        self.videos.insert(id, video);
        id
    }

    /// `isReadyForNewFrame(id)`.
    pub fn is_ready_for_new_frame(&mut self, id: i32) -> bool {
        let now = self.now();
        let Some(v) = self.videos.get_mut(&id) else { return false };
        match v.due {
            None => {
                v.due = Some(now);
                true
            }
            Some(due) => now >= due,
        }
    }

    /// `processFrame(id)`: decodes the current frame now.
    pub fn process_frame(&mut self, id: i32) {
        if let Some(v) = self.videos.get_mut(&id) {
            v.pending = false;
            v.decode_current();
        }
    }

    /// `processFrameAsync(id)`: starts decoding the current frame.
    pub fn process_frame_async(&mut self, id: i32) {
        if let Some(v) = self.videos.get_mut(&id) {
            v.finish_pending();
            v.pending = true;
        }
    }

    /// `processFrameAsyncWait(id, ms)`: completes the decode started by
    /// [`Self::process_frame_async`]. Decoding is synchronous here, so it
    /// never times out.
    pub fn process_frame_async_wait(&mut self, id: i32, _timeout_ms: i32) -> bool {
        match self.videos.get_mut(&id) {
            Some(v) => {
                v.finish_pending();
                true
            }
            None => false,
        }
    }

    /// `nextFrame(id)`: the next frame (wrapping to 1 after the last),
    /// due one frame period after the current one.
    pub fn next_frame(&mut self, id: i32) {
        let now = self.now();
        let Some(v) = self.videos.get_mut(&id) else { return };
        v.finish_pending();
        v.frame_num = if v.frame_num >= v.file.header.frames { 1 } else { v.frame_num + 1 };
        if let Some(due) = v.due {
            v.due = Some((due + v.period).max(now));
        }
    }

    /// `shouldSkipFrame(id)`: the library never asked to skip in the
    /// measured runs (slow callers slow playback instead).
    pub fn should_skip_frame(&mut self, id: i32) -> bool {
        !self.videos.contains_key(&id)
    }

    /// `isEndOfVideo(id)`: `FrameNum == Frames`.
    pub fn is_end_of_video(&mut self, id: i32) -> bool {
        self.videos.get(&id).is_none_or(|v| v.frame_num == v.file.header.frames)
    }

    /// `closeVideo(id)`: the native method does nothing; the video stays
    /// open.
    pub fn close_video(&mut self, _id: i32) {}

    /// `getCurrentFrameData(id)`: copies the last completed frame into the
    /// video's buffer as RGBA rows of `align16(width * 4)` bytes and
    /// returns the buffer.
    pub fn current_frame_data(&mut self, id: i32) -> Option<&[u8]> {
        let v = self.videos.get_mut(&id)?;
        let pitch = v.pitch();
        if let Some(p) = v.decoder.planes() {
            yuv::copy_rgba(&p, &mut v.buffer, pitch);
        }
        Some(&v.buffer)
    }

    /// `getFrameDataSize(id)`: `width * height * 4`.
    pub fn frame_data_size(&self, id: i32) -> i32 {
        self.videos.get(&id).map_or(0, |v| (v.file.header.width * v.file.header.height * 4) as i32)
    }

    /// `BINK.FrameNum` (1-based), for inspection.
    pub fn frame_num(&self, id: i32) -> Option<u32> {
        self.videos.get(&id).map(|v| v.frame_num)
    }

    /// The video's header, for inspection.
    pub fn header(&self, id: i32) -> Option<&Header> {
        self.videos.get(&id).map(|v| &v.file.header)
    }
}

thread_local! {
    static MANAGER: RefCell<VideoManager> = RefCell::new(VideoManager::new());
}

/// `BinkManager.GetInstance()`.
pub fn with_manager<R>(f: impl FnOnce(&mut VideoManager) -> R) -> R {
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::Cursor;

    /// A decoder whose frame is a flat grey of the frame's first video
    /// byte, so copies show which frame was decoded.
    struct Flat {
        w: usize,
        h: usize,
        y: Vec<u8>,
        c: Vec<u8>,
        decoded: Rc<RefCell<Vec<u32>>>,
    }

    impl FrameDecoder for Flat {
        fn decode(&mut self, index: u32, video: &[u8], _key: bool) -> Result<(), String> {
            self.decoded.borrow_mut().push(index);
            self.y.fill(video[0]);
            Ok(())
        }
        fn planes(&self) -> Option<yuv::FramePlanes<'_>> {
            if self.decoded.borrow().is_empty() {
                return None;
            }
            let cw = self.w.div_ceil(2);
            Some(yuv::FramePlanes {
                width: self.w,
                height: self.h,
                y: yuv::Plane { data: &self.y, pitch: self.w },
                cr: yuv::Plane { data: &self.c, pitch: cw },
                cb: yuv::Plane { data: &self.c, pitch: cw },
                a: None,
            })
        }
    }

    /// A 4x2 file of `n` frames (frame i's video byte is 16 + 20 * i) at
    /// 30 fps, a manager stepping a shared clock, and the decode log.
    type DecodeLog = Rc<RefCell<Vec<u32>>>;

    fn setup(n: u32) -> (VideoManager, Rc<Cell<Duration>>, DecodeLog, i32) {
        let mut pkts = Vec::new();
        for i in 0..n {
            let mut p = 0u32.to_le_bytes().to_vec();
            p.push(16 + 20 * i as u8);
            pkts.push((i == 0, p));
        }
        let bytes = bink::tests::build(b'i', 4, 2, &[(48000, 0x7000)], &pkts);
        let mut m = VideoManager::new();
        let t = Rc::new(Cell::new(Duration::ZERO));
        let t2 = t.clone();
        m.set_clock(move || t2.get());
        let log = Rc::new(RefCell::new(Vec::new()));
        let l2 = log.clone();
        m.set_decoders(move |h| {
            let (w, ht) = (h.width as usize, h.height as usize);
            Some(Box::new(Flat {
                w,
                h: ht,
                y: vec![0; w * ht],
                c: vec![128; w.div_ceil(2) * ht.div_ceil(2)],
                decoded: l2.clone(),
            }))
        });
        let f = BinkFile::from_source(Box::new(Cursor::new(bytes))).unwrap();
        let id = m.open_file(f);
        (m, t, log, id)
    }

    fn shown(m: &mut VideoManager, id: i32) -> u8 {
        m.current_frame_data(id).unwrap()[0]
    }

    /// `VideoTexture.RenderFrameAsync`'s native calls; true when a frame
    /// was uploaded.
    fn render_frame_async(m: &mut VideoManager, id: i32) -> Option<u8> {
        if !m.is_ready_for_new_frame(id) {
            return None;
        }
        if m.process_frame_async_wait(id, 1000) {
            while m.should_skip_frame(id) {
                m.next_frame(id);
                m.process_frame_async(id);
                m.process_frame_async_wait(id, -1);
            }
            m.next_frame(id);
            m.process_frame_async(id);
        }
        Some(shown(m, id))
    }

    #[test]
    fn ids_and_failed_opens() {
        let mut m = VideoManager::new();
        assert_eq!(m.open_video("/nonexistent/file.bik"), -1);
        let (mut m2, _, _, id) = setup(3);
        assert_eq!(id, 1);
        let bytes = bink::tests::build(b'i', 4, 2, &[], &[(true, vec![1])]);
        let f = BinkFile::from_source(Box::new(Cursor::new(bytes))).unwrap();
        assert_eq!(m2.open_file(f), 2);
        // Without a decoder the open fails and uses no id.
        let bytes = bink::tests::build(b'i', 4, 2, &[], &[(true, vec![1])]);
        m.set_decoders(|_| None);
        let f = BinkFile::from_source(Box::new(Cursor::new(bytes))).unwrap();
        assert_eq!(m.open_file(f), -1);
        assert_eq!(m.frame_data_size(1), 0);
        assert_eq!(m2.frame_data_size(1), 32);
        // align16(4 * 4) * 2 + 16
        assert_eq!(m2.current_frame_data(1).unwrap().len(), 48);
    }

    #[test]
    fn async_copies_lag_the_decode_in_flight() {
        let (mut m, t, log, id) = setup(3);
        // LoadVideoFile starts decoding frame 1.
        m.process_frame_async(id);
        assert!(log.borrow().is_empty());
        // Nothing completed yet: the buffer is still blank.
        assert_eq!(shown(&mut m, id), 0);
        // First RenderFrame: ready at once; frame 1 completes and is shown
        // while frame 2 decodes.
        let grey = |y: u8| yuv_grey(y);
        assert_eq!(render_frame_async(&mut m, id), Some(grey(16)));
        assert_eq!(m.frame_num(id), Some(2));
        assert_eq!(*log.borrow(), vec![0]);
        // Not due yet.
        t.set(Duration::from_millis(10));
        assert_eq!(render_frame_async(&mut m, id), None);
        t.set(Duration::from_millis(34));
        assert_eq!(render_frame_async(&mut m, id), Some(grey(36)));
        assert_eq!(m.frame_num(id), Some(3));
        assert!(m.is_end_of_video(id));
        t.set(Duration::from_millis(67));
        assert_eq!(render_frame_async(&mut m, id), Some(grey(56)));
        // Wrapped to the first frame.
        assert_eq!(m.frame_num(id), Some(1));
        assert!(!m.is_end_of_video(id));
        assert_eq!(*log.borrow(), vec![0, 1, 2]);
    }

    #[test]
    fn slow_callers_get_one_frame_per_call() {
        let (mut m, t, _, id) = setup(10);
        m.process_frame_async(id);
        assert!(render_frame_async(&mut m, id).is_some());
        // Ten periods late: still only one frame advanced per call, and the
        // next frame is due right away.
        t.set(Duration::from_millis(340));
        assert!(render_frame_async(&mut m, id).is_some());
        assert_eq!(m.frame_num(id), Some(3));
        assert!(render_frame_async(&mut m, id).is_some());
        assert_eq!(m.frame_num(id), Some(4));
        // Then paced again from now.
        assert!(render_frame_async(&mut m, id).is_none());
        t.set(Duration::from_millis(340 + 34));
        assert!(render_frame_async(&mut m, id).is_some());
    }

    #[test]
    fn sync_path_decodes_then_advances() {
        let (mut m, _, log, id) = setup(2);
        // RenderFrame with useAsync false.
        assert!(m.is_ready_for_new_frame(id));
        m.process_frame(id);
        m.next_frame(id);
        assert_eq!(shown(&mut m, id), yuv_grey(16));
        assert_eq!(*log.borrow(), vec![0]);
        assert!(m.is_end_of_video(id));
        m.close_video(id);
        // closeVideo does nothing.
        assert!(m.frame_num(id).is_some());
    }

    /// The red byte `copy_rgba` gives a flat grey `y` at dither 0.
    fn yuv_grey(y: u8) -> u8 {
        let yp = [y];
        let c = [128u8];
        let p = yuv::FramePlanes {
            width: 1,
            height: 1,
            y: yuv::Plane { data: &yp, pitch: 1 },
            cr: yuv::Plane { data: &c, pitch: 1 },
            cb: yuv::Plane { data: &c, pitch: 1 },
            a: None,
        };
        let mut out = [0u8; 4];
        yuv::copy_rgba(&p, &mut out, 4);
        out[0]
    }

    #[test]
    fn game_videos_do_not_open_yet() {
        let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
        let p = Path::new(&dir).join("media/videos/title_screen_background.bik");
        let mut m = VideoManager::new();
        assert!(BinkFile::open(&p).is_ok());
        assert_eq!(m.open_video(p.to_str().unwrap()), -1);
    }
}
