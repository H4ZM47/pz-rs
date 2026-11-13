//! A RakNet client connection over a UDP socket: the part of
//! `RakNetPeerInterface` / `UdpEngine` a game client uses to reach a server.
//!
//! [`RakClient::connect`] runs the offline handshake (stepping down through
//! [`MTU_SIZES`]) and the connected handshake, then [`RakClient::poll`]
//! pumps the socket: it answers the server's connected pings, sends its own
//! every [`PING_INTERVAL`], and hands game packets up.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::packet_types::{parse_header, PacketType};
use crate::packets::NetPacket;
use crate::raknet::connected::ControlMessage;
use crate::raknet::datagram::is_connected;
use crate::raknet::offline::OfflineMessage;
use crate::raknet::session::Session;
use crate::raknet::{id, Reliability, MTU_SIZES, PROTOCOL_VERSION};

/// How often the client sends a connected ping (RakNet's occasional ping).
pub const PING_INTERVAL: Duration = Duration::from_secs(5);

/// RakNet's default `SetTimeoutTime`: silence after which a connection is lost.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Offline handshake attempts per MTU size before stepping down.
const ATTEMPTS_PER_MTU: u32 = 4;
const ATTEMPT_WAIT: Duration = Duration::from_millis(500);

/// Why a connection attempt failed, in terms of the RakNet message the game
/// would have seen.
#[derive(Debug)]
pub enum ConnectError {
    Io(io::Error),
    /// `ID_CONNECTION_ATTEMPT_FAILED`: no answer.
    NoResponse,
    /// `ID_INCOMPATIBLE_PROTOCOL_VERSION`.
    IncompatibleProtocol(u8),
    /// `ID_INVALID_PASSWORD`.
    InvalidPassword,
    /// Another refusal (`ID_NO_FREE_INCOMING_CONNECTIONS`, `ID_CONNECTION_BANNED`, ...).
    Refused(u8),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "socket error: {e}"),
            Self::NoResponse => f.write_str("connection attempt failed: no response"),
            Self::IncompatibleProtocol(v) => write!(f, "server uses RakNet protocol {v}"),
            Self::InvalidPassword => f.write_str("invalid server password"),
            Self::Refused(id) => write!(f, "server refused the connection (message {id})"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// What [`RakClient::poll`] produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientEvent {
    /// A game packet: its type and body.
    Packet(PacketType, Vec<u8>),
    /// A user packet whose id is outside the table, or another message the
    /// transport does not handle itself.
    Other(Vec<u8>),
    /// `ID_DISCONNECTION_NOTIFICATION` from the server.
    Disconnected,
    /// `ID_CONNECTION_LOST`: nothing arrived for the timeout.
    ConnectionLost,
}

pub struct RakClient {
    socket: UdpSocket,
    server: SocketAddr,
    server_guid: u64,
    session: Session,
    start: Instant,
    last_ping: Instant,
    timeout: Duration,
    closed: bool,
}

impl RakClient {
    fn now_ms(start: Instant) -> u64 {
        start.elapsed().as_millis() as u64
    }

    /// `RakPeer::Connect` followed by waiting for
    /// `ID_CONNECTION_REQUEST_ACCEPTED`. `password` is the raw bytes the
    /// server compares (the game sends its hashed server password, or
    /// nothing when the server has none).
    pub fn connect(
        server: SocketAddr,
        password: &[u8],
        client_guid: u64,
    ) -> Result<Self, ConnectError> {
        let bind: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        }
        .parse()
        .unwrap();
        let socket = UdpSocket::bind(bind)?;
        socket.connect(server)?;
        let start = Instant::now();

        let (server_guid, mtu) = Self::open_connection(&socket, server, client_guid)?;
        let mut session = Session::new(mtu, Self::now_ms(start));
        let request = ControlMessage::ConnectionRequest {
            client_guid,
            time: Self::now_ms(start),
            security: false,
            password: password.to_vec(),
        };
        session.send(&request.encode(), Reliability::Reliable, 0);

