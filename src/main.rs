use async_std::{io, task};
use futures::{future, prelude::*};
use libp2p::kad::record::store::MemoryStore;
use libp2p::kad::{record::Key, GetClosestPeersOk, Kademlia, KademliaEvent, QueryResult};
use libp2p::swarm::{ExpandedSwarm, NetworkBehaviour, SwarmEvent};
use libp2p::tcp::TcpConfig;
use libp2p::{
    core::{muxing, upgrade},
    identify::{Identify, IdentifyEvent, IdentifyInfo},
    identity,
    mdns::{Mdns, MdnsEvent},
    mplex::MplexConfig,
    multiaddr::Multiaddr,
    ping::{self, Ping, PingConfig, PingEvent, PingFailure, PingSuccess},
    swarm::NetworkBehaviourEventProcess,
    NetworkBehaviour, PeerId, Swarm, Transport,
};
use std::str::FromStr;
use std::sync::Arc;
use std::{
    error::Error,
    string::String,
    task::{Context, Poll},
};
use std::borrow::Borrow;

// We create a custom network behaviour that combines Kademlia protocol and mDNS protocol.
// mDNS enables detecting other peers in a local network
// Kademlia is a DTH to identify other nodes and exchange information
#[derive(NetworkBehaviour)]
struct P2PNetworkBehaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    ping: Ping,
    identify: Identify,
}

