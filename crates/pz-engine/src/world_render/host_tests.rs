use super::*;

#[test]
fn failed_chunk_input_keeps_current_and_later_notifications_for_retry() {
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};
    let path = std::path::Path::new("unused-render-host-fixture");
    let mut fs = GameFs::new(path);
    let mut textures = TextureManager::new();
    let mut renderer = WorldRender::new(path, &mut fs, &mut textures);
    let mut first = IsoChunk::new(0, 0);
    first.set_square(1, 1, 0, Some(IsoGridSquare::new(1, 1, 0).into()));
    let square = first.get_grid_square(1, 1, 0).unwrap();
    let mut later = IsoChunk::new(1, 0);
    later.set_square(0, 1, 0, Some(IsoGridSquare::new(8, 1, 0).into()));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), first);
    cell.chunks.insert((1, 0), later);
    let meta = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    let mut events = RenderChunkEvents {
        added: vec![(0, 0), (1, 0)],
        removed: vec![],
    };
    let guard = square.borrow_mut();
    assert!(matches!(
        renderer.sync_chunks(&view, &mut events, &fs, &mut textures),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert_eq!(events.added, vec![(0, 0), (1, 0)]);
    assert!(!renderer.world.chunks.contains_key(&(1, 0)));
    drop(guard);
    renderer
        .sync_chunks(&view, &mut events, &fs, &mut textures)
        .unwrap();
    assert!(events.added.is_empty());
    assert!(renderer.world.chunks.contains_key(&(1, 0)));
}
