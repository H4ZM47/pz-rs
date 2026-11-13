//! Multiplayer support.
//!
//! - The client's offline network data: the saved server list and its
//!   accounts (`zombie.network.Server`, `zombie.network.Account` and
//!   `zombie.savefile.AccountDBHelper`).
//! - The packet type table every game message is framed with
//!   (`zombie.network.PacketTypes`) and the role capabilities it checks
//!   (`zombie.characters.Capability`).

pub mod account_db;
pub mod capability;
pub mod crypt;
pub mod datetime;
pub mod packet_types;
pub mod server;

pub use account_db::AccountDb;
pub use capability::Capability;
pub use datetime::LocalDateTime;
pub use packet_types::{PacketPriority, PacketReliability, PacketSetting, PacketType};
pub use server::{Account, AccountRef, Server};
