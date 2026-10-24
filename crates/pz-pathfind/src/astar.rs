//! `AStar`: the generic A* driver shared by the low-level (`VGAStar`) and
//! high-level (`HLAStar`) searches.
//!
//! The native library replaced the Java `astar` package's
//! `PriorityQueue`/closed-set implementation with a Detour-style open list
//! (`dtNodeQueue`, a binary heap keyed on a `float` total cost) and
//! open/closed flag bits on each node. That changes tie-breaking and allows
//! closed nodes to be reopened, so the search here follows the native
//! algorithm, not the Java one:
//!
//! * the key of a node is `(float)(floor((h + g) * 1000 + 0.5) / 1000)`,
//!   the multiply-add fused as on arm64;
//! * a successor is (re)queued when it is unvisited or when the new key is
//!   strictly smaller than its stored key, whether it is open or closed;
//! * there is no step limit (`maxSteps` is -1 for both searches).

/// Node is in the open list.
pub const FLAG_OPEN: u8 = 1;
/// Node has been expanded.
pub const FLAG_CLOSED: u8 = 2;

/// The node storage and graph callbacks the search needs (`ISearchNode`
/// plus `IGoalNode::inGoal`).
pub(crate) trait AStarHost {
    fn g(&self, n: u32) -> f64;
    fn set_g(&mut self, n: u32, g: f64);
    fn h(&mut self, n: u32) -> f64;
    fn c(&mut self, from: u32, to: u32) -> f64;
    fn successors(&mut self, n: u32, out: &mut Vec<u32>);
    fn parent(&self, n: u32) -> Option<u32>;
    fn set_parent(&mut self, n: u32, parent: Option<u32>);
    fn depth(&self, n: u32) -> i32;
    fn set_depth(&mut self, n: u32, depth: i32);
    fn total(&self, n: u32) -> f32;
    fn set_total(&mut self, n: u32, total: f32);
    fn flags(&self, n: u32) -> u8;
    fn set_flags(&mut self, n: u32, flags: u8);
    fn in_goal(&self, n: u32) -> bool;
}

/// `dtNodeQueue`.
struct NodeQueue {
    heap: Vec<u32>,
}

impl NodeQueue {
    fn bubble_up<H: AStarHost>(&mut self, host: &H, mut i: usize, node: u32) {
        let total = host.total(node);
        while i > 0 {
            let parent = (i - 1) / 2;
            if host.total(self.heap[parent]) <= total {
                break;
            }
            self.heap[i] = self.heap[parent];
            i = parent;
        }
        self.heap[i] = node;
    }

    fn trickle_down<H: AStarHost>(&mut self, host: &H, mut i: usize, node: u32) {
        let count = self.heap.len();
        let mut child = i * 2 + 1;
        while child < count {
            if child + 1 < count && host.total(self.heap[child + 1]) < host.total(self.heap[child])
            {
                child += 1;
            }
            self.heap[i] = self.heap[child];
            i = child;
            child = i * 2 + 1;
        }
        self.bubble_up(host, i, node);
    }

    fn push<H: AStarHost>(&mut self, host: &H, node: u32) {
        let i = self.heap.len();
        self.heap.push(node);
        self.bubble_up(host, i, node);
    }

    fn pop<H: AStarHost>(&mut self, host: &H) -> u32 {
        let best = self.heap[0];
        let last = self.heap.pop().unwrap();
        if !self.heap.is_empty() {
            self.trickle_down(host, 0, last);
        }
        best
    }
}

/// `AStar::search`: returns the goal node, or `None` when the open list
/// runs dry.
pub(crate) fn search<H: AStarHost>(host: &mut H, start: u32) -> Option<u32> {
    host.set_depth(start, 0);
    let mut queue = NodeQueue { heap: Vec::new() };
    queue.push(host, start);
    let f = host.flags(start);
    host.set_flags(start, (f & 0xf8) | FLAG_OPEN);
    let _ = host.h(start);

    let mut successors: Vec<u32> = Vec::new();
    while !queue.heap.is_empty() {
        let best = queue.pop(host);
        let f = host.flags(best);
        host.set_flags(best, (f & 0xfc) | FLAG_CLOSED);
        if host.in_goal(best) {
            return Some(best);
        }
        successors.clear();
        host.successors(best, &mut successors);
        for i in 0..successors.len() {
            let s = successors[i];
            if Some(s) == host.parent(best) {
                continue;
            }
            let old_flags = host.flags(s);
            let g = host.g(best) + host.c(best, s);
            let h = host.h(s);
            let key = ((h + g).mul_add(1000.0, 0.5).floor() / 1000.0) as f32;
            let open = old_flags & FLAG_OPEN != 0;
            let closed = host.flags(s) & FLAG_CLOSED != 0;
            if (!open || key < host.total(s)) && (!closed || key < host.total(s)) {
                host.set_parent(s, Some(best));
                let d = host.depth(best);
                host.set_depth(s, d + 1);
                let f = host.flags(s);
                host.set_flags(s, f & !FLAG_CLOSED);
                host.set_g(s, g);
                host.set_total(s, key);
                if !open {
                    let f = host.flags(s);
                    host.set_flags(s, f | FLAG_OPEN);
                    queue.push(host, s);
                } else if let Some(idx) = queue.heap.iter().position(|&n| n == s) {
                    queue.bubble_up(host, idx, s);
                }
            }
        }
    }
    None
}

/// `AStar::shortestPath`: the node chain from the start to the goal.
pub(crate) fn shortest_path<H: AStarHost>(host: &mut H, start: u32) -> Option<Vec<u32>> {
    let end = search(host, start)?;
    let mut path = vec![end];
    let mut cur = end;
    while let Some(p) = host.parent(cur) {
        path.push(p);
        if path.len() > 5000 {
            // The native code throws "circular path?".
            return None;
        }
        cur = p;
    }
    path.reverse();
    Some(path)
}