impl NetworkBehaviourEventProcess<MdnsEvent> for P2PNetworkBehaviour {
    // Called when `mdns` produces an event.
    fn inject_event(&mut self, event: MdnsEvent) {
        if let MdnsEvent::Discovered(list) = event {
            for (peer_id, multiaddr) in list {
                self.kademlia.add_address(&peer_id, multiaddr);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for P2PNetworkBehaviour {
    // Called when `kademlia` produces an event.
    fn inject_event(&mut self, message: KademliaEvent) {
        match message {
            /* KademliaEvent::RoutingUpdated {
                peer, addresses, ..
            } => {
                println!("peer: {:?}, added address: {:?} ", peer, addresses.into_vec().last().unwrap()),
            }*/
            KademliaEvent::QueryResult { result, .. } => match result {
                QueryResult::GetClosestPeers(Ok(GetClosestPeersOk { key, peers })) => {
                    let target = peers
                        .iter()
                        .find(|&p| String::from_utf8(key.clone()).unwrap() == p.to_base58());
                    if target.is_some() {
                        let addr_vec = &self.mdns.addresses_of_peer(target.unwrap());
                        let address = addr_vec.iter().last().unwrap();
                        println!("Found Target on address {:?}", address);
                    } else {
                        println!("Not found; TODO: implement recursive search for target");
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl NetworkBehaviourEventProcess<PingEvent> for P2PNetworkBehaviour {
    // Called when `ping` produces an event.
    fn inject_event(&mut self, event: PingEvent) {
        match event {
            PingEvent {
                peer,
                result: Result::Ok(PingSuccess::Ping { rtt }),
            } => {
                println!(
                    "ping: rtt to {} is {} ms",
                    peer.to_base58(),
                    rtt.as_millis()
                );
            }
            PingEvent {
                peer,
                result: Result::Ok(PingSuccess::Pong),
            } => {
                println!("ping: pong from {}", peer.to_base58());
            }
            PingEvent {
                peer,
                result: Result::Err(PingFailure::Timeout),
            } => {
                println!("ping: timeout to {}", peer.to_base58());
            }
            PingEvent {
                peer,
                result: Result::Err(PingFailure::Other { error }),
            } => {
                println!("ping: failure with {}: {}", peer.to_base58(), error);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for P2PNetworkBehaviour {
    // Called when `identify` produces an event.
    fn inject_event(&mut self, event: IdentifyEvent) {
        println!("identify: {:?}", event);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Create a random PeerId
    let local_keys = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_keys.public());
    println!("Local peer id: {:?}", local_peer_id);

    // create a transport
    let transport = libp2p::build_development_transport(local_keys)?;

    // Create a Swarm that establishes connections through the given transport
    // Use custom behaviour P2PNetworkBehaviour
    let mut swarm = {
        // Create a Kademlia behaviour.
        let store = MemoryStore::new(local_peer_id.clone());
        let kademlia = Kademlia::new(local_peer_id.clone(), store);
        let mdns = Mdns::new()?;
        let ping = Ping::new(PingConfig::new());
        let identify = Identify::new(
            "/ipfs/0.1.0".into(),
            "rust-ipfs-example".into(),
            identity::Keypair::generate_ed25519().public(),
        );
        let behaviour = P2PNetworkBehaviour {
            kademlia,
            mdns,
            ping,
            identify,
        };
        Swarm::new(transport, behaviour, local_peer_id)
    };

    let mut stdin = io::BufReader::new(io::stdin()).lines();

    // Tell the swarm to listen on all interfaces and a random, OS-assigned port.
    Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse()?)?;

    let mut listening = false;
    task::block_on(future::poll_fn(move |cx: &mut Context<'_>| {
        loop {
            // poll for user input in stdin
            match stdin.try_poll_next_unpin(cx)? {
                Poll::Ready(Some(peer_id)) => {
                    if PeerId::from_str(&peer_id).is_ok() {
                        let target = &PeerId::from_str(&peer_id)?;
                        let dial = Swarm::dial(&mut swarm, target);
                        if dial.is_ok() {
                            println!(
                                "dialed peer {:?}, connections: {:?}",
                                target,
                                Swarm::network_info(&swarm).num_connections
                            );
                            if let Some(_) = Swarm::connection_info(&mut swarm, target) {
                                println!("Connected!");
                            } else {
                                println!("Why is this not working? ._.");
                            }
                        } else {
                            println!("Err dialing target peer {:?}", dial.err());
                        }
                    } else {
                        println!("PeerId faulty");
                    }
                    // handle_input_line(&mut swarm.kademlia, line)
                }
                Poll::Ready(None) => panic!("Stdin closed"),
                Poll::Pending => break,
            }
        }
        loop {
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => println!("Received sth: {:?}", event),
                Poll::Ready(None) => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => {
                    if !listening {
                        if let Some(a) = Swarm::listeners(&swarm).next() {
                            println!("Listening on {:?}", a);
                            println!("Type LIST to view current bucket entries and FIND <peer-id> to get the address for any peer");
                            listening = true;
                        }
                    }
                    println!(
                        "connections: {:?}",
                        Swarm::network_info(&swarm).num_connections
                    );
                    break;
                }
            }
        }
        Poll::Pending
    }))
}

async fn send_msg(address: &Multiaddr) {
    let temp_transport = TcpConfig::new()
        .and_then(move |c, e| upgrade::apply(c, MplexConfig::new(), e, upgrade::Version::V1));
    let client = temp_transport.dial(address.clone()).unwrap().await.unwrap();
    let mut outbound = muxing::outbound_from_ref_and_wrap(Arc::new(client))
        .await
        .unwrap();
    println!("asdasd");
    outbound.write_all(b"hello world").await.unwrap();
    outbound.close().await.unwrap()
}

fn handle_input_line(kademlia: &mut Kademlia<MemoryStore>, line: String) {
    let mut args = line.split(" ");

    match args.next() {
        Some("FIND") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        println!("Expected target peer id");
                        return;
                    }
                }
            };
            kademlia.get_closest_peers(key);
        }
        Some("LIST") => {
            println!("Current Buckets:");
            for bucket in kademlia.kbuckets() {
                for entry in bucket.iter() {
                    println!(
                        "key: {:?}, values: {:?}",
                        entry.node.key.preimage(),
                        entry.node.value
                    );
                }
            }
        }
        _ => println!(
            "Options: LIST: lists current buckets in routing table, FIND: find address for peer"
        ),
    }
}
