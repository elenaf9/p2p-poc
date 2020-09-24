use crate::command_protocol::{CommandCodec, CommandProtocol, CommandRequest};
use crate::network_behaviour::P2PNetworkBehaviour;
use async_std::{
    io::{stdin, BufReader},
    task,
};
use futures::{future, prelude::*};
use libp2p::{
    build_development_transport,
    identity::Keypair,
    kad::{record::store::MemoryStore, Kademlia},
    mdns::Mdns,
    request_response::{ProtocolSupport, RequestResponse, RequestResponseConfig},
    swarm::{ExpandedSwarm, IntoProtocolsHandler, NetworkBehaviour, ProtocolsHandler},
    PeerId, Swarm,
};
use std::{
    error::Error,
    iter,
    str::FromStr,
    str::SplitWhitespace,
    string::String,
    task::{Context, Poll},
};

mod dht_proto {
    include!(concat!(env!("OUT_DIR"), "/dht.pb.rs"));
}
mod command_protocol;
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

    // Create RequestResponse behaviour with CommandProtocol
    let msg_proto = {
        // set request_timeout and connection_keep_alive if necessary
        let cfg = RequestResponseConfig::default();
        let protocols = iter::once((CommandProtocol(), ProtocolSupport::Full));
        RequestResponse::new(CommandCodec(), protocols.clone(), cfg)
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

    // Tell the swarm to listen on all interfaces and a random, OS-assigned port.
    Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse()?)?;

    poll_input(swarm)
}
fn poll_input(mut swarm: P2PNetworkSwarm) -> Result<(), Box<dyn Error>> {
    let mut stdin = BufReader::new(stdin()).lines();
    let mut listening = false;
    task::block_on(future::poll_fn(move |cx: &mut Context<'_>| {
        loop {
            // poll for user input in stdin
            match stdin.try_poll_next_unpin(cx)? {
                Poll::Ready(Some(line)) => handle_input_line(&mut swarm, line),
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
                            println!("Type CMD <peer_id> <message> to send a command / message to another peer");
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

fn handle_input_line(swarm: &mut P2PNetworkSwarm, line: String) {
    let mut args = line.split_whitespace();
    match args.next() {
        Some("PING") => send_ping_to_peer(args, &mut swarm.msg_proto),
        Some("CMD") => send_cmd_to_peer(args, &mut swarm.msg_proto),
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

fn send_ping_to_peer(mut args: SplitWhitespace, msg_proto: &mut RequestResponse<CommandCodec>) {
    if let Some(peer_id) = args.next() {
        if let Ok(peer) = PeerId::from_str(peer_id) {
            let ping = CommandRequest::Ping;
            println!("Sending Ping to peer {:?}", peer);
            msg_proto.send_request(&peer, ping);
        } else {
            println!("Faulty target peer id");
        }
    } else {
        println!("Expected target peer id");
    }
}

fn send_cmd_to_peer(mut args: SplitWhitespace, msg_proto: &mut RequestResponse<CommandCodec>) {
    if let Some(peer_id) = args.next() {
        if let Ok(peer) = PeerId::from_str(peer_id) {
            let cmd = {
                match args.next() {
                    Some(c) => c,
                    None => {
                        println!("Expected command");
                        ""
                    }
                }
            };
            let other = CommandRequest::Other(cmd.as_bytes().to_vec());
            println!("Sending command {:?} to peer: {:?}", cmd, peer);
            msg_proto.send_request(&peer, other);
        } else {
            println!("Faulty target peer id");
        }
    } else {
        println!("Expected target peer id");
    }
}
