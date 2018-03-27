#![allow(dead_code)]

use std::time::Duration;

use futures::future::{self, Empty};
use jsonrpc_core::{Error, IoHandler};
use jsonrpc_http_server::{self, hyper, ServerBuilder};
use jsonrpc_http_server::hyper::server::{Request, Response, Service};

// Generate server API trait. Actual implementation at bottom of file.
build_rpc_trait! {
    pub trait MockRpcServerApi {
        #[rpc(name = "to_upper")]
        fn to_upper(&self, String) -> Result<String, Error>;

        #[rpc(name = "slow_to_upper")]
        fn slow_to_upper(&self, String, u64) -> Result<String, Error>;

        #[rpc(name = "sleep")]
        fn sleep(&self, u64) -> Result<(), Error>;
    }
}

// Generate client struct with same API as server.
jsonrpc_client!(pub struct MockRpcClient {
    pub fn to_upper(&mut self, string: &str) -> RpcRequest<String>;
    pub fn slow_to_upper(&mut self, string: &str, time: u64) -> RpcRequest<String>;
    pub fn sleep(&mut self, time: u64) -> RpcRequest<()>;
});


/// Simple struct that will implement the RPC API defined at the top of this file.
pub struct MockRpcServer;

impl MockRpcServer {
    pub fn spawn() -> jsonrpc_http_server::Server {
        let mut io = IoHandler::new();
        io.extend_with(MockRpcServer.to_delegate());

        ServerBuilder::new(io)
            .start_http(&"127.0.0.1:0".parse().unwrap())
            .expect("failed to spawn server")
    }
}

impl MockRpcServerApi for MockRpcServer {
    fn to_upper(&self, s: String) -> Result<String, Error> {
        Ok(s.to_uppercase())
    }

    fn slow_to_upper(&self, s: String, time: u64) -> Result<String, Error> {
        ::std::thread::sleep(Duration::from_millis(time));
        Ok(s.to_uppercase())
    }

    fn sleep(&self, time: u64) -> Result<(), Error> {
        println!("Sleeping on server");
        ::std::thread::sleep(Duration::from_secs(time));
        Ok(())
    }
}

pub struct UnresponsiveService;

impl Service for UnresponsiveService {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = Empty<Self::Response, Self::Error>;

    fn call(&self, _: Self::Request) -> Self::Future {
        future::empty()
    }
}
