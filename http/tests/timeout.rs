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
use jsonrpc_client_http::HttpTransport;
use std::time::Duration;
use tokio_core::reactor::Core;

// Use a simple RPC API for testing purposes.
use common::test_server::{Server, TestClient};


#[test]
fn long_request_should_timeout() {
    // Spawn a server hosting the `ServerApi` API.
    let (_server, uri) = spawn_server();
    println!("Testing towards slow server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::new()
        .shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = TestClient::new(transport);
    client.set_timeout(Some(Duration::from_millis(500)));

    let rpc_future = client.to_upper("HARD string TAKES too LONG");
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
    // Spawn a server hosting the `ServerApi` API.
    let (_server, uri) = spawn_server();
    println!("Testing towards slow server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::new()
        .shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = TestClient::new(transport);
    client.set_timeout(Some(Duration::from_secs(2)));

    let rpc_future = client.to_upper("HARD string TAKES too LONG");
    let result = core.run(rpc_future).unwrap();

    assert_eq!("HARD STRING TAKES TOO LONG", result);
}

#[test]
fn transport_handles_can_have_different_timeouts() {
    // Spawn a server hosting the `ServerApi` API.
    let (_server, uri) = spawn_server();
    println!("Testing towards slow server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle. Set the default
    // timeout so that long requests succeed.
    let transport = HttpTransport::new()
        .timeout(Duration::from_secs(2))
        .shared(&core.handle())
        .unwrap();

    // Perform a request using a handle that uses the default timeout.
    let handle1 = transport.handle(&uri).unwrap();
    let mut client1 = TestClient::new(handle1);
    client1.set_timeout(Some(Duration::from_secs(2)));

    let rpc_future1 = client1.to_upper("HARD string TAKES too LONG").then(Ok);

    // Perform a request using a handle configured with a shorter timeout.
    let handle2 = transport.handle(&uri).unwrap();
    let mut client2 = TestClient::new(handle2);
    client2.set_timeout(Some(Duration::from_millis(500)));

    let rpc_future2 = client2.to_upper("HARD string TAKES too LONG").then(Ok);

    let joined_future = rpc_future1.join(rpc_future2);
    let results: Result<_, ()> = core.run(joined_future);
    let (result1, result2) = results.unwrap();

    let timeout_error =
        jsonrpc_client_http::Error::from(jsonrpc_client_http::ErrorKind::RequestTimeout);
    let expected_error = jsonrpc_client_core::Error::with_chain(
        timeout_error,
        jsonrpc_client_core::ErrorKind::TransportError,
    );

    assert_eq!(result1.unwrap(), "HARD STRING TAKES TOO LONG");
    assert_err!(result2.unwrap_err(), expected_error);
}

fn spawn_server() -> (jsonrpc_http_server::Server, String) {
    let server = Server::with_delay(Duration::from_secs(1)).spawn();
    let uri = format!("http://{}", server.address());
    (server, uri)
}
