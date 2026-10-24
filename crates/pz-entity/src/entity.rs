//! `GameEntity`'s component storage and `saveEntity`/`loadEntity`.

use pz_scripts::objects::entity::ComponentType;

use crate::components::*;
use crate::context::EntityContext;
use crate::fluids::FluidContainer;
use crate::io::{block_load, err, Reader, Result, Writer};

/// One component's state.
#[derive(Clone, Debug, PartialEq)]
pub enum Component {
    Attributes(AttributeContainer),
    FluidContainer(FluidContainer),
    SpriteConfig(SpriteConfig),
    Script(EntityScriptInfo),
    UiConfig(UiConfig),
    CraftLogic(CraftLogic),
    DryingCraftLogic(DryingCraftLogic),
    MetaTag(MetaTag),
    Resources(Resources),
    CraftBench(CraftBench),
    Durability(Durability),
    SpriteOverlayConfig(SpriteOverlayConfig),
    /// A component whose `save` writes nothing: `Lua`, `Parts`, `Signals`,
    /// `TestComponent`, `CraftRecipe`, `ContextMenuConfig`,
    /// `CraftBenchSounds`, `WallCoveringConfig`.
    Empty(ComponentType),
    /// `FurnaceLogic`, `MashingLogic` and `DryingLogic`: not ported; the
    /// saved bytes are kept as they are.
    Opaque {
        kind: ComponentType,
        data: Vec<u8>,
    },
}

impl Component {
    /// `ComponentType.CreateComponent()`: a fresh (reset) component.
    /// `None` for `Undefined` (no component class).
    pub fn create(kind: ComponentType) -> Option<Component> {
        use ComponentType as T;
        Some(match kind {
            T::Attributes => Component::Attributes(Default::default()),
            T::FluidContainer => Component::FluidContainer(Default::default()),
            T::SpriteConfig => Component::SpriteConfig(Default::default()),
            T::Script => Component::Script(Default::default()),
            T::UiConfig => Component::UiConfig(Default::default()),
            T::CraftLogic => Component::CraftLogic(Default::default()),
            T::DryingCraftLogic => Component::DryingCraftLogic(Default::default()),
            T::MetaTag => Component::MetaTag(Default::default()),
            T::Resources => Component::Resources(Default::default()),
            T::CraftBench => Component::CraftBench(Default::default()),
            T::Durability => Component::Durability(Default::default()),
            T::SpriteOverlayConfig => Component::SpriteOverlayConfig(Default::default()),
            T::Lua
            | T::Parts
            | T::Signals
            | T::TestComponent
            | T::CraftRecipe
            | T::ContextMenuConfig
            | T::CraftBenchSounds
            | T::WallCoveringConfig => Component::Empty(kind),
            T::FurnaceLogic | T::MashingLogic | T::DryingLogic => Component::Opaque {
                kind,
                data: Vec::new(),
            },
            T::Undefined => return None,
        })
    }

    /// `getComponentType()`.
    pub fn kind(&self) -> ComponentType {
        use ComponentType as T;
        match self {
            Component::Attributes(_) => T::Attributes,
            Component::FluidContainer(_) => T::FluidContainer,
            Component::SpriteConfig(_) => T::SpriteConfig,
            Component::Script(_) => T::Script,
            Component::UiConfig(_) => T::UiConfig,
            Component::CraftLogic(_) => T::CraftLogic,
            Component::DryingCraftLogic(_) => T::DryingCraftLogic,
            Component::MetaTag(_) => T::MetaTag,
            Component::Resources(_) => T::Resources,
            Component::CraftBench(_) => T::CraftBench,
            Component::Durability(_) => T::Durability,
            Component::SpriteOverlayConfig(_) => T::SpriteOverlayConfig,
            Component::Empty(k) | Component::Opaque { kind: k, .. } => *k,
        }
    }

