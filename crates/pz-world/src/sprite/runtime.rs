//! Actual unmanaged `IsoSprite` and pooled `IsoSpriteInstance` allocations.
//! Tile-definition arena indices remain an explicit separate representation.
use super::{IsoSprite, IsoSpriteManager, SpriteTables};
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{EntityOwner, RetiredComponents};
use pz_render::{TextureHostError, TextureRef};
use std::cell::{Cell, Ref, RefCell};
use std::collections::VecDeque;
use std::ops::Deref;
use std::rc::Rc;

pub type SpriteManagerRef = Rc<RefCell<IsoSpriteManager>>;
pub type RuntimeSpriteRef = Rc<RefCell<RuntimeSprite>>;
pub type SpriteInstanceRef = Rc<RefCell<RuntimeSpriteInstance>>;
pub type SpriteTintRef = Rc<RefCell<[f32; 4]>>;
pub type SpriteObserver = fn(&RuntimeSpriteRef);
pub type SpriteInstanceObserver = fn(&SpriteInstanceRef);

/// A single exclusive object sprite slot. Archive IDs are deliberately not
/// usable runtime sprites and never stand in for a freshly allocated sprite.
#[derive(Debug, Default)]
pub enum ObjectSprite {
    #[default]
    None,
    Tile(super::SpriteIdx),
    ArchiveUnmanaged(i32),
    Native(RuntimeSpriteRef),
}
impl ObjectSprite {
    pub fn from_tile(sprite: Option<super::SpriteIdx>) -> Self {
        sprite.map_or(Self::None, Self::Tile)
    }
    pub fn native_slot(&self) -> Option<&RuntimeSpriteRef> {
        match self {
            Self::Native(sprite) => Some(sprite),
            _ => None,
        }
    }
    /// Tile-only data/geometry projection, never a native nullable getter.
    pub fn tile_projection(&self) -> Option<super::SpriteIdx> {
        match self {
            Self::Tile(index) => Some(*index),
            _ => None,
        }
    }
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
    pub fn is_some(&self) -> bool {
        !self.is_none()
    }
    pub fn retained(&self) -> Self {
        match self {
            Self::None => Self::None,
            Self::Tile(index) => Self::Tile(*index),
            Self::ArchiveUnmanaged(id) => Self::ArchiveUnmanaged(*id),
            Self::Native(sprite) => Self::Native(sprite.clone()),
        }
    }
    pub fn try_read<'a>(
        &'a self,
        manager: &'a IsoSpriteManager,
    ) -> HostResult<Option<SpriteRead<'a>>> {
        match self {
            Self::None => Ok(None),
            Self::Tile(index) => manager
                .sprites
                .get(*index)
                .map(|sprite| Some(SpriteRead::Tile(sprite)))
                .ok_or(RuntimeHostError::InvalidHostState(
                    "object tile sprite index",
                )),
            Self::ArchiveUnmanaged(_) => Err(RuntimeHostError::Unsupported {
                operation: "encoded unmanaged sprite has no runtime body",
                kind: None,
            }),
            Self::Native(sprite) => sprite
                .try_borrow()
                .map(|sprite| Some(SpriteRead::Native(sprite)))
                .map_err(|_| RuntimeHostError::Borrowed("actual object sprite")),
        }
    }
    pub fn try_id(&self, manager: &IsoSpriteManager) -> HostResult<i32> {
        match self {
            Self::None => Ok(-1),
            Self::ArchiveUnmanaged(id) => Ok(*id),
            _ => Ok(self.try_read(manager)?.expect("nonnull sprite").id),
        }
    }
    /// Owned data at one geometry/archive read phase, never a persistent
    /// mutable mirror. An archive-only ID carries no property body.
    pub fn metadata_projection(&self, manager: &IsoSpriteManager) -> HostResult<Option<IsoSprite>> {
        if matches!(self, Self::ArchiveUnmanaged(_)) {
            return Ok(None);
        }
        Ok(self.try_read(manager)?.map(|body| (*body).clone()))
    }
    pub fn try_archive_copy(&self) -> HostResult<Self> {
        Ok(match self {
            Self::Native(sprite) => Self::Native(archive_copy_sprite(sprite)?),
            _ => self.retained(),
        })
    }
    pub fn check_archive_copy(&self) -> HostResult<()> {
        if let Self::Native(sprite) = self {
            let body = sprite
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("archive sprite"))?;
            if let Some(tint) = body.tint_slot() {
                drop(
                    tint.try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("archive sprite tint"))?,
                );
            }
            if let Some(def) = body.def_slot() {
                archive_parent_is_self(sprite, def)?;
            }
        }
        Ok(())
    }
}
impl From<Option<super::SpriteIdx>> for ObjectSprite {
    fn from(value: Option<super::SpriteIdx>) -> Self {
        Self::from_tile(value)
    }
}
impl PartialEq<Option<super::SpriteIdx>> for ObjectSprite {
    fn eq(&self, other: &Option<super::SpriteIdx>) -> bool {
        match (self, other) {
            (Self::None, None) => true,
            (Self::Tile(a), Some(b)) => a == b,
            _ => false,
        }
    }
}
impl Clone for ObjectSprite {
    fn clone(&self) -> Self {
        self.try_archive_copy()
            .expect("archive object sprite copy requires unborrowed native fields")
    }
}
impl PartialEq for ObjectSprite {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Tile(a), Self::Tile(b)) => a == b,
            (Self::ArchiveUnmanaged(a), Self::ArchiveUnmanaged(b)) => a == b,
            (Self::Native(a), Self::Native(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}
pub enum SpriteRead<'a> {
    Tile(&'a IsoSprite),
    Native(Ref<'a, RuntimeSprite>),
}
impl Deref for SpriteRead<'_> {
    type Target = IsoSprite;
    fn deref(&self) -> &IsoSprite {
        match self {
            Self::Tile(sprite) => sprite,
            Self::Native(sprite) => &sprite.data,
        }
    }
}

thread_local! {
    static SPRITE_OBSERVER: Cell<Option<SpriteObserver>> = const { Cell::new(None) };
    static INSTANCE_OBSERVER: Cell<Option<SpriteInstanceObserver>> = const { Cell::new(None) };
    static INSTANCE_POOL: RefCell<VecDeque<SpriteInstanceRef>> = const { RefCell::new(VecDeque::new()) };
    static MULTIPLIER: Cell<f32> = const { Cell::new(1.0) };
}

pub fn install_sprite_observer(observer: Option<SpriteObserver>) -> Option<SpriteObserver> {
    SPRITE_OBSERVER.with(|slot| slot.replace(observer))
}
pub fn install_instance_observer(
    observer: Option<SpriteInstanceObserver>,
) -> Option<SpriteInstanceObserver> {
    INSTANCE_OBSERVER.with(|slot| slot.replace(observer))
}
pub fn observe_sprite(sprite: &RuntimeSpriteRef) {
    if let Some(observer) = SPRITE_OBSERVER.with(Cell::get) {
        observer(sprite);
    }
}
pub fn observe_instance(instance: &SpriteInstanceRef) {
    if let Some(observer) = INSTANCE_OBSERVER.with(Cell::get) {
        observer(instance);
    }
}

/// Sole mutable body of a fresh unmanaged sprite. Its properties are the
/// actual fresh PropertyContainer body; no tile arena entry mirrors it.
pub struct RuntimeSprite {
    /// The actual metadata/property body, never mirrored in the tile arena.
    pub data: IsoSprite,
    parent_manager: Option<SpriteManagerRef>,
    tint_mod: Option<SpriteTintRef>,
    def: Option<SpriteInstanceRef>,
    texture: Option<TextureRef>,
}
impl std::fmt::Debug for RuntimeSprite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeSprite")
            .field("data", &self.data)
            .field(
                "parent_manager",
                &self.parent_manager.as_ref().map(Rc::as_ptr),
            )
            .field("def", &self.def.as_ref().map(Rc::as_ptr))
            .field("texture", &self.texture.as_ref().map(Rc::as_ptr))
            .finish()
    }
}
impl RuntimeSprite {
    pub fn parent_manager_slot(&self) -> Option<&SpriteManagerRef> {
        self.parent_manager.as_ref()
    }
    pub fn tint_slot(&self) -> Option<&SpriteTintRef> {
        self.tint_mod.as_ref()
    }
    pub fn def_slot(&self) -> Option<&SpriteInstanceRef> {
        self.def.as_ref()
    }
    pub fn texture_slot(&self) -> Option<&TextureRef> {
        self.texture.as_ref()
    }
    pub fn has_no_textures(&self) -> bool {
        self.texture.is_none()
    }
    /// Collector detachment moves physical slots only. It does not invoke
    /// Dispose, reset, animation disposal or a native pool operation.
    pub fn take_slots_for_gc(&mut self) -> impl std::any::Any {
        (
            self.parent_manager.take(),
            self.tint_mod.take(),
            self.def.take(),
            self.texture.take(),
        )
    }
}

