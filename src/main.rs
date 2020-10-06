use crate::mailbox_protocol::{MailboxCodec, MailboxProtocol, MailboxRecord, MailboxRequest};
use crate::network_behaviour::P2PNetworkBehaviour;
use async_std::{
    io::{stdin, BufReader},
    task,
};
use futures::{future, prelude::*};
use libp2p::{
    build_development_transport,
    core::Multiaddr,
    identity::Keypair,
    kad::{record::store::MemoryStore, record::Key, Kademlia, Quorum},
    mdns::Mdns,
    request_response::{ProtocolSupport, RequestResponse, RequestResponseConfig},
    swarm::{ExpandedSwarm, IntoProtocolsHandler, NetworkBehaviour, ProtocolsHandler},
    PeerId, Swarm,
};
use std::{
    error::Error,
    iter,
    str::{FromStr, SplitWhitespace},
    string::String,
    task::{Context, Poll},
};

mod dht_proto {
    include!(concat!(env!("OUT_DIR"), "/dht.pb.rs"));
}
mod mailbox_protocol;
mod network_behaviour;

type P2PNetworkSwarm = ExpandedSwarm<
    P2PNetworkBehaviour,
    <<<P2PNetworkBehaviour as NetworkBehaviour>::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::InEvent,
    <<<P2PNetworkBehaviour as NetworkBehaviour>::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::OutEvent,
    <P2PNetworkBehaviour as NetworkBehaviour>::ProtocolsHandler,
    PeerId,
>;

fn main() -> Result<(), Box<dyn Error>> {
    // Create a random PeerId
    let local_keys = Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_keys.public());
    println!("Local peer id: {:?}", local_peer_id);

    // create a transport
    let transport = build_development_transport(local_keys)?;

    // Create a Kademlia behaviour.
    let kademlia = {
        let store = MemoryStore::new(local_peer_id.clone());
        Kademlia::new(local_peer_id.clone(), store)
    };
    let mdns = Mdns::new()?;

    // Create RequestResponse behaviour with MailboxProtocol
    let msg_proto = {
        // set request_timeout and connection_keep_alive if necessary
        let cfg = RequestResponseConfig::default();
        let protocols = iter::once((MailboxProtocol(), ProtocolSupport::Full));
        RequestResponse::new(MailboxCodec(), protocols, cfg)
    };
    // Create a Swarm that establishes connections through the given transport
    // Use custom behaviour P2PNetworkBehaviour
    let mut swarm = {
        let behaviour = P2PNetworkBehaviour {
            kademlia,
            mdns,
            msg_proto,
        };
        Swarm::new(transport, behaviour, local_peer_id)
    };

    let mut is_swarm_listening = false;
    if let Some(i) = std::env::args().position(|arg| arg == "--port") {
        if let Some(port) = std::env::args().nth(i + 1) {
            let addr = format!("/ip4/0.0.0.0/tcp/{}", port).parse()?;
            Swarm::listen_on(&mut swarm, addr)?;
            is_swarm_listening = true;
        }
    }

    if !is_swarm_listening {
        Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/16384".parse()?)?;
    }

    let mailbox_peer = attempt_connect_mailbox(&mut swarm);

    poll_input(swarm, mailbox_peer)
}

fn attempt_connect_mailbox(swarm: &mut P2PNetworkSwarm) -> Result<PeerId, ()> {
    if let Some(i) = std::env::args().position(|arg| arg == "--mailbox") {
        // Dial peer at fixed addr to connect to p2p network
        if let Some(addr) = std::env::args().nth(i + 1) {
            if let Ok(remote) = Multiaddr::from_str(&*addr) {
                if Swarm::dial_addr(swarm, remote.clone()).is_ok() {
                    println!("Dialed {}", addr);
                    if let Some(peer_id) = std::env::args().nth(i + 2) {
                        if let Ok(peer) = PeerId::from_str(&*peer_id) {
                            swarm.kademlia.add_address(&peer, remote);
                            if swarm.kademlia.bootstrap().is_ok() {
                                println!("Successful bootstrapping");
                            } else {
                                eprintln!("Could not bootstrap");
                            }
                            return Ok(peer);
                        } else {
                            eprintln!("Invalid Peer Id {}", peer_id);
                        }
                    }
                }
            } else {
                eprintln!("Invalid multiaddress {}", addr);
            }
        } else {
            eprintln!("Missing multiaddress");
        }
    }
    Err(())
}

