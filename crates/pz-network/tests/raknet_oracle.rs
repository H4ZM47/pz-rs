//! Handshake against a running original game server.
//!
//! Opt-in: set `PZ_RAKNET_ORACLE=host:port` to a dedicated server started
//! from a local install with `-nosteam` and no server password. Without it
//! the test returns early.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use pz_network::raknet::connected::ControlMessage;
use pz_network::raknet::datagram::{ranges_from_sorted, Datagram, Frame};
use pz_network::raknet::offline::OfflineMessage;
use pz_network::raknet::{Reliability, MTU_SIZES, PROTOCOL_VERSION};

fn recv_until<T>(sock: &UdpSocket, mut f: impl FnMut(&[u8]) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        if let Ok(n) = sock.recv(&mut buf) {
            if let Some(v) = f(&buf[..n]) {
                return v;
            }
        }
    }
    panic!("no expected reply from the oracle");
}

#[test]
fn connects_to_the_original_server() {
    let Ok(target) = std::env::var("PZ_RAKNET_ORACLE") else {
        return;
    };
    let server: SocketAddr = target.parse().expect("PZ_RAKNET_ORACLE must be host:port");
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.connect(server).unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let client_guid = 0x5a5a_0000_1234_5678u64;

    // Offline handshake.
    let req1 = OfflineMessage::OpenConnectionRequest1 {
        protocol: PROTOCOL_VERSION,
        mtu: MTU_SIZES[0],
    };
    sock.send(&req1.encode()).unwrap();
    let (server_guid, mtu) = recv_until(&sock, |b| match OfflineMessage::decode(b) {
        Ok(Some(OfflineMessage::OpenConnectionReply1 {
            server_guid,
            cookie: None,
            mtu,
        })) => Some((server_guid, mtu)),
        _ => None,
    });
    assert!(mtu <= MTU_SIZES[0]);
    let req2 = OfflineMessage::OpenConnectionRequest2 {
        server_address: server,
        mtu,
        client_guid,
    };
    sock.send(&req2.encode()).unwrap();
    let client_address = recv_until(&sock, |b| match OfflineMessage::decode(b) {
        Ok(Some(OfflineMessage::OpenConnectionReply2 {
            server_guid: g,
            client_address,
            mtu: m,
            security: false,
        })) if g == server_guid && m == mtu => Some(client_address),
        _ => None,
    });
    assert_eq!(client_address, sock.local_addr().unwrap());

    // Connected handshake.
    let request = ControlMessage::ConnectionRequest {
        client_guid,
        time: 1,
        security: false,
        password: vec![],
    };
    sock.send(
        &Datagram::data(0, vec![Frame::new(Reliability::Reliable, request.encode())]).encode(),
    )
    .unwrap();
    let mut acked = false;
    let (accept, their_number) = recv_until(&sock, |b| match Datagram::decode(b) {
        Ok(Datagram::Ack { ranges, .. }) => {
            acked |= ranges.iter().any(|r| r.min == 0);
            None
        }
        Ok(Datagram::Data { number, frames, .. }) => {
            frames
                .iter()
                .find_map(|f| match ControlMessage::decode(&f.payload) {
                    Ok(Some(m @ ControlMessage::ConnectionRequestAccepted { .. })) => {
                        Some((m, number))
                    }
                    _ => None,
                })
        }
        _ => None,
    });
    let ControlMessage::ConnectionRequestAccepted {
        client_address: echoed,
        request_time,
        ..
    } = accept
    else {
        unreachable!()
    };
    assert_eq!(echoed, client_address);
    assert_eq!(request_time, 1);
    eprintln!("oracle accepted: server guid {server_guid:#x}, mtu {mtu}, client {client_address}");

    sock.send(
        &Datagram::Ack {
            ranges: ranges_from_sorted(&[their_number]),
            arrival_rate: None,
        }
        .encode(),
    )
    .unwrap();
    let mut incoming = Frame::new(
        Reliability::ReliableOrdered,
        ControlMessage::NewIncomingConnection {
            server_address: server,
            internal_addresses: vec![],
            ping_time: 1,
            pong_time: 1,
        }
        .encode(),
    );
    incoming.reliable_index = 1;
    sock.send(&Datagram::data(1, vec![incoming]).encode())
        .unwrap();
    // Our datagram 0 must have been acknowledged by now (possibly bundled
    // after the accept); give the server a moment otherwise.
    if !acked {
        recv_until(&sock, |b| match Datagram::decode(b) {
            Ok(Datagram::Ack { ranges, .. }) if ranges.iter().any(|r| r.min <= 1) => Some(()),
            _ => None,
        });
    }

    // Stay connected long enough for the server's Java side to register the
    // connection before it is torn down.
    std::thread::sleep(Duration::from_secs(1));
    let mut bye = Frame::new(
        Reliability::ReliableOrdered,
        ControlMessage::Disconnect.encode(),
    );
    bye.reliable_index = 2;
    bye.ordering_index = 1;
    sock.send(&Datagram::data(2, vec![bye]).encode()).unwrap();
}