pub struct RuntimeSpriteInstance {
    parent_sprite: Option<RuntimeSpriteRef>,
    pub off_x: f32,
    pub off_y: f32,
    pub off_z: f32,
    pub tint_r: f32,
    pub tint_g: f32,
    pub tint_b: f32,
    pub frame: f32,
    pub alpha: f32,
    pub target_alpha: f32,
    pub copy_target_alpha: bool,
    pub multiply_object_alpha: bool,
    pub flip: bool,
    pub anim_frame_increase: f32,
    pub looped: bool,
    pub finished: bool,
    pub next_frame: bool,
    pub scale_x: f32,
    pub scale_y: f32,
}
impl std::fmt::Debug for RuntimeSpriteInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeSpriteInstance")
            .field(
                "parent_sprite",
                &self.parent_sprite.as_ref().map(Rc::as_ptr),
            )
            .field("scale", &(self.scale_x, self.scale_y))
            .field("frame", &self.frame)
            .finish()
    }
}
impl RuntimeSpriteInstance {
    fn new() -> Self {
        Self {
            parent_sprite: None,
            off_x: 0.0,
            off_y: 0.0,
            off_z: 0.0,
            tint_r: 1.0,
            tint_g: 1.0,
            tint_b: 1.0,
            frame: 0.0,
            alpha: 1.0,
            target_alpha: 1.0,
            copy_target_alpha: true,
            multiply_object_alpha: false,
            flip: false,
            anim_frame_increase: 1.0,
            looped: true,
            finished: false,
            next_frame: false,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }
    pub fn parent_sprite_slot(&self) -> Option<&RuntimeSpriteRef> {
        self.parent_sprite.as_ref()
    }
    pub fn take_parent_for_gc(&mut self) -> Option<RuntimeSpriteRef> {
        self.parent_sprite.take()
    }
    /// Native reset retains the parentSprite allocation.
    pub fn reset(&mut self) {
        self.tint_b = 1.0;
        self.tint_g = 1.0;
        self.tint_r = 1.0;
        self.frame = 0.0;
        self.alpha = 1.0;
        self.target_alpha = 1.0;
        self.copy_target_alpha = true;
        self.multiply_object_alpha = false;
        self.flip = false;
        self.off_z = 0.0;
        self.off_x = 0.0;
        self.off_y = 0.0;
        self.anim_frame_increase = 1.0;
        MULTIPLIER.with(|value| value.set(1.0));
        self.looped = true;
        self.finished = false;
        self.next_frame = false;
        self.scale_x = 1.0;
        self.scale_y = 1.0;
    }
    /// The installed method is exactly an empty return.
    pub fn dispose(&mut self) {}
    pub fn set_scale(&mut self, x: f32, y: f32) {
        self.scale_x = x;
        self.scale_y = y;
    }
    pub fn scale_aspect(
        &mut self,
        source_w: f32,
        source_h: f32,
        mut target_w: f32,
        mut target_h: f32,
    ) {
        // fcmpl/ifle rejects NaN and nonpositive inputs; preserve native
        // multiplication/division order rather than collapsing to one ratio.
        if !(source_w > 0.0 && source_h > 0.0 && target_w > 0.0 && target_h > 0.0) {
            return;
        }
        let width = target_h * source_w / source_h;
        let height = target_w * source_h / source_w;
        if width <= target_w {
            target_w = width;
        } else {
            target_h = height;
        }
        self.scale_x = target_w / source_w;
        self.scale_y = target_h / source_h;
    }
}

/// Explicit Rust archive policy: copy mutable sprite/tint/def bodies, retain
/// the same shared asset and manager allocations, and never touch native pool.
pub fn archive_copy_sprite(source: &RuntimeSpriteRef) -> HostResult<RuntimeSpriteRef> {
    let body = source
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("archive sprite"))?;
    let parent_self = body
        .def
        .as_ref()
        .map(|instance| archive_parent_is_self(source, instance))
        .transpose()?;
    let tint = body
        .tint_mod
        .as_ref()
        .map(|tint| {
            tint.try_borrow()
                .map(|value| Rc::new(RefCell::new(*value)))
                .map_err(|_| RuntimeHostError::Borrowed("archive sprite tint"))
        })
        .transpose()?;
    let def = body
        .def
        .as_ref()
        .map(|instance| {
            instance
                .try_borrow()
                .map(|value| RuntimeSpriteInstance {
                    parent_sprite: None,
                    off_x: value.off_x,
                    off_y: value.off_y,
                    off_z: value.off_z,
                    tint_r: value.tint_r,
                    tint_g: value.tint_g,
                    tint_b: value.tint_b,
                    frame: value.frame,
                    alpha: value.alpha,
                    target_alpha: value.target_alpha,
                    copy_target_alpha: value.copy_target_alpha,
                    multiply_object_alpha: value.multiply_object_alpha,
                    flip: value.flip,
                    anim_frame_increase: value.anim_frame_increase,
                    looped: value.looped,
                    finished: value.finished,
                    next_frame: value.next_frame,
                    scale_x: value.scale_x,
                    scale_y: value.scale_y,
                })
                .map_err(|_| RuntimeHostError::Borrowed("archive sprite instance"))
        })
        .transpose()?;
    let copy = Rc::new(RefCell::new(RuntimeSprite {
        data: body.data.clone(),
        parent_manager: body.parent_manager.clone(),
        tint_mod: tint,
        def: None,
        texture: body.texture.clone(),
    }));
    drop(body);
    observe_sprite(&copy);
    if let Some(def) = def {
        let def = Rc::new(RefCell::new(def));
        observe_instance(&def);
        if parent_self == Some(true) {
            def.borrow_mut().parent_sprite = Some(copy.clone());
        }
        copy.borrow_mut().def = Some(def);
        observe_sprite(&copy);
    }
    Ok(copy)
}

