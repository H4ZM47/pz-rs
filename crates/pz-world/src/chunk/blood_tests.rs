use super::*;

fn chunk() -> IsoChunk {
    let mut ch = IsoChunk::new(1, 2);
    ch.set_square(3, 4, 0, Some(IsoGridSquare::new(11, 20, 0).into()));
    ch.set_square(5, 4, 0, Some(IsoGridSquare::new(13, 20, 0).into()));
    ch
}

#[test]
fn blood_splat_lands_on_a_solid_floor_square_of_its_chunk() {
    let mut ch = chunk();
    let solid = |sq: &IsoGridSquare| sq.x == 11;
    // Outside the chunk, on a missing square, on a square without floor.
    assert_eq!(ch.add_blood_splat(7.5, 20.5, 0.0, 3, 1.0, solid), None);
    assert_eq!(ch.add_blood_splat(12.5, 20.5, 0.0, 3, 1.0, solid), None);
    assert_eq!(ch.add_blood_splat(13.5, 20.5, 0.0, 3, 1.0, solid), None);
    assert!(ch.saved.floor_blood_splats.is_empty());
    assert_eq!(ch.add_blood_splat(11.25, 20.75, 0.5, 3, 2.5, solid), Some(0));
    assert_eq!(
        ch.saved.floor_blood_splats,
        vec![FloorBloodSplat { x: 3.25, y: 4.75, z: 0.5, kind: 3, world_age: 2.5, index: 1 }]
    );
}

#[test]
fn small_splats_cycle_their_index_and_the_queue_keeps_the_last_thousand() {
    let mut ch = chunk();
    for i in 0..10 {
        ch.add_blood_splat(11.5, 20.5, 0.0, i % 8, 0.0, |_| true);
    }
    let index: Vec<i8> = ch.saved.floor_blood_splats.iter().map(|b| b.index).collect();
    assert_eq!(index, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(ch.saved.next_splat_index, 0);
    // Large splats (type 8 and up) take no index.
    ch.add_blood_splat(11.5, 20.5, 0.0, 19, 0.0, |_| true);
    assert_eq!(ch.saved.floor_blood_splats.last().unwrap().index, 0);
    assert_eq!(ch.saved.next_splat_index, 0);
    for _ in 0..1000 {
        ch.add_blood_splat(11.5, 20.5, 0.0, 12, 7.0, |_| true);
    }
    assert_eq!(ch.saved.floor_blood_splats.len(), 1000);
    assert!(ch.saved.floor_blood_splats.iter().all(|b| b.kind == 12));
}
