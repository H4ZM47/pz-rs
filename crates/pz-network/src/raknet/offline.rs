//! Connectionless messages: pings and the two-step open-connection handshake
//! that settles the MTU before any datagram is sent.

use std::net::SocketAddr;

use super::codec::{DecodeError, DecodeResult, Reader, Writer};
use super::{id, UDP_HEADER_SIZE};

#[derive(Clone, Debug, PartialEq)]
pub enum OfflineMessage {
    /// `ID_UNCONNECTED_PING` (or `_OPEN_CONNECTIONS` when `open_only`).
    UnconnectedPing {
        time: u64,
        client_guid: u64,
        open_only: bool,
    },
    /// `ID_UNCONNECTED_PONG`: echoes the ping time; `data` is the server's
    /// offline ping response (empty for the game's server).
    UnconnectedPong {
        time: u64,
        server_guid: u64,
        data: Vec<u8>,
    },
    /// `ID_OPEN_CONNECTION_REQUEST_1`, zero padded so the datagram probes the
    /// MTU. `mtu` counts [`UDP_HEADER_SIZE`].
    OpenConnectionRequest1 { protocol: u8, mtu: u16 },
    /// `ID_OPEN_CONNECTION_REPLY_1`. `cookie` is present only when the server
    /// uses security, which the game's server does not.
    OpenConnectionReply1 {
        server_guid: u64,
        cookie: Option<u32>,
        mtu: u16,
    },
    /// `ID_OPEN_CONNECTION_REQUEST_2` without security.
    OpenConnectionRequest2 {
        server_address: SocketAddr,
        mtu: u16,
        client_guid: u64,
    },
    /// `ID_OPEN_CONNECTION_REPLY_2`.
    OpenConnectionReply2 {
        server_guid: u64,
        client_address: SocketAddr,
        mtu: u16,
        security: bool,
    },
    /// `ID_INCOMPATIBLE_PROTOCOL_VERSION`.
    IncompatibleProtocolVersion { protocol: u8, server_guid: u64 },
    /// `ID_ALREADY_CONNECTED`, `ID_NO_FREE_INCOMING_CONNECTIONS`,
    /// `ID_CONNECTION_BANNED`, `ID_IP_RECENTLY_CONNECTED`: a refusal carrying
    /// only the magic and the server GUID.
    Refused { id: u8, server_guid: u64 },
}

