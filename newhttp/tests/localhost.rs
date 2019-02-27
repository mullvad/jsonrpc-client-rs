mod common;
use common::spawn_client_server;

use futures::{future::Either, Future};
use std::time::{Duration, Instant};


#[test]
fn localhost_ping_pong() {
    let (mut rt, mut client, _server) = spawn_client_server(None);

    let rpc_future1 = client.to_upper("MaKe me UppeRcase!!1", 0);
    let rpc_future2 = client.to_upper("foobar", 0);

    let joined_future = rpc_future1.join(rpc_future2);
    let (result1, result2) = rt.block_on(joined_future).unwrap();

    assert_eq!("MAKE ME UPPERCASE!!1", result1);
    assert_eq!("FOOBAR", result2);
}

#[test]
fn dropped_rpc_request_should_not_crash_transport() {
    let (mut rt, mut client, _server) = spawn_client_server(None);

    let rpc = client.to_upper("", 1000).map_err(|e| e.to_string());
    let timeout = tokio_timer::Delay::new(Instant::now() + Duration::from_millis(50));
    match rt.block_on(rpc.select2(timeout)) {
        Ok(Either::B(((), _rpc))) => (),
        _ => panic!("The timeout did not finish first"),
    }

    // Now, sending a second request should still work. This is a regression test catching a
    // previous error where a dropped `RpcRequest` would crash the future running on the event loop.
    match rt.block_on(client.to_upper("foo", 0)) {
        Ok(ref s) if s == "FOO" => (),
        _ => panic!("Sleep did not return as it should"),
    }
}
