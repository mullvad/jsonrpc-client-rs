#![allow(dead_code)]

use futures::Future;
use jsonrpc_client_core::Transport;
use jsonrpc_client_newhttp::{
    hyper::{Client, Uri},
    HttpTransport,
};
use jsonrpc_core::{Error, IoHandler};
use jsonrpc_http_server::{self, ServerBuilder};
use std::{thread, time::Duration};

// Generate client struct with same API as server.
jsonrpc_client_core::jsonrpc_client!(pub struct MockRpcClient {
    pub fn to_upper(&mut self, s: &str, time: u64) -> Future<String>;
});

#[jsonrpc_derive::rpc]
pub trait MockRpcServerApi {
    #[rpc(name = "to_upper")]
    fn to_upper(&self, s: String, time: u64) -> Result<String, Error>;
}

pub struct MockRpcServer;
impl MockRpcServerApi for MockRpcServer {
    fn to_upper(&self, s: String, time: u64) -> Result<String, Error> {
        thread::sleep(Duration::from_millis(time));
        Ok(s.to_uppercase())
    }
}

impl MockRpcServer {
    pub fn spawn() -> jsonrpc_http_server::Server {
        let mut io = IoHandler::new();
        io.extend_with(MockRpcServer.to_delegate());

        ServerBuilder::new(io)
            .start_http(&"127.0.0.1:0".parse().unwrap())
            .expect("failed to spawn server")
    }
}

/// Helper method that just creates a client and server talking to each other.
pub fn spawn_client_server(
    client_timeout: Option<Duration>,
) -> (
    tokio::runtime::Runtime,
    MockRpcClient,
    jsonrpc_http_server::Server,
) {
    let server = MockRpcServer::spawn();
    let uri = format!("http://{}", server.address())
        .parse::<Uri>()
        .unwrap();

    let mut rt = tokio::runtime::Runtime::new().unwrap();

    let client: Client<_, hyper::Body> = Client::new();

    let jsonrpc_transport = HttpTransport::new(client, client_timeout);
    let (jsonrpc_client_future, client_handle) = jsonrpc_transport.handle(uri).into_client();
    let client = MockRpcClient::new(client_handle);

    rt.spawn(jsonrpc_transport);
    rt.spawn(jsonrpc_client_future.map_err(|e| log::error!("Error in JSON-RPC 2.0 client: {}", e)));

    (rt, client, server)
}
