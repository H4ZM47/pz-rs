//! Control messages exchanged inside frames once the offline handshake is
//! done: the connection request and its acceptance, connected pings, and
//! disconnection.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use super::codec::{DecodeError, DecodeResult, Reader, Writer};
use super::{id, MAX_INTERNAL_IDS};

/// Filler for unused internal-address slots when this side writes fewer than
/// [`MAX_INTERNAL_IDS`] addresses. Not yet compared with what the original
/// sends in those slots.
pub fn unassigned_address() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, 0))
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlMessage {
    /// `ID_CONNECTED_PING`.
    ConnectedPing { time: u64 },
    /// `ID_CONNECTED_PONG`.
    ConnectedPong { ping_time: u64, pong_time: u64 },
    /// `ID_CONNECTION_REQUEST`. `password` is the raw trailing bytes compared
    /// against the server's incoming password.
    ConnectionRequest {
        client_guid: u64,
        time: u64,
        security: bool,
        password: Vec<u8>,
    },
    /// `ID_CONNECTION_REQUEST_ACCEPTED`.
    ConnectionRequestAccepted {
        client_address: SocketAddr,
        system_index: u16,
        internal_addresses: Vec<SocketAddr>,
        request_time: u64,
        accept_time: u64,
    },
    /// `ID_NEW_INCOMING_CONNECTION`.
    NewIncomingConnection {
        server_address: SocketAddr,
        internal_addresses: Vec<SocketAddr>,
        ping_time: u64,
        pong_time: u64,
    },
    /// `ID_DISCONNECTION_NOTIFICATION`.
    Disconnect,
}

fn write_internal(w: &mut Writer, addrs: &[SocketAddr]) {
    for i in 0..MAX_INTERNAL_IDS {
        w.address(addrs.get(i).unwrap_or(&unassigned_address()));
    }
}

fn read_internal(r: &mut Reader) -> DecodeResult<Vec<SocketAddr>> {
    (0..MAX_INTERNAL_IDS).map(|_| r.address()).collect()
}

impl ControlMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::ConnectedPing { time } => {
                w.u8(id::CONNECTED_PING).u64_be(*time);
            }
            Self::ConnectedPong {
                ping_time,
                pong_time,
            } => {
                w.u8(id::CONNECTED_PONG)
                    .u64_be(*ping_time)
                    .u64_be(*pong_time);
            }
            Self::ConnectionRequest {
                client_guid,
                time,
                security,
                password,
            } => {
                w.u8(id::CONNECTION_REQUEST)
                    .u64_be(*client_guid)
                    .u64_be(*time)
                    .bool(*security)
                    .bytes(password);
            }
            Self::ConnectionRequestAccepted {
                client_address,
                system_index,
                internal_addresses,
                request_time,
                accept_time,
            } => {
                w.u8(id::CONNECTION_REQUEST_ACCEPTED)
                    .address(client_address)
                    .u16_be(*system_index);
                write_internal(&mut w, internal_addresses);
                w.u64_be(*request_time).u64_be(*accept_time);
            }
            Self::NewIncomingConnection {
                server_address,
                internal_addresses,
                ping_time,
                pong_time,
            } => {
                w.u8(id::NEW_INCOMING_CONNECTION).address(server_address);
                write_internal(&mut w, internal_addresses);
                w.u64_be(*ping_time).u64_be(*pong_time);
            }
            Self::Disconnect => {
                w.u8(id::DISCONNECTION_NOTIFICATION);
            }
        }
        w.into_vec()
    }

    /// Decodes a frame payload whose first byte is a control message id;
    /// `Ok(None)` for anything else (game packets and the rest).
    pub fn decode(buf: &[u8]) -> DecodeResult<Option<Self>> {
        let Some(&first) = buf.first() else {
            return Err(DecodeError("empty message"));
        };
        let mut r = Reader::new(&buf[1..]);
        let msg = match first {
            id::CONNECTED_PING => Self::ConnectedPing { time: r.u64_be()? },
            id::CONNECTED_PONG => Self::ConnectedPong {
                ping_time: r.u64_be()?,
                pong_time: r.u64_be()?,
            },
            id::CONNECTION_REQUEST => {
                let client_guid = r.u64_be()?;
                let time = r.u64_be()?;
                let security = r.bool()?;
                Self::ConnectionRequest {
                    client_guid,
                    time,
                    security,
                    password: r.rest().to_vec(),
                }
            }
            id::CONNECTION_REQUEST_ACCEPTED => {
                let client_address = r.address()?;
                let system_index = r.u16_be()?;
                let internal_addresses = read_internal(&mut r)?;
                let request_time = r.u64_be()?;
                let accept_time = r.u64_be()?;
                Self::ConnectionRequestAccepted {
                    client_address,
                    system_index,
                    internal_addresses,
                    request_time,
                    accept_time,
                }
            }
            id::NEW_INCOMING_CONNECTION => {
                let server_address = r.address()?;
                let internal_addresses = read_internal(&mut r)?;
                let ping_time = r.u64_be()?;
                let pong_time = r.u64_be()?;
                Self::NewIncomingConnection {
                    server_address,
                    internal_addresses,
                    ping_time,
                    pong_time,
                }
            }
            id::DISCONNECTION_NOTIFICATION => Self::Disconnect,
            _ => return Ok(None),
        };
        Ok(Some(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let a: SocketAddr = "127.0.0.1:52000".parse().unwrap();
        let s: SocketAddr = "127.0.0.1:16261".parse().unwrap();
        for msg in [
            ControlMessage::ConnectedPing { time: 1 },
            ControlMessage::ConnectedPong {
                ping_time: 1,
                pong_time: 2,
            },
            ControlMessage::ConnectionRequest {
                client_guid: 7,
                time: 8,
                security: false,
                password: b"pw".to_vec(),
            },
            ControlMessage::ConnectionRequestAccepted {
                client_address: a,
                system_index: 0,
                internal_addresses: vec![s; MAX_INTERNAL_IDS],
                request_time: 3,
                accept_time: 4,
            },
            ControlMessage::NewIncomingConnection {
                server_address: s,
                internal_addresses: vec![a; MAX_INTERNAL_IDS],
                ping_time: 5,
                pong_time: 6,
            },
            ControlMessage::Disconnect,
        ] {
            assert_eq!(ControlMessage::decode(&msg.encode()), Ok(Some(msg)));
        }
        assert_eq!(ControlMessage::decode(&[0x86, 0, 1]), Ok(None));
    }

    #[test]
    fn missing_internal_addresses_are_padded() {
        let msg = ControlMessage::NewIncomingConnection {
            server_address: "127.0.0.1:16261".parse().unwrap(),
            internal_addresses: vec![],
            ping_time: 0,
            pong_time: 0,
        };
        let Some(ControlMessage::NewIncomingConnection {
            internal_addresses, ..
        }) = ControlMessage::decode(&msg.encode()).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            internal_addresses,
            vec![unassigned_address(); MAX_INTERNAL_IDS]
        );
    }
}
