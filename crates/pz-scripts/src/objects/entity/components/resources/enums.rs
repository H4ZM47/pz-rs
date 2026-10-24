//! The `zombie.entity.components.resources` enums scripts store.

entity_enum!(
    /// `ResourceType`.
    ResourceType,
    "zombie.entity.components.resources.ResourceType",
    [Item, Fluid, Energy, Any]
);

impl ResourceType {
    /// `getId`.
    pub fn id(self) -> i8 {
        match self {
            ResourceType::Item => 1,
            ResourceType::Fluid => 2,
            ResourceType::Energy => 3,
            ResourceType::Any => 0,
        }
    }
}

entity_enum!(
    /// `ResourceIO`.
    ResourceIO,
    "zombie.entity.components.resources.ResourceIO",
    [Input, Output, Any]
);

impl ResourceIO {
    /// `getId`.
    pub fn id(self) -> i8 {
        match self {
            ResourceIO::Input => 1,
            ResourceIO::Output => 2,
            ResourceIO::Any => 3,
        }
    }
}

entity_enum!(
    /// `ResourceChannel`.
    #[allow(non_camel_case_types)]
    ResourceChannel,
    "zombie.entity.components.resources.ResourceChannel",
    [
        NO_CHANNEL, Channel_Red, Channel_Yellow, Channel_Blue, Channel_Orange, Channel_Green, Channel_Purple,
        Channel_Cyan, Channel_Magenta,
    ]
);

impl ResourceChannel {
    /// `getByteId`.
    pub fn byte_id(self) -> i8 {
        self as i8
    }

    /// `getBits`.
    pub fn bits(self) -> i32 {
        match self {
            ResourceChannel::NO_CHANNEL => 0,
            c => 1 << (c as i32 - 1),
        }
    }
}

entity_enum!(
    /// `ResourceFlag`.
    ResourceFlag,
    "zombie.entity.components.resources.ResourceFlag",
    [AutoDecay, Temporary]
);

impl ResourceFlag {
    /// `getByteId`.
    pub fn byte_id(self) -> i8 {
        match self {
            ResourceFlag::AutoDecay => 3,
            ResourceFlag::Temporary => 4,
        }
    }

    /// `getBits`.
    pub fn bits(self) -> i32 {
        match self {
            ResourceFlag::AutoDecay => 4,
            ResourceFlag::Temporary => 8,
        }
    }
}
