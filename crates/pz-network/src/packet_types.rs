//! `zombie.network.PacketTypes`: the game's packet type table.
//!
//! Every game message travels as a RakNet user packet: the byte
//! [`ID_USER_PACKET`] followed by the packet type's id as a big-endian
//! `short`, then the packet body (`PacketType.doPacket`). The id is the
//! type's position in the table (`PacketType.getId` returns the ordinal), so
//! the table order is part of the wire format and must match the original.
//!
//! Each type also carries the RakNet send settings the original passes to
//! `UdpConnection.endPacket` (priority, reliability, ordering channel), the
//! capability a connection needs before the server handles it, and which
//! sides handle it at all (`PacketSetting.HandlingType`).

use crate::capability::Capability;

/// RakNet `ID_USER_PACKET_ENUM` as built into the game's RakNet library. Every
/// game packet starts with this byte (`PacketType.doPacket` writes 134).
pub const ID_USER_PACKET: u8 = 134;

/// The int `PacketTypes.doPingPacket` writes.
pub const PING_PACKET_VALUE: i32 = 28;

/// `PacketTypes.PacketOrdering_*` / `ORDERING_*`: RakNet ordering channels.
pub mod ordering {
    pub const GENERAL: u8 = 0;
    pub const ITEMS: u8 = 1;
    pub const OBJECT: u8 = 3;
    pub const MAP: u8 = 4;
    pub const PLAYER: u8 = 5;
    pub const ANIMAL: u8 = 7;
    pub const VEHICLE: u8 = 8;
    pub const PLAYER_INJURIES: u8 = 9;
    pub const PLAYER_DAMAGE: u8 = 10;
    pub const PLAYER_XP: u8 = 11;
    pub const PLAYER_STATS: u8 = 12;
    pub const PLAYER_EFFECTS: u8 = 13;
    pub const PLAYER_HEALTH: u8 = 14;
    pub const MAX: u8 = 31;
}

/// `PacketSetting.HandlingType` bits: which side runs a received packet.
pub mod handling {
    pub const NONE: u8 = 0;
    pub const SERVER: u8 = 1;
    pub const CLIENT: u8 = 2;
    pub const CLIENT_LOADING: u8 = 4;
    pub const ALL: u8 = 7;

    /// `HandlingType.getType`.
    pub fn get_type(server: bool, client: bool, client_loading: bool) -> u8 {
        (server as u8 * SERVER) | (client as u8 * CLIENT) | (client_loading as u8 * CLIENT_LOADING)
    }
}

/// RakNet `PacketPriority`, as the int the game passes to the native send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketPriority {
    Immediate = 0,
    High = 1,
    Medium = 2,
    Low = 3,
}

impl PacketPriority {
    pub fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::Immediate,
            1 => Self::High,
            2 => Self::Medium,
            3 => Self::Low,
            _ => return None,
        })
    }
}

/// RakNet `PacketReliability`, limited to the five values the game uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketReliability {
    Unreliable = 0,
    UnreliableSequenced = 1,
    Reliable = 2,
    ReliableOrdered = 3,
    ReliableSequenced = 4,
}

impl PacketReliability {
    pub fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::Unreliable,
            1 => Self::UnreliableSequenced,
            2 => Self::Reliable,
            3 => Self::ReliableOrdered,
            4 => Self::ReliableSequenced,
            _ => return None,
        })
    }

    pub fn is_reliable(self) -> bool {
        matches!(
            self,
            Self::Reliable | Self::ReliableOrdered | Self::ReliableSequenced
        )
    }

    pub fn is_sequenced(self) -> bool {
        matches!(self, Self::UnreliableSequenced | Self::ReliableSequenced)
    }

    pub fn is_ordered(self) -> bool {
        matches!(self, Self::ReliableOrdered)
    }
}

/// `zombie.network.anticheats.AntiCheat` constants, in declaration order.
/// Only the names live here; the checks themselves belong to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AntiCheat {
    Safety,
    HitDamage,
    HitVehicle,
    HitLongDistance,
    HitShortDistance,
    HitWeapon,
    PacketRakNet,
    PacketException,
    PacketType,
    XPUpdate,
    ChecksumUpdate,
    Target,
    Player,
    PlayerUpdate,
    Power,
    Capability,
    SafeHouseOwner,
    SafeHouseMember,
    SafeHousePlayer,
    Speed,
    NoClip,
    None,
}

