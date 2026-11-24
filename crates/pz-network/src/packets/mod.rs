//! Game packet bodies (`zombie.network.packets.*`), each paired with its
//! [`PacketType`](crate::PacketType).

pub mod connection;

use crate::buffer::{NetReader, NetWriter, ReadResult};
use crate::packet_types::PacketType;

/// A packet body with the original's `write` / `parse` byte layout.
pub trait NetPacket: Sized {
    const TYPE: PacketType;

    fn write(&self, w: &mut NetWriter);

    fn parse(r: &mut NetReader) -> ReadResult<Self>;

    /// The full user packet: header and body (`doPacket` + `write`).
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        Self::TYPE.write_header(&mut out);
        let mut w = NetWriter { buf: out };
        self.write(&mut w);
        w.into_vec()
    }
}
