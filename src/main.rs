use crate::command_protocol::{CommandCodec, CommandProtocol, CommandRequest, CommandResponse};
use async_std::{io, task};
use futures::{future, prelude::*};
use libp2p::{
    identity,
    kad::{record::store::MemoryStore, GetClosestPeersOk, Kademlia, KademliaEvent, QueryResult},
    mdns::{Mdns, MdnsEvent},
    request_response::{
        ProtocolSupport, RequestResponse, RequestResponseConfig, RequestResponseEvent,
        RequestResponseMessage,
    },
    swarm::{NetworkBehaviour, NetworkBehaviourEventProcess},
    NetworkBehaviour, PeerId, Swarm,
};
use std::{
    error::Error,
    iter,
    string::String,
    task::{Context, Poll},
};

use std::str::FromStr;

mod dht_proto {
    include!(concat!(env!("OUT_DIR"), "/dht.pb.rs"));
}
mod command_protocol;

// We create a custom network behaviour that combines Kademlia protocol and mDNS protocol.
// mDNS enables detecting other peers in a local network
// Kademlia is a DTH to identify other nodes and exchange information
#[derive(NetworkBehaviour)]
struct P2PNetworkBehaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    msg_proto: RequestResponse<CommandCodec>,
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

impl NetworkBehaviourEventProcess<RequestResponseEvent<CommandRequest, CommandResponse>>
    for P2PNetworkBehaviour
{
    // Called when `ping` produces an event.
    fn inject_event(&mut self, event: RequestResponseEvent<CommandRequest, CommandResponse>) {
        match event {
            RequestResponseEvent::Message { peer: _, message } => match message {
                RequestResponseMessage::Request {
                    request_id: _,
                    request,
                    channel,
                } => match request {
                    CommandRequest::Ping => {
                        println!("Received Ping, we will send a Pong back");
                        self.msg_proto.send_response(channel, CommandResponse::Pong);
                    }
                    CommandRequest::Other { cmd, args } => {
                        println!("Received command: {:?} {:?}, we will Send a 'success' back", cmd, args);
                        self.msg_proto.send_response(channel, CommandResponse::Other(String::from("Success").into_bytes()))
                    }
                },
                RequestResponseMessage::Response {
                    request_id,
                    response,
                } => {
                    match response {
                        CommandResponse::Pong => {
                            println!("Received Pong for request {:?}", request_id);
                        }
                        CommandResponse::Other(result) => {
                            println!("Received Result for request {:?}: {:?}", request_id, String::from_utf8(result));
                        }
                    }
                }
            },
            RequestResponseEvent::OutboundFailure {
                peer,
                request_id,
                error,
            } => println!(
                "Outbound Failure for request {:?} to peer: {:?}: {:?}",
                request_id, peer, error
            ),
            RequestResponseEvent::InboundFailure {
                peer,
                request_id,
                error,
            } => println!(
                "Inbound Failure for request {:?} to peer: {:?}: {:?}",
                request_id, peer, error
            ),
        }
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
        let cfg = RequestResponseConfig::default();
        let protocols = iter::once((CommandProtocol(), ProtocolSupport::Full));
        let msg_proto = RequestResponse::new(CommandCodec(), protocols.clone(), cfg);
        let behaviour = P2PNetworkBehaviour {
            kademlia,
            mdns,
            msg_proto,
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
                Poll::Ready(Some(line)) => {
                    let mut args = line.split_whitespace();
                    match args.next() {
                        Some("PING") => {
                            if let Some(peer_id) = args.next() {
                                if let Some(peer) = PeerId::from_str(peer_id).ok() {
                                    let ping = CommandRequest::Ping;
                                    println!("Sending Ping to peer {:?}", peer);
                                    swarm.msg_proto.send_request(&peer, ping);
                                } else {
                                    println!("Faulty target peer id");
                                }
                            } else {
                                println!("Expected target peer id");
                            }
                        }
                        Some("CMD") => {
                            if let Some(peer_id) = args.next() {
                                if let Some(peer) = PeerId::from_str(peer_id).ok() {
                                    let cmd = {
                                        match args.next() {
                                            Some(c) => c,
                                            None => {
                                                println!("Expected command");
                                                ""
                                            }
                                        }
                                    };
                                    let other = CommandRequest::Other {
                                        cmd: String::from(cmd),
                                        args: vec![args.collect()],
                                    };
                                    println!("Sending command '{:?}' to peer: {:?}", cmd, peer);
                                    swarm.msg_proto.send_request(&peer, other);
                                } else {
                                    println!("Faulty target peer id");
                                }
                            } else {
                                println!("Expected target peer id");
                            }
                        }
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
                        if let Some(a) = Swarm::listeners(&swarm).next() {
                            println!("Listening on {:?}", a);
                            println!("Type LIST to view current bucket entries");
                            println!("Type PING <peer_id> to ping another peer");
                            println!("Type CMD <peer_id> <message> to send a command(message to another peer");
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
