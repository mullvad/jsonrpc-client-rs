// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

extern crate env_logger;
extern crate futures;
#[macro_use]
extern crate jsonrpc_client_core;
extern crate jsonrpc_client_http;

extern crate tokio_core;

extern crate jsonrpc_core;
extern crate jsonrpc_http_server;
#[macro_use]
extern crate jsonrpc_macros;

use futures::Future;

use jsonrpc_client_http::HttpTransport;
use jsonrpc_core::{Error, IoHandler};
use jsonrpc_http_server::ServerBuilder;


// Generate server API trait. Actual implementation at bottom of file.
build_rpc_trait! {
    pub trait ServerApi {
        #[rpc(name = "to_upper")]
        fn to_upper(&self, String) -> Result<String, Error>;
    }
}

// Generate client struct with same API as server.
jsonrpc_client!(pub struct ToUpperClient {
    pub fn to_upper(&mut self, string: &str) -> RpcRequest<String>;
});


#[test]
fn localhost_ping_pong() {
    env_logger::init().unwrap();

    // Spawn a server hosting the `ServerApi` API.
    let server = spawn_server();

    let uri = format!("http://{}", server.address());
    println!("Testing towards server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = tokio_core::reactor::Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = ToUpperClient::new(transport);

    // Just calling the method gives back a `RpcRequest`, which is a future
    // that can be used to execute the actual RPC call.
    let rpc_future1 = client.to_upper("MaKe me UppeRcase!!1");
    let rpc_future2 = client.to_upper("foobar");

    let joined_future = rpc_future1.join(rpc_future2);
    // Run the event loop with the two calls to make them actually run.
    let (result1, result2) = core.run(joined_future).unwrap();

    assert_eq!("MAKE ME UPPERCASE!!1", result1);
    assert_eq!("FOOBAR", result2);
}


/// Simple struct that will implement the RPC API defined at the top of this file.
struct Server;

impl ServerApi for Server {
    fn to_upper(&self, s: String) -> Result<String, Error> {
        Ok(s.to_uppercase())
    }
}

fn spawn_server() -> jsonrpc_http_server::Server {
    let server = Server;
    let mut io = IoHandler::new();
    io.extend_with(server.to_delegate());

    ServerBuilder::new(io)
        .start_http(&"127.0.0.1:0".parse().unwrap())
        .unwrap()
}
