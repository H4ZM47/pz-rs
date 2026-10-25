//! The region dump the parity examples compare against the oracle
//! harnesses' (`IsoRegionsOracle.dump`): per listed chunk the active levels'
//! flags and chunk regions, then the world regions in first-seen order.

use std::fmt::Write as _;

use pz_world::isoregion::{ChunkRegionRef, IsoRegions, WorldRegionRef};

struct Norm {
    cr: Vec<ChunkRegionRef>,
    wr: Vec<WorldRegionRef>,
}

impl Norm {
    fn cr(&mut self, r: Option<ChunkRegionRef>) -> i64 {
        let Some(r) = r else { return -1 };
        if let Some(i) = self.cr.iter().position(|&x| x == r) {
            return i as i64;
        }
        self.cr.push(r);
        self.cr.len() as i64 - 1
    }
    fn wr(&mut self, r: Option<WorldRegionRef>) -> i64 {
        let Some(r) = r else { return -1 };
        if let Some(i) = self.wr.iter().position(|&x| x == r) {
            return i as i64;
        }
        self.wr.push(r);
        self.wr.len() as i64 - 1
    }
    fn crs(&mut self, l: &[ChunkRegionRef]) -> String {
        let v: Vec<String> = l.iter().map(|&r| self.cr(Some(r)).to_string()).collect();
        format!("[{}]", v.join(","))
    }
    fn wrs(&mut self, l: &[WorldRegionRef]) -> String {
        let v: Vec<String> = l.iter().map(|&r| self.wr(Some(r)).to_string()).collect();
        format!("[{}]", v.join(","))
    }
}

fn jlist(v: impl Iterator<Item = i32>) -> String {
    let v: Vec<String> = v.map(|x| x.to_string()).collect();
    format!("[{}]", v.join(", "))
}

pub fn dump(r: &IsoRegions, order: &[(i32, i32)]) -> String {
    let mut n = Norm {
        cr: Vec::new(),
        wr: Vec::new(),
    };
    let m = r.manager();
    let mut out = String::new();
    writeln!(
        out,
        "counts cr={} wr={} nextId={} idstack={}",
        m.chunk_region_count(),
        m.world_region_count(),
        m.next_id(),
        jlist(m.region_id_stack())
    )
    .unwrap();
    writeln!(
        out,
        "discovered={}",
        jlist(r.discovered_chunks().iter().copied())
    )
    .unwrap();
    writeln!(
        out,
        "threadDiscovered={}",
        jlist(r.thread_discovered_chunks().iter().copied())
    )
    .unwrap();
    for &(wx, wy) in order {
        let Some(dc) = r.get_data_chunk(wx, wy) else {
            writeln!(out, "chunk {wx} {wy} none").unwrap();
            continue;
        };
        writeln!(out, "chunk {wx} {wy} hz={}", dc.highest_z()).unwrap();
        for z in 0..32 {
            if !dc.is_active(z) {
                continue;
            }
            let mut fl = String::new();
            let mut rg = String::new();
            for y in 0..8 {
                for x in 0..8 {
                    write!(fl, "{:02x}", dc.get_square_checked(x, y, z, true) as u8).unwrap();
                    write!(rg, " {}", n.cr(dc.get_iso_chunk_region(x, y, z))).unwrap();
                }
            }
            writeln!(out, " z{z} flags={fl} regions{rg}").unwrap();
            for &cr in dc.chunk_regions(z) {
                let c = r.chunk_region(cr);
                let line = format!(
                    "  cr {} id={} z={} sz={} roof={} border={} encl={} wr={} conn={} all={}",
                    n.cr(Some(cr)),
                    c.id(),
                    c.z_layer(),
                    c.square_size(),
                    c.roof_cnt(),
                    c.chunk_border_squares_cnt(),
                    r.chunk_region_is_enclosed(cr),
                    n.wr(c.iso_world_region()),
                    n.crs(c.connected_neighbors()),
                    n.crs(c.all_neighbors())
                );
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    let mut i = 0;
    while i < n.wr.len() {
        let wr = n.wr[i];
        let w = r.world_region(wr);
        let line = format!(
            "wr {i} id={} sz={} roof={} size={} encl={} fog={} nb={} crs={}",
            w.id(),
            w.square_size(),
            w.roof_cnt(),
            w.size(),
            r.is_enclosed(wr),
            r.is_fog_mask(wr),
            n.wrs(w.neighbors()),
            n.crs(w.chunk_regions())
        );
        out.push_str(&line);
        out.push('\n');
        i += 1;
    }
    out
}
