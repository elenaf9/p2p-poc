// We create a custom network behaviour that combines Kademlia protocol and mDNS protocol.
// mDNS enables detecting other peers in a local network
// Kademlia is a DTH to identify other nodes and exchange information
// RequestResponse Protocol with generic Request / Responde messages for custom behaviour

use crate::command_protocol::{
    CommandCodec,
    CommandRequest::{self, Other as OtherReq, Ping},
    CommandResponse::{self, Other as OtherRes, Pong},
};
use libp2p::{
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    mdns::{Mdns, MdnsEvent},
    request_response::{
        RequestId, RequestResponse,
        RequestResponseEvent::{self, InboundFailure, Message, OutboundFailure},
        RequestResponseMessage::{Request, Response},
        ResponseChannel,
    },
    swarm::NetworkBehaviourEventProcess,
    NetworkBehaviour,
};

#[derive(NetworkBehaviour)]
pub struct P2PNetworkBehaviour {
    pub(crate) kademlia: Kademlia<MemoryStore>,
    pub(crate) mdns: Mdns,
    pub(crate) msg_proto: RequestResponse<CommandCodec>,
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
    fn inject_event(&mut self, _message: KademliaEvent) {}
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<CommandRequest, CommandResponse>>
    for P2PNetworkBehaviour
{
    // Called when the command_protocol produces an event.
    fn inject_event(&mut self, event: RequestResponseEvent<CommandRequest, CommandResponse>) {
        match event {
            Message { peer: _, message } => match message {
                Request {
                    request_id: _,
                    request,
                    channel,
                } => self.handle_request_msg(request, channel),
                Response {
                    request_id,
                    response,
                } => self.handle_response_msg(request_id, response),
            },
            OutboundFailure {
                peer,
                request_id,
                error,
            } => println!(
                "Outbound Failure for request {:?} to peer: {:?}: {:?}",
                request_id, peer, error
            ),
            InboundFailure {
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


impl P2PNetworkBehaviour {
    fn handle_request_msg(
        &mut self,
        request: CommandRequest,
        channel: ResponseChannel<CommandResponse>,
    ) {
        match request {
            Ping => {
                println!("Received Ping, we will send a Pong back");
                self.msg_proto.send_response(channel, Pong);
            }
            OtherReq(cmd) => {
                println!(
                    "Received command: {:?}, we will Send a 'success' back",
                    String::from_utf8(cmd)
                );
                // TODO: react to received command
                self.msg_proto
                    .send_response(channel, OtherRes(String::from("Success").into_bytes()))
            }
        }
    }

    fn handle_response_msg(&mut self, request_id: RequestId, response: CommandResponse) {
        match response {
            Pong => {
                println!("Received Pong for request {:?}", request_id);
            }
            OtherRes(result) => {
                println!(
                    "Received Result for request {:?}: {:?}",
                    request_id,
                    String::from_utf8(result)
                );
            }
        }
    }
}
