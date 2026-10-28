//! `zombie.entity.ComponentType`: the component kinds, their ids and flags,
//! and the script class each one loads with (`CreateComponentScript`).

use super::components::{
    attributes::AttributesScript, combat::DurabilityScript, contextmenuconfig::ContextMenuConfigScript, crafting::*,
    fluids::FluidContainerScript, lua::LuaComponentScript, parts::PartsScript, resources::ResourcesScript,
    signals::SignalsScript, sound::CraftBenchSoundsScript, spriteconfig::SpriteConfigScript,
    spriteconfig::SpriteOverlayConfigScript, test::TestComponentScript, ui::UiConfigScript,
};
use super::{runtime_exception, ComponentScript};
use crate::base::ScriptResult;

entity_enum!(
    /// `zombie.entity.ComponentType`.
    ComponentType,
    "zombie.entity.ComponentType",
    [
        Attributes, FluidContainer, SpriteConfig, Lua, Parts, Signals, Script, UiConfig, CraftLogic, FurnaceLogic,
        TestComponent, MashingLogic, DryingLogic, MetaTag, Resources, CraftBench, CraftRecipe, Durability,
        DryingCraftLogic, ContextMenuConfig, SpriteOverlayConfig, CraftBenchSounds, WallCoveringConfig, Undefined,
    ]
);

impl ComponentType {
    /// `GetID`.
    pub fn id(self) -> i16 {
        use ComponentType::*;
        match self {
            Attributes => 1,
            FluidContainer => 2,
            SpriteConfig => 3,
            Lua => 6,
            Parts => 7,
            Signals => 8,
            Script => 9,
            UiConfig => 11,
            CraftLogic => 12,
            FurnaceLogic => 13,
            TestComponent => 14,
            MashingLogic => 15,
            DryingLogic => 16,
            MetaTag => 17,
            Resources => 18,
            CraftBench => 19,
            CraftRecipe => 20,
            Durability => 21,
            DryingCraftLogic => 22,
            ContextMenuConfig => 23,
            SpriteOverlayConfig => 24,
            CraftBenchSounds => 25,
            WallCoveringConfig => 26,
            Undefined => 0,
        }
    }

    fn flags(self) -> i32 {
        use ComponentType::*;
        match self {
            FluidContainer => 2,
            CraftLogic | FurnaceLogic | MashingLogic | DryingLogic | Resources | DryingCraftLogic => 3,
            _ => 0,
        }
    }

    /// `isAddToEngine`.
    pub fn is_add_to_engine(self) -> bool {
        self.flags() & 1 != 0
    }

    /// `isRunInMeta`.
    pub fn is_run_in_meta(self) -> bool {
        self.flags() & 2 != 0
    }

    /// `isRenderLast`.
    pub fn is_render_last(self) -> bool {
        self.flags() & 4 != 0
    }

    /// `FromId`: `Undefined` for unknown ids.
    pub fn from_id(id: i16) -> ComponentType {
        Self::ALL.iter().copied().find(|t| t.id() == id).unwrap_or(ComponentType::Undefined)
    }

    /// `GetList`: every type except `Undefined`.
    pub fn list() -> &'static [ComponentType] {
        &Self::ALL[..Self::ALL.len() - 1]
    }

    /// `CreateComponentScript`.
    pub fn create_component_script(self) -> ScriptResult<Box<dyn ComponentScript>> {
        use ComponentType as T;
        Ok(match self {
            T::Attributes => Box::new(AttributesScript::new()),
            T::FluidContainer => Box::new(FluidContainerScript::new()),
            T::SpriteConfig => Box::new(SpriteConfigScript::new()),
            T::Lua => Box::new(LuaComponentScript::new()),
            T::Parts => Box::new(PartsScript::new()),
            T::Signals => Box::new(SignalsScript::new()),
            T::UiConfig => Box::new(UiConfigScript::new()),
            T::CraftLogic => Box::new(CraftLogicScript::new(T::CraftLogic)),
            T::FurnaceLogic => Box::new(FurnaceLogicScript::new()),
            T::TestComponent => Box::new(TestComponentScript::new()),
            T::MashingLogic => Box::new(MashingLogicScript::new()),
            T::DryingLogic => Box::new(DryingLogicScript::new()),
            T::Resources => Box::new(ResourcesScript::new()),
            T::CraftBench => Box::new(CraftBenchScript::new()),
            T::CraftRecipe => Box::new(CraftRecipeComponentScript::new()),
            T::Durability => Box::new(DurabilityScript::new()),
            T::DryingCraftLogic => Box::new(CraftLogicScript::new(T::DryingCraftLogic)),
            T::ContextMenuConfig => Box::new(ContextMenuConfigScript::new()),
            T::SpriteOverlayConfig => Box::new(SpriteOverlayConfigScript::new()),
            T::CraftBenchSounds => Box::new(CraftBenchSoundsScript::new()),
            T::WallCoveringConfig => Box::new(WallCoveringConfigScript::new()),
            T::Script | T::MetaTag | T::Undefined => {
                return Err(runtime_exception(format!(
                    "Unable to create script for component (No script class defined?): {}",
                    self.name()
                )))
            }
        })
    }
}
