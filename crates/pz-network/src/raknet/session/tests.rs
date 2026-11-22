use super::*;

const MTU: u16 = 576;

/// Moves datagrams between two sessions over a link that can drop or
/// reorder them, deterministically.
struct Link {
    a: Session,
    b: Session,
    now: u64,
    rng: u64,
    drop_percent: u64,
    shuffle: bool,
}

impl Link {
    fn new(drop_percent: u64, shuffle: bool) -> Self {
        Self {
            a: Session::new(MTU, 0),
            b: Session::new(MTU, 0),
            now: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
            drop_percent,
            shuffle,
        }
    }

    fn roll(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng % 100
    }

    fn step(&mut self) {
        self.now += 50;
        let mut ab = self.a.poll_transmit(self.now);
        let mut ba = self.b.poll_transmit(self.now);
        if self.shuffle {
            ab.reverse();
            ba.reverse();
        }
        for d in ab {
            if self.roll() >= self.drop_percent {
                self.b.handle_datagram(&d, self.now).unwrap();
            }
        }
        for d in ba {
            if self.roll() >= self.drop_percent {
                self.a.handle_datagram(&d, self.now).unwrap();
            }
        }
    }

    fn run(&mut self, steps: usize) {
        for _ in 0..steps {
            self.step();
        }
    }
}

fn drain(s: &mut Session) -> Vec<Delivered> {
    std::iter::from_fn(|| s.receive()).collect()
}

#[test]
fn ordered_messages_survive_loss_and_reordering() {
    let mut link = Link::new(30, true);
    for i in 0..200u32 {
        link.a
            .send(&i.to_be_bytes(), Reliability::ReliableOrdered, 5);
        if i % 20 == 0 {
            link.step();
        }
    }
    link.run(400);
    let got: Vec<u32> = drain(&mut link.b)
        .iter()
        .map(|d| u32::from_be_bytes(d.payload[..].try_into().unwrap()))
        .collect();
    assert_eq!(got, (0..200).collect::<Vec<_>>());
    assert!(!link.a.has_unacked());
    assert!(link.a.stats.resent_frames > 0);
}

#[test]
fn large_messages_are_split_and_reassembled() {
    let mut link = Link::new(20, true);
    let big: Vec<u8> = (0..20_000u32).map(|i| (i * 7) as u8).collect();
    link.a.send(&big, Reliability::ReliableOrdered, 0);
    link.a.send(b"after", Reliability::ReliableOrdered, 0);
    // An unreliable message too big for one datagram is sent reliably.
    let big2 = vec![0xab; 3000];
    link.a.send(&big2, Reliability::Unreliable, 0);
    link.run(400);
    let got = drain(&mut link.b);
    assert_eq!(got.len(), 3);
    // The unordered message may arrive anywhere relative to the ordered two.
    let (unordered, ordered): (Vec<_>, Vec<_>) = got
        .into_iter()
        .partition(|d| d.reliability == Reliability::Reliable);
    assert_eq!(ordered.len(), 2);
    assert!(ordered[0].payload == big, "first ordered message corrupted");
    assert_eq!(ordered[1].payload, b"after");
    assert!(
        unordered[0].payload == big2,
        "split unreliable message corrupted"
    );
}

#[test]
fn every_datagram_fits_the_mtu() {
    let mut s = Session::new(MTU, 0);
    s.send(&vec![1; 10_000], Reliability::ReliableOrdered, 0);
    for i in 0..100u8 {
        s.send(&[i; 37], Reliability::Unreliable, 0);
    }
    for d in s.poll_transmit(0) {
        assert!(d.len() <= usize::from(MTU - UDP_HEADER_SIZE), "{}", d.len());
    }
}

#[test]
fn duplicates_are_dropped() {
    let mut a = Session::new(MTU, 0);
    let mut b = Session::new(MTU, 0);
    a.send(b"x", Reliability::Reliable, 0);
    let d = a.poll_transmit(0).pop().unwrap();
    b.handle_datagram(&d, 0).unwrap();
    b.handle_datagram(&d, 0).unwrap();
    assert_eq!(drain(&mut b).len(), 1);
    assert_eq!(b.stats.duplicate_frames, 1);
}

#[test]
fn stale_sequenced_messages_are_dropped() {
    let mut a = Session::new(MTU, 0);
    let mut b = Session::new(MTU, 0);
    a.send(b"1", Reliability::UnreliableSequenced, 2);
    let first = a.poll_transmit(0).pop().unwrap();
    a.send(b"2", Reliability::UnreliableSequenced, 2);
    let second = a.poll_transmit(0).pop().unwrap();
    b.handle_datagram(&second, 0).unwrap();
    b.handle_datagram(&first, 0).unwrap();
    let got = drain(&mut b);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload, b"2");
}

#[test]
fn sequenced_messages_wait_for_the_ordered_message_before_them() {
    let mut a = Session::new(MTU, 0);
    let mut b = Session::new(MTU, 0);
    a.send(b"ordered", Reliability::ReliableOrdered, 1);
    let ordered = a.poll_transmit(0).pop().unwrap();
    a.send(b"seq", Reliability::ReliableSequenced, 1);
    let seq = a.poll_transmit(0).pop().unwrap();
    b.handle_datagram(&seq, 0).unwrap();
    assert!(b.receive().is_none());
    b.handle_datagram(&ordered, 0).unwrap();
    let got: Vec<Vec<u8>> = drain(&mut b).into_iter().map(|d| d.payload).collect();
    assert_eq!(got, [b"ordered".to_vec(), b"seq".to_vec()]);
}

#[test]
fn unacked_frames_are_resent_after_the_timeout() {
    let mut a = Session::new(MTU, 0);
    a.send(b"r", Reliability::Reliable, 0);
    assert_eq!(a.poll_transmit(0).len(), 1);
    assert!(a.poll_transmit(INITIAL_RTO_MS - 1).is_empty());
    let resent = a.poll_transmit(INITIAL_RTO_MS);
    assert_eq!(resent.len(), 1);
    let Datagram::Data { number, frames, .. } = Datagram::decode(&resent[0]).unwrap() else {
        panic!()
    };
    assert_eq!(number, 1);
    assert_eq!(frames[0].reliable_index, 0);
}

#[test]
fn naks_trigger_immediate_resend() {
    let mut a = Session::new(MTU, 0);
    let mut b = Session::new(MTU, 0);
    a.send(b"lost", Reliability::Reliable, 0);
    let _lost = a.poll_transmit(0);
    a.send(b"kept", Reliability::Reliable, 0);
    for d in a.poll_transmit(10) {
        b.handle_datagram(&d, 10).unwrap();
    }
    // b ACKs datagram 1 and NAKs datagram 0.
    for d in b.poll_transmit(20) {
        a.handle_datagram(&d, 20).unwrap();
    }
    for d in a.poll_transmit(30) {
        b.handle_datagram(&d, 30).unwrap();
    }
    let got: Vec<Vec<u8>> = drain(&mut b).into_iter().map(|d| d.payload).collect();
    assert_eq!(got, [b"kept".to_vec(), b"lost".to_vec()]);
    assert!(a.rtt_ms().is_some());
}

#[test]
fn sequence_space_wraps() {
    assert!(u24_newer(0, U24_MASK));
    assert!(!u24_newer(U24_MASK, 0));
    assert_eq!(u24_add(U24_MASK, 1), 0);
}