impl OfflineMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::UnconnectedPing {
                time,
                client_guid,
                open_only,
            } => {
                let msg = if *open_only {
                    id::UNCONNECTED_PING_OPEN_CONNECTIONS
                } else {
                    id::UNCONNECTED_PING
                };
                w.u8(msg).u64_be(*time).magic().u64_be(*client_guid);
            }
            Self::UnconnectedPong {
                time,
                server_guid,
                data,
            } => {
                w.u8(id::UNCONNECTED_PONG)
                    .u64_be(*time)
                    .u64_be(*server_guid)
                    .magic()
                    .bytes(data);
            }
            Self::OpenConnectionRequest1 { protocol, mtu } => {
                w.u8(id::OPEN_CONNECTION_REQUEST_1).magic().u8(*protocol);
                let len = usize::from(mtu.saturating_sub(UDP_HEADER_SIZE));
                w.buf.resize(len.max(w.buf.len()), 0);
            }
            Self::OpenConnectionReply1 {
                server_guid,
                cookie,
                mtu,
            } => {
                w.u8(id::OPEN_CONNECTION_REPLY_1)
                    .magic()
                    .u64_be(*server_guid);
                match cookie {
                    Some(c) => w.u8(1).u32_be(*c),
                    None => w.u8(0),
                };
                w.u16_be(*mtu);
            }
            Self::OpenConnectionRequest2 {
                server_address,
                mtu,
                client_guid,
            } => {
                w.u8(id::OPEN_CONNECTION_REQUEST_2)
                    .magic()
                    .address(server_address)
                    .u16_be(*mtu)
                    .u64_be(*client_guid);
            }
            Self::OpenConnectionReply2 {
                server_guid,
                client_address,
                mtu,
                security,
            } => {
                w.u8(id::OPEN_CONNECTION_REPLY_2)
                    .magic()
                    .u64_be(*server_guid)
                    .address(client_address)
                    .u16_be(*mtu)
                    .bool(*security);
            }
            Self::IncompatibleProtocolVersion {
                protocol,
                server_guid,
            } => {
                w.u8(id::INCOMPATIBLE_PROTOCOL_VERSION)
                    .u8(*protocol)
                    .magic()
                    .u64_be(*server_guid);
            }
            Self::Refused { id, server_guid } => {
                w.u8(*id).magic().u64_be(*server_guid);
            }
        }
        w.into_vec()
    }

    /// Decodes a datagram whose first byte names an offline message. Returns
    /// `Ok(None)` for any other first byte, so the caller can treat the
    /// datagram as connected traffic.
    pub fn decode(buf: &[u8]) -> DecodeResult<Option<Self>> {
        let Some(&first) = buf.first() else {
            return Err(DecodeError("empty datagram"));
        };
        let mut r = Reader::new(&buf[1..]);
        let msg = match first {
            id::UNCONNECTED_PING | id::UNCONNECTED_PING_OPEN_CONNECTIONS => {
                let time = r.u64_be()?;
                r.expect_magic()?;
                let client_guid = r.u64_be()?;
                Self::UnconnectedPing {
                    time,
                    client_guid,
                    open_only: first == id::UNCONNECTED_PING_OPEN_CONNECTIONS,
                }
            }
            id::UNCONNECTED_PONG => {
                let time = r.u64_be()?;
                let server_guid = r.u64_be()?;
                r.expect_magic()?;
                Self::UnconnectedPong {
                    time,
                    server_guid,
                    data: r.rest().to_vec(),
                }
            }
            id::OPEN_CONNECTION_REQUEST_1 => {
                r.expect_magic()?;
                let protocol = r.u8()?;
                Self::OpenConnectionRequest1 {
                    protocol,
                    mtu: buf.len() as u16 + UDP_HEADER_SIZE,
                }
            }
            id::OPEN_CONNECTION_REPLY_1 => {
                r.expect_magic()?;
                let server_guid = r.u64_be()?;
                let cookie = if r.bool()? { Some(r.u32_be()?) } else { None };
                Self::OpenConnectionReply1 {
                    server_guid,
                    cookie,
                    mtu: r.u16_be()?,
                }
            }
            id::OPEN_CONNECTION_REQUEST_2 => {
                r.expect_magic()?;
                let server_address = r.address()?;
                let mtu = r.u16_be()?;
                Self::OpenConnectionRequest2 {
                    server_address,
                    mtu,
                    client_guid: r.u64_be()?,
                }
            }
            id::OPEN_CONNECTION_REPLY_2 => {
                r.expect_magic()?;
                let server_guid = r.u64_be()?;
                let client_address = r.address()?;
                let mtu = r.u16_be()?;
                Self::OpenConnectionReply2 {
                    server_guid,
                    client_address,
                    mtu,
                    security: r.bool()?,
                }
            }
            id::INCOMPATIBLE_PROTOCOL_VERSION => {
                let protocol = r.u8()?;
                r.expect_magic()?;
                Self::IncompatibleProtocolVersion {
                    protocol,
                    server_guid: r.u64_be()?,
                }
            }
            id::ALREADY_CONNECTED
            | id::NO_FREE_INCOMING_CONNECTIONS
            | id::CONNECTION_BANNED
            | id::IP_RECENTLY_CONNECTED => {
                r.expect_magic()?;
                Self::Refused {
                    id: first,
                    server_guid: r.u64_be()?,
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Replies captured from the original dedicated server.
    #[test]
    fn decodes_original_server_replies() {
        let pong = hex("1c000001a1d677448d0006602db7b5f7b700ffff00fefefefefdfdfdfd12345678");
        assert_eq!(
            OfflineMessage::decode(&pong),
            Ok(Some(OfflineMessage::UnconnectedPong {
                time: 0x01a1d677448d,
                server_guid: 0x0006602db7b5f7b7,
                data: vec![]
            }))
        );
        let incompatible = hex("190600ffff00fefefefefdfdfdfd123456780006602db7b5f7b7");
        assert_eq!(
            OfflineMessage::decode(&incompatible),
            Ok(Some(OfflineMessage::IncompatibleProtocolVersion {
                protocol: 6,
                server_guid: 0x0006602db7b5f7b7
            }))
        );
        let reply1 = hex("0600ffff00fefefefefdfdfdfd123456780006602db7b5f7b7000416");
        assert_eq!(
            OfflineMessage::decode(&reply1),
            Ok(Some(OfflineMessage::OpenConnectionReply1 {
                server_guid: 0x0006602db7b5f7b7,
                cookie: None,
                mtu: 1046
            }))
        );
    }

    #[test]
    fn request1_is_padded_to_the_mtu() {
        let msg = OfflineMessage::OpenConnectionRequest1 {
            protocol: 6,
            mtu: 1492,
        };
        let bytes = msg.encode();
        assert_eq!(bytes.len(), 1492 - 28);
        assert_eq!(OfflineMessage::decode(&bytes), Ok(Some(msg)));
    }

    #[test]
    fn round_trips() {
        let addr: SocketAddr = "127.0.0.1:16261".parse().unwrap();
        for msg in [
            OfflineMessage::UnconnectedPing {
                time: 5,
                client_guid: 9,
                open_only: false,
            },
            OfflineMessage::UnconnectedPing {
                time: 5,
                client_guid: 9,
                open_only: true,
            },
            OfflineMessage::OpenConnectionReply1 {
                server_guid: 1,
                cookie: Some(77),
                mtu: 1200,
            },
            OfflineMessage::OpenConnectionRequest2 {
                server_address: addr,
                mtu: 1492,
                client_guid: 3,
            },
            OfflineMessage::OpenConnectionReply2 {
                server_guid: 1,
                client_address: addr,
                mtu: 1492,
                security: false,
            },
            OfflineMessage::Refused {
                id: id::NO_FREE_INCOMING_CONNECTIONS,
                server_guid: 4,
            },
        ] {
            assert_eq!(OfflineMessage::decode(&msg.encode()), Ok(Some(msg)));
        }
    }

    #[test]
    fn connected_traffic_is_not_offline() {
        assert_eq!(OfflineMessage::decode(&[0x84, 0, 0, 0]), Ok(None));
    }
}
