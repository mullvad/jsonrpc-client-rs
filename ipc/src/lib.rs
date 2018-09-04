//! An IPC transport for JSON-RPC. Allows one to connect to a JSON-RPC server through a Unix socket
//! or a Named Pipe on Windows.
#![deny(missing_docs)]
extern crate futures;
extern crate jsonrpc_client_core;
extern crate jsonrpc_server_utils;
extern crate parity_tokio_ipc;
extern crate tokio;
extern crate tokio_io;

use futures::stream::Stream;
use jsonrpc_client_core::Transport;
use jsonrpc_server_utils::codecs;
use parity_tokio_ipc::IpcConnection;
use tokio::reactor::Handle;
use tokio_io::AsyncRead;

use std::io;
use std::path::Path;

/// IpcTransport encapsulates a connection to a local IPC socket or a named pipe. It is implemented
/// using `parity_tokio_ipc`.
pub struct IpcTransport {
    connection: IpcConnection,
}

impl IpcTransport {
    /// Constructs a new IpcTransport for a given path.
    pub fn new(path: &impl AsRef<Path>, handle: &Handle) -> io::Result<IpcTransport> {
        Ok(IpcTransport {
            connection: IpcConnection::connect(path, handle)?,
        })
    }
}

type IpcSink = futures::stream::SplitSink<
    tokio_io::codec::Framed<
        parity_tokio_ipc::IpcConnection,
        jsonrpc_server_utils::codecs::StreamCodec,
    >,
>;
type IpcStream = futures::stream::SplitStream<
    tokio_io::codec::Framed<
        parity_tokio_ipc::IpcConnection,
        jsonrpc_server_utils::codecs::StreamCodec,
    >,
>;


impl Transport for IpcTransport {
    type Error = io::Error;
    type Sink = IpcSink;
    type Stream = IpcStream;

    fn io_pair(self) -> (Self::Sink, Self::Stream) {
        let codec =
            codecs::StreamCodec::new(codecs::Separator::Empty, codecs::Separator::default());
        self.connection.framed(codec).split()
    }
}
