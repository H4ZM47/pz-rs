//! The reliability layer of one connection (`ReliabilityLayer`), without
//! I/O: the caller feeds received datagrams in and sends what
//! [`Session::poll_transmit`] returns.
//!
//! It numbers outgoing frames, packs them into datagrams no larger than the
//! negotiated MTU, splits messages that do not fit, resends reliable frames
//! until they are acknowledged (or NAKed), and on the receive side ACKs every
//! data datagram, NAKs gaps, drops duplicate reliable frames, reassembles
//! split messages and delivers ordered and sequenced messages the way RakNet
//! does: ordered messages in index order per channel, sequenced messages
//! only if newer than the last one delivered on their channel.

use std::collections::{BTreeMap, HashMap, VecDeque};

use super::codec::{DecodeResult, Writer};
use super::datagram::{ranges_from_sorted, Datagram, Frame, Range, Split, DATA_HEADER_SIZE};
use super::Reliability;
use super::UDP_HEADER_SIZE;

/// RakNet's `NUMBER_OF_ORDERED_STREAMS`.
pub const ORDERING_CHANNELS: usize = 32;

/// Retransmission timeout before any round trip is measured.
pub const INITIAL_RTO_MS: u64 = 1000;
pub const MIN_RTO_MS: u64 = 100;
pub const MAX_RTO_MS: u64 = 10_000;

/// Largest split message accepted, as a guard against a peer announcing an
/// absurd part count.
pub const MAX_SPLIT_PARTS: u32 = 1 << 16;

const U24_MASK: u32 = 0x00ff_ffff;

fn u24_add(a: u32, b: u32) -> u32 {
    a.wrapping_add(b) & U24_MASK
}

/// `a` is newer than `b` in 24-bit sequence space.
fn u24_newer(a: u32, b: u32) -> bool {
    a != b && (a.wrapping_sub(b) & U24_MASK) < (1 << 23)
}

struct SentDatagram {
    sent_ms: u64,
    /// Reliable frames to resend if this datagram is lost.
    reliable: Vec<Frame>,
}

struct SplitAssembly {
    parts: Vec<Option<Vec<u8>>>,
    have: u32,
    template: Frame,
}

/// Counters for diagnostics (`RakNetStatistics` subset).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub datagrams_sent: u64,
    pub datagrams_received: u64,
    pub resent_frames: u64,
    pub duplicate_frames: u64,
}

pub struct Session {
    mtu: u16,
    // Send side.
    next_datagram: u32,
    next_reliable: u32,
    next_split_id: u16,
    ordered_write: [u32; ORDERING_CHANNELS],
    sequenced_write: [u32; ORDERING_CHANNELS],
    outgoing: VecDeque<Frame>,
    unacked: BTreeMap<u32, SentDatagram>,
    srtt_ms: Option<f64>,
    rttvar_ms: f64,
    // Receive side.
    ack_queue: Vec<u32>,
    nak_queue: Vec<u32>,
    highest_datagram: Option<u32>,
    reliable_base: u32,
    reliable_seen: BTreeMap<u32, ()>,
    ordered_read: [u32; ORDERING_CHANNELS],
    highest_sequenced_read: [Option<u32>; ORDERING_CHANNELS],
    /// Early ordered/sequenced frames per channel, keyed by [`held_key`].
    ordered_held: Vec<BTreeMap<(u32, u8, u32), Frame>>,
    splits: HashMap<u16, SplitAssembly>,
    delivered: VecDeque<Delivered>,
    last_receive_ms: u64,
    pub stats: SessionStats,
}

/// A message handed up to the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub reliability: Reliability,
    pub channel: u8,
    pub payload: Vec<u8>,
}

impl Session {
    /// `mtu` is the value settled in the offline handshake (it includes
    /// [`UDP_HEADER_SIZE`]).
    pub fn new(mtu: u16, now_ms: u64) -> Self {
        Self {
            mtu,
            next_datagram: 0,
            next_reliable: 0,
            next_split_id: 0,
            ordered_write: [0; ORDERING_CHANNELS],
            sequenced_write: [0; ORDERING_CHANNELS],
            outgoing: VecDeque::new(),
            unacked: BTreeMap::new(),
            srtt_ms: None,
            rttvar_ms: 0.0,
            ack_queue: Vec::new(),
            nak_queue: Vec::new(),
            highest_datagram: None,
            reliable_base: 0,
            reliable_seen: BTreeMap::new(),
            ordered_read: [0; ORDERING_CHANNELS],
            highest_sequenced_read: [None; ORDERING_CHANNELS],
            ordered_held: (0..ORDERING_CHANNELS).map(|_| BTreeMap::new()).collect(),
            splits: HashMap::new(),
            delivered: VecDeque::new(),
            last_receive_ms: now_ms,
            stats: SessionStats::default(),
        }
    }

