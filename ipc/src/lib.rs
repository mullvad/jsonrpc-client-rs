//! An IPC transport for JSON-RPC. Allows one to connect to a JSON-RPC server through a Unix socket
//! or a Named Pipe on Windows.
#![deny(missing_docs)]
extern crate futures;
extern crate jsonrpc_client_core;
extern crate jsonrpc_server_utils;
extern crate parity_tokio_ipc;
extern crate tokio;
extern crate tokio_core;
extern crate tokio_io;


use futures::sink::Sink;
use futures::stream::Stream;
use jsonrpc_client_core::{Client, ClientHandle};
use jsonrpc_server_utils::codecs;
use parity_tokio_ipc::IpcConnection;
use tokio_core::reactor::Handle;
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

    /// Creates a pair of a sink and a stream where the transferred item is a string representing a
    /// single JSON object.
    pub fn io_pair(
        self,
    ) -> (
        impl Sink<SinkItem = String, SinkError = io::Error>,
        impl Stream<Item = String, Error = io::Error>,
    ) {
        let codec =
            codecs::StreamCodec::new(codecs::Separator::Empty, codecs::Separator::default());
        self.connection.framed(codec).split()
    }

    /// Constructs a Client and a handle from the transport.
    pub fn into_client(
        self,
    ) -> (
        Client<
            impl futures::Sink<SinkItem = String, SinkError = io::Error>,
            impl futures::Stream<Item = String, Error = io::Error>,
            io::Error,
        >,
        ClientHandle,
    ) {
        let (tx, rx) = self.io_pair();
        Client::new(tx, rx)
    }
}
