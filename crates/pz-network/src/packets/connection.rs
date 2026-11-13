//! `zombie.network.packets.connection`: login and its refusal.

use super::NetPacket;
use crate::buffer::{NetReader, NetWriter, ReadResult};
use crate::packet_types::PacketType;

/// `LoginPacket.authType` values the client sends.
pub mod auth_type {
    /// Username and password (`GameClient.authType` default).
    pub const PASSWORD: i32 = 1;
}

/// `LoginPacket`: the client's first game packet after RakNet connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginPacket {
    pub username: String,
    pub password: String,
    /// `Core.getGameAndBuildVersion()`; the server refuses any other value.
    pub client_version: String,
    pub auth_type: i32,
}

impl NetPacket for LoginPacket {
    const TYPE: PacketType = PacketType::Login;

    fn write(&self, w: &mut NetWriter) {
        w.put_utf(&self.username)
            .put_utf(&self.password)
            .put_utf(&self.client_version)
            .put_int(self.auth_type);
    }

    /// `LoginPacket.parse` trims the three strings.
    fn parse(r: &mut NetReader) -> ReadResult<Self> {
        Ok(Self {
            username: r.get_utf()?.trim().to_owned(),
            password: r.get_utf()?.trim().to_owned(),
            client_version: r.get_utf()?.trim().to_owned(),
            auth_type: r.get_int()?,
        })
    }
}

/// `AccessDeniedPacket`: why the server refused a login. The reason is a
/// translation key suffix, optionally followed by `##`-separated arguments
/// (`ClientVersionMismatch##<client>##<server>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessDeniedPacket {
    pub reason: String,
}

impl AccessDeniedPacket {
    /// The reason key and its arguments (`reason.split("##")`).
    pub fn parts(&self) -> Vec<&str> {
        self.reason.split("##").collect()
    }
}

impl NetPacket for AccessDeniedPacket {
    const TYPE: PacketType = PacketType::AccessDenied;

    fn write(&self, w: &mut NetWriter) {
        w.put_utf(&self.reason);
    }

    fn parse(r: &mut NetReader) -> ReadResult<Self> {
        Ok(Self {
            reason: r.get_utf()?,
        })
    }
}

/// `Core.getGameAndBuildVersion()` of the build this table matches.
pub const GAME_AND_BUILD_VERSION: &str = "42.22.2";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet_types::parse_header;

    #[test]
    fn login_round_trip() {
        let p = LoginPacket {
            username: "survivor".into(),
            password: "pw".into(),
            client_version: GAME_AND_BUILD_VERSION.into(),
            auth_type: auth_type::PASSWORD,
        };
        let bytes = p.encode();
        let (ty, body) = parse_header(&bytes).unwrap();
        assert_eq!(ty, PacketType::Login);
        assert_eq!(LoginPacket::parse(&mut NetReader::new(body)), Ok(p));
    }

    #[test]
    fn access_denied_parts() {
        let p = AccessDeniedPacket {
            reason: "ClientVersionMismatch##1.0##42.22.2".into(),
        };
        assert_eq!(p.parts(), ["ClientVersionMismatch", "1.0", "42.22.2"]);
        let bytes = p.encode();
        let (_, body) = parse_header(&bytes).unwrap();
        assert_eq!(AccessDeniedPacket::parse(&mut NetReader::new(body)), Ok(p));
    }
}
