#[macro_use]
extern crate error_chain;
extern crate websocket;
extern crate native_tls;
extern crate futures;
extern crate tokio_core;

extern crate jsonrpc_client_core;

use futures::Future;
use futures::sync::{mpsc, oneshot};
use websocket::ClientBuilder;
use websocket::client::async::Client;
use websocket::stream::async::Stream;
use tokio_core::reactor::Handle;

use jsonrpc_client_core::{BoxFuture, Transport};

error_chain! {}

type CoreSender = mpsc::UnboundedSender<(Vec<u8>, oneshot::Sender<Result<Vec<u8>>>)>;
type CoreReceiver = mpsc::UnboundedReceiver<(Vec<u8>, oneshot::Sender<Result<Vec<u8>>>)>;

#[derive(Clone)]
pub struct WsTransport {
    request_tx: CoreSender,
}

impl WsTransport {
    pub fn new(client: Client<Box<Stream + Send>>, handle: &Handle) {
        
    }
}

impl Transport for WsTransport {
    type Error = Error;

    fn get_next_id(&mut self) -> u64 {
        1
    }

    fn send(&self, json_data: Vec<u8>) -> BoxFuture<Vec<u8>, Error> {
        // let request = self.create_request(json_data.clone());
        // let (response_tx, response_rx) = oneshot::channel();
        // let future = future::result(self.request_tx.unbounded_send((request, response_tx)))
        //     .map_err(|e| {
        //         Error::with_chain(e, ErrorKind::TokioCoreError("Not listening for requests"))
        //     })
        //     .and_then(move |_| {
        //         response_rx.map_err(|e| {
        //             Error::with_chain(
        //                 e,
        //                 ErrorKind::TokioCoreError("Died without returning response"),
        //             )
        //         })
        //     })
        //     .and_then(future::result);
        // Box::new(future)
        unimplemented!();
    }
}
