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

use std::sync::mpsc;
use std::time::Duration;

use futures::future::{Either, Future};
use jsonrpc_client_core::Transport;
use jsonrpc_client_http::{ErrorKind, HttpTransport};
use jsonrpc_http_server::hyper::server::Http;
use tokio_core::reactor::{Core, Timeout};

// Use a simple RPC API for testing purposes.
use common::{MockRpcClient, MockRpcServer, UnresponsiveService};


#[test]
fn long_request_should_timeout() {
    // Spawn a server hosting the `MockRpcServerApi` API.
    let server = MockRpcServer::spawn();
    let uri = format!("http://{}", server.address());
    println!("Testing towards slow server at {}", uri);

    // Create the HTTP transport handle and create a RPC client with that handle.
    let transport = HttpTransport::new()
        .timeout(Duration::from_millis(50))
        .standalone()
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = MockRpcClient::new(transport);

    let rpc_future = client.slow_to_upper("HARD string TAKES too LONG", 10_000);
    let result = rpc_future.wait();

    assert!(result.is_err());
}

#[test]
fn short_request_should_succeed() {
    let server = MockRpcServer::spawn();
    let uri = format!("http://{}", server.address());

    let transport = HttpTransport::new()
        .timeout(Duration::from_secs(10))
        .standalone()
        .unwrap()
        .handle(&uri)
        .unwrap();
    let mut client = MockRpcClient::new(transport);

    let rpc_future = client.to_upper("FAST sHoRt strIng");
    let result = rpc_future.wait().unwrap();

    assert_eq!("FAST SHORT STRING", result);
}

#[test]
fn timeout_error() {
    let mut reactor = Core::new().unwrap();
    let handle = reactor.handle();

    let (address_tx, address_rx) = mpsc::channel();

    ::std::thread::spawn(move || {
        let address = "127.0.0.1:0".parse().unwrap();
        let server = Http::new()
            .bind(&address, || Ok(UnresponsiveService))
            .unwrap();

        address_tx.send(server.local_addr().unwrap()).unwrap();

        server.run().unwrap();
    });

    let address = address_rx.recv().unwrap();

    let transport = HttpTransport::new()
        .timeout(Duration::from_millis(100))
        .shared(&handle)
        .unwrap()
        .handle(&format!("http://{}", address))
        .unwrap();

    let send_operation = transport.send(vec![1, 2, 3, 4]);

    let test_timeout = Timeout::new(Duration::from_secs(1), &handle).unwrap();
    let test_operation = test_timeout.select2(send_operation);

    match reactor.run(test_operation) {
        Ok(Either::A(_)) => panic!("test timed out!"),
        Ok(Either::B(_)) => panic!("request didn't time out as expected"),
        Err(Either::A((error, _))) => panic!("test timeout error: {}", error),
        Err(Either::B((error, _))) => match error.kind() {
            &ErrorKind::RequestTimeout => (),
            _ => panic!("failed to send request: {}", error),
        },
    }
}
