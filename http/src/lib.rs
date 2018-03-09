// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! HTTP transport implementation for the JSON-RPC 2.0 clients generated by
//! [`jsonrpc-client-core`](../jsonrpc_client_core/index.html).
//!
//! Uses the async Tokio based version of Hyper to implement a JSON-RPC 2.0 compliant HTTP
//! transport.
//!
//! # Reusing connections
//!
//! Each [`HttpTransport`](struct.HttpTransport.html) instance is backed by exactly one Hyper
//! `Client` and all [`HttpHandle`s](struct.HttpHandle.html) created through the same
//! `HttpTransport` also point to that same `Client` instance.
//!
//! By default Hyper `Client`s have keep-alive activated and open connections will be kept and
//! reused if more requests are sent to the same destination before the keep-alive timeout is
//! reached.
//!
//! # TLS / HTTPS
//!
//! TLS support is compiled if the "tls" feature is enabled (it is enabled by default).
//!
//! When TLS support is compiled in the instances returned by
//! [`HttpTransport::new`](struct.HttpTransport.html#method.new) and
//! [`HttpTransport::shared`](struct.HttpTransport.html#method.shared) support both plaintext http
//! and https over TLS, backed by the `hyper_tls::HttpsConnector` connector.
//!
//! # Examples
//!
//! See the integration test in `tests/localhost.rs` for code that creates an actual HTTP server
//! with `jsonrpc_http_server`, and sends requests to it with this crate.
//!
//! Here is a small example of how to use this crate together with `jsonrpc_core`:
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
//!     let transport = HttpTransport::new().standalone().unwrap();
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

#![deny(missing_docs)]

#[macro_use]
extern crate error_chain;
#[macro_use]
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

use futures::{Future, IntoFuture, Poll, Stream};
use futures::future::{self, Flatten, FutureResult};
use futures::sync::{mpsc, oneshot};
use hyper::{Client, Request, StatusCode, Uri};
pub use hyper::header;
use jsonrpc_client_core::Transport;
use std::io;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use tokio_core::reactor::{Core, Timeout};
pub use tokio_core::reactor::Handle;

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
        /// When the request times out.
        RequestTimeout {
            description("Timeout while waiting for a request")
        }
        /// When there was an error setting up a timeout.
        TimeoutError {
            description("Failed to configure a timeout")
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


/// The main struct of the HTTP transport implementation for
/// [`jsonrpc_client_core`](../jsonrpc_client_core).
///
/// Acts as a handle to a stream running on the Tokio `Core` event loop thread. The handle allows
/// sending Hyper `Request`s to the event loop and the stream running there will then send it
/// to the destination and wait for the response.
///
/// This is just a handle without any destination (URI), and it does not implement
/// [`Transport`](../jsonrpc_client_core/trait.Transport.html).
/// To get a handle implementing `Transport` to use with an RPC client you call the
/// [`handle`](#method.handle) method with a URI.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    request_tx: CoreSender,
    id: Arc<AtomicUsize>,
}

impl HttpTransport {
    #[cfg(not(feature = "tls"))]
    /// Starts the creation of a `HttpTransport`, which can be finished by the
    /// [`standalone()`](struct.HttpTransportBuilder.html#method.standalone) method, where it is
    /// backed by its own Tokio `Core` running in a separate thread, or by the
    /// [`shared(handle)`](struct.HttpTransportBuilder.html#method.shared) method, where it is
    /// backed by the Tokio `Handle` given to it.
    ///
    /// The final transport that is created will not support https. Either compile the crate with
    /// the "tls" feature to get that functionality, or provide a custom Hyper client via the
    /// [`with_client`](#method.with_client) that supports TLS.
    pub fn new() -> HttpTransportBuilder<DefaultClient> {
        HttpTransportBuilder::new(DefaultClient)
    }

    #[cfg(feature = "tls")]
    /// Starts the creation of a `HttpTransport`, which can be finished by the
    /// [`standalone()`](struct.HttpTransportBuilder.html#method.standalone) method, where it is
    /// backed by its own Tokio `Core` running in a separate thread, or by the
    /// [`shared(handle)`](struct.HttpTransportBuilder.html#method.shared) method, where it is
    /// backed by the Tokio `Handle` given to it.
    ///
    /// The final transport that is created uses the `hyper_tls::HttpsConnector` connector, and
    /// supports both http and https connections.
    pub fn new() -> HttpTransportBuilder<DefaultTlsClient> {
        HttpTransportBuilder::new(DefaultTlsClient)
    }

