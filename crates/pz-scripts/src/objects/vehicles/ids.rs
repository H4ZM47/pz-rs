//! The string-id enums and key records of `zombie.scripting.objects` that
//! name vehicle parts, areas, seats and scripts (`VehicleArea`,
//! `VehiclePart`, `VehicleKey`, ...). Each `toString` is [`id`](VehicleArea::id).

use std::fmt;

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $id:literal,)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            /// `values()`, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// `toString`.
            pub fn id(self) -> &'static str {
                match self {
                    $($name::$variant => $id,)*
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.id())
            }
        }
    };
}

/// `record VehicleKey(String id)`. The original's constants for vanilla
/// vehicles are not copied here; read them from the game jar if needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleKey(pub &'static str);

impl VehicleKey {
    /// `id()` / `toString`.
    pub fn id(self) -> &'static str {
        self.0
    }

    /// `toStringBase`: `"Base." + id`.
    pub fn to_string_base(self) -> String {
        format!("Base.{}", self.0)
    }
}

impl fmt::Display for VehicleKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// `record VehicleTemplateKey(String id)` (constants not copied, as above).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleTemplateKey(pub &'static str);

impl VehicleTemplateKey {
    /// `id()` / `toString`.
    pub fn id(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for VehicleTemplateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// `VehiclePartModel`: a part's model reference, used from Lua.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VehiclePartModel {
    pub part_id: Option<String>,
    pub part_model_id: Option<String>,
    pub model_id: Option<String>,
}

string_enum! {
    /// `VehicleArea`.
    VehicleArea {
        AnimalEntry = "AnimalEntry",
        Engine = "Engine",
        GasTank = "GasTank",
        LeftSiren = "LeftSiren",
        RightSiren = "RightSiren",
        SeatFrontLeft = "SeatFrontLeft",
        SeatFrontRight = "SeatFrontRight",
        SeatMiddleLeft = "SeatMiddleLeft",
        SeatMiddleRight = "SeatMiddleRight",
        SeatRearLeft = "SeatRearLeft",
        SeatRearRight = "SeatRearRight",
        TireFrontLeft = "TireFrontLeft",
        TireFrontRight = "TireFrontRight",
        TireRearLeft = "TireRearLeft",
        TireRearRight = "TireRearRight",
        TrailerAnimalFood = "TrailerAnimalFood",
        TruckBed = "TruckBed",
    }
}

string_enum! {
    /// `VehicleAttachment`.
    VehicleAttachment {
        Trailer = "trailer",
        Trailerfront = "trailerfront",
    }
}

string_enum! {
    /// `VehicleCategory`.
    VehicleCategory {
        Bodywork = "bodywork",
        Brakes = "brakes",
        Door = "door",
        Engine = "engine",
        Gastank = "gastank",
        Lights = "lights",
        Nodisplay = "nodisplay",
        Seat = "seat",
        Suspension = "suspension",
        Tire = "tire",
    }
}

string_enum! {
    /// `VehicleEngineRpmType`.
    VehicleEngineRpmType {
        Firebird = "firebird",
        Van = "van",
    }
}

string_enum! {
    /// `VehicleItemEquip`.
    VehicleItemEquip {
        Primary = "primary",
        Secondary = "secondary",
    }
}

string_enum! {
    /// `VehicleMechanicArea`.
    VehicleMechanicArea {
        Back = "Back",
        Engine = "Engine",
        Interior = "Interior",
        Left = "Left",
        Right = "Right",
        Under = "Under",
    }
}

string_enum! {
    /// `VehiclePart`.
    VehiclePart {
        Battery = "Battery",
        Brake = "Brake",
        BrakeFrontLeft = "BrakeFrontLeft",
        BrakeFrontRight = "BrakeFrontRight",
        BrakeRearLeft = "BrakeRearLeft",
        BrakeRearRight = "BrakeRearRight",
        Door = "Door",
        DoorFrontLeft = "DoorFrontLeft",
        DoorFrontRight = "DoorFrontRight",
        DoorMiddleLeft = "DoorMiddleLeft",
        DoorMiddleRight = "DoorMiddleRight",
        DoorRear = "DoorRear",
        DoorRearLeft = "DoorRearLeft",
        DoorRearRight = "DoorRearRight",
        Engine = "Engine",
        EngineDoor = "EngineDoor",
        GasTank = "GasTank",
        GloveBox = "GloveBox",
        Headlight = "Headlight",
        HeadlightLeft = "HeadlightLeft",
        HeadlightRearLeft = "HeadlightRearLeft",
        HeadlightRearRight = "HeadlightRearRight",
        HeadlightRight = "HeadlightRight",
        Heater = "Heater",
        HoodOrnament = "HoodOrnament",
        Lightbar = "lightbar",
        Muffler = "Muffler",
        PassengerCompartment = "PassengerCompartment",
        Radio = "Radio",
        Seat = "Seat",
        SeatFrontLeft = "SeatFrontLeft",
        SeatFrontRight = "SeatFrontRight",
        SeatMiddleLeft = "SeatMiddleLeft",
        SeatMiddleRight = "SeatMiddleRight",
        SeatRear = "SeatRear",
        SeatRearLeft = "SeatRearLeft",
        SeatRearRight = "SeatRearRight",
        Suspension = "Suspension",
        SuspensionFrontLeft = "SuspensionFrontLeft",
        SuspensionFrontRight = "SuspensionFrontRight",
        SuspensionRearLeft = "SuspensionRearLeft",
        SuspensionRearRight = "SuspensionRearRight",
        TestPart1 = "TestPart1",
        Tire = "Tire",
        TireFrontLeft = "TireFrontLeft",
        TireFrontRight = "TireFrontRight",
        TireRearLeft = "TireRearLeft",
        TireRearRight = "TireRearRight",
        TrailerAnimalEggs = "TrailerAnimalEggs",
        TrailerAnimalFood = "TrailerAnimalFood",
        TrailerTrunk = "TrailerTrunk",
        TruckBed = "TruckBed",
        TruckBedOpen = "TruckBedOpen",
        TrunkDoor = "TrunkDoor",
        TrunkDoorOpened = "TrunkDoorOpened",
        Window = "Window",
        WindowFrontLeft = "WindowFrontLeft",
        WindowFrontRight = "WindowFrontRight",
        WindowMiddleLeft = "WindowMiddleLeft",
        WindowMiddleRight = "WindowMiddleRight",
        WindowRearLeft = "WindowRearLeft",
        WindowRearRight = "WindowRearRight",
        Windshield = "Windshield",
        WindshieldRear = "WindshieldRear",
    }
}

string_enum! {
    /// `VehiclePassenger`.
    VehiclePassenger {
        All = "*",
        FrontLeft = "FrontLeft",
        FrontRight = "FrontRight",
        MiddleLeft = "MiddleLeft",
        MiddleRight = "MiddleRight",
        RearLeft = "RearLeft",
        RearRight = "RearRight",
    }
}

string_enum! {
    /// `VehiclePosition`.
    VehiclePosition {
        Outside = "outside",
        Inside = "inside",
        Outside2 = "outside2",
    }
}

string_enum! {
    /// `VehicleSwitchSeat`.
    VehicleSwitchSeat {
        FrontLeft = "FrontLeft",
        FrontRight = "FrontRight",
        MiddleLeft = "MiddleLeft",
        MiddleRight = "MiddleRight",
        RearLeft = "RearLeft",
        RearRight = "RearRight",
    }
}

string_enum! {
    /// `VehicleTable`.
    VehicleTable {
        Headlight = "headlight",
        Install = "install",
        Uninstall = "uninstall",
    }
}

string_enum! {
    /// `VehicleWheel`.
    VehicleWheel {
        RearLeft = "RearLeft",
        RearRight = "RearRight",
        FrontRight = "FrontRight",
        FrontLeft = "FrontLeft",
    }
}