fn archive_parent_is_self(
    source: &RuntimeSpriteRef,
    instance: &SpriteInstanceRef,
) -> HostResult<bool> {
    let instance = instance
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("archive sprite instance"))?;
    match instance.parent_sprite_slot() {
        None => Ok(false),
        Some(parent) if Rc::ptr_eq(parent, source) => Ok(true),
        Some(_) => Err(RuntimeHostError::Unsupported {
            operation: "archive copy of reused pooled sprite instance parent graph",
            kind: None,
        }),
    }
}

pub fn create_runtime_sprite(
    manager: &SpriteManagerRef,
    tables: &SpriteTables,
    retired: &mut RetiredComponents,
) -> HostResult<RuntimeSpriteRef> {
    let data = IsoSprite::new(tables);
    let sprite = Rc::new(RefCell::new(RuntimeSprite {
        data,
        parent_manager: Some(manager.clone()),
        tint_mod: Some(Rc::new(RefCell::new([1.0; 4]))),
        def: None,
        texture: None,
    }));
    retired.push_owner(EntityOwner::from_ref(&sprite));
    // Metadata sees the actual allocation before def->sprite->def cycles.
    observe_sprite(&sprite);
    let instance = get_instance(&sprite, retired)?;
    sprite
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("new sprite def"))?
        .def = Some(instance);
    observe_sprite(&sprite);
    Ok(sprite)
}

