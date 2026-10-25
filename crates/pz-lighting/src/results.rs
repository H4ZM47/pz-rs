//! Per-level result arrays (`ResultLevel`). Each state keeps a front set
//! (read by the getters) and a back set (written by the update); the two
//! are swapped when the main thread picks up a finished update.

use crate::model::ResultLights;

#[derive(Clone, Debug, Default)]
pub(crate) struct ResultData {
    pub vis: Vec<u8>,
    pub r: Vec<u8>,
    pub g: Vec<u8>,
    pub b: Vec<u8>,
    pub dark: Vec<f32>,
    pub target: Vec<f32>,
    pub level: Vec<u32>,
    /// Corner maxima before blending (`CalculateMaxVertLight1/2`).
    pub inter: [Vec<u32>; 8],
    /// Final vertex colours `0xFFbbggrr`.
    pub vert: [Vec<u32>; 8],
    pub statics: Vec<ResultLights>,
    pub torches: Vec<ResultLights>,
    /// Lit by an active room light.
    pub lit: Vec<u8>,
    pub dirty: Vec<u8>,
    pub chunk_dirty: Vec<u8>,
}

impl ResultData {
    fn new(w: i32, h: i32) -> ResultData {
        let n = (w.max(0) * h.max(0)) as usize;
        let chunks = ((w / 8) * (h / 8)).max(0) as usize;
        let u32s = || vec![0u32; n];
        ResultData {
            vis: vec![0; n],
            r: vec![0; n],
            g: vec![0; n],
            b: vec![0; n],
            dark: vec![0.0; n],
            target: vec![0.0; n],
            level: vec![0; n],
            inter: std::array::from_fn(|_| u32s()),
            vert: std::array::from_fn(|_| u32s()),
            statics: vec![ResultLights::default(); n],
            torches: vec![ResultLights::default(); n],
            lit: vec![0; n],
            dirty: vec![1; n],
            chunk_dirty: vec![1; chunks],
        }
    }

    /// `ResultLevel::setDirty`: marks a square and its chunk dirty.
    #[inline]
    pub fn set_dirty(&mut self, w: i32, idx: usize) {
        self.dirty[idx] = 1;
        let i = idx as i32;
        let row = if w != 0 { i / w } else { 0 };
        let col = i - row * w;
        self.chunk_dirty[((row / 8) * (w / 8) + col / 8) as usize] = 1;
    }
}

/// One level of results. The arrays are allocated on first use.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResultLevel {
    /// World square coordinates of index 0.
    pub ox: i32,
    pub oy: i32,
    pub data: Option<Box<ResultData>>,
}

impl ResultLevel {
    pub fn new(ox: i32, oy: i32) -> ResultLevel {
        ResultLevel { ox, oy, data: None }
    }

    #[inline]
    pub fn data_mut(&mut self, w: i32, h: i32) -> &mut ResultData {
        self.data.get_or_insert_with(|| Box::new(ResultData::new(w, h)))
    }
}
