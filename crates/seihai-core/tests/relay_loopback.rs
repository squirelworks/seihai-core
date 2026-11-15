//! Authored loopback relay; exercises production sockets, not native execution.
use seihai_core::relay::{Client, Player};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

struct Link {
    socket: TcpStream,
    input: Vec<u8>,
    room: String,
    peers: BTreeSet<usize>,
    live: bool,
}

#[test]
fn three_clients_pair_full_mesh_exchange_state_rejoin_and_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let shutdown = stop.clone();
    let captured = Arc::new(Mutex::new(Vec::<(usize, Vec<u8>)>::new()));
    let wire = captured.clone();
    let confirmed = Arc::new(Mutex::new(BTreeSet::<(usize, usize)>::new()));
    let confirmations = confirmed.clone();
    let inject = Arc::new(AtomicBool::new(false));
    let inject_server = inject.clone();
    let server = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(15);
        let mut links: Vec<Link> = Vec::new();
        while !shutdown.load(Ordering::Relaxed) && Instant::now() < until {
            while let Ok((socket, _)) = listener.accept() {
                socket.set_nonblocking(true).unwrap();
                links.push(Link {
                    socket,
                    input: Vec::new(),
                    room: String::new(),
                    peers: BTreeSet::new(),
                    live: true,
                });
            }
            let mut commands = Vec::new();
            for (i, link) in links.iter_mut().enumerate().filter(|(_, l)| l.live) {
                let mut b = [0; 512];
                match link.socket.read(&mut b) {
                    Ok(0) => {
                        link.live = false;
                        commands.push((i, b"C".to_vec()));
                    }
                    Ok(n) => link.input.extend_from_slice(&b[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => panic!("test relay read: {e}"),
                }
                while let Some(end) = link.input.iter().position(|&b| b == b'\n') {
                    let mut line: Vec<_> = link.input.drain(..=end).collect();
                    line.pop();
                    commands.push((i, line));
                }
            }
            for (i, line) in commands {
                wire.lock().unwrap().push((i, line.clone()));
                let Some((&command, body)) = line.split_first() else {
                    continue;
                };
                match command {
                    b'N' => {
                        let room = String::from_utf8_lossy(body)
                            .split('#')
                            .next()
                            .unwrap()
                            .to_owned();
                        links[i].room = room.clone();
                        // Deliberately fragment identity and coalesce the room-mode lines.
                        links[i].socket.write_all(b"NR\nNL\nI0000").unwrap();
                        links[i]
                            .socket
                            .write_all(format!("{:04}\n", i + 1).as_bytes())
                            .unwrap();
                        for j in 0..links.len() {
                            if i != j && links[j].live && links[j].room == room {
                                links[i]
                                    .socket
                                    .write_all(format!("+{:08}def\n", j + 1).as_bytes())
                                    .unwrap();
                                links[j]
                                    .socket
                                    .write_all(format!("+{:08}def\n", i + 1).as_bytes())
                                    .unwrap();
                            }
                        }
                    }
                    b'P' => {
                        let j = std::str::from_utf8(body).unwrap().parse::<usize>().unwrap() - 1;
                        // An accepted request joins both existing parties,
                        // rather than adding only one isolated pair edge.
                        let mut party = BTreeSet::from([i, j]);
                        loop {
                            let before = party.len();
                            for member in party.iter().copied().collect::<Vec<_>>() {
                                party.extend(links[member].peers.iter().copied());
                            }
                            if party.len() == before {
                                break;
                            }
                        }
                        party.retain(|&member| {
                            links[member].live && links[member].room == links[i].room
                        });
                        for &member in &party {
                            for &peer in &party {
                                if member != peer && links[member].peers.insert(peer) {
                                    confirmations.lock().unwrap().insert((member, peer + 1));
                                    links[member]
                                        .socket
                                        .write_all(format!("P{:08}\n", peer + 1).as_bytes())
                                        .unwrap();
                                }
                            }
                        }
                    }
                    b'D' => {
                        let mut data = line;
                        data.push(b'\n');
                        let peers: Vec<_> = links[i].peers.iter().copied().collect();
                        for j in peers {
                            if links[j].live {
                                links[j].socket.write_all(&data).unwrap();
                            }
                        }
                    }
                    b'-' => {}
                    b'C' => {
                        links[i].room.clear();
                        links[i].peers.clear();
                        for (j, link) in links.iter_mut().enumerate() {
                            if j != i && link.live && link.peers.remove(&i) {
                                link.socket
                                    .write_all(format!("C{:08}\n-{:08}\n", i + 1, i + 1).as_bytes())
                                    .unwrap();
                            }
                        }
                    }
                    _ => {}
                }
            }
            if inject_server.swap(false, Ordering::Relaxed) {
                // Unknown sender 97 must not create a player; a malformed
                // coordinate from known sender 2 must not mutate its position.
                // The final valid chat is a processing barrier for assertions.
                links[3]
                    .socket
                    .write_all(
                        b"+00000002def\n-00000002\nD \x81NUnknown\nD \x22P\0FF\nD \x22Cbarrier\n",
                    )
                    .unwrap();
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let player = |nick: &str, x| Player {
        nickname: nick.into(),
        level: "generated.graal".into(),
        x,
        y: 18.5,
        sprite: 3,
        colors: [1, 2, 3, 4, 5],
    };
    let mut a = Client::connect(&endpoint, player("Alpha", 12.5)).unwrap();
    assert_eq!(
        a.chat("not ready yet").unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let mut b = Client::connect(&endpoint, player("Beta", 20.0)).unwrap();
    fn until(a: &mut Client, b: &mut Client, predicate: impl Fn(&Client, &Client) -> bool) {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            a.poll().unwrap();
            b.poll().unwrap();
            if predicate(a, b) {
                break;
            }
            assert!(Instant::now() < end, "relay condition timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }
    until(&mut a, &mut b, |a, b| {
        a.peers()
            .any(|p| p.nickname == "Beta" && p.colors == [1, 2, 3, 4, 5])
            && b.peers()
                .any(|p| p.nickname == "Alpha" && p.colors == [1, 2, 3, 4, 5])
    });
    let peer = b.peers().next().unwrap();
    assert_eq!((peer.x, peer.y, peer.colors), (12.5, 18.5, [1, 2, 3, 4, 5]));
    {
        let wire = captured.lock().unwrap();
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 0 && line == b"NGraal8#def"));
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 1 && line == b"NG8generated.graal#def"));
        // Independent literal bytes: sender 2, x=12.5, y=18.5, sprite=3.
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 1 && line == b"D \x22P\x39\x45\x23"));
        assert!(!wire
            .iter()
            .any(|(_, line)| line.windows(5).any(|part| part == b"GSERV")));
    }
    inject.store(true, Ordering::Relaxed);
    until(&mut a, &mut b, |_, b| {
        b.peers().any(|p| p.chat == "barrier")
    });
    assert_eq!(b.peers().count(), 1);
    assert_eq!(
        (b.peers().next().unwrap().x, b.peers().next().unwrap().y),
        (12.5, 18.5)
    );
    b.take_chats();
    a.update("generated.graal", 14.0, 19.0, 7).unwrap();
    a.chat("generated chat").unwrap();
    until(&mut a, &mut b, |_, b| {
        b.peers()
            .any(|p| p.x == 14.0 && p.sprite == 7 && p.chat == "generated chat")
    });
    assert!(b
        .take_chats()
        .iter()
        .any(|(_, text)| text == "generated chat"));
    {
        let wire = captured.lock().unwrap();
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 1 && line == b"D \x22P\x3c\x46\x27"));
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 1 && line == b"D \x22Cgenerated chat"));
        assert_eq!(
            wire.iter()
                .filter(|(i, line)| *i == 3 && line == b"P00000002")
                .count(),
            1,
            "duplicate advertisement must not request the same partner again"
        );
    }
    // Identical repeated text is a fresh event, not deduplicated by value.
    a.chat("generated chat").unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        a.poll().unwrap();
        b.poll().unwrap();
        if b.take_chats()
            .iter()
            .any(|(_, text)| text == "generated chat")
        {
            break;
        }
        assert!(Instant::now() < end, "identical chat event missing");
        thread::sleep(Duration::from_millis(2));
    }
    let mut c = Client::connect(&endpoint, player("Charlie", 8.5)).unwrap();
    fn until_three(
        a: &mut Client,
        b: &mut Client,
        c: &mut Client,
        predicate: impl Fn(&Client, &Client, &Client) -> bool,
    ) {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            a.poll().unwrap();
            b.poll().unwrap();
            c.poll().unwrap();
            if predicate(a, b, c) {
                break;
            }
            assert!(
                Instant::now() < end,
                "three-client relay condition timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
    until_three(&mut a, &mut b, &mut c, |a, b, c| {
        a.peers().count() == 2
            && b.peers().count() == 2
            && c.peers().count() == 2
            && a.peers()
                .any(|p| p.nickname == "Charlie" && p.colors == [1, 2, 3, 4, 5])
            && b.peers()
                .any(|p| p.nickname == "Charlie" && p.colors == [1, 2, 3, 4, 5])
            && c.peers().any(|p| p.nickname == "Alpha" && p.x == 14.0)
            && c.peers().any(|p| p.nickname == "Beta")
            && captured
                .lock()
                .unwrap()
                .iter()
                .any(|(i, line)| *i == 4 && line == b"D \x25NCharlie")
    });
    {
        let wire = captured.lock().unwrap();
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 4 && line == b"NGraal8#def"));
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 5 && line == b"NG8generated.graal#def"));
        // The newcomer requests its first advertised peer, not every peer.
        // Joining that peer's existing party must still connect all three.
        assert!(wire.iter().any(|(i, line)| *i == 4 && line == b"P00000001"));
        assert!(wire.iter().any(|(i, line)| *i == 5 && line == b"P00000002"));
        assert!(!wire.iter().any(
            |(i, line)| (*i == 4 && line == b"P00000003") || (*i == 5 && line == b"P00000004")
        ));
        // Confirm all directed party edges in BOTH rooms, independently of
        // the level-only peers exposed by the production rendering API.
        let confirmed = confirmed.lock().unwrap();
        for edge in [
            (0, 3),
            (0, 5),
            (2, 1),
            (2, 5),
            (4, 1),
            (4, 3),
            (1, 4),
            (1, 6),
            (3, 2),
            (3, 6),
            (5, 2),
            (5, 4),
        ] {
            assert!(
                confirmed.contains(&edge),
                "missing fixed/level party confirmation: {edge:?}"
            );
        }
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 4 && line == b"D \x25NCharlie"));
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 5 && line == b"D \x26NCharlie"));
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 5 && line == b"D \x26P\x31\x45\x23"));
    }
    c.update("generated.graal", 24.0, 25.0, 6).unwrap();
    c.chat("third client chat").unwrap();
    until_three(&mut a, &mut b, &mut c, |a, b, _| {
        [a, b].iter().all(|client| {
            client.peers().any(|p| {
                p.id == 6
                    && p.x == 24.0
                    && p.y == 25.0
                    && p.sprite == 6
                    && p.chat == "third client chat"
            })
        })
    });
    assert!(a
        .take_chats()
        .iter()
        .any(|(id, text)| *id == 6 && text == "third client chat"));
    assert!(b
        .take_chats()
        .iter()
        .any(|(id, text)| *id == 6 && text == "third client chat"));
    {
        let wire = captured.lock().unwrap();
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 5 && line == b"D \x26P\x50\x52\x26"));
        assert!(wire
            .iter()
            .any(|(i, line)| *i == 5 && line == b"D \x26Cthird client chat"));
        assert!(!wire
            .iter()
            .any(|(i, line)| *i == 4 && line == b"D \x25Cthird client chat"));
    }
    a.update("other.graal", 10.0, 10.0, 0).unwrap();
    until_three(&mut a, &mut b, &mut c, |a, b, c| {
        a.peers().next().is_none()
            && b.peers().count() == 1
            && c.peers().count() == 1
            && b.peers().any(|p| p.nickname == "Charlie")
            && c.peers().any(|p| p.nickname == "Beta")
    });
    a.update("generated.graal", 12.0, 12.0, 2).unwrap();
    until_three(&mut a, &mut b, &mut c, |a, b, c| {
        a.peers().count() == 2
            && b.peers().count() == 2
            && c.peers().count() == 2
            && a.peers().any(|p| p.nickname == "Charlie")
            && a.peers().any(|p| p.nickname == "Beta")
            && b.peers().any(|p| p.nickname == "Alpha")
            && c.peers().any(|p| p.nickname == "Alpha")
    });
    drop(c);
    until(&mut a, &mut b, |a, b| {
        a.peers().count() == 1 && b.peers().count() == 1
    });
    a.update("generated.graal", 13.0, 12.0, 3).unwrap();
    a.chat("surviving party").unwrap();
    until(&mut a, &mut b, |_, b| {
        b.peers()
            .any(|p| p.id == 2 && p.x == 13.0 && p.chat == "surviving party")
    });
    drop(a);
    let end = Instant::now() + Duration::from_secs(3);
    while b.peers().next().is_some() {
        b.poll().unwrap();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
}
