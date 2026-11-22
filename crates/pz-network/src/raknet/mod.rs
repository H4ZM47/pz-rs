//! A pure-Rust RakNet transport, wire compatible with the RakNet library the
//! original game links natively (`libRakNet` behind
//! `zombie.core.raknet.RakNetPeerInterface`).
//!
//! This module holds the wire formats: offline (connectionless) handshake
//! messages, the connected datagram layer with its ACK/NAK ranges and
//! frames, and the connected control messages. The session state machine
//! (resends, ordering, split reassembly) builds on these.
//!
//! Facts checked against the original dedicated server (build 42) run
//! locally: it speaks the stock RakNet 4 offline protocol with
//! [`PROTOCOL_VERSION`] 6, answers other versions with
//! `ID_INCOMPATIBLE_PROTOCOL_VERSION`, offers no security (no cookie) in open
//! connection reply 1, and sends no extra data in its unconnected pong.

pub mod codec;
pub mod connected;
pub mod datagram;
pub mod offline;

pub use codec::{DecodeError, DecodeResult, Reader, Writer};

/// `RAKNET_PROTOCOL_VERSION` of the game's RakNet build.
pub const PROTOCOL_VERSION: u8 = 6;

/// `OFFLINE_MESSAGE_DATA_ID`: marks connectionless messages.
pub const OFFLINE_MAGIC: [u8; 16] = [
    0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78,
];

/// IP + UDP header bytes RakNet counts in its MTU figures.
pub const UDP_HEADER_SIZE: u16 = 28;

/// MTU sizes a RakNet client tries in order during the offline handshake
/// (`mtuSizes` in `RakPeer::Connect`).
pub const MTU_SIZES: [u16; 3] = [1492, 1200, 576];

/// `MAXIMUM_NUMBER_OF_INTERNAL_IDS`: local addresses exchanged on connect.
pub const MAX_INTERNAL_IDS: usize = 10;

/// RakNet `DefaultMessageIDTypes` the transport and the game look at.
pub mod id {
    pub const CONNECTED_PING: u8 = 0x00;
    pub const UNCONNECTED_PING: u8 = 0x01;
    pub const UNCONNECTED_PING_OPEN_CONNECTIONS: u8 = 0x02;
    pub const CONNECTED_PONG: u8 = 0x03;
    pub const DETECT_LOST_CONNECTIONS: u8 = 0x04;
    pub const OPEN_CONNECTION_REQUEST_1: u8 = 0x05;
    pub const OPEN_CONNECTION_REPLY_1: u8 = 0x06;
    pub const OPEN_CONNECTION_REQUEST_2: u8 = 0x07;
    pub const OPEN_CONNECTION_REPLY_2: u8 = 0x08;
    pub const CONNECTION_REQUEST: u8 = 0x09;
    pub const CONNECTION_REQUEST_ACCEPTED: u8 = 0x10;
    pub const CONNECTION_ATTEMPT_FAILED: u8 = 0x11;
    pub const ALREADY_CONNECTED: u8 = 0x12;
    pub const NEW_INCOMING_CONNECTION: u8 = 0x13;
    pub const NO_FREE_INCOMING_CONNECTIONS: u8 = 0x14;
    pub const DISCONNECTION_NOTIFICATION: u8 = 0x15;
    pub const CONNECTION_LOST: u8 = 0x16;
    pub const CONNECTION_BANNED: u8 = 0x17;
    pub const INVALID_PASSWORD: u8 = 0x18;
    pub const INCOMPATIBLE_PROTOCOL_VERSION: u8 = 0x19;
    pub const IP_RECENTLY_CONNECTED: u8 = 0x1a;
    pub const TIMESTAMP: u8 = 0x1b;
    pub const UNCONNECTED_PONG: u8 = 0x1c;
    /// `ID_USER_PACKET_ENUM`; game packets start here.
    pub const USER_PACKET_ENUM: u8 = 0x86;
}

/// RakNet `PacketReliability`, including the ACK-receipt variants that exist
/// only locally (they travel as their base reliability).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Reliability {
    Unreliable = 0,
    UnreliableSequenced = 1,
    Reliable = 2,
    ReliableOrdered = 3,
    ReliableSequenced = 4,
    UnreliableWithAckReceipt = 5,
    ReliableWithAckReceipt = 6,
    ReliableOrderedWithAckReceipt = 7,
}

impl Reliability {
    pub fn from_bits(v: u8) -> Self {
        match v & 7 {
            0 => Self::Unreliable,
            1 => Self::UnreliableSequenced,
            2 => Self::Reliable,
            3 => Self::ReliableOrdered,
            4 => Self::ReliableSequenced,
            5 => Self::UnreliableWithAckReceipt,
            6 => Self::ReliableWithAckReceipt,
            _ => Self::ReliableOrderedWithAckReceipt,
        }
    }

    /// Carries a reliable message number.
    pub fn is_reliable(self) -> bool {
        matches!(
            self,
            Self::Reliable
                | Self::ReliableOrdered
                | Self::ReliableSequenced
                | Self::ReliableWithAckReceipt
                | Self::ReliableOrderedWithAckReceipt
        )
    }

    /// Carries a sequencing index.
    pub fn is_sequenced(self) -> bool {
        matches!(self, Self::UnreliableSequenced | Self::ReliableSequenced)
    }

    /// Carries an ordering index and channel.
    pub fn is_ordered(self) -> bool {
        self.is_sequenced()
            || matches!(
                self,
                Self::ReliableOrdered | Self::ReliableOrderedWithAckReceipt
            )
    }
}

impl From<crate::packet_types::PacketReliability> for Reliability {
    fn from(r: crate::packet_types::PacketReliability) -> Self {
        Self::from_bits(r as u8)
    }
}