/// The fixed settings of one packet type (`PacketType`'s fields after its
/// constructor ran).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketSetting {
    pub priority: PacketPriority,
    pub reliability: PacketReliability,
    pub ordering: u8,
    pub required_capability: Capability,
    /// [`handling`] bits.
    pub handling_type: u8,
    pub is_cached: bool,
    /// Anti-cheat checks the server runs before `processServer`. Empty for
    /// packet types whose setting keeps the default `{AntiCheat.None}` and
    /// for callback-only types, which have none.
    pub anticheats: &'static [AntiCheat],
}

macro_rules! packet_types {
    ($($name:ident: $pr:ident, $rel:ident, $ord:expr, $cap:ident, $h:expr, $cached:expr, [$($ac:expr),*];)*) => {
        /// `PacketTypes.PacketType`. The discriminant is the wire id.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u16)]
        pub enum PacketType {
            $($name,)*
        }

        impl PacketType {
            /// `PacketType.values()`.
            pub const ALL: &'static [PacketType] = &[$(PacketType::$name,)*];

            /// `Enum.name()`.
            pub fn name(self) -> &'static str {
                match self {
                    $(PacketType::$name => stringify!($name),)*
                }
            }

            /// The type's settings.
            pub fn setting(self) -> &'static PacketSetting {
                match self {
                    $(PacketType::$name => &PacketSetting {
                        priority: PacketPriority::$pr,
                        reliability: PacketReliability::$rel,
                        ordering: $ord,
                        required_capability: Capability::$cap,
                        handling_type: $h,
                        is_cached: $cached,
                        anticheats: &[$($ac),*],
                    },)*
                }
            }
        }
    };
}

impl PacketType {
    /// `PacketType.getId`: the ordinal as a `short`.
    pub fn id(self) -> i16 {
        self as u16 as i16
    }

    /// `PacketTypes.packetTypes.get(id)`.
    pub fn from_id(id: i16) -> Option<PacketType> {
        usize::try_from(id)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }

    /// `PacketType.valueOf(name)`; `None` where Java throws.
    pub fn from_name(name: &str) -> Option<PacketType> {
        Self::ALL.iter().copied().find(|t| t.name() == name)
    }

    pub fn priority(self) -> PacketPriority {
        self.setting().priority
    }

    pub fn reliability(self) -> PacketReliability {
        self.setting().reliability
    }

    pub fn ordering_channel(self) -> u8 {
        self.setting().ordering
    }

    pub fn required_capability(self) -> Capability {
        self.setting().required_capability
    }

    pub fn handling_type(self) -> u8 {
        self.setting().handling_type
    }

    pub fn handled_by_server(self) -> bool {
        self.handling_type() & handling::SERVER != 0
    }

    pub fn handled_by_client(self) -> bool {
        self.handling_type() & handling::CLIENT != 0
    }

    pub fn handled_while_loading(self) -> bool {
        self.handling_type() & handling::CLIENT_LOADING != 0
    }

    /// `PacketType.doPacket`: the user-packet byte and the big-endian id.
    pub fn write_header(self, out: &mut Vec<u8>) {
        out.push(ID_USER_PACKET);
        out.extend_from_slice(&self.id().to_be_bytes());
    }

    /// `PacketType.getFlags`, e.g. `"0-H-RO"`: ordering channel, priority
    /// letter, reliability letters.
    pub fn flags(self) -> String {
        let s = self.setting();
        let priority = match s.priority {
            PacketPriority::Immediate => "I",
            PacketPriority::High => "H",
            PacketPriority::Medium => "M",
            PacketPriority::Low => "L",
        };
        let reliability = match s.reliability {
            PacketReliability::Unreliable => "U",
            PacketReliability::UnreliableSequenced => "US",
            PacketReliability::Reliable => "R",
            PacketReliability::ReliableOrdered => "RO",
            PacketReliability::ReliableSequenced => "RS",
        };
        format!("{}-{}-{}", s.ordering, priority, reliability)
    }
}

/// Splits a received user packet into its type and body. Returns `None` for
/// RakNet's own messages (first byte other than [`ID_USER_PACKET`]), a packet
/// shorter than the header, or an id outside the table.
pub fn parse_header(packet: &[u8]) -> Option<(PacketType, &[u8])> {
    match packet {
        [ID_USER_PACKET, hi, lo, body @ ..] => {
            PacketType::from_id(i16::from_be_bytes([*hi, *lo])).map(|t| (t, body))
        }
        _ => None,
    }
}

// Table body ported from `zombie.network.PacketTypes.PacketType`: one row per
// constant, in declaration order (the id is the ordinal). Rows for packet
// classes take their values from that class's `@PacketSetting`.