    /// Starts the creation of a `HttpTransport`, just like [`new`](#method.new), but with a custom
    /// Hyper Client.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # extern crate jsonrpc_client_http;
    /// # extern crate hyper;
    /// # use std::io;
    /// # use jsonrpc_client_http::{HttpTransport, Handle};
    ///
    /// # fn main() {
    /// HttpTransport::with_client(|handle: &Handle| {
    ///     Ok(hyper::Client::configure()
    ///         .keep_alive(false)
    ///         .build(handle)
    ///     ) as Result<_, io::Error>
    /// }).standalone().unwrap();
    /// # }
    /// ```
    pub fn with_client<C: ClientCreator>(client_creator: C) -> HttpTransportBuilder<C> {
        HttpTransportBuilder::new(client_creator)
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
            headers: header::Headers::new(),
        })
    }
}

/// Builder type for `HttpTransport`.
pub struct HttpTransportBuilder<C: ClientCreator> {
    client_creator: C,
    timeout: Option<Duration>,
}

impl<C: ClientCreator> HttpTransportBuilder<C> {
    fn new(client_creator: C) -> Self {
        HttpTransportBuilder {
            client_creator,
            timeout: None,
        }
    }

    /// Configure the timeout for RPC requests.
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Creates the final `HttpTransport` backed by its own Tokio `Core` running in a separate
    /// thread that is exclusive to this transport instance. To make the transport run on an
    /// existing event loop, use the [`shared`](#method.shared) method instead.
    pub fn standalone(self) -> Result<HttpTransport> {
        let (tx, rx) = ::std::sync::mpsc::channel();
        thread::spawn(
            move || match create_standalone_core(self.client_creator, self.timeout) {
                Err(e) => {
                    tx.send(Err(e)).unwrap();
                }
                Ok((mut core, request_tx, future)) => {
                    tx.send(Ok(Self::build(request_tx))).unwrap();
                    if let Err(_) = core.run(future) {
                        error!("JSON-RPC processing thread had an error");
                    }
                    debug!("Standalone HttpTransport thread exiting");
                }
            },
        );

        rx.recv().unwrap()
    }

    /// Creates the final `HttpTransport` backed by the Tokio `Handle` given to it. Use the
    /// [`standalone`](#method.standalone) method to make it create its own internal event loop.
    pub fn shared(self, handle: &Handle) -> Result<HttpTransport> {
        let client = self.client_creator
            .create(handle)
            .chain_err(|| ErrorKind::ClientCreatorError)?;
        let (request_tx, request_rx) = mpsc::unbounded();
        handle.spawn(create_request_processing_future(
            request_rx,
            client,
            self.timeout,
            handle.clone(),
        ));
        Ok(Self::build(request_tx))
    }

    fn build(request_tx: CoreSender) -> HttpTransport {
        HttpTransport {
            request_tx,
            id: Arc::new(AtomicUsize::new(1)),
        }
    }
}

/// A Future that times out and returns an error with the kind RequestTimeout.
#[derive(Debug)]
pub struct RequestTimeout<T> {
    timeout_future: Flatten<FutureResult<Timeout, io::Error>>,
    _item_type: PhantomData<T>,
}

impl<T> RequestTimeout<T> {
    /// Create a new `RequestTimeout`, ready to be used.
    pub fn new(duration: Duration, handle: &Handle) -> Self {
        RequestTimeout {
            timeout_future: Timeout::new(duration, handle).into_future().flatten(),
            _item_type: PhantomData,
        }
    }
}

impl<T> Future for RequestTimeout<T> {
    type Item = T;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        try_ready!(
            self.timeout_future
                .poll()
                .chain_err(|| ErrorKind::TimeoutError)
        );

        Err(ErrorKind::RequestTimeout.into())
    }
}

/// Creates all the components needed to run the `HttpTransport` in standalone mode.
fn create_standalone_core<C: ClientCreator>(
    client_creator: C,
    timeout: Option<Duration>,
) -> Result<(Core, CoreSender, Box<Future<Item = (), Error = ()>>)> {
    let core = Core::new().chain_err(|| ErrorKind::TokioCoreError("Unable to create"))?;
    let handle = core.handle();
    let client = client_creator
        .create(&handle)
        .chain_err(|| ErrorKind::ClientCreatorError)?;
    let (request_tx, request_rx) = mpsc::unbounded();
    let future = create_request_processing_future(request_rx, client, timeout, handle);
    Ok((core, request_tx, future))
}