    pub fn mtu(&self) -> u16 {
        self.mtu
    }

    /// Bytes available for frames in one datagram.
    fn datagram_budget(&self) -> usize {
        usize::from(self.mtu - UDP_HEADER_SIZE) - DATA_HEADER_SIZE
    }

    /// Milliseconds since anything arrived from the peer.
    pub fn idle_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_receive_ms)
    }

    /// Smoothed round-trip time, once measured.
    pub fn rtt_ms(&self) -> Option<f64> {
        self.srtt_ms
    }

    fn rto_ms(&self) -> u64 {
        match self.srtt_ms {
            None => INITIAL_RTO_MS,
            Some(s) => ((s + 4.0 * self.rttvar_ms) as u64).clamp(MIN_RTO_MS, MAX_RTO_MS),
        }
    }

    /// Whether reliable data is still waiting for an acknowledgement.
    pub fn has_unacked(&self) -> bool {
        !self.unacked.is_empty() || !self.outgoing.is_empty()
    }

    /// Queues a message (`ReliabilityLayer::Send`). `channel` is used only
    /// by ordered and sequenced reliabilities.
    pub fn send(&mut self, payload: &[u8], reliability: Reliability, channel: u8) {
        let ch = usize::from(channel) % ORDERING_CHANNELS;
        let mut frame = Frame::new(reliability, Vec::new());
        frame.ordering_channel = ch as u8;
        if reliability.is_sequenced() {
            frame.ordering_index = self.ordered_write[ch];
            frame.sequencing_index = self.sequenced_write[ch];
            self.sequenced_write[ch] = u24_add(self.sequenced_write[ch], 1);
        } else if reliability.is_ordered() {
            frame.ordering_index = self.ordered_write[ch];
            self.ordered_write[ch] = u24_add(self.ordered_write[ch], 1);
            self.sequenced_write[ch] = 0;
        }

        let budget = self.datagram_budget();
        if Frame::header_size(reliability, false) + payload.len() <= budget {
            frame.payload = payload.to_vec();
            self.queue_frame(frame);
            return;
        }
        // Split parts must arrive, so unreliable messages become reliable.
        frame.reliability = match reliability {
            Reliability::Unreliable | Reliability::UnreliableWithAckReceipt => {
                Reliability::Reliable
            }
            Reliability::UnreliableSequenced => Reliability::ReliableSequenced,
            r => r,
        };
        let part = budget - Frame::header_size(frame.reliability, true);
        let count = payload.len().div_ceil(part) as u32;
        let id = self.next_split_id;
        self.next_split_id = self.next_split_id.wrapping_add(1);
        for (index, chunk) in payload.chunks(part).enumerate() {
            let mut p = frame.clone();
            p.split = Some(Split {
                count,
                id,
                index: index as u32,
            });
            p.payload = chunk.to_vec();
            self.queue_frame(p);
        }
    }

    fn queue_frame(&mut self, mut frame: Frame) {
        if frame.reliability.is_reliable() {
            frame.reliable_index = self.next_reliable;
            self.next_reliable = u24_add(self.next_reliable, 1);
        }
        self.outgoing.push_back(frame);
    }

    /// Takes the next message the peer sent, in delivery order.
    pub fn receive(&mut self) -> Option<Delivered> {
        self.delivered.pop_front()
    }

    /// Feeds one connected datagram from the peer.
    pub fn handle_datagram(&mut self, buf: &[u8], now_ms: u64) -> DecodeResult<()> {
        let d = Datagram::decode(buf)?;
        self.last_receive_ms = now_ms;
        self.stats.datagrams_received += 1;
        match d {
            Datagram::Ack { ranges, .. } => {
                for n in expand(&ranges) {
                    if let Some(sent) = self.unacked.remove(&n) {
                        self.sample_rtt(now_ms.saturating_sub(sent.sent_ms) as f64);
                    }
                }
            }
            Datagram::Nak { ranges } => {
                for n in expand(&ranges) {
                    if let Some(sent) = self.unacked.remove(&n) {
                        self.stats.resent_frames += sent.reliable.len() as u64;
                        // Resend first, ahead of new data.
                        for f in sent.reliable.into_iter().rev() {
                            self.outgoing.push_front(f);
                        }
                    }
                }
            }
            Datagram::Data { number, frames, .. } => {
                self.ack_queue.push(number);
                match self.highest_datagram {
                    None => {
                        for missing in 0..number.min(1 << 10) {
                            self.nak_queue.push(missing);
                        }
                        self.highest_datagram = Some(number);
                    }
                    Some(h) if u24_newer(number, h) => {
                        let mut m = u24_add(h, 1);
                        while m != number && self.nak_queue.len() < 1 << 10 {
                            self.nak_queue.push(m);
                            m = u24_add(m, 1);
                        }
                        self.highest_datagram = Some(number);
                    }
                    _ => self.nak_queue.retain(|&m| m != number),
                }
                for f in frames {
                    self.handle_frame(f);
                }
            }
        }
        Ok(())
    }

    fn sample_rtt(&mut self, rtt: f64) {
        match self.srtt_ms {
            None => {
                self.srtt_ms = Some(rtt);
                self.rttvar_ms = rtt / 2.0;
            }
            Some(s) => {
                self.rttvar_ms = 0.75 * self.rttvar_ms + 0.25 * (s - rtt).abs();
                self.srtt_ms = Some(0.875 * s + 0.125 * rtt);
            }
        }
    }

    /// Records a reliable index; false if it was already seen.
    fn first_sighting(&mut self, index: u32) -> bool {
        if index != self.reliable_base && !u24_newer(index, self.reliable_base) {
            return false;
        }
        if self.reliable_seen.insert(index, ()).is_some() {
            return false;
        }
        while self.reliable_seen.remove(&self.reliable_base).is_some() {
            self.reliable_base = u24_add(self.reliable_base, 1);
        }
        true
    }

    fn handle_frame(&mut self, frame: Frame) {
        if frame.reliability.is_reliable() && !self.first_sighting(frame.reliable_index) {
            self.stats.duplicate_frames += 1;
            return;
        }
        let frame = match frame.split {
            Some(split) => match self.assemble(frame, split) {
                Some(whole) => whole,
                None => return,
            },
            None => frame,
        };
        let ch = usize::from(frame.ordering_channel) % ORDERING_CHANNELS;
        if !frame.reliability.is_ordered() {
            self.deliver(frame);
            return;
        }
        if frame.ordering_index != self.ordered_read[ch] {
            // Early: hold until the ordered message before it is delivered.
            // Late: an ordered duplicate or a stale sequenced message.
            if u24_newer(frame.ordering_index, self.ordered_read[ch]) {
                self.ordered_held[ch].insert(held_key(&frame), frame);
            }
            return;
        }
        self.deliver_in_order(ch, frame);
        // Drain everything now due on this channel: sequenced messages for the
        // current index first, then the next ordered message, repeatedly.
        loop {
            let read = self.ordered_read[ch];
            let Some(&key) = self.ordered_held[ch]
                .range((read, 0, 0)..=(read, 1, u32::MAX))
                .next()
                .map(|(k, _)| k)
            else {
                break;
            };
            let next = self.ordered_held[ch].remove(&key).unwrap();
            self.deliver_in_order(ch, next);
        }
    }

    /// Delivers an ordered or sequenced frame whose ordering index is the
    /// channel's current read index.
    fn deliver_in_order(&mut self, ch: usize, frame: Frame) {
        if frame.reliability.is_sequenced() {
            let newer = match self.highest_sequenced_read[ch] {
                None => true,
                Some(h) => u24_newer(frame.sequencing_index, h),
            };
            if newer {
                self.highest_sequenced_read[ch] = Some(frame.sequencing_index);
                self.deliver(frame);
            }
        } else {
            self.ordered_read[ch] = u24_add(self.ordered_read[ch], 1);
            self.highest_sequenced_read[ch] = None;
            self.deliver(frame);
        }
    }

    fn assemble(&mut self, frame: Frame, split: Split) -> Option<Frame> {
        if split.count > MAX_SPLIT_PARTS {
            return None;
        }
        let entry = self
            .splits
            .entry(split.id)
            .or_insert_with(|| SplitAssembly {
                parts: vec![None; split.count as usize],
                have: 0,
                template: Frame {
                    payload: Vec::new(),
                    split: None,
                    ..frame.clone()
                },
            });
        if entry.parts.len() != split.count as usize {
            return None;
        }
        let slot = &mut entry.parts[split.index as usize];
        if slot.is_none() {
            *slot = Some(frame.payload);
            entry.have += 1;
        }
        if entry.have < split.count {
            return None;
        }
        let done = self.splits.remove(&split.id)?;
        let mut whole = done.template;
        whole.payload = done.parts.into_iter().flatten().flatten().collect();
        Some(whole)
    }

    fn deliver(&mut self, frame: Frame) {
        self.delivered.push_back(Delivered {
            reliability: frame.reliability,
            channel: frame.ordering_channel,
            payload: frame.payload,
        });
    }

    /// Datagrams to put on the wire now: pending ACKs and NAKs, resends of
    /// reliable frames whose timeout passed, then queued frames.
    pub fn poll_transmit(&mut self, now_ms: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if !self.ack_queue.is_empty() {
            let mut acks = std::mem::take(&mut self.ack_queue);
            acks.sort_unstable();
            acks.dedup();
            for chunk in range_chunks(&ranges_from_sorted(&acks), self.datagram_budget()) {
                out.push(
                    Datagram::Ack {
                        ranges: chunk,
                        arrival_rate: None,
                    }
                    .encode(),
                );
            }
        }
        if !self.nak_queue.is_empty() {
            let mut naks = std::mem::take(&mut self.nak_queue);
            naks.sort_unstable();
            naks.dedup();
            for chunk in range_chunks(&ranges_from_sorted(&naks), self.datagram_budget()) {
                out.push(Datagram::Nak { ranges: chunk }.encode());
            }
        }

        let rto = self.rto_ms();
        let expired: Vec<u32> = self
            .unacked
            .iter()
            .filter(|(_, s)| now_ms.saturating_sub(s.sent_ms) >= rto)
            .map(|(&n, _)| n)
            .collect();
        for n in expired.into_iter().rev() {
            if let Some(sent) = self.unacked.remove(&n) {
                self.stats.resent_frames += sent.reliable.len() as u64;
                for f in sent.reliable.into_iter().rev() {
                    self.outgoing.push_front(f);
                }
            }
        }

        let budget = self.datagram_budget();
        while !self.outgoing.is_empty() {
            let mut frames = Vec::new();
            let mut used = 0;
            while let Some(f) = self.outgoing.front() {
                let size = f.encoded_size();
                if used + size > budget && !frames.is_empty() {
                    break;
                }
                used += size;
                frames.push(self.outgoing.pop_front().unwrap());
            }
            let number = self.next_datagram;
            self.next_datagram = u24_add(self.next_datagram, 1);
            let reliable: Vec<Frame> = frames
                .iter()
                .filter(|f| f.reliability.is_reliable())
                .cloned()
                .collect();
            out.push(Datagram::data(number, frames).encode());
            if !reliable.is_empty() {
                self.unacked.insert(
                    number,
                    SentDatagram {
                        sent_ms: now_ms,
                        reliable,
                    },
                );
            }
        }
        self.stats.datagrams_sent += out.len() as u64;
        out
    }
}

/// Sequenced frames sort before the ordered frame sharing their ordering
/// index, since they were sent before it.
fn held_key(f: &Frame) -> (u32, u8, u32) {
    if f.reliability.is_sequenced() {
        (f.ordering_index, 0, f.sequencing_index)
    } else {
        (f.ordering_index, 1, 0)
    }
}

fn expand(ranges: &[Range]) -> impl Iterator<Item = u32> + '_ {
    ranges.iter().flat_map(|r| {
        let len = (r.max.wrapping_sub(r.min) & U24_MASK).min(1 << 16);
        (0..=len).map(move |i| u24_add(r.min, i))
    })
}

/// Splits a range list so each ACK/NAK datagram fits the budget.
fn range_chunks(ranges: &[Range], budget: usize) -> Vec<Vec<Range>> {
    // Header byte + count + up to 7 bytes per range.
    let per = (budget.saturating_sub(3) / 7).max(1);
    ranges.chunks(per).map(<[Range]>::to_vec).collect()
}

/// Encodes a single frame on its own, for callers that need a hand-built
/// datagram.
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut w = Writer::new();
    frame.encode(&mut w);
    w.into_vec()
}

#[cfg(test)]
mod tests;
