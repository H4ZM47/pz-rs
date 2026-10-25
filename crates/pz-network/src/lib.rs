//! The client's offline network data: the saved server list and its
//! accounts (`zombie.network.Server`, `zombie.network.Account` and
//! `zombie.savefile.AccountDBHelper`). Nothing here talks to the network.

pub mod account_db;
pub mod crypt;
pub mod datetime;
pub mod server;

pub use account_db::AccountDb;
pub use datetime::LocalDateTime;
pub use server::{Account, AccountRef, Server};
