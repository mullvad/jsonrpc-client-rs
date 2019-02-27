use futures::Future;
use hyper::{Client, Uri};
use jsonrpc_client_core::Transport;
use std::error::Error;

fn main() {
    env_logger::init();
    let mut rt = tokio::runtime::Runtime::new().unwrap();
    let https = hyper_rustls::HttpsConnector::new(4);
    let client: Client<_, hyper::Body> = Client::builder().build(https);

    let jsonrpc_transport = jsonrpc_client_newhttp::HttpTransport::new(client, None);

    let uri = "https://api.mullvad.net/rpc/".parse::<Uri>().unwrap();
    let (jsonrpc_client, client_handle) = jsonrpc_transport.handle(uri).into_client();

    rt.spawn(jsonrpc_transport);
    rt.spawn(jsonrpc_client.map_err(|e| {
        let mut msg = e.to_string();
        let mut source = e.source();
        while let Some(s) = source {
            msg.push_str("\nCaused by: ");
            msg.push_str(&s.to_string());
            source = s.source();
        }
        log::error!("Error in JSON-RPC 2.0 client: {}", msg);
    }));

    let mut app_version_proxy = AppVersionProxy::new(client_handle);
    let is_supported = rt.block_on(app_version_proxy.is_app_version_supported("bogus version"));
    match is_supported {
        Ok(is_supported) => println!("Is version supported? {}", is_supported),
        Err(e) => eprintln!("Error processing request: {}", e),
    }
}

jsonrpc_client_core::jsonrpc_client!(pub struct AppVersionProxy {
    pub fn is_app_version_supported(&mut self, version: &str) -> Future<bool>;
});
