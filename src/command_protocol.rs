use crate::dht_proto as proto;
use async_trait::async_trait;
use futures::{prelude::*, AsyncRead, AsyncWrite};
use libp2p::{
    core::{
        upgrade::{read_one, write_one},
        ProtocolName,
    },
    request_response::RequestResponseCodec,
};
use prost::Message;
use std::io;

#[derive(Debug, Clone)]
pub struct CommandProtocol();
#[derive(Clone)]
pub struct CommandCodec();

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRequest {
    Ping,
    Other { cmd: String, args: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResponse {
    Pong,
    Other(Vec<u8>),
}

impl ProtocolName for CommandProtocol {
    fn protocol_name(&self) -> &[u8] {
        b"/custom-retrieve/1.0.0"
    }
}

#[async_trait]
impl RequestResponseCodec for CommandCodec {
    type Protocol = CommandProtocol;
    type Request = CommandRequest;
    type Response = CommandResponse;

    async fn read_request<T>(
        &mut self,
        _: &CommandProtocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_one(io, 1024)
            .map(|req| match req {
                Ok(bytes) => {
                    let request = proto::Message::decode(io::Cursor::new(bytes))?;
                    proto_msg_to_req(request)
                }
                Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            })
            .await
    }

    async fn read_response<T>(
        &mut self,
        _: &CommandProtocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_one(io, 1024)
            .map(|res| match res {
                Ok(bytes) => {
                    let response = proto::Message::decode(io::Cursor::new(bytes))?;
                    proto_msg_to_res(response)
                }
                Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            })
            .await
    }

    async fn write_request<T>(
        &mut self,
        _: &CommandProtocol,
        io: &mut T,
        req: CommandRequest,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let proto_struct = req_to_proto_msg(req);
        let mut buf = Vec::with_capacity(proto_struct.encoded_len());
        proto_struct
            .encode(&mut buf)
            .expect("Vec<u8> provides capacity as needed");
        write_one(io, buf).await
    }

    async fn write_response<T>(
        &mut self,
        _: &CommandProtocol,
        io: &mut T,
        res: CommandResponse,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let proto_struct = res_to_proto_msg(res);
        let mut buf = Vec::with_capacity(proto_struct.encoded_len());
        proto_struct
            .encode(&mut buf)
            .expect("Vec<u8> provides capacity as needed");
        write_one(io, buf).await
    }
}

fn proto_msg_to_req(msg: proto::Message) -> Result<CommandRequest, io::Error> {
    let msg_type = proto::message::MessageType::from_i32(msg.r#type)
        .ok_or_else(|| invalid_data(format!("unknown message type: {}", msg.r#type)))?;
    match msg_type {
        proto::message::MessageType::Ping => Ok(CommandRequest::Ping),
        proto::message::MessageType::Other => {
            let mut args = Vec::with_capacity(msg.args.len());
            for arg in msg.args.clone().into_iter() {
                args.push(bytes_to_string(arg).ok().unwrap());
            }
            Ok(CommandRequest::Other {
                cmd: String::from(args.first().unwrap()),
                args: args.as_slice()[1..].to_vec(),
            })
        }
    }
}

fn proto_msg_to_res(msg: proto::Message) -> Result<CommandResponse, io::Error> {
    let msg_type = proto::message::MessageType::from_i32(msg.r#type)
        .ok_or_else(|| invalid_data(format!("unknown message type: {}", msg.r#type)))?;
    match msg_type {
        proto::message::MessageType::Ping => Ok(CommandResponse::Pong),
        proto::message::MessageType::Other => {
            let result = msg.result;
            Ok(CommandResponse::Other(result))
        }
    }
}

fn req_to_proto_msg(req: CommandRequest) -> proto::Message {
    match req {
        CommandRequest::Ping => proto::Message {
            r#type: proto::message::MessageType::Ping as i32,
            ..proto::Message::default()
        },
        CommandRequest::Other { cmd, args: _ } => {
            let mut all_args = Vec::new();
            all_args.push(cmd.clone().into_bytes());
            // all_args.copy_from_slice(args.into_iter().map(|a| a.into_bytes()).collect());
            proto::Message {
                r#type: proto::message::MessageType::Other as i32,
                args: all_args.clone(),
                ..proto::Message::default()
            }
        }
    }
}

fn res_to_proto_msg(res: CommandResponse) -> proto::Message {
    match res {
        CommandResponse::Pong => proto::Message {
            r#type: proto::message::MessageType::Ping as i32,
            ..proto::Message::default()
        },
        CommandResponse::Other(result) => proto::Message {
            r#type: proto::message::MessageType::Other as i32,
            result,
            ..proto::Message::default()
        },
    }
}

fn bytes_to_string(bytes: Vec<u8>) -> Result<String, io::Error> {
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Creates an `io::Error` with `io::ErrorKind::InvalidData`.
fn invalid_data<E>(e: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::InvalidData, e)
}
