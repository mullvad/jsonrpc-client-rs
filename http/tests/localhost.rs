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
use futures::future::Either;
use jsonrpc_client_http::HttpTransport;
use std::time::Duration;
use tokio_core::reactor::{Core, Timeout};

// Use a simple RPC API for testing purposes.
use common::test_server::{Server, TestClient};


#[test]
fn localhost_ping_pong() {
    // Spawn a server hosting the `ServerApi` API.
    let (_server, uri) = spawn_server();
    println!("Testing towards server at {}", uri);

    // Create the Tokio Core event loop that will drive the RPC client and the async requests.
    let mut core = Core::new().unwrap();

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::new()
        .shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = TestClient::new(transport);

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

#[test]
fn dropped_rpc_request_should_not_crash_transport() {
    let (_server, uri) = spawn_server();

    let mut core = Core::new().unwrap();
    let transport = HttpTransport::new()
        .shared(&core.handle())
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = TestClient::new(transport);

    let rpc = client.sleep(5).map_err(|e| e.to_string());
    let timeout = Timeout::new(Duration::from_millis(100), &core.handle()).unwrap();
    match core.run(rpc.select2(timeout.map_err(|e| e.to_string()))) {
        Ok(Either::B(((), _rpc))) => (),
        _ => panic!("The timeout did not finish first"),
    }

    // Now, sending a second request should still work. This is a regression test catching a
    // previous error where a dropped `RpcRequest` would crash the future running on the event loop.
    match core.run(client.sleep(0)) {
        Ok(()) => (),
        _ => panic!("Sleep did not return as it should"),
    }
}

#[test]
fn long_request_should_timeout() {
    // Spawn a server hosting the `ServerApi` API.
    let (_server, uri) = spawn_slow_server();
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
    let mut client = TestClient::new(transport);

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
    let (_server, uri) = spawn_slow_server();
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
    let mut client = TestClient::new(transport);

    let rpc_future = client.to_upper("HARD string TAKES too LONG");
    let result = core.run(rpc_future).unwrap();

    assert_eq!("HARD STRING TAKES TOO LONG", result);
}

fn spawn_server() -> (jsonrpc_http_server::Server, String) {
    let server = Server::new().spawn();
    let uri = format!("http://{}", server.address());
    (server, uri)
}

fn spawn_slow_server() -> (jsonrpc_http_server::Server, String) {
    let server = Server::with_delay(Duration::from_secs(1)).spawn();
    let uri = format!("http://{}", server.address());
    (server, uri)
}
