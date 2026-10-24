//! `zombie.scripting.itemConfig.enums`.

entity_enum!(
    /// `RootType`.
    RootType,
    "zombie.scripting.itemConfig.enums.RootType",
    [Attribute, FluidContainer, LuaFunc]
);

impl RootType {
    /// `isRequiresId`.
    pub fn is_requires_id(self) -> bool {
        !matches!(self, RootType::LuaFunc)
    }
}

entity_enum!(
    /// `SelectorType`.
    SelectorType,
    "zombie.scripting.itemConfig.enums.SelectorType",
    [Default, Zone, Room, Container, Tile, WorldAge, Situated, Vehicle, OnCreate, None]
);

impl SelectorType {
    /// `isAllowChaining`.
    pub fn is_allow_chaining(self) -> bool {
        matches!(
            self,
            SelectorType::Zone
                | SelectorType::Room
                | SelectorType::Container
                | SelectorType::Tile
                | SelectorType::Vehicle
        )
    }
}

entity_enum!(
    /// `SituatedType`.
    SituatedType,
    "zombie.scripting.itemConfig.enums.SituatedType",
    [None, Interior, Exterior, Shop, Junk]
);