/// Offline handshake; returns the settled MTU.
fn open_connection(sock: &UdpSocket, server: SocketAddr, client_guid: u64) -> u16 {
    let req1 = OfflineMessage::OpenConnectionRequest1 {
        protocol: PROTOCOL_VERSION,
        mtu: MTU_SIZES[0],
    };
    sock.send(&req1.encode()).unwrap();
    let mtu = recv_until(sock, |b| match OfflineMessage::decode(b) {
        Ok(Some(OfflineMessage::OpenConnectionReply1 { mtu, .. })) => Some(mtu),
        _ => None,
    });
    let req2 = OfflineMessage::OpenConnectionRequest2 {
        server_address: server,
        mtu,
        client_guid,
    };
    sock.send(&req2.encode()).unwrap();
    recv_until(sock, |b| match OfflineMessage::decode(b) {
        Ok(Some(OfflineMessage::OpenConnectionReply2 { mtu, .. })) => Some(mtu),
        _ => None,
    })
}

#[test]
fn holds_a_session_with_the_original_server() {
    use pz_network::raknet::session::Session;
    let Ok(target) = std::env::var("PZ_RAKNET_ORACLE") else {
        return;
    };
    let server: SocketAddr = target.parse().expect("PZ_RAKNET_ORACLE must be host:port");
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.connect(server).unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(10)))
        .unwrap();
    let mtu = open_connection(&sock, server, 0x5a5a_0000_8765_4321);

    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut session = Session::new(mtu, now());
    let request = ControlMessage::ConnectionRequest {
        client_guid: 0x5a5a_0000_8765_4321,
        time: now(),
        security: false,
        password: vec![],
    };
    session.send(&request.encode(), Reliability::Reliable, 0);

    let mut accepted = false;
    let mut pings = 0;
    let mut buf = [0u8; 2048];
    let hold = Duration::from_secs(6);
    while start.elapsed() < hold {
        if let Ok(n) = sock.recv(&mut buf) {
            session
                .handle_datagram(&buf[..n], now())
                .expect("server datagram decodes");
        }
        while let Some(msg) = session.receive() {
            match ControlMessage::decode(&msg.payload).expect("control message decodes") {
                Some(ControlMessage::ConnectionRequestAccepted {
                    request_time,
                    accept_time,
                    ..
                }) => {
                    accepted = true;
                    let reply = ControlMessage::NewIncomingConnection {
                        server_address: server,
                        internal_addresses: vec![],
                        ping_time: request_time,
                        pong_time: accept_time,
                    };
                    session.send(&reply.encode(), Reliability::ReliableOrdered, 0);
                }
                Some(ControlMessage::ConnectedPing { time }) => {
                    pings += 1;
                    let pong = ControlMessage::ConnectedPong {
                        ping_time: time,
                        pong_time: now(),
                    };
                    session.send(&pong.encode(), Reliability::Unreliable, 0);
                }
                other => eprintln!("oracle sent {other:?} ({} bytes)", msg.payload.len()),
            }
        }
        for d in session.poll_transmit(now()) {
            sock.send(&d).unwrap();
        }
    }
    assert!(accepted, "server never accepted the connection");
    assert!(
        !session.has_unacked(),
        "server left reliable frames unacknowledged"
    );
    assert!(session.idle_ms(now()) < 6000, "server went silent");
    eprintln!(
        "session held {hold:?}: {pings} pings answered, rtt {:?} ms, {:?}",
        session.rtt_ms(),
        session.stats
    );

    session.send(
        &ControlMessage::Disconnect.encode(),
        Reliability::ReliableOrdered,
        0,
    );
    for d in session.poll_transmit(now()) {
        sock.send(&d).unwrap();
    }
}
