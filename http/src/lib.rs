// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


#[macro_use]
extern crate error_chain;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate jsonrpc_client_core;
#[macro_use]
extern crate log;
extern crate tokio_core;

use futures::{future, BoxFuture, Future, Stream};
use futures::sync::{mpsc, oneshot};

use hyper::{Client, Request, StatusCode, Uri};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;

use jsonrpc_client_core::Transport;

use std::str::FromStr;
use std::thread;

use tokio_core::reactor::{Core, Handle};


error_chain! {
    errors {
        /// When there was an error initializing the TLS layer
        HttpsConnectorError {
            description("Unable to create the Hyper TLS connector")
        }
        /// When the http status code of the response is not 200 OK
        HttpError(http_code: StatusCode) {
            description("Http error. Server did not return 200 OK")
            display("Http error. Status code {}", http_code)
        }
        /// When there was an error in the Tokio Core.
        TokioCoreError(msg: &'static str) {
            description("Error with the Tokio Core")
            display("Error with the Tokio Core: {}", msg)
        }
    }
    foreign_links {
        Hyper(hyper::Error);
        Uri(hyper::error::UriError);
    }
}


/// The main struct of the HTTP transport implementation for `jsonrpc-client-core`.
///
/// Can be created both with its own internal Tokio Core running in a separate thread using the
/// `HttpCore::standalone()` constructor. Or can run on an existing Tokio Core if created with the
/// `HttpCore::shared(&handle)` constructor.
#[derive(Clone)]
pub struct HttpCore {
    request_tx: mpsc::UnboundedSender<(Request, oneshot::Sender<Result<Vec<u8>>>)>,
}

impl HttpCore {
    /// Creates a new `HttpCore` running on an existing Tokio Core.
    pub fn shared(handle: &Handle) -> Result<Self> {
        let client = Self::create_client(handle)?;
        let (request_tx, request_rx) = mpsc::unbounded();
        handle.spawn(Self::create_request_processing_future(request_rx, client));
        Ok(HttpCore { request_tx })
    }

    /// Spawns a Tokio Core event loop in a new thread and returns a `HttpCore` based on
    /// this internal event loop. The thread and the event loop will run for as long as the
    /// returned `HttpCore`, or any `HttpHandle` to it, exists.
    ///
    /// Use `HttpCore::shared()` To create an instance that operate on a Tokio Core managed by you.
    pub fn standalone() -> Result<Self> {
        let (tx, rx) = ::std::sync::mpsc::channel();
        thread::spawn(move || {
            match Self::create_standalone_core() {
                Err(e) => {
                    tx.send(Err(e)).unwrap();
                }
                Ok((mut core, http_core, future)) => {
                    tx.send(Ok(http_core)).unwrap();
                    let _ = core.run(future);
                }
            }
            debug!("Standalone HttpCore thread exiting");
        });

        rx.recv().unwrap()
    }

    /// Creates all the components needed to run the `HttpCore` in standalone mode.
    fn create_standalone_core() -> Result<(Core, HttpCore, Box<Future<Item = (), Error = ()>>)> {
        let core = Core::new().chain_err(|| ErrorKind::TokioCoreError("Unable to create"))?;
        let client = Self::create_client(&core.handle())?;
        let (request_tx, request_rx) = mpsc::unbounded();
        let future = Self::create_request_processing_future(request_rx, client);
        Ok((core, HttpCore { request_tx }, future))
    }

    /// Creates the `Future` that, when running on a Tokio Core, processes incoming RPC call
    /// requests.
    fn create_request_processing_future(
        request_rx: mpsc::UnboundedReceiver<(Request, oneshot::Sender<Result<Vec<u8>>>)>,
        client: Client<HttpsConnector<HttpConnector>, hyper::Body>,
    ) -> Box<Future<Item = (), Error = ()>> {
        let f = request_rx.for_each(move |(request, response_tx)| {
            client
                .request(request)
                .from_err()
                .and_then(|response: hyper::Response| {
                    if response.status() == hyper::StatusCode::Ok {
                        future::ok(response)
                    } else {
                        future::err(ErrorKind::HttpError(response.status()).into())
                    }
                })
                .and_then(|response: hyper::Response| {
                    response.body().concat2().from_err()
                })
                .map(|response_chunk| response_chunk.to_vec())
                .then(move |response_result| {
                    response_tx.send(response_result).map_err(|_| {
                        warn!("Unable to send response back to caller");
                        ()
                    })
                })
        });
        Box::new(f) as Box<Future<Item = (), Error = ()>>
    }

    /// Creates and returns the Hyper `Client`.
    fn create_client(
        handle: &Handle,
    ) -> Result<Client<HttpsConnector<HttpConnector>, hyper::Body>> {
        let https_connector =
            HttpsConnector::new(1, handle).chain_err(|| ErrorKind::HttpsConnectorError)?;
        Ok(
            Client::configure()
                .connector(https_connector)
                .build(handle),
        )
    }

    /// Returns a handle to this `HttpCore` valid for a given URI.
    pub fn handle(&self, uri: &str) -> Result<HttpHandle> {
        let uri = Uri::from_str(uri)?;
        Ok(HttpHandle {
            request_tx: self.request_tx.clone(),
            uri,
        })
    }
}

/// A handle to a `HttpCore`. This implements `jsonrpc_client_core::Transport` and can be used as
/// the transport object for an RPC client generated by `jsonrpc_client_core`.
#[derive(Clone)]
pub struct HttpHandle {
    request_tx: mpsc::UnboundedSender<(Request, oneshot::Sender<Result<Vec<u8>>>)>,
    uri: Uri,
}

impl HttpHandle {
    /// Creates a Hyper POST request with JSON content type and the given body data.
    fn create_request(&self, body: Vec<u8>) -> Request {
        let mut request = hyper::Request::new(hyper::Method::Post, self.uri.clone());
        request
            .headers_mut()
            .set(hyper::header::ContentType::json());
        request
            .headers_mut()
            .set(hyper::header::ContentLength(body.len() as u64));
        request.set_body(body);
        request
    }
}

impl Transport<Error> for HttpHandle {
    fn send(&self, json_data: Vec<u8>) -> BoxFuture<Vec<u8>, Error> {
        let request = self.create_request(json_data.clone());
        let (response_tx, response_rx) = oneshot::channel();
        future::result(mpsc::UnboundedSender::send(
            &self.request_tx,
            (request, response_tx),
        )).map_err(|e| {
            Error::with_chain(e, ErrorKind::TokioCoreError("Not listening for requests"))
        })
            .and_then(move |_| {
                response_rx.map_err(|e| {
                    Error::with_chain(
                        e,
                        ErrorKind::TokioCoreError("Died without returning response"),
                    )
                })
            })
            .and_then(future::result)
            .boxed()
    }
}