pub fn get_instance(
    sprite: &RuntimeSpriteRef,
    retired: &mut RetiredComponents,
) -> HostResult<SpriteInstanceRef> {
    let instance = INSTANCE_POOL
        .with(|pool| {
            pool.try_borrow_mut()
                .map(|mut pool| pool.pop_front())
                .map_err(|_| RuntimeHostError::Borrowed("sprite instance pool"))
        })?
        .unwrap_or_else(|| Rc::new(RefCell::new(RuntimeSpriteInstance::new())));
    retired.push_owner(EntityOwner::from_ref(&instance));
    observe_instance(&instance);
    {
        let mut body = instance
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("sprite instance get"))?;
        if let Some(old) = body.parent_sprite.replace(sprite.clone()) {
            retired.push_owner(EntityOwner::from_ref(&old));
        }
        body.reset();
    }
    observe_instance(&instance);
    Ok(instance)
}

/// Native `add(null)` returns. Reset happens even when the FIFO is full.
pub fn add_instance(
    instance: Option<&SpriteInstanceRef>,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let Some(instance) = instance else {
        return Ok(());
    };
    retired.push_owner(EntityOwner::from_ref(instance));
    instance
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("sprite instance reset"))?
        .reset();
    INSTANCE_POOL.with(|pool| {
        let mut pool = pool
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("sprite instance pool"))?;
        if pool.len() < 2000 {
            pool.push_back(instance.clone());
        }
        Ok(())
    })
}