packet_types! {
    Checksum: High, ReliableOrdered, 0, LoginOnServer, 7, false, [];
    Login: High, ReliableOrdered, 0, None, 1, false, [];
    LoginQueueRequest: Immediate, ReliableOrdered, 0, LoginOnServer, 5, false, [];
    LoginQueueDone: Immediate, ReliableOrdered, 0, LoginOnServer, 5, false, [];
    LoadPlayerProfile: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    CreatePlayer: High, ReliableOrdered, 0, LoginOnServer, 7, false, [];
    PlayerConnect: High, ReliableOrdered, 0, LoginOnServer, 1, false, [];
    ConnectedPlayer: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    ConnectCoop: High, Reliable, 0, LoginOnServer, 1, false, [];
    ConnectedCoop: High, Reliable, 0, LoginOnServer, 2, false, [];
    GoogleAuthKey: Immediate, Reliable, 0, None, 7, false, [];
    GoogleAuth: Immediate, Reliable, 0, None, 1, false, [];
    GoogleAuthRequest: Immediate, Reliable, 0, None, 4, false, [];
    PlayerUpdateReliable: High, ReliableOrdered, 5, LoginOnServer, 3, false, [AntiCheat::Power, AntiCheat::Speed, AntiCheat::NoClip, AntiCheat::Player];
    PlayerUpdateUnreliable: Medium, UnreliableSequenced, 5, LoginOnServer, 3, false, [AntiCheat::Power, AntiCheat::Speed, AntiCheat::NoClip];
    PlayerDataRequest: High, Reliable, 0, LoginOnServer, 1, false, [];
    HumanVisual: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncClothing: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    SyncVisuals: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    Equip: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    AddXPMultiplier: Medium, Reliable, 0, LoginOnServer, 2, false, [];
    HaloText: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerDeath: Immediate, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    ChangeSafety: High, Reliable, 0, LoginOnServer, 3, false, [];
    BodyDamageUpdate: High, Reliable, 5, LoginOnServer, 3, false, [];
    GameCharacterAttachedItem: High, Reliable, 0, LoginOnServer, 3, false, [];
    VariableSync: High, ReliableOrdered, 0, LoginOnServer, 7, false, [];
    SyncPlayerStats: High, Reliable, 0, CanModifyBodyStats, 3, false, [];
    SyncPlayerFields: High, Reliable, 0, LoginOnServer, 2, false, [];
    ForageItemFound: High, Reliable, 0, LoginOnServer, 1, false, [];
    ForageRequestZone: High, Reliable, 0, LoginOnServer, 1, false, [];
    ForagePool: High, Reliable, 0, LoginOnServer, 2, false, [];
    ForageSpot: High, Reliable, 0, LoginOnServer, 1, false, [];
    PlayerDropHeldItems: High, ReliableOrdered, 1, LoginOnServer, 1, false, [];
    ClothingWetness: High, Reliable, 0, LoginOnServer, 2, false, [];
    RequestMedicalCheck: High, Reliable, 0, LoginOnServer, 3, false, [];
    HiddenAuthors: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    ServerMap: Immediate, ReliableOrdered, 4, LoginOnServer, 6, false, [];
    SentChunk: Immediate, Reliable, 4, LoginOnServer, 0, false, [];
    RequestZipList: High, Reliable, 4, LoginOnServer, 1, false, [];
    NotRequiredInZip: Immediate, Reliable, 4, LoginOnServer, 1, false, [];
    RequestData: High, Reliable, 4, LoginOnServer, 7, false, [];
    ChunkNotReady: High, Reliable, 4, LoginOnServer, 0, false, [];
    ZombieList: High, Unreliable, 0, LoginOnServer, 2, false, [];
    ZombieSimulationUnreliable: Medium, Unreliable, 0, LoginOnServer, 1, false, [];
    ZombieSimulationReliable: Immediate, Reliable, 0, LoginOnServer, 1, false, [];
    ZombieSynchronizationUnreliable: Medium, Unreliable, 0, LoginOnServer, 2, false, [];
    ZombieSynchronizationReliable: Immediate, Reliable, 0, LoginOnServer, 2, false, [];
    ZombieDeleteOnClient: High, Reliable, 0, LoginOnServer, 2, false, [];
    ZombieRequest: High, Reliable, 0, LoginOnServer, 1, false, [];
    ZombieStagger: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    ZombieDeath: Immediate, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    SlowFactor: High, Reliable, 0, LoginOnServer, 2, false, [];
    ZombieControl: Immediate, Reliable, 0, LoginOnServer, 2, false, [];
    Thump: High, Reliable, 0, LoginOnServer, 3, false, [];
    ZombieFall: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    AnimalCommand: High, ReliableOrdered, 1, AnimalCheats, 3, false, [];
    AnimalOwnership: Immediate, ReliableOrdered, 7, LoginOnServer, 2, false, [];
    AnimalPacket: Medium, Unreliable, 7, LoginOnServer, 3, false, [];
    AnimalUpdateReliable: Immediate, Reliable, 7, LoginOnServer, 3, false, [];
    AnimalUpdateUnreliable: Medium, Unreliable, 7, LoginOnServer, 3, false, [];
    AnimalEvent: Medium, Reliable, 7, LoginOnServer, 3, false, [];
    AnimalDeath: Immediate, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    AnimalTracks: High, Reliable, 0, LoginOnServer, 3, false, [];
    AddTrack: High, Reliable, 0, LoginOnServer, 2, false, [];
    ManageAnimalTrackOnWorldMap: Medium, Reliable, 7, AnimalCheats, 2, false, [];
    VehicleRequest: High, Reliable, 0, LoginOnServer, 1, false, [];
    VehicleFullUpdate: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    VehicleUpdate: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    VehiclePhysicsReliable: High, ReliableOrdered, 8, LoginOnServer, 3, false, [AntiCheat::Speed];
    VehiclePhysicsUnreliable: Medium, UnreliableSequenced, 8, LoginOnServer, 3, false, [AntiCheat::Speed];
    VehicleTowingAttach: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    VehicleTowingDetach: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    VehicleTowingState: High, Reliable, 0, LoginOnServer, 2, false, [];
    VehicleRemove: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    VehicleCollide: High, Reliable, 0, LoginOnServer, 1, false, [];
    VehiclePassengerPosition: High, Reliable, 0, LoginOnServer, 3, false, [];
    VehiclePassengerRequest: High, Reliable, 0, LoginOnServer, 1, false, [];
    VehiclePassengerResponse: High, Reliable, 0, LoginOnServer, 2, false, [];
    VehicleEnter: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    VehicleExit: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    VehicleSwitchSeat: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    VehicleSoundAddVehicle: Medium, Reliable, 8, LoginOnServer, 6, false, [];
    VehicleSoundRemoveVehicle: Medium, Reliable, 8, LoginOnServer, 6, false, [];
    VehicleSoundUpdateVehicle: Medium, Reliable, 8, LoginOnServer, 6, false, [];
    State: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    Helicopter: High, Reliable, 0, LoginOnServer, 2, false, [];
    SmashWindow: High, Reliable, 0, LoginOnServer, 3, false, [];
    EatFood: High, Reliable, 0, LoginOnServer, 3, false, [];
    Drink: High, Reliable, 0, LoginOnServer, 1, false, [];
    BurnCorpse: Medium, Reliable, 0, LoginOnServer, 1, false, [];
    SneezeCough: Low, Unreliable, 0, LoginOnServer, 2, false, [];
    AddBlood: Low, Reliable, 0, LoginOnServer, 2, false, [];
    RemoveBlood: Medium, Reliable, 0, LoginOnServer, 3, false, [];
    WakeUpPlayer: Medium, Reliable, 0, LoginOnServer, 3, false, [];
    EatBody: High, Reliable, 0, LoginOnServer, 3, false, [];
    ReadAnnotedMap: High, Reliable, 0, LoginOnServer, 1, false, [];
    ZombieHelmetFalling: High, Reliable, 0, LoginOnServer, 2, false, [];
    TimeSync: Immediate, Reliable, 0, LoginOnServer, 7, false, [];
    SetMultiplier: Medium, Reliable, 0, LoginOnServer, 2, false, [];
    AccessDenied: Immediate, Reliable, 0, LoginOnServer, 4, false, [];
    StartPause: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    StopPause: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    PlayerTimeout: High, Reliable, 0, LoginOnServer, 6, false, [];
    SyncClock: High, Reliable, 0, LoginOnServer, 6, false, [];
    SandboxOptions: High, Reliable, 0, SandboxOptions, 3, false, [];
    ServerQuit: Immediate, Reliable, 0, LoginOnServer, 2, false, [];
    MessageForAdmin: Medium, Reliable, 0, CanSeeMessageForAdmin, 2, false, [];
    MetaData: High, Reliable, 0, None, 6, false, [];
    MetaGrid: High, Reliable, 0, LoginOnServer, 2, false, [];
    WorldMessage: Medium, Reliable, 0, WorkWithUserlog, 1, false, [];
    ReceiveCommand: Medium, ReliableOrdered, 0, LoginOnServer, 1, false, [];
    ReloadOptions: High, Reliable, 0, LoginOnServer, 2, false, [];
    Kicked: Immediate, Reliable, 0, LoginOnServer, 6, false, [];
    Ping: Immediate, Unreliable, 0, None, 7, false, [];
    SpawnRegion: High, Reliable, 0, LoginOnServer, 6, false, [];
    WorldMapPlayerPosition: High, UnreliableSequenced, 0, LoginOnServer, 3, false, [];
    WorldMap: High, Reliable, 0, LoginOnServer, 7, false, [];
    ItemTransaction: High, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    GeneralAction: High, ReliableOrdered, 1, LoginOnServer, 1, false, [];
    NetTimedAction: High, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    BuildAction: High, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    FishingAction: High, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    AddInventoryItemToContainer: High, ReliableOrdered, 1, LoginOnServer, 2, false, [];
    RemoveInventoryItemFromContainer: High, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    ReplaceInventoryItemInContainer: High, ReliableOrdered, 1, LoginOnServer, 2, false, [];
    SyncItemDelete: High, ReliableOrdered, 1, EditItem, 3, false, [];
    AddItemToMap: High, ReliableOrdered, 1, LoginOnServer, 2, false, [];
    AddObjectToMap: High, ReliableOrdered, 1, AddItem, 3, false, [];
    AddCorpseToMap: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    RemoveItemFromSquare: High, ReliableOrdered, 1, LoginOnServer, 2, false, [];
    RemoveCorpseFromMap: High, Reliable, 0, LoginOnServer, 3, false, [];
    RequestItemsForContainer: High, Reliable, 0, LoginOnServer, 1, false, [];
    RemoveContestedItemsFromInventory: High, Reliable, 0, LoginOnServer, 2, false, [];
    ItemStats: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    AddBrokenGlass: High, Reliable, 0, LoginOnServer, 3, false, [];
    AddExplosiveTrap: High, Reliable, 0, LoginOnServer, 3, false, [];
    SledgehammerDestroy: High, Reliable, 0, UseDebugContextMenu, 3, false, [];
    SyncPlayerAlarmClock: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncWorldAlarmClock: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncExtendedPlacement: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncItemActivated: High, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    GlobalObjects: High, Reliable, 0, LoginOnServer, 3, false, [];
    ReceiveModData: High, Reliable, 0, LoginOnServer, 3, false, [];
    ReceiveContainerModData: High, Reliable, 0, LoginOnServer, 3, false, [];
    GlobalModData: Immediate, Reliable, 0, LoginOnServer, 3, false, [];
    GlobalModDataRequest: Immediate, Reliable, 0, LoginOnServer, 1, false, [];
    SyncItemFields: High, Reliable, 1, LoginOnServer, 3, false, [];
    SyncItemModData: Medium, Reliable, 1, LoginOnServer, 2, false, [];
    SyncHandWeaponFields: High, Reliable, 1, LoginOnServer, 3, false, [];
    PlayerHitSquare: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitLongDistance];
    PlayerHitObject: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitLongDistance, AntiCheat::HitWeapon];
    PlayerHitVehicle: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitLongDistance];
    PlayerHitZombie: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitDamage, AntiCheat::HitLongDistance, AntiCheat::HitWeapon];
    PlayerHitPlayer: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitDamage, AntiCheat::HitLongDistance, AntiCheat::HitWeapon, AntiCheat::Safety];
    PlayerHitAnimal: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitDamage, AntiCheat::HitLongDistance, AntiCheat::HitWeapon];
    ZombieHitPlayer: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitShortDistance, AntiCheat::Target];
    AnimalHitPlayer: Immediate, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    AnimalHitAnimal: Immediate, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    AnimalHitThumpable: Immediate, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    ZombieHitThumpable: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitShortDistance];
    VehicleHitZombie: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitDamage, AntiCheat::HitShortDistance, AntiCheat::Speed, AntiCheat::HitVehicle];
    VehicleHitPlayer: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [AntiCheat::HitDamage, AntiCheat::HitShortDistance, AntiCheat::Safety, AntiCheat::Speed, AntiCheat::HitVehicle];
    VehicleHitAnimal: Immediate, ReliableOrdered, 0, LoginOnServer, 1, false, [AntiCheat::HitDamage, AntiCheat::HitShortDistance, AntiCheat::Speed, AntiCheat::HitVehicle];
    PlayerDamage: Medium, ReliableOrdered, 10, LoginOnServer, 2, false, [];
    PlayerStats: Medium, ReliableOrdered, 12, LoginOnServer, 2, false, [];
    PlayerHealth: Medium, ReliableOrdered, 14, LoginOnServer, 2, false, [];
    PlayerInjuries: Medium, ReliableOrdered, 9, LoginOnServer, 2, false, [];
    PlayerEffects: Medium, ReliableOrdered, 13, LoginOnServer, 2, false, [];
    PlayerXp: Medium, ReliableOrdered, 11, CanModifyPlayerStatsInThePlayerStatsUI, 3, false, [];
    AttackCollisionCheckPacket: Immediate, ReliableOrdered, 0, LoginOnServer, 1, false, [];
    PlayerEmptyShot: Medium, Unreliable, 5, LoginOnServer, 3, false, [];
    PlayerSteppedOnGlass: Medium, Reliable, 0, LoginOnServer, 1, false, [];
    PlaySound: High, Reliable, 0, LoginOnServer, 3, false, [];
    RangedWeaponSound: High, Reliable, 0, LoginOnServer, 3, false, [];
    LoopedRangedWeaponSound: High, Reliable, 0, LoginOnServer, 3, false, [];
    WorldSoundPacket: High, Reliable, 0, LoginOnServer, 3, false, [];
    AddAmbient: High, Reliable, 0, LoginOnServer, 2, false, [];
    ZombieSound: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayWorldSound: High, Reliable, 0, LoginOnServer, 3, false, [];
    StopSound: High, Reliable, 0, LoginOnServer, 3, false, [];
    PlaySoundEveryPlayer: High, Reliable, 0, LoginOnServer, 2, false, [];
    Statistics: Low, UnreliableSequenced, 0, GetStatistic, 3, false, [];
    ScoreboardUpdate: High, Reliable, 0, LoginOnServer, 3, false, [];
    Weather: High, Reliable, 0, LoginOnServer, 2, false, [];
    StartRain: High, Reliable, 0, LoginOnServer, 2, false, [];
    StopRain: High, Reliable, 0, LoginOnServer, 2, false, [];
    ClimateManagerPacket: High, Reliable, 0, ClimateManager, 3, false, [];
    SyncPuddles: High, Reliable, 0, LoginOnServer, 2, false, [];
    IsoRegionServerPacket: High, Reliable, 0, LoginOnServer, 2, false, [];
    IsoRegionClientRequestFullUpdate: High, Reliable, 0, LoginOnServer, 1, false, [];
    SyncNonPvpZone: High, Reliable, 0, CanSetupNonPVPZone, 3, false, [];
    SyncThumpable: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncIsoObject: High, Reliable, 3, LoginOnServer, 3, false, [];
    ClientCommand: High, Reliable, 0, LoginOnServer, 3, false, [];
    ObjectModData: Medium, ReliableOrdered, 1, LoginOnServer, 3, false, [];
    ObjectChange: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
    BloodSplatter: High, Reliable, 0, LoginOnServer, 2, false, [];
    ZombieDescriptors: High, Reliable, 0, LoginOnServer, 2, false, [];
    StartFire: High, Reliable, 0, LoginOnServer, 2, false, [];
    StopFire: High, Reliable, 0, LoginOnServer, 3, false, [];
    UpdateItemSprite: High, Reliable, 0, LoginOnServer, 3, false, [];
    SendCustomColor: Medium, Reliable, 0, LoginOnServer, 2, false, [];
    SyncCompost: High, Reliable, 0, LoginOnServer, 3, false, [];
    GetModData: Medium, Reliable, 0, LoginOnServer, 1, false, [];
    SyncPerks: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncEquippedRadioFreq: High, Reliable, 0, LoginOnServer, 3, false, [];
    UpdateOverlaySprite: High, Reliable, 0, LoginOnServer, 3, false, [];
    AddAlarm: High, Reliable, 0, LoginOnServer, 2, false, [];
    ChangeTextColor: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncCustomLightSettings: High, Reliable, 0, LoginOnServer, 3, false, [];
    ChunkObjectStateResponse: High, Reliable, 3, LoginOnServer, 2, false, [];
    ChunkObjectStateRequest: High, Reliable, 3, LoginOnServer, 1, false, [];
    StartFishSplash: High, Reliable, 0, LoginOnServer, 3, false, [];
    FishingData: High, Reliable, 0, LoginOnServer, 3, false, [];
    ToxicBuilding: High, Reliable, 0, LoginOnServer, 2, false, [];
    AddItemInInventory: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerInventory: High, Reliable, 0, InspectPlayerInventory, 3, false, [];
    ExtraInfo: High, Reliable, 0, LoginOnServer, 3, false, [];
    Teleport: Immediate, Reliable, 0, TeleportToCoordinates, 3, false, [];
    InvMngReqItem: High, Reliable, 0, InspectPlayerInventory, 3, false, [];
    InvMngGetItem: High, Reliable, 0, LoginOnServer, 3, false, [];
    InvMngRemoveItem: High, Reliable, 0, InspectPlayerInventory, 3, false, [];
    InvMngUpdateItem: High, Reliable, 0, InspectPlayerInventory, 1, false, [];
    ChangePlayerStats: High, Reliable, 0, CanSeePlayersStats, 3, false, [];
    NetworkUsers: Medium, Reliable, 0, SeeNetworkUsers, 2, false, [];
    PVPEvents: Medium, Reliable, 0, PVPLogTool, 3, false, [];
    RequestNetworkUsers: Medium, Reliable, 0, SeeNetworkUsers, 1, false, [];
    NetworkUserAction: Medium, Reliable, 0, ModifyNetworkUsers, 1, false, [];
    BanUnbanUserAction: High, ReliableOrdered, 0, BanUnbanUser, 1, false, [];
    TeleportUserAction: High, ReliableOrdered, 0, TeleportPlayerToAnotherPlayer, 1, false, [];
    TeleportToHimUserAction: High, ReliableOrdered, 0, TeleportToPlayer, 1, false, [];
    Roles: Medium, Reliable, 0, LoginOnServer, 2, false, [];
    RequestRoles: Medium, Reliable, 0, RolesRead, 1, false, [];
    RolesEdit: Medium, Reliable, 0, RolesWrite, 1, false, [];
    RequestUserLog: Medium, Reliable, 0, ReadUserLog, 3, false, [];
    AddUserlog: Medium, Reliable, 0, AddUserlog, 1, false, [];
    RemoveUserlog: Medium, Reliable, 0, WorkWithUserlog, 1, false, [];
    AddWarningPoint: Medium, Reliable, 0, AddUserlog, 1, false, [];
    ConstructedZone: High, Reliable, 0, LoginOnServer, 3, false, [];
    RegisterZone: High, Reliable, 0, LoginOnServer, 3, false, [];
    SyncZone: Medium, Reliable, 0, LoginOnServer, 3, false, [];
    FactionInvite: High, Reliable, 0, LoginOnServer, 3, false, [];
    FactionAccept: High, Reliable, 0, LoginOnServer, 3, false, [];
    FactionRemoveMember: High, Reliable, 0, LoginOnServer, 1, false, [];
    FactionChangeOwner: High, Reliable, 0, LoginOnServer, 1, false, [];
    FactionChangeTag: High, Reliable, 0, LoginOnServer, 1, false, [];
    FactionChangeTitle: High, Reliable, 0, LoginOnServer, 3, false, [];
    FactionDisband: High, Reliable, 0, LoginOnServer, 3, false, [];
    FactionCreate: High, Reliable, 0, LoginOnServer, 1, false, [];
    FactionSync: High, Reliable, 0, LoginOnServer, 2, false, [];
    FactionStats: Low, Reliable, 0, LoginOnServer, 3, false, [];
    AddTicket: Medium, Reliable, 0, LoginOnServer, 1, false, [];
    ViewedTicket: Medium, Reliable, 0, AnswerTickets, 1, false, [];
    ViewTickets: Medium, Reliable, 0, LoginOnServer, 3, false, [];
    RemoveTicket: Medium, Reliable, 0, AnswerTickets, 1, false, [];
    ViewBannedIPs: High, Reliable, 0, BanUnbanUser, 3, false, [];
    ViewBannedSteamIDs: High, Reliable, 0, BanUnbanUser, 3, false, [];
    RequestTrading: High, Reliable, 0, LoginOnServer, 3, false, [];
    TradingUIAddItem: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    TradingUIRemoveItem: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    TradingUIUpdateState: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    WarStateSync: High, Reliable, 0, LoginOnServer, 1, false, [];
    WarSync: High, Reliable, 0, LoginOnServer, 2, false, [];
    InitPlayerChat: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerJoinChat: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerLeaveChat: High, Reliable, 0, LoginOnServer, 2, false, [];
    ChatMessageFromPlayer: High, Reliable, 0, LoginOnServer, 1, false, [];
    ChatMessageToPlayer: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerStartPMChat: High, Reliable, 0, LoginOnServer, 1, false, [];
    AddChatTab: High, Reliable, 0, LoginOnServer, 2, false, [];
    RemoveChatTab: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerConnectedToChat: High, Reliable, 0, LoginOnServer, 2, false, [];
    PlayerNotFound: High, Reliable, 0, LoginOnServer, 2, false, [];
    SafehouseInvite: High, Reliable, 0, LoginOnServer, 3, false, [AntiCheat::SafeHouseOwner];
    SafehouseAccept: High, Reliable, 0, LoginOnServer, 3, false, [AntiCheat::SafeHousePlayer];
    SafehouseChangeMember: High, Reliable, 0, LoginOnServer, 1, false, [AntiCheat::SafeHouseMember];
    SafehouseChangeOwner: High, Reliable, 0, LoginOnServer, 1, false, [AntiCheat::SafeHouseOwner];
    SafehouseChangeRespawn: High, Reliable, 0, LoginOnServer, 1, false, [AntiCheat::SafeHouseMember];
    SafehouseChangeTitle: High, Reliable, 0, LoginOnServer, 1, false, [AntiCheat::SafeHouseOwner];
    SafehouseRelease: High, Reliable, 0, LoginOnServer, 3, false, [AntiCheat::SafeHouseOwner];
    SafehouseClaim: High, Reliable, 0, LoginOnServer, 1, false, [AntiCheat::SafeHousePlayer];
    SafezoneClaim: High, Reliable, 0, CanSetupSafehouses, 1, false, [AntiCheat::SafeHousePlayer];
    SafehouseSync: High, Reliable, 0, LoginOnServer, 2, false, [];
    PopmanDebugCommand: High, Unreliable, 0, ConnectWithDebug, 3, false, [];
    ServerDebugInfo: High, Unreliable, 0, ConnectWithDebug, 3, false, [];
    ServerLOS: High, Unreliable, 0, ConnectWithDebug, 3, false, [];
    WaveSignal: High, Reliable, 0, LoginOnServer, 3, false, [];
    PlayerListensChannel: High, Reliable, 0, LoginOnServer, 1, false, [];
    RadioServerData: High, Reliable, 0, LoginOnServer, 3, false, [];
    RadioDeviceDataState: High, Reliable, 0, LoginOnServer, 3, false, [];
    RadioPostSilenceEvent: Immediate, Reliable, 0, LoginOnServer, 2, false, [];
    SyncRadioData: Immediate, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    BodyPartSync: High, Reliable, 0, LoginOnServer, 2, false, [];
    DebugStory: Medium, Reliable, 0, CreateStory, 1, false, [];
    SendItemListNet: High, Reliable, 0, LoginOnServer, 3, false, [];
    GameEntity: High, ReliableOrdered, 0, LoginOnServer, 3, false, [];
    SyncEntityResource: High, ReliableOrdered, 0, LoginOnServer, 2, false, [];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_ordinals() {
        assert_eq!(PacketType::ALL.len(), 298);
        for (i, t) in PacketType::ALL.iter().enumerate() {
            assert_eq!(t.id() as usize, i);
            assert_eq!(PacketType::from_id(t.id()), Some(*t));
            assert_eq!(PacketType::from_name(t.name()), Some(*t));
        }
        assert_eq!(PacketType::Checksum.id(), 0);
        assert_eq!(PacketType::Login.id(), 1);
        assert_eq!(PacketType::from_id(-1), None);
        assert_eq!(PacketType::from_id(298), None);
    }

    #[test]
    fn header_round_trip() {
        let mut out = Vec::new();
        PacketType::Login.write_header(&mut out);
        out.extend_from_slice(b"body");
        assert_eq!(&out[..3], &[134, 0, 1]);
        assert_eq!(parse_header(&out), Some((PacketType::Login, &b"body"[..])));
        assert_eq!(parse_header(&[134, 0]), None);
        assert_eq!(parse_header(&[16, 0, 1]), None);
    }

    #[test]
    fn settings_follow_constructor_rules() {
        // Callback rows: handling type comes from which callbacks exist.
        let ping = PacketType::Ping.setting();
        assert_eq!(ping.priority, PacketPriority::Immediate);
        assert_eq!(ping.reliability, PacketReliability::Unreliable);
        assert_eq!(ping.required_capability, Capability::None);
        assert_eq!(ping.handling_type, handling::ALL);
        let receive_command = PacketType::ReceiveCommand.setting();
        assert_eq!(receive_command.handling_type, handling::SERVER);
        assert_eq!(PacketType::SyncIsoObject.ordering_channel(), 3);
        assert_eq!(PacketType::Drink.flags(), "0-H-R");
        // Class rows: values from the packet class's @PacketSetting.
        assert_eq!(PacketType::Login.flags(), "0-H-RO");
        assert_eq!(PacketType::Login.required_capability(), Capability::None);
        assert!(PacketType::Login.handled_by_server());
        assert!(!PacketType::Login.handled_by_client());
        assert_eq!(
            PacketType::PlayerUpdateUnreliable.setting().anticheats,
            &[AntiCheat::Power, AntiCheat::Speed, AntiCheat::NoClip]
        );
        assert_eq!(PacketType::PlayerUpdateUnreliable.flags(), "5-M-US");
    }

    #[test]
    fn handling_type_bits() {
        assert_eq!(handling::get_type(true, false, false), handling::SERVER);
        assert_eq!(handling::get_type(true, true, true), handling::ALL);
        assert_eq!(handling::get_type(false, true, true), 6);
    }
}
