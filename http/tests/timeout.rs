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

use futures::Future;
use jsonrpc_client_http::HttpTransportBuilder;
use std::time::Duration;

// Use a simple RPC API for testing purposes.
use common::test_server::{Server, TestClient};


#[test]
fn long_request_should_timeout() {
    // Spawn a server hosting the `ServerApi` API.
    let server = Server.spawn();
    let uri = format!("http://{}", server.address());
    println!("Testing towards slow server at {}", uri);

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransportBuilder::new()
        .timeout(Duration::from_millis(50))
        .standalone()
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = TestClient::new(transport);

    let rpc_future = client.slow_to_upper("HARD string TAKES too LONG", 100);
    let result = rpc_future.wait();

    assert_error_chain_message!(result,
        jsonrpc_client_http::RequestTimeout
        -> jsonrpc_client_core::TransportError
    );
}

#[test]
fn long_request_should_succeed_with_long_timeout() {
    let server = Server.spawn();
    let uri = format!("http://{}", server.address());
    println!("Testing towards slow server at {}", uri);

    let transport = HttpTransportBuilder::new()
        .timeout(Duration::from_millis(150))
        .standalone()
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = TestClient::new(transport);

    let rpc_future = client.slow_to_upper("HARD string TAKES too LONG", 100);
    let result = rpc_future.wait().unwrap();

    assert_eq!("HARD STRING TAKES TOO LONG", result);
}
