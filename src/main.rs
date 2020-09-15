use async_std::{io, task};
use futures::{future, prelude::*};
use libp2p::kad::record::store::MemoryStore;
use libp2p::kad::{record::Key, GetClosestPeersOk, Kademlia, KademliaEvent, QueryResult};
use libp2p::swarm::NetworkBehaviour;
use libp2p::{
    core::upgrade,
    identity,
    mdns::{Mdns, MdnsEvent},
    noise,
    swarm::NetworkBehaviourEventProcess,
    tcp::TcpConfig,
    yamux, NetworkBehaviour, PeerId, Swarm, Transport,
};
use std::{
    error::Error,
    string::String,
    task::{Context, Poll},
};
use std::ops::Deref;

// We create a custom network behaviour that combines Kademlia protocol and mDNS protocol.
// mDNS enables detecting other peers in a local network
// Kademlia is a DTH to identify other nodes and exchange information
#[derive(NetworkBehaviour)]
struct P2PNetworkBehaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
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
                    let target = peers.iter().find(|&p| {
                        String::from_utf8(key.clone()).unwrap() == p.to_base58()
                    });
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

fn main() -> Result<(), Box<dyn Error>> {
    // Create a random PeerId
    let local_keys = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_keys.public());
    println!("Local peer id: {:?}", local_peer_id);
    // Create a noise key pair for authentication
    let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&local_keys)
        .unwrap();
    let noise = noise::NoiseConfig::xx(noise_keys).into_authenticated();
    let yamux = yamux::Config::default();

    // create a transport that negotiates the noise and yamux protocols on all connections.
    let transport = TcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(noise)
        .multiplex(yamux);

    // Create a Swarm that establishes connections through the given transport
    // Use custom behaviour 2PNetworkBehaviour
    let mut swarm = {
        // Create a Kademlia behaviour.
        let store = MemoryStore::new(local_peer_id.clone());
        let kademlia = Kademlia::new(local_peer_id.clone(), store);
        let mdns = Mdns::new()?;
        let behaviour = P2PNetworkBehaviour { kademlia, mdns };
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
                Poll::Ready(Some(line)) => handle_input_line(&mut swarm.kademlia, line),
                Poll::Ready(None) => panic!("Stdin closed"),
                Poll::Pending => break,
            }
        }
        loop {
            // poll for incoming events from the swarm
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => println!("Received sth: {:?}", event),
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => {
                    if !listening {
                        if let Some(a) = Swarm::listeners(&swarm).next() {
                            println!("Listening on {:?}", a);
                            println!("Type LIST to view current bucket entries and FIND <peer-id> to get the address for any peer");
                            listening = true;
                        }
                    }
                    break;
                }
            }
        }
        Poll::Pending
    }))
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
