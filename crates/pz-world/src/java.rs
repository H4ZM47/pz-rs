//! Small Java/PZ runtime helpers whose exact semantics the map code relies on.

/// `PZMath.fastfloor(float)`.
pub fn fastfloor(val: f32) -> i32 {
    let xi = val as i32;
    if val < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `PZMath.coordmodulo(int, int)` (computed through `float`, like the original).
pub fn coordmodulo(value: i32, divisor: i32) -> i32 {
    value - fastfloor(value as f32 / divisor as f32) * divisor
}

/// `String.trim()`: strips leading/trailing chars `<= ' '`.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.hashCode()` over UTF-16 code units.
pub fn string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(u as i32);
    }
    h
}

/// Iteration order of a `java.util.HashMap<String, _>` (default capacity and
/// load factor) after inserting `keys` in order without removals.
///
/// Buckets are visited by index; within a bucket entries keep insertion order
/// (resizes split buckets order-preservingly). Treeified bins (more than 8
/// colliding keys in one bucket) are not modelled.
pub fn hashmap_order<'a>(keys: &[&'a str]) -> Vec<&'a str> {
    let mut cap: usize = 16;
    while keys.len() as f64 > cap as f64 * 0.75 {
        cap *= 2;
    }
    let mut indexed: Vec<(usize, usize, &str)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let h = string_hash(k);
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (cap - 1), i, *k)
        })
        .collect();
    indexed.sort_by_key(|&(bucket, i, _)| (bucket, i));
    indexed.into_iter().map(|(_, _, k)| k).collect()
}

/// The key order of a `java.util.HashMap<Integer, _>` with default capacity
/// and load factor, following its insertions (no removals): resizes, list
/// bins, and the red-black tree bins buckets with more than eight keys turn
/// into (`treeifyBin`, `putTreeVal`, `moveRootToFront`, `split`,
/// `untreeify`), whose iteration order differs from insertion order.
#[derive(Clone, Debug, Default)]
pub struct JavaIntHashMap {
    nodes: Vec<HNode>,
    table: Vec<Option<usize>>,
    size: usize,
    threshold: usize,
}

#[derive(Clone, Debug)]
struct HNode {
    hash: i32,
    key: i32,
    next: Option<usize>,
    // Tree bins only.
    tree: bool,
    prev: Option<usize>,
    parent: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    red: bool,
}

const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;

impl JavaIntHashMap {
    fn spread(key: i32) -> i32 {
        key ^ ((key as u32) >> 16) as i32
    }