pub fn texture_error(error: TextureHostError) -> RuntimeHostError {
    match error {
        TextureHostError::Borrowed(operation) => RuntimeHostError::Borrowed(operation),
        TextureHostError::Unavailable(operation) => RuntimeHostError::Unsupported {
            operation,
            kind: None,
        },
    }
}

/// Native LoadSingleTexture disposes animations, then calls the provider,
/// then stores and returns that same nullable actual texture allocation.
/// Fresh unmanaged sprites have no animation map/currentAnim/stack. Animated
/// sprite publication is a separate, currently unsupported method family.
pub fn load_single_texture(
    sprite: &RuntimeSpriteRef,
    name: &str,
    retired: &mut RetiredComponents,
) -> HostResult<Option<TextureRef>> {
    // A real borrow is needed at disposeAnimation, even though its fresh
    // nullable animation fields make this particular phase an empty branch.
    drop(
        sprite
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("sprite disposeAnimation"))?,
    );
    let texture = pz_render::get_shared_texture_ref(name).map_err(texture_error)?;
    if let Some(texture) = &texture {
        retired.push_erased(texture.clone());
    }
    let old = std::mem::replace(
        &mut sprite
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("sprite texture assignment"))?
            .texture,
        texture.clone(),
    );
    if let Some(old) = old {
        retired.push_erased(old);
    }
    observe_sprite(sprite);
    Ok(texture)
}

pub fn set_tint_mod(sprite: &RuntimeSpriteRef, rgba: [f32; 4]) -> HostResult<()> {
    let tint = sprite
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("sprite tint pointer"))?
        .tint_mod
        .clone()
        .ok_or(RuntimeHostError::InvalidHostState("detached sprite tint"))?;
    *tint
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("sprite tint body"))? = rgba;
    Ok(())
}

pub fn set_scale(sprite: &RuntimeSpriteRef, x: f32, y: f32) -> HostResult<()> {
    let def = sprite
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("sprite def pointer"))?
        .def
        .clone()
        .ok_or(RuntimeHostError::InvalidHostState("detached sprite def"))?;
    def.try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("sprite def scale"))?
        .set_scale(x, y);
    Ok(())
}

#[cfg(test)]
mod tests;
