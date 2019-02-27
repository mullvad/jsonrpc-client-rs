mod common;
use common::spawn_client_server;

use std::time::Duration;

#[test]
fn slow_request_should_timeout() {
    let (mut rt, mut client, _server) = spawn_client_server(Some(Duration::from_millis(50)));

    let slow_rpc = client.to_upper("hard string takes too long to process ;)", 1000);
    assert!(rt.block_on(slow_rpc).is_err());
}

#[test]
fn fast_request_should_succeed() {
    let (mut rt, mut client, _server) = spawn_client_server(Some(Duration::from_millis(5000)));

    let fast_rpc = client.to_upper("foobar", 0);
    match rt.block_on(fast_rpc) {
        Ok(ref s) if s == "FOOBAR" => (),
        _ => panic!("did not receive expected response"),
    }
}