/// Creates the `Future` that, when running on a Tokio Core, processes incoming RPC call
/// requests.
fn create_request_processing_future<CC: hyper::client::Connect>(
    request_rx: CoreReceiver,
    client: Client<CC, hyper::Body>,
    timeout: Option<Duration>,
    handle: Handle,
) -> Box<Future<Item = (), Error = ()>> {
    let f = request_rx.for_each(move |(request, response_tx)| {
        trace!("Sending request to {}", request.uri());
        let request = client.request(request).from_err();
        let request_with_timeout: Box<
            Future<Item = hyper::Response, Error = Error> + 'static,
        > = match timeout {
            Some(duration) => Box::new(
                request
                    .select(RequestTimeout::new(duration, &handle))
                    .map(|(result, _)| result)
                    .map_err(|(error, _)| error),
            ),
            None => Box::new(request),
        };

        request_with_timeout
            .and_then(|response: hyper::Response| {
                if response.status() == hyper::StatusCode::Ok {
                    future::ok(response)
                } else {
                    future::err(ErrorKind::HttpError(response.status()).into())
                }
            })
            .and_then(|response: hyper::Response| response.body().concat2().from_err())
            .map(|response_chunk| response_chunk.to_vec())
            .then(move |response_result| {
                if let Err(_) = response_tx.send(response_result) {
                    warn!("Unable to send response back to caller");
                }
                Ok(())
            })
    });
    Box::new(f) as Box<Future<Item = (), Error = ()>>
}

/// A handle to a [`HttpTransport`](struct.HttpTransport.html). This implements
/// `jsonrpc_client_core::Transport` and can be used as the transport for a RPC client generated
/// by the `jsonrpc_client!` macro.
#[derive(Debug, Clone)]
pub struct HttpHandle {
    request_tx: CoreSender,
    uri: Uri,
    id: Arc<AtomicUsize>,
    headers: header::Headers,
}

impl HttpHandle {
    /// Configure a custom HTTP header for all requests sent through this transport.
    ///
    /// Replaces any header set by this library or by Hyper, such as the ContentType, ContentLength
    /// and Host headers.
    pub fn set_header<H: header::Header>(&mut self, header: H) -> &mut Self {
        self.headers.set(header);
        self
    }

    /// Creates a Hyper POST request with JSON content type and the given body data.
    fn create_request(&self, body: Vec<u8>) -> Request {
        let mut request = hyper::Request::new(hyper::Method::Post, self.uri.clone());
        {
            let headers = request.headers_mut();
            headers.set(hyper::header::ContentType::json());
            headers.set(hyper::header::ContentLength(body.len() as u64));
            headers.extend(self.headers.iter());
        }
        request.set_body(body);
        request
    }
}

impl Transport for HttpHandle {
    type Future = Box<Future<Item = Vec<u8>, Error = Self::Error> + Send>;
    type Error = Error;

    fn get_next_id(&mut self) -> u64 {
        self.id.fetch_add(1, Ordering::SeqCst) as u64
    }

    fn send(&self, json_data: Vec<u8>) -> Self::Future {
        let request = self.create_request(json_data);
        let (response_tx, response_rx) = oneshot::channel();
        let future = future::result(self.request_tx.unbounded_send((request, response_tx)))
            .map_err(|e| {
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
            .and_then(future::result);
        Box::new(future)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use hyper::client::HttpConnector;
    use std::io;

    #[test]
    fn new_shared() {
        let core = Core::new().unwrap();
        HttpTransport::new().shared(&core.handle()).unwrap();
    }

    #[test]
    fn new_standalone() {
        HttpTransport::new().standalone().unwrap();
    }

    #[test]
    fn new_custom_client() {
        HttpTransport::with_client(|handle: &Handle| {
            Ok(Client::configure().keep_alive(false).build(handle)) as Result<_>
        }).standalone()
            .unwrap();
    }

    #[test]
    fn failing_client_creator() {
        let error = HttpTransport::with_client(|_: &Handle| {
            Err(io::Error::new(io::ErrorKind::Other, "Dummy error"))
                as ::std::result::Result<Client<HttpConnector, hyper::Body>, io::Error>
        }).standalone()
            .unwrap_err();
        match error.kind() {
            &ErrorKind::ClientCreatorError => (),
            kind => panic!("invalid error kind response: {:?}", kind),
        }
    }
}