    /// `Component.load(input, worldVersion)` over the block body.
    pub fn load(&mut self, r: &mut Reader<'_>, wv: i32, ctx: &dyn EntityContext) -> Result<()> {
        match self {
            Component::Attributes(c) => c.load(r, ctx),
            Component::FluidContainer(c) => c.load(r, ctx),
            Component::SpriteConfig(c) => c.load(r, ctx),
            Component::Script(c) => c.load(r),
            Component::UiConfig(c) => c.load(r),
            Component::CraftLogic(c) => c.load(r, wv, ctx),
            Component::DryingCraftLogic(c) => c.load(r, wv, ctx),
            Component::MetaTag(c) => {
                c.stored_id = r.i64()?;
                Ok(())
            }
            Component::Resources(c) => c.load(r, wv, ctx),
            Component::CraftBench(c) => c.load(r),
            Component::Durability(c) => c.load(r),
            Component::SpriteOverlayConfig(c) => c.load(r, ctx),
            Component::Empty(_) => Ok(()),
            Component::Opaque { data, .. } => {
                *data = r.bytes(r.remaining())?.to_vec();
                Ok(())
            }
        }
    }

    /// `Component.save(output)`.
    pub fn save(&self, w: &mut Writer) {
        match self {
            Component::Attributes(c) => c.save(w),
            Component::FluidContainer(c) => c.save(w),
            Component::SpriteConfig(c) => c.save(w),
            Component::Script(c) => c.save(w),
            Component::UiConfig(c) => c.save(w),
            Component::CraftLogic(c) => c.save(w),
            Component::DryingCraftLogic(c) => c.save(w),
            Component::MetaTag(c) => w.i64(c.stored_id),
            Component::Resources(c) => c.save(w),
            Component::CraftBench(c) => c.save(w),
            Component::Durability(c) => c.save(w),
            Component::SpriteOverlayConfig(c) => c.save(w),
            Component::Empty(_) => {}
            Component::Opaque { data, .. } => w.bytes(data),
        }
    }
}

/// A `GameEntity`'s components (`ComponentContainer`): one per type, in the
/// order they were added.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Components {
    pub list: Vec<Component>,
}

/// What a load did besides filling the components.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LoadInfo {
    /// Bytes read: the count byte and every block.
    pub consumed: usize,
    /// The types loaded, in block order (`loadEntity`'s `loaded` list).
    pub loaded: Vec<ComponentType>,
    /// Blocks the original logs and skips (unknown type, a component whose
    /// load threw) or reads short.
    pub issues: Vec<String>,
}