    /// `put(key, v)` for a key not in the map yet (a present key is left
    /// alone, as `put` keeps its position).
    pub fn insert(&mut self, key: i32) {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = Self::spread(key);
        let n = self.table.len();
        let i = (hash as u32 as usize) & (n - 1);
        let new = self.nodes.len();
        match self.table[i] {
            None => {
                self.push_node(hash, key, None);
                self.table[i] = Some(new);
            }
            Some(head) => {
                if self.nodes[head].key == key {
                    return;
                }
                if self.nodes[head].tree {
                    if self.contains_in_bin(head, key) {
                        return;
                    }
                    self.put_tree_val(i, hash, key);
                } else {
                    let mut p = head;
                    let mut bin_count = 0;
                    loop {
                        match self.nodes[p].next {
                            None => {
                                self.push_node(hash, key, None);
                                self.nodes[p].next = Some(new);
                                if bin_count >= TREEIFY_THRESHOLD - 1 {
                                    self.treeify_bin(hash);
                                }
                                break;
                            }
                            Some(e) => {
                                if self.nodes[e].key == key {
                                    return;
                                }
                                p = e;
                            }
                        }
                        bin_count += 1;
                    }
                }
            }
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
    }

    /// The keys in iteration order.
    pub fn order(&self) -> Vec<i32> {
        let mut out = Vec::with_capacity(self.size);
        for b in &self.table {
            let mut e = *b;
            while let Some(i) = e {
                out.push(self.nodes[i].key);
                e = self.nodes[i].next;
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    fn contains_in_bin(&self, head: usize, key: i32) -> bool {
        let mut e = Some(head);
        while let Some(i) = e {
            if self.nodes[i].key == key {
                return true;
            }
            e = self.nodes[i].next;
        }
        false
    }

    fn push_node(&mut self, hash: i32, key: i32, next: Option<usize>) -> usize {
        self.nodes.push(HNode {
            hash,
            key,
            next,
            tree: false,
            prev: None,
            parent: None,
            left: None,
            right: None,
            red: false,
        });
        self.nodes.len() - 1
    }

    fn index(&self, hash: i32) -> usize {
        (hash as u32 as usize) & (self.table.len() - 1)
    }

    fn resize(&mut self) {
        let old_cap = self.table.len();
        let new_cap = if old_cap == 0 { 16 } else { old_cap * 2 };
        self.threshold = new_cap * 3 / 4;
        let old = std::mem::replace(&mut self.table, vec![None; new_cap]);
        for (j, b) in old.into_iter().enumerate() {
            let Some(e) = b else {
                continue;
            };
            if self.nodes[e].next.is_none() {
                let idx = self.index(self.nodes[e].hash);
                self.table[idx] = Some(e);
            } else if self.nodes[e].tree {
                self.split(e, j, old_cap);
            } else {
                let (lo, hi) = self.split_list(e, old_cap);
                self.table[j] = lo.first().copied();
                self.table[j + old_cap] = hi.first().copied();
            }
        }
    }

    /// Splits the `next` chain from `e` by `hash & bit`, relinking each
    /// half in order.
    fn split_list(&mut self, e: usize, bit: usize) -> (Vec<usize>, Vec<usize>) {
        let mut lo = Vec::new();
        let mut hi = Vec::new();
        let mut cur = Some(e);
        while let Some(i) = cur {
            cur = self.nodes[i].next;
            if (self.nodes[i].hash as u32 as usize) & bit == 0 {
                lo.push(i);
            } else {
                hi.push(i);
            }
        }
        for half in [&lo, &hi] {
            for (k, &i) in half.iter().enumerate() {
                self.nodes[i].next = half.get(k + 1).copied();
                self.nodes[i].prev = if k == 0 { None } else { Some(half[k - 1]) };
            }
        }
        (lo, hi)
    }

    /// `TreeNode.split(map, tab, index, bit)`.
    fn split(&mut self, b: usize, index: usize, bit: usize) {
        let (lo, hi) = self.split_list(b, bit);
        if let Some(&lo_head) = lo.first() {
            if lo.len() <= UNTREEIFY_THRESHOLD {
                self.untreeify(&lo);
                self.table[index] = Some(lo_head);
            } else {
                self.table[index] = Some(lo_head);
                if !hi.is_empty() {
                    self.treeify(lo_head);
                }
            }
        }
        if let Some(&hi_head) = hi.first() {
            if hi.len() <= UNTREEIFY_THRESHOLD {
                self.untreeify(&hi);
                self.table[index + bit] = Some(hi_head);
            } else {
                self.table[index + bit] = Some(hi_head);
                if !lo.is_empty() {
                    self.treeify(hi_head);
                }
            }
        }
    }

    fn untreeify(&mut self, list: &[usize]) {
        for &i in list {
            let n = &mut self.nodes[i];
            n.tree = false;
            n.prev = None;
            n.parent = None;
            n.left = None;
            n.right = None;
            n.red = false;
        }
    }

    fn treeify_bin(&mut self, hash: i32) {
        if self.table.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
            return;
        }
        let index = self.index(hash);
        let Some(head) = self.table[index] else {
            return;
        };
        let mut prev = None;
        let mut e = Some(head);
        while let Some(i) = e {
            self.nodes[i].tree = true;
            self.nodes[i].prev = prev;
            prev = Some(i);
            e = self.nodes[i].next;
        }
        self.treeify(head);
    }

    /// `TreeNode.treeify(tab)` from bin head `hd`.
    fn treeify(&mut self, hd: usize) {
        let mut root: Option<usize> = None;
        let mut x = Some(hd);
        while let Some(xi) = x {
            let next = self.nodes[xi].next;
            self.nodes[xi].left = None;
            self.nodes[xi].right = None;
            match root {
                None => {
                    self.nodes[xi].parent = None;
                    self.nodes[xi].red = false;
                    root = Some(xi);
                }
                Some(r) => {
                    let h = self.nodes[xi].hash;
                    let mut p = r;
                    loop {
                        let dir_left = self.nodes[p].hash > h;
                        let xp = p;
                        let child = if dir_left {
                            self.nodes[p].left
                        } else {
                            self.nodes[p].right
                        };
                        match child {
                            Some(c) => p = c,
                            None => {
                                self.nodes[xi].parent = Some(xp);
                                if dir_left {
                                    self.nodes[xp].left = Some(xi);
                                } else {
                                    self.nodes[xp].right = Some(xi);
                                }
                                root = Some(self.balance_insertion(r, xi));
                                break;
                            }
                        }
                    }
                }
            }
            x = next;
        }
        self.move_root_to_front(root.unwrap());
    }

    fn put_tree_val(&mut self, index: usize, h: i32, key: i32) {
        let mut root = self.table[index].unwrap();
        while let Some(p) = self.nodes[root].parent {
            root = p;
        }
        let mut p = root;
        loop {
            let dir_left = self.nodes[p].hash > h;
            let xp = p;
            let child = if dir_left {
                self.nodes[p].left
            } else {
                self.nodes[p].right
            };
            match child {
                Some(c) => p = c,
                None => {
                    let xpn = self.nodes[xp].next;
                    let x = self.push_node(h, key, xpn);
                    self.nodes[x].tree = true;
                    if dir_left {
                        self.nodes[xp].left = Some(x);
                    } else {
                        self.nodes[xp].right = Some(x);
                    }
                    self.nodes[xp].next = Some(x);
                    self.nodes[x].parent = Some(xp);
                    self.nodes[x].prev = Some(xp);
                    if let Some(n) = xpn {
                        self.nodes[n].prev = Some(x);
                    }
                    let r = self.balance_insertion(root, x);
                    self.move_root_to_front(r);
                    return;
                }
            }
        }
    }

    fn move_root_to_front(&mut self, root: usize) {
        let index = self.index(self.nodes[root].hash);
        let first = self.table[index];
        if first == Some(root) {
            return;
        }
        self.table[index] = Some(root);
        let rp = self.nodes[root].prev;
        let rn = self.nodes[root].next;
        if let Some(rn) = rn {
            self.nodes[rn].prev = rp;
        }
        if let Some(rp) = rp {
            self.nodes[rp].next = rn;
        }
        if let Some(f) = first {
            self.nodes[f].prev = Some(root);
        }
        self.nodes[root].next = first;
        self.nodes[root].prev = None;
    }

    fn rotate_left(&mut self, mut root: usize, p: usize) -> usize {
        if let Some(r) = self.nodes[p].right {
            let rl = self.nodes[r].left;
            self.nodes[p].right = rl;
            if let Some(rl) = rl {
                self.nodes[rl].parent = Some(p);
            }
            let pp = self.nodes[p].parent;
            self.nodes[r].parent = pp;
            match pp {
                None => {
                    root = r;
                    self.nodes[r].red = false;
                }
                Some(pp) => {
                    if self.nodes[pp].left == Some(p) {
                        self.nodes[pp].left = Some(r);
                    } else {
                        self.nodes[pp].right = Some(r);
                    }
                }
            }
            self.nodes[r].left = Some(p);
            self.nodes[p].parent = Some(r);
        }
        root
    }

    fn rotate_right(&mut self, mut root: usize, p: usize) -> usize {
        if let Some(l) = self.nodes[p].left {
            let lr = self.nodes[l].right;
            self.nodes[p].left = lr;
            if let Some(lr) = lr {
                self.nodes[lr].parent = Some(p);
            }
            let pp = self.nodes[p].parent;
            self.nodes[l].parent = pp;
            match pp {
                None => {
                    root = l;
                    self.nodes[l].red = false;
                }
                Some(pp) => {
                    if self.nodes[pp].right == Some(p) {
                        self.nodes[pp].right = Some(l);
                    } else {
                        self.nodes[pp].left = Some(l);
                    }
                }
            }
            self.nodes[l].right = Some(p);
            self.nodes[p].parent = Some(l);
        }
        root
    }

    /// `TreeNode.balanceInsertion(root, x)`.
    fn balance_insertion(&mut self, mut root: usize, mut x: usize) -> usize {
        self.nodes[x].red = true;
        loop {
            let Some(xp) = self.nodes[x].parent else {
                self.nodes[x].red = false;
                return x;
            };
            if !self.nodes[xp].red {
                return root;
            }
            let Some(xpp) = self.nodes[xp].parent else {
                return root;
            };
            let xppl = self.nodes[xpp].left;
            if Some(xp) == xppl {
                let xppr = self.nodes[xpp].right;
                if let Some(u) = xppr.filter(|&u| self.nodes[u].red) {
                    self.nodes[u].red = false;
                    self.nodes[xp].red = false;
                    self.nodes[xpp].red = true;
                    x = xpp;
                } else {
                    let (mut xp, mut xpp) = (Some(xp), Some(xpp));
                    if self.nodes[xp.unwrap()].right == Some(x) {
                        x = xp.unwrap();
                        root = self.rotate_left(root, x);
                        xp = self.nodes[x].parent;
                        xpp = xp.and_then(|p| self.nodes[p].parent);
                    }
                    if let Some(xp) = xp {
                        self.nodes[xp].red = false;
                        if let Some(xpp) = xpp {
                            self.nodes[xpp].red = true;
                            root = self.rotate_right(root, xpp);
                        }
                    }
                }
            } else if let Some(u) = xppl.filter(|&u| self.nodes[u].red) {
                self.nodes[u].red = false;
                self.nodes[xp].red = false;
                self.nodes[xpp].red = true;
                x = xpp;
            } else {
                let (mut xp, mut xpp) = (Some(xp), Some(xpp));
                if self.nodes[xp.unwrap()].left == Some(x) {
                    x = xp.unwrap();
                    root = self.rotate_right(root, x);
                    xp = self.nodes[x].parent;
                    xpp = xp.and_then(|p| self.nodes[p].parent);
                }
                if let Some(xp) = xp {
                    self.nodes[xp].red = false;
                    if let Some(xpp) = xpp {
                        self.nodes[xpp].red = true;
                        root = self.rotate_left(root, xpp);
                    }
                }
            }
        }
    }
}

/// `String.equalsIgnoreCase` (per UTF-16 unit: equal, or equal after
/// upper-casing, or equal after upper- then lower-casing).
pub fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    let (a, b): (Vec<u16>, Vec<u16>) = (a.encode_utf16().collect(), b.encode_utf16().collect());
    if a.len() != b.len() {
        return false;
    }
    fn map(u: u16, f: impl Fn(char) -> Option<char>) -> u16 {
        char::from_u32(u as u32)
            .and_then(f)
            .and_then(|c| u16::try_from(c as u32).ok())
            .unwrap_or(u)
    }
    let upper = |c: char| {
        let mut it = c.to_uppercase();
        let r = it.next();
        if it.next().is_some() {
            Some(c)
        } else {
            r
        }
    };
    let lower = |c: char| {
        let mut it = c.to_lowercase();
        let r = it.next();
        if it.next().is_some() {
            Some(c)
        } else {
            r
        }
    };
    a.iter().zip(&b).all(|(&x, &y)| {
        if x == y {
            return true;
        }
        let (ux, uy) = (map(x, upper), map(y, upper));
        ux == uy || map(ux, lower) == map(uy, lower)
    })
}

/// `Boolean.parseBoolean`.
pub fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

/// Random source matching `zombie.core.random.Rand` (`RandAbstract` over a
/// `java.util.Random`-like generator). The engine supplies the real
/// generator; map loading only consumes it.
pub trait GameRand {
    /// `Rand.Next(int max)`: `nextInt(max)`, or 0 without drawing when
    /// `max <= 0`.
    fn next(&mut self, max: i32) -> i32;

    /// The generator's `nextFloat()` in `[0, 1)`. Generators that are never
    /// asked for floats may keep the default, which draws nothing.
    fn next_float(&mut self) -> f32 {
        0.0
    }

    /// `Rand.Next(int min, int max)`.
    fn next_range(&mut self, min: i32, max: i32) -> i32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.next(max.wrapping_sub(min)).wrapping_add(min)
    }

    /// `Rand.Next(float min, float max)`.
    fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        min + self.next_float() * (max - min)
    }
}