        let mut client = Self {
            socket,
            server,
            server_guid,
            session,
            start,
            last_ping: Instant::now(),
            timeout: DEFAULT_TIMEOUT,
            closed: false,
        };
        let deadline = Instant::now() + DEFAULT_TIMEOUT;
        let mut buf = vec![0u8; usize::from(MTU_SIZES[0])];
        while Instant::now() < deadline {
            client.flush()?;
            let Some(n) = client.recv(&mut buf, Duration::from_millis(50))? else {
                continue;
            };
            let data = &buf[..n];
            if !is_connected(data) {
                match data[0] {
                    id::INVALID_PASSWORD => return Err(ConnectError::InvalidPassword),
                    id::ALREADY_CONNECTED
                    | id::NO_FREE_INCOMING_CONNECTIONS
                    | id::CONNECTION_BANNED
                    | id::IP_RECENTLY_CONNECTED => return Err(ConnectError::Refused(data[0])),
                    _ => continue,
                }
            }
            let now = Self::now_ms(client.start);
            if client.session.handle_datagram(data, now).is_err() {
                continue;
            }
            while let Some(msg) = client.session.receive() {
                match ControlMessage::decode(&msg.payload) {
                    Ok(Some(ControlMessage::ConnectionRequestAccepted {
                        request_time,
                        accept_time,
                        ..
                    })) => {
                        let reply = ControlMessage::NewIncomingConnection {
                            server_address: server,
                            internal_addresses: vec![],
                            ping_time: request_time,
                            pong_time: accept_time,
                        };
                        client
                            .session
                            .send(&reply.encode(), Reliability::ReliableOrdered, 0);
                        client.flush()?;
                        return Ok(client);
                    }
                    Ok(Some(ControlMessage::ConnectedPing { time })) => client.pong(time),
                    _ if msg.payload.first() == Some(&id::INVALID_PASSWORD) => {
                        return Err(ConnectError::InvalidPassword);
                    }
                    _ => {}
                }
            }
        }
        Err(ConnectError::NoResponse)
    }

    fn open_connection(
        socket: &UdpSocket,
        server: SocketAddr,
        client_guid: u64,
    ) -> Result<(u64, u16), ConnectError> {
        let mut buf = vec![0u8; usize::from(MTU_SIZES[0])];
        socket.set_read_timeout(Some(ATTEMPT_WAIT))?;
        for &mtu in &MTU_SIZES {
            for _ in 0..ATTEMPTS_PER_MTU {
                socket.send(
                    &OfflineMessage::OpenConnectionRequest1 {
                        protocol: PROTOCOL_VERSION,
                        mtu,
                    }
                    .encode(),
                )?;
                let deadline = Instant::now() + ATTEMPT_WAIT;
                while Instant::now() < deadline {
                    let n = match socket.recv(&mut buf) {
                        Ok(n) => n,
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) =>
                        {
                            break
                        }
                        // An unreachable port reports as a reset on some systems.
                        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => break,
                        Err(e) => return Err(e.into()),
                    };
                    match OfflineMessage::decode(&buf[..n]) {
                        Ok(Some(OfflineMessage::OpenConnectionReply1 {
                            mtu: reply_mtu, ..
                        })) => {
                            let mtu = reply_mtu.min(mtu);
                            return Self::open_connection_2(
                                socket,
                                server,
                                client_guid,
                                mtu,
                                &mut buf,
                            );
                        }
                        Ok(Some(OfflineMessage::IncompatibleProtocolVersion {
                            protocol, ..
                        })) => {
                            return Err(ConnectError::IncompatibleProtocol(protocol));
                        }
                        Ok(Some(OfflineMessage::Refused { id, .. })) => {
                            return Err(ConnectError::Refused(id))
                        }
                        _ => {}
                    }
                }
            }
        }
        Err(ConnectError::NoResponse)
    }

    fn open_connection_2(
        socket: &UdpSocket,
        server: SocketAddr,
        client_guid: u64,
        mtu: u16,
        buf: &mut [u8],
    ) -> Result<(u64, u16), ConnectError> {
        let request = OfflineMessage::OpenConnectionRequest2 {
            server_address: server,
            mtu,
            client_guid,
        }
        .encode();
        for _ in 0..ATTEMPTS_PER_MTU {
            socket.send(&request)?;
            let deadline = Instant::now() + ATTEMPT_WAIT;
            while Instant::now() < deadline {
                let n = match socket.recv(buf) {
                    Ok(n) => n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        break
                    }
                    Err(e) => return Err(e.into()),
                };
                match OfflineMessage::decode(&buf[..n]) {
                    Ok(Some(OfflineMessage::OpenConnectionReply2 {
                        server_guid, mtu, ..
                    })) => {
                        return Ok((server_guid, mtu));
                    }
                    Ok(Some(OfflineMessage::Refused { id, .. })) => {
                        return Err(ConnectError::Refused(id))
                    }
                    _ => {}
                }
            }
        }
        Err(ConnectError::NoResponse)
    }

    pub fn server_address(&self) -> SocketAddr {
        self.server
    }

    /// The server's RakNet GUID (`getGuidOfPacket` on its packets).
    pub fn server_guid(&self) -> u64 {
        self.server_guid
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Queues raw bytes (a complete user packet, header included).
    pub fn send_raw(&mut self, data: &[u8], reliability: Reliability, channel: u8) {
        self.session.send(data, reliability, channel);
    }

    /// `PacketType.send`: queues a packet with its type's reliability and
    /// ordering channel.
    pub fn send_packet<P: NetPacket>(&mut self, packet: &P) {
        let setting = P::TYPE.setting();
        self.session.send(
            &packet.encode(),
            setting.reliability.into(),
            setting.ordering,
        );
    }

    fn pong(&mut self, time: u64) {
        let pong = ControlMessage::ConnectedPong {
            ping_time: time,
            pong_time: Self::now_ms(self.start),
        };
        self.session
            .send(&pong.encode(), Reliability::Unreliable, 0);
    }

    fn recv(&self, buf: &mut [u8], wait: Duration) -> io::Result<Option<usize>> {
        self.socket
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        match self.socket.recv(buf) {
            Ok(n) if n > 0 => Ok(Some(n)),
            Ok(_) => Ok(None),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Sends everything the session has ready.
    pub fn flush(&mut self) -> io::Result<()> {
        for d in self.session.poll_transmit(Self::now_ms(self.start)) {
            self.socket.send(&d)?;
        }
        Ok(())
    }

    /// Pumps the connection for up to `wait`: receives datagrams, answers
    /// pings, flushes ACKs and queued sends, and returns what arrived.
    pub fn poll(&mut self, wait: Duration) -> io::Result<Vec<ClientEvent>> {
        let mut events = Vec::new();
        if self.closed {
            return Ok(events);
        }
        let deadline = Instant::now() + wait;
        let mut buf = vec![0u8; usize::from(MTU_SIZES[0])];
        loop {
            if self.last_ping.elapsed() >= PING_INTERVAL {
                self.last_ping = Instant::now();
                let ping = ControlMessage::ConnectedPing {
                    time: Self::now_ms(self.start),
                };
                self.session
                    .send(&ping.encode(), Reliability::Unreliable, 0);
            }
            self.flush()?;
            let left = deadline.saturating_duration_since(Instant::now());
            if let Some(n) = self.recv(&mut buf, left.min(Duration::from_millis(20)))? {
                let now = Self::now_ms(self.start);
                if is_connected(&buf[..n]) && self.session.handle_datagram(&buf[..n], now).is_ok() {
                    self.drain(&mut events);
                }
            }
            if self.closed || Instant::now() >= deadline {
                break;
            }
        }
        if !self.closed
            && self.session.idle_ms(Self::now_ms(self.start)) >= self.timeout.as_millis() as u64
        {
            self.closed = true;
            events.push(ClientEvent::ConnectionLost);
        }
        self.flush()?;
        Ok(events)
    }

    fn drain(&mut self, events: &mut Vec<ClientEvent>) {
        while let Some(msg) = self.session.receive() {
            match ControlMessage::decode(&msg.payload) {
                Ok(Some(ControlMessage::ConnectedPing { time })) => self.pong(time),
                Ok(Some(ControlMessage::ConnectedPong { .. })) => {}
                Ok(Some(ControlMessage::Disconnect)) => {
                    self.closed = true;
                    events.push(ClientEvent::Disconnected);
                }
                Ok(Some(_)) => {}
                _ => match parse_header(&msg.payload) {
                    Some((ty, body)) => events.push(ClientEvent::Packet(ty, body.to_vec())),
                    None => events.push(ClientEvent::Other(msg.payload)),
                },
            }
        }
    }

    /// `RakPeer::CloseConnection` with notification.
    pub fn disconnect(&mut self) -> io::Result<()> {
        if !self.closed {
            self.session.send(
                &ControlMessage::Disconnect.encode(),
                Reliability::ReliableOrdered,
                0,
            );
            self.flush()?;
            self.closed = true;
        }
        Ok(())
    }
}

impl Drop for RakClient {
    fn drop(&mut self) {
        let _ = self.disconnect();
    }
}