impl Components {
    pub fn new() -> Self {
        Components::default()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `requiresEntitySave()`.
    pub fn requires_save(&self) -> bool {
        !self.list.is_empty()
    }

    pub fn get(&self, kind: ComponentType) -> Option<&Component> {
        self.list.iter().find(|c| c.kind() == kind)
    }

    pub fn get_mut(&mut self, kind: ComponentType) -> Option<&mut Component> {
        self.list.iter_mut().find(|c| c.kind() == kind)
    }

    pub fn fluid_container(&self) -> Option<&FluidContainer> {
        match self.get(ComponentType::FluidContainer) {
            Some(Component::FluidContainer(f)) => Some(f),
            _ => None,
        }
    }

    /// `addComponent`: ignored when a component of the type exists (the
    /// original logs and returns false), and past 127 components.
    pub fn add(&mut self, c: Component) -> bool {
        if self.list.len() >= 127 || self.get(c.kind()).is_some() {
            return false;
        }
        self.list.push(c);
        true
    }

    /// `GameEntityFactory.AddComponent(entity, replace, component)`.
    pub fn add_replace(&mut self, c: Component, replace: bool) {
        if let Some(i) = self.list.iter().position(|x| x.kind() == c.kind()) {
            if !replace {
                return;
            }
            self.list.remove(i);
        }
        self.list.push(c);
    }

    pub fn remove(&mut self, kind: ComponentType) -> Option<Component> {
        let i = self.list.iter().position(|c| c.kind() == kind)?;
        Some(self.list.remove(i))
    }

    /// `InventoryItem.load`'s follow-up: components that were not in the
    /// save are removed.
    pub fn retain_loaded(&mut self, loaded: &[ComponentType]) {
        self.list.retain(|c| loaded.contains(&c.kind()));
    }
}

/// `GameEntity.saveEntity`: a count byte, then per component a `ByteBlock`
/// holding the type id and the component's `save`.
pub fn save_components(c: &Components) -> Vec<u8> {
    let mut w = Writer::new();
    write_components(&mut w, c);
    w.buf
}

/// [`save_components`] appending to a writer.
pub fn write_components(w: &mut Writer, c: &Components) {
    w.i8(c.list.len() as i8);
    for comp in &c.list {
        let start = w.block_start();
        w.i16(comp.kind().id());
        comp.save(w);
        w.block_end(start);
    }
}

/// `GameEntity.loadEntity` into a fresh entity. `bytes` starts at the count
/// byte and may run past the entity data; [`LoadInfo::consumed`] says where
/// it ended.
pub fn load_components(
    bytes: &[u8],
    world_version: i32,
    ctx: &dyn EntityContext,
) -> Result<(Components, LoadInfo)> {
    let mut c = Components::new();
    let info = load_components_into(&mut c, bytes, world_version, ctx)?;
    Ok((c, info))
}

/// `GameEntity.loadEntity` into an entity that may already have components
/// (the ones its constructor or `GameEntityFactory` made): a saved component
/// of an existing type loads in place; others are created and appended.
pub fn load_components_into(
    comps: &mut Components,
    bytes: &[u8],
    world_version: i32,
    ctx: &dyn EntityContext,
) -> Result<LoadInfo> {
    let mut r = Reader::new(bytes);
    let mut info = LoadInfo::default();
    let count = r.i8()?;
    for _ in 0..count.max(0) {
        // A bad length throws out of loadEntity (outside its try).
        let (start, len) = block_load(&mut r)?;
        if r.remaining() < len {
            return err("BufferUnderflowException");
        }
        let body = &bytes[start..start + len];
        r.seek(start + len)?;
        let mut br = Reader::new(body);
        let id = match br.i16() {
            Ok(id) => id,
            Err(e) => {
                info.issues.push(format!("component block: {e}"));
                continue;
            }
        };
        let kind = ComponentType::from_id(id);
        let existing = comps
            .list
            .iter()
            .position(|c| c.kind() == kind && kind != ComponentType::Undefined);
        let result = match existing {
            Some(i) => comps.list[i]
                .load(&mut br, world_version, ctx)
                .map(|_| None),
            None => match Component::create(kind) {
                None => {
                    info.issues
                        .push(format!("Component with id '{id}' not found."));
                    continue;
                }
                Some(mut c) => c.load(&mut br, world_version, ctx).map(|_| Some(c)),
            },
        };
        match result {
            Ok(created) => {
                if br.remaining() > 0 {
                    info.issues.push(format!(
                        "{}: {} unread bytes skipped",
                        kind.name(),
                        br.remaining()
                    ));
                }
                if let Some(c) = created {
                    comps.add(c);
                }
                info.loaded.push(kind);
            }
            Err(e) => info.issues.push(format!("{}: {e}", kind.name())),
        }
    }
    info.consumed = r.pos();
    Ok(info)
}

/// The length of saved entity data starting at `bytes` (the count byte and
/// every block), without decoding the components.
pub fn entity_len(bytes: &[u8]) -> Result<usize> {
    let mut r = Reader::new(bytes);
    let count = r.i8()?;
    for _ in 0..count.max(0) {
        let (_, len) = block_load(&mut r)?;
        r.bytes(len)?;
    }
    Ok(r.pos())
}