fn poll_input(
    mut swarm: P2PNetworkSwarm,
    mailbox_peer: Result<PeerId, ()>,
) -> Result<(), Box<dyn Error>> {
    let mut stdin = BufReader::new(stdin()).lines();
    let mut listening = false;
    task::block_on(future::poll_fn(move |cx: &mut Context<'_>| {
        loop {
            // poll for user input in stdin
            match stdin.try_poll_next_unpin(cx)? {
                Poll::Ready(Some(line)) => handle_input_line(&mut swarm, line, &mailbox_peer),
                Poll::Ready(None) => panic!("Stdin closed"),
                Poll::Pending => break,
            }
        }
        loop {
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => println!("{:?}", event),
                Poll::Ready(None) => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => {
                    if !listening {
                        for a in Swarm::listeners(&swarm) {
                            println!("Listening on {:?}", a);
                        }
                        listening = true;
                        println!("Type LIST to view current bucket entries");
                        println!("Type PING <peer_id> to ping another peer");
                        println!("Type GET <key> to fetch a record");
                        if mailbox_peer.is_ok() {
                            println!("Type PUT <key> <value> <peer_id:OPTIONAL> to let another peer publish a record, if no peer_id it will use the mailbox peer_id");
                        } else {
                            println!("Type PUT <key> <value> <peer_id> to let another peer publish a record");
                        }
                    }
                    break;
                }
            }
        }
        Poll::Pending
    }))
}

fn handle_input_line(swarm: &mut P2PNetworkSwarm, line: String, mailbox_peer: &Result<PeerId, ()>) {
    let mut args = line.split_whitespace();
    match args.next() {
        Some("PING") => send_ping_to_peer(args, &mut swarm.msg_proto),
        Some("GET") => fetch_record(args, &mut swarm.kademlia),
        Some("PUT") => publish_record(args, &mut swarm.msg_proto, &mailbox_peer),
        Some("LIST") => {
            println!("Current Buckets:");
            for bucket in swarm.kademlia.kbuckets() {
                for entry in bucket.iter() {
                    println!(
                        "key: {:?}, values: {:?}",
                        entry.node.key.preimage(),
                        entry.node.value
                    );
                }
            }
        }
        _ => println!("No valid command"),
    }
}

fn send_ping_to_peer(mut args: SplitWhitespace, msg_proto: &mut RequestResponse<MailboxCodec>) {
    if let Some(peer_id) = args.next() {
        if let Ok(peer) = PeerId::from_str(peer_id) {
            let ping = MailboxRequest::Ping;
            println!("Sending Ping to peer {:?}", peer);
            msg_proto.send_request(&peer, ping);
        } else {
            println!("Faulty target peer id");
        }
    } else {
        println!("Expected target peer id");
    }
}

fn publish_record(
    mut args: SplitWhitespace,
    msg_proto: &mut RequestResponse<MailboxCodec>,
    mailbox_peer: &Result<PeerId, ()>,
) {
    if let Some(key) = args.next() {
        if let Some(value) = args.next() {
            let record = MailboxRecord {
                key: String::from(key),
                value: String::from(value),
            };
            if let Some(peer_id) = args.next() {
                if let Ok(peer) = PeerId::from_str(peer_id) {
                    println!(
                        "Sending publish request for record {:?}:{:?} to peer: {:?}",
                        key, value, peer
                    );
                    msg_proto.send_request(&peer, MailboxRequest::Publish(record));
                } else {
                    println!("Faulty target peer id");
                }
            } else if let Ok(peer) = mailbox_peer {
                println!(
                    "Sending publish request for record {:?}:{:?} to peer: {:?}",
                    key, value, peer
                );
                msg_proto.send_request(&peer, MailboxRequest::Publish(record));
            } else {
                println!("Missing target peer");
            }
        } else {
            println!("Missing value");
        }
    } else {
        println!("Missing key");
    }
}

fn fetch_record(mut args: SplitWhitespace, kademlia: &mut Kademlia<MemoryStore>) {
    let key = {
        match args.next() {
            Some(key) => Key::new(&key),
            None => {
                eprintln!("Expected key");
                return;
            }
        }
    };
    kademlia.get_record(&key, Quorum::One);
}
