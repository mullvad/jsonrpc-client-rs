// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

extern crate futures;
#[macro_use]
extern crate jsonrpc_client_core;
extern crate jsonrpc_client_http;

extern crate tokio_core;

extern crate jsonrpc_core;
extern crate jsonrpc_http_server;
#[macro_use]
extern crate jsonrpc_macros;

#[macro_use]
mod common;

use jsonrpc_client_http::HttpTransport;
use std::time::Duration;
use tokio_core::reactor::Core;

// Use a simple RPC API for testing purposes.
use common::{MockRpcClient, MockRpcServer};


#[test]
fn long_request_should_timeout() {
    // Spawn a server hosting the `MockRpcServerApi` API.
    let (_server, uri) = spawn_server();
    println!("Testing towards slow server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::new()
        .timeout(Duration::from_millis(500))
        .shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = MockRpcClient::new(transport);

    let rpc_future = client.slow_to_upper("HARD string TAKES too LONG", 1000);
    let error = core.run(rpc_future).unwrap_err();

    let timeout_error =
        jsonrpc_client_http::Error::from(jsonrpc_client_http::ErrorKind::RequestTimeout);
    let expected_error = jsonrpc_client_core::Error::with_chain(
        timeout_error,
        jsonrpc_client_core::ErrorKind::TransportError,
    );

    assert_err!(error, expected_error);
}

#[test]
fn long_request_should_succeed_with_long_timeout() {
    // Spawn a server hosting the `MockRpcServerApi` API.
    let (_server, uri) = spawn_server();
    println!("Testing towards slow server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::new()
        .timeout(Duration::from_secs(2))
        .shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = MockRpcClient::new(transport);

    let rpc_future = client.slow_to_upper("HARD string TAKES too LONG", 1000);
    let result = core.run(rpc_future).unwrap();

    assert_eq!("HARD STRING TAKES TOO LONG", result);
}

fn spawn_server() -> (jsonrpc_http_server::Server, String) {
    let server = MockRpcServer::spawn();
    let uri = format!("http://{}", server.address());
    (server, uri)
}
