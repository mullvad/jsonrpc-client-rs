// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! HTTP transport implementation for the JSON-RPC 2.0 clients generated by `jsonrpc-client-core`.
//!
//! Uses the async Tokio based version of Hyper to implement a JSON-RPC 2.0 compliant HTTP
//! transport.
//!
//! # Reusing connections
//!
//! Each `HttpTransport` instance is backed by exactly one Hyper `Client` and all `HttpHandle`s
//! created through the same `HttpTransport` also point to that one `Client` instance.
//!
//! By default Hyper `Client`s have keep-alive activated and open connections will be kept and
//! reused.
//!
//! # TLS / HTTPS
//!
//! TLS support is compiled if the "tls" feature is activated (it is active by default).
//!
//! All the TLS support adds is the convenient builder constructor with TLS activated, at
//! `HttpTransport::tls_builder()`.
//!
//! Please note that the TLS activated transport supports unencrypted HTTP as well. The backing
//! `hyper_tls::HttpsConnector` skips TLS if it sees the URI is `http://`. So it is perfectly fine
//! to create one TLS activated instance and use for backing all clients, encrypted and not.
//!
//! # Usage
//!
//! ```rust,no_run
//! #[macro_use] extern crate jsonrpc_client_core;
//! extern crate jsonrpc_client_http;
//!
//! use jsonrpc_client_http::HttpTransport;
//!
//! jsonrpc_client!(pub struct FizzBuzzClient {
//!     /// Returns the fizz-buzz string for the given number.
//!     pub fn fizz_buzz(&mut self, number: u64) -> RpcRequest<String>;
//! });
//!
//! fn main() {
//!     let transport = HttpTransport::tls_builder().build().unwrap();
//!     let transport_handle = transport.handle("https://api.fizzbuzzexample.org/rpc/").unwrap();
//!     let mut client = FizzBuzzClient::new(transport_handle);
//!     let result1 = client.fizz_buzz(3).call().unwrap();
//!     let result2 = client.fizz_buzz(4).call().unwrap();
//!     let result3 = client.fizz_buzz(5).call().unwrap();
//!
//!     // Should print "fizz 4 buzz" if the server implemented the service correctly
//!     println!("{} {} {}", result1, result2, result3);
//! }
//! ```

// #![deny(missing_docs)]

#[macro_use]
extern crate error_chain;
extern crate futures;
extern crate hyper;
extern crate jsonrpc_client_core;
#[macro_use]
extern crate log;
extern crate tokio_core;

#[cfg(feature = "tls")]
extern crate hyper_tls;
#[cfg(feature = "tls")]
extern crate native_tls;

use futures::{future, BoxFuture, Future, Stream};
use futures::sync::{mpsc, oneshot};

use hyper::{Client, Request, StatusCode, Uri};

use jsonrpc_client_core::Transport;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use tokio_core::reactor::{Core, Handle};

mod client_creator;
pub use client_creator::*;

error_chain! {
    errors {
        /// When there was an error creating the Hyper `Client` from the given creator.
        ClientCreatorError {
            description("Failed to create the Hyper Client")
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
        Hyper(hyper::Error) #[doc = "An error occured in Hyper."];
        Uri(hyper::error::UriError) #[doc = "The string given was not a valid URI."];
    }
}


type CoreSender = mpsc::UnboundedSender<(Request, oneshot::Sender<Result<Vec<u8>>>)>;
type CoreReceiver = mpsc::UnboundedReceiver<(Request, oneshot::Sender<Result<Vec<u8>>>)>;

pub enum CoreOption {
    Standalone,
    Shared(Handle),
}

/// The main struct of the HTTP transport implementation for `jsonrpc-client-core`.
///
/// Created with the `HttpTransportBuilder` builder.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    request_tx: CoreSender,
    id: Arc<AtomicUsize>,
}

impl HttpTransport {
    pub fn new<C>(client_creator: C, core: CoreOption) -> Result<HttpTransport>
    where C: ClientCreator
    {
        match core {
            CoreOption::Standalone => Self::new_standalone(client_creator),
            CoreOption::Shared(handle) => Self::new_shared(client_creator, handle)
        }
    }

    pub fn default(core: CoreOption) -> Result<HttpTransport> {
        Self::new(DefaultClient, core)
    }

