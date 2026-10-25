//! Synthetic checks of the broadphase port.

use pz_bullet::bt::broadphase::dbvt::*;
use pz_bullet::bt::broadphase::dbvt_broadphase::*;
use pz_bullet::bt::broadphase::pair_cache::*;
use pz_bullet::bt::broadphase::simple_pair_cache::*;
use pz_bullet::bt::linear_math::Vec3;
use pz_bullet::bt::union_find::UnionFind;

fn bx(x: f64, y: f64, z: f64, h: f64) -> (Vec3, Vec3) {
    (
        Vec3::new(x - h, y - h, z - h),
        Vec3::new(x + h, y + h, z + h),
    )
}

#[test]
fn proxies_pair_up_and_separate() {
    let mut bp = DbvtBroadphase::new();
    let (a0, a1) = bx(0.0, 0.0, 0.0, 1.0);
    let (b0, b1) = bx(1.5, 0.0, 0.0, 1.0);
    let (c0, c1) = bx(10.0, 0.0, 0.0, 1.0);
    let a = bp.create_proxy(&a0, &a1, 0, 1, -1);
    let b = bp.create_proxy(&b0, &b1, 1, 1, -1);
    let c = bp.create_proxy(&c0, &c1, 2, 1, -1);
    bp.calculate_overlapping_pairs(None);
    assert_eq!(bp.paircache.get_num_overlapping_pairs(), 1);
    let p = bp.paircache.overlapping_pair_array[0];
    assert_eq!((p.proxy0, p.proxy1), (a, b));
    // move b far away: the pair is cleaned up by collide()
    let (b0, b1) = bx(-20.0, 0.0, 0.0, 1.0);
    bp.set_aabb(b, &b0, &b1);
    for _ in 0..3 {
        bp.calculate_overlapping_pairs(None);
    }
    assert_eq!(bp.paircache.get_num_overlapping_pairs(), 0);
    bp.destroy_proxy(c, None);
    bp.destroy_proxy(b, None);
    bp.destroy_proxy(a, None);
    bp.reset_pool();
    assert!(bp.sets[0].empty() && bp.sets[1].empty());
}

#[test]
fn filter_masks_block_pairs() {
    let mut bp = DbvtBroadphase::new();
    let (a0, a1) = bx(0.0, 0.0, 0.0, 1.0);
    bp.create_proxy(&a0, &a1, 0, 2, !2);
    bp.create_proxy(&a0, &a1, 1, 2, !2);
    assert_eq!(bp.paircache.get_num_overlapping_pairs(), 0);
}

#[test]
fn dbvt_queries() {
    let mut t = Dbvt::new();
    for i in 0..50 {
        let (mn, mx) = bx(i as f64 * 2.0, 0.0, 0.0, 0.6);
        t.insert(&DbvtVolume::from_mm(&mn, &mx), i);
    }
    t.optimize_incremental(-1);
    let (mn, mx) = bx(10.0, 0.0, 0.0, 1.5);
    let mut hits = Vec::new();
    t.collide_tv(t.root, &DbvtVolume::from_mm(&mn, &mx), &mut |n| {
        hits.push(t.node(n).data)
    });
    hits.sort();
    assert_eq!(hits, vec![4, 5, 6]);
    let mut rays = Vec::new();
    t.ray_test(
        t.root,
        &Vec3::new(-5.0, 0.0, 0.0),
        &Vec3::new(5.0, 0.0, 0.0),
        &mut |n| rays.push(t.node(n).data),
    );
    rays.sort();
    assert_eq!(rays, vec![0, 1, 2]);
    assert_eq!(t.count_leaves(t.root), 50);
}

#[test]
fn pair_cache_grows_and_removes() {
    let mut pc = HashedOverlappingPairCache::new();
    let prox: Vec<BroadphaseProxy> = (0..20)
        .map(|i| BroadphaseProxy {
            unique_id: i + 1,
            collision_filter_group: 1,
            collision_filter_mask: -1,
            ..Default::default()
        })
        .collect();
    for i in 0..20 {
        for j in i + 1..20 {
            pc.add_overlapping_pair(i, &prox[i], j, &prox[j]);
        }
    }
    assert_eq!(pc.get_num_overlapping_pairs(), 190);
    assert_eq!(pc.capacity, 256);
    assert!(pc.find_pair(&prox[7], &prox[3]).is_some());
    pc.remove_overlapping_pairs_containing_proxy(3, None);
    assert_eq!(pc.get_num_overlapping_pairs(), 171);
    assert!(pc.find_pair(&prox[7], &prox[3]).is_none());
    let mut sc = HashedSimplePairCache::new();
    sc.add_overlapping_pair(1, 2);
    sc.add_overlapping_pair(2, 1);
    assert_eq!(sc.remove_overlapping_pair(1, 2), Some(None));
    assert_eq!(sc.find_pair(2, 1), Some(0));
}

#[test]
fn union_find_islands() {
    let mut uf = UnionFind::new();
    uf.reset(6);
    uf.unite(0, 3);
    uf.unite(3, 5);
    uf.unite(1, 2);
    uf.sort_islands();
    let ids: Vec<i32> = uf.elements.iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![2, 2, 4, 5, 5, 5]);
}