/// `Integer.parseInt(s)`: an optional sign and decimal digits.
pub fn parse_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `Float.parseFloat(s)` for decimal input: surrounding whitespace is
/// ignored and a trailing `f`/`d` type suffix is accepted.
pub fn parse_float(s: &str) -> Option<f32> {
    let t = java_trim(s);
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    if t.is_empty()
        || t.bytes()
            .any(|b| !(b.is_ascii_digit() || b"+-.eE".contains(&b)))
    {
        return None;
    }
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_modulo() {
        assert_eq!(fastfloor(-0.5), -1);
        assert_eq!(fastfloor(-1.0), -1);
        assert_eq!(fastfloor(3.9), 3);
        assert_eq!(coordmodulo(-1, 8), 7);
        assert_eq!(coordmodulo(256, 300), 256);
        assert_eq!(coordmodulo(600, 300), 0);
    }

    #[test]
    fn hashes() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("abc"), 96354);
        assert_eq!(java_trim(" \t a b\r\n"), "a b");
        assert!(java_equals_ignore_case("ParkingStall", "parkingSTALL"));
        assert!(!java_equals_ignore_case("Farm", "FarmLand"));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_int("-12"), Some(-12));
        assert_eq!(parse_int("+3"), Some(3));
        assert_eq!(parse_int(" 3"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_float(" 255 "), Some(255.0));
        assert_eq!(parse_float("1.5f"), Some(1.5));
        assert_eq!(parse_float("x"), None);
    }

    struct Seq(Vec<i32>);
    impl GameRand for Seq {
        fn next(&mut self, max: i32) -> i32 {
            assert!(max > 0);
            self.0.remove(0)
        }
        fn next_float(&mut self) -> f32 {
            0.5
        }
    }

    #[test]
    fn rand_ranges() {
        let mut r = Seq(vec![3]);
        assert_eq!(r.next_range(5, 5), 5);
        assert_eq!(r.next_range(10, 2), 5);
        assert!(r.0.is_empty());
        assert_eq!(r.next_float_range(1.0, 0.0), 0.5);
    }

    /// Key orders `java.util.HashMap<Integer, _>` gives for these insertions
    /// (checked against the JDK): a bucket that turns into a tree with its
    /// root moved to the front, a tree built from descending keys, a tree
    /// split by a resize, and chunk-hash keys.
    #[test]
    fn int_hashmap_order_matches_java() {
        let run = |keys: &[i32]| {
            let mut m = JavaIntHashMap::default();
            for &k in keys {
                m.insert(k);
            }
            m.order()
        };
        let fill: Vec<i32> = (1..=25).collect();
        let mut k1 = fill.clone();
        k1.extend((1..=9).map(|i| 64 * i));
        assert_eq!(
            run(&k1),
            vec![
                256, 64, 128, 192, 320, 384, 448, 512, 576, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
                13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25
            ]
        );
        let mut k2 = fill.clone();
        k2.extend((1..=12).rev().map(|i| 64 * i));
        assert_eq!(
            run(&k2),
            vec![
                576, 768, 704, 640, 512, 448, 384, 320, 256, 192, 128, 64, 1, 2, 3, 4, 5, 6, 7, 8,
                9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25
            ]
        );
        let mut k3 = fill.clone();
        k3.extend((0..14).map(|i| 128 * (i + 1) + (i % 2) * 64));
        k3.extend((0..21).map(|i| 1000 + i));
        assert_eq!(
            run(&k3),
            vec![
                384, 128, 640, 896, 1152, 1408, 1664, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,
                14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 576, 320, 832, 1088, 1344, 1600,
                1856, 1000, 1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012,
                1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020
            ]
        );
        let k4: Vec<i32> = (0..200)
            .map(|i| ((1250 + i / 13) << 16) ^ (1350 + i % 13))
            .collect();
        assert_eq!(
            run(&k4)[..32],
            [
                82380105, 82183494, 82249031, 82314568, 82445642, 82511179, 82576716, 82642253,
                82707790, 82773327, 82838864, 82380104, 82183495, 82249030, 82314569, 82445643,
                82511178, 82576717, 82642252, 82707791, 82773326, 82838865, 82380107, 82052422,
                82117959, 82314570, 82445640, 82511177, 82576718, 82642255, 82707788, 82773325
            ]
        );
    }

    #[test]
    fn hashmap_order_by_bucket() {
        // "b" (98) lands in bucket 2, "a" (97) in bucket 1.
        assert_eq!(hashmap_order(&["b", "a"]), vec!["a", "b"]);
    }
}
