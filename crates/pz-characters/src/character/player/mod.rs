//! The local player's control loop: `IsoPlayer.update` and the input it
//! reads ([`PlayerInput`]).

pub mod input;

pub use input::{JoypadAxis2dBinding, NoInput, PlayerInput};
