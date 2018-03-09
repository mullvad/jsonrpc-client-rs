use std::time::Duration;

use jsonrpc_core::{Error, IoHandler};
use jsonrpc_http_server::{self, ServerBuilder};

// Generate server API trait. Actual implementation at bottom of file.
build_rpc_trait! {
    pub trait ServerApi {
        #[rpc(name = "to_upper")]
        fn to_upper(&self, String) -> Result<String, Error>;

        #[rpc(name = "sleep")]
        fn sleep(&self, u64) -> Result<(), Error>;
    }
}

// Generate client struct with same API as server.
jsonrpc_client!(pub struct TestClient {
    pub fn to_upper(&mut self, string: &str) -> RpcRequest<String>;
    pub fn sleep(&mut self, time: u64) -> RpcRequest<()>;
});


/// Simple struct that will implement the RPC API defined at the top of this file.
///
/// Can be configured to delay its responses to simulate long operations.
pub struct Server {
    delay: Option<Duration>,
}

impl Server {
    pub fn new() -> Self {
        Server { delay: None }
    }

    pub fn with_delay(delay: Duration) -> Self {
        Server { delay: Some(delay) }
    }

    pub fn spawn(self) -> jsonrpc_http_server::Server {
        let mut io = IoHandler::new();
        io.extend_with(self.to_delegate());

        ServerBuilder::new(io)
            .start_http(&"127.0.0.1:0".parse().unwrap())
            .expect("failed to spawn server")
    }

    fn simulate_delay(&self) {
        if let Some(delay) = self.delay {
            ::std::thread::sleep(delay);
        }
    }
}

impl ServerApi for Server {
    fn to_upper(&self, s: String) -> Result<String, Error> {
        self.simulate_delay();
        Ok(s.to_uppercase())
    }

    fn sleep(&self, time: u64) -> Result<(), Error> {
        println!("Sleeping on server");
        ::std::thread::sleep(Duration::from_secs(time));
        self.simulate_delay();
        Ok(())
    }
}