    #[cfg(feature = "tls")]
    pub fn with_tls(core: CoreOption) -> Result<HttpTransport> {
        Self::new(DefaultTlsClient, core)
    }

    fn new_shared<C>(client_creator: C, handle: Handle) -> Result<HttpTransport>
    where C: ClientCreator
    {
        let client = client_creator
            .create(&handle)
            .chain_err(|| ErrorKind::ClientCreatorError)?;
        let (request_tx, request_rx) = mpsc::unbounded();
        handle.spawn(create_request_processing_future(request_rx, client));
        Ok(HttpTransport::new_internal(request_tx))
    }

    fn new_standalone<C>(client_creator: C) -> Result<HttpTransport>
    where C: ClientCreator
    {
        let (tx, rx) = ::std::sync::mpsc::channel();
        thread::spawn(move || {
            match create_standalone_core(client_creator) {
                Err(e) => {
                    tx.send(Err(e)).unwrap();
                }
                Ok((mut core, request_tx, future)) => {
                    tx.send(Ok(HttpTransport::new_internal(request_tx))).unwrap();
                    let _ = core.run(future);
                }
            }
            debug!("Standalone HttpTransport thread exiting");
        });

        rx.recv().unwrap()
    }

    fn new_internal(request_tx: CoreSender) -> HttpTransport {
        HttpTransport {
            request_tx,
            id: Arc::new(AtomicUsize::new(1)),
        }
    }

    /// Returns a handle to this `HttpTransport` valid for a given URI.
    ///
    /// Used to create instances implementing `jsonrpc_client_core::Transport` for use with RPC
    /// clients.
    pub fn handle(&self, uri: &str) -> Result<HttpHandle> {
        let uri = Uri::from_str(uri)?;
        Ok(HttpHandle {
            request_tx: self.request_tx.clone(),
            uri,
            id: self.id.clone(),
        })
    }
}

/// Creates all the components needed to run the `HttpTransport` in standalone mode.
fn create_standalone_core<C: ClientCreator>(
    client_creator: C,
) -> Result<(Core, CoreSender, Box<Future<Item = (), Error = ()>>)> {
    let core = Core::new().chain_err(|| ErrorKind::TokioCoreError("Unable to create"))?;
    let client = client_creator
        .create(&core.handle())
        .chain_err(|| ErrorKind::ClientCreatorError)?;
    let (request_tx, request_rx) = mpsc::unbounded();
    let future = create_request_processing_future(request_rx, client);
    Ok((core, request_tx, future))
}

/// Creates the `Future` that, when running on a Tokio Core, processes incoming RPC call
/// requests.
fn create_request_processing_future<CC: hyper::client::Connect>(
    request_rx: CoreReceiver,
    client: Client<CC, hyper::Body>,
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

/// A handle to a `HttpTransport`. This implements `jsonrpc_client_core::Transport` and can be used
/// as the transport object for an RPC client generated by `jsonrpc_client_core`.
#[derive(Debug, Clone)]
pub struct HttpHandle {
    request_tx: CoreSender,
    uri: Uri,
    id: Arc<AtomicUsize>,
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
    fn get_next_id(&mut self) -> u64 {
        self.id.fetch_add(1, Ordering::SeqCst) as u64
    }

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


#[cfg(test)]
mod tests {
    use super::*;
    use hyper::client::HttpConnector;
    use std::io;

    #[test]
    fn builder_accept_handle() {
        let core = Core::new().unwrap();
        HttpTransport::builder()
            .handle(core.handle())
            .build()
            .unwrap();
    }

    #[test]
    fn builder_no_handle() {
        HttpTransport::builder().build().unwrap();
    }

    #[test]
    fn builder_closure_client_creator() {
        HttpTransport::builder()
            .client(|handle: &Handle| {
                Ok(Client::new(handle)) as Result<Client<HttpConnector, hyper::Body>>
            })
            .build()
            .unwrap();
    }

    #[test]
    fn builder_client_creator_fails() {
        let error = HttpTransport::builder()
            .client(|_: &Handle| {
                Err(io::Error::new(io::ErrorKind::Other, "Dummy error")) as
                    ::std::result::Result<Client<HttpConnector, hyper::Body>, io::Error>
            })
            .build()
            .unwrap_err();
        match error.kind() {
            &ErrorKind::ClientCreatorError => (),
            kind => panic!("invalid error kind response: {:?}", kind),
        }
    }
}
