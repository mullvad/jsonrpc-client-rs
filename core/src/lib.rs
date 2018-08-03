// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


//! A crate for generating transport agnostic, auto serializing, strongly typed JSON-RPC 2.0
//! clients.
//!
//! This crate mainly provides a macro, `jsonrpc_client`. The macro generates structs that can be
//! used for calling JSON-RPC 2.0 APIs. The macro lets you list methods on the struct with
//! arguments and a return type. The macro then generates a struct which will automatically
//! serialize the arguments, send the request and deserialize the response into the target type.
//!
//! # Transports
//!
//! The `jsonrpc-client-core` crate itself and the structs generated by the `jsonrpc_client` macro
//! are transport agnostic. They can use any type implementing the `Transport` trait.
//!
//! The main (and so far only) transport implementation is the Hyper based HTTP implementation
//! in the [`jsonrpc-client-http`](../jsonrpc_client_http/index.html) crate.
//!
//! # Example
//!
//! ```rust,ignore
//! #[macro_use]
//! extern crate jsonrpc_client_core;
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
//!     let transport_handle = transport
//!         .handle("http://api.fizzbuzzexample.org/rpc/")
//!         .unwrap();
//!     let mut client = FizzBuzzClient::new(transport_handle);
//!     let result1 = client.fizz_buzz(3).call().unwrap();
//!     let result2 = client.fizz_buzz(4).call().unwrap();
//!     let result3 = client.fizz_buzz(5).call().unwrap();
//!
//!     // Should print "fizz 4 buzz" if the server implemented the service correctly
//!     println!("{} {} {}", result1, result2, result3);
//! }
//! ```
//!

#![deny(missing_docs)]

#[macro_use]
pub extern crate error_chain;
extern crate futures;
extern crate jsonrpc_core;
#[macro_use]
extern crate log;
extern crate serde;
#[cfg_attr(test, macro_use)]
extern crate serde_json;

use futures::future::{self, Future};
use futures::stream::Peekable;
use futures::sync::{mpsc, oneshot};
use futures::{Async, AsyncSink};
use futures::{Sink, Stream};
use jsonrpc_core::types::{
    Failure as RpcFailure, Id, MethodCall, Notification, Output, Params, Success as RpcSuccess,
    Version,
};
use serde_json::Value as JsonValue;


use std::collections::HashMap;

/// Contains the main macro of this crate, `jsonrpc_client`.
#[macro_use]
mod macros;

/// Module containing an example client. To show in the docs what a generated struct look like.
pub mod example;

error_chain! {
    errors {
        /// Error in the underlying transport layer.
        TransportError {
            description("Unable to send the JSON-RPC 2.0 request")
        }
        /// Error while serializing method parameters.
        SerializeError {
            description("Unable to serialize the method parameters")
        }
        /// Error when deserializing server response
        DeserializeError {
            description("Unable to deserialize response")
        }
        /// Error while deserializing or parsing the response data.
        ResponseError(msg: &'static str) {
            description("Unable to deserialize the response into the desired type")
            display("Unable to deserialize the response: {}", msg)
        }
        /// The server returned a response with an incorrect version
        InvalidVersion {
            description("Method call returned a response that was not specified as JSON-RPC 2.0")
        }
        /// Error when trying to send a new message to the server because the client is already
        /// shut down.
        Shutdown {
            description("RPC Client already shut down")
        }
        /// The request was replied to, but with a JSON-RPC 2.0 error.
        JsonRpcError(error: jsonrpc_core::Error) {
            description("Method call returned JSON-RPC 2.0 error")
            display("JSON-RPC 2.0 Error: {} ({})", error.code.description(), error.message)
        }
    }
}


/// This handle allows one to create futures for RPC invocations. For the requests to ever be
/// resolved, the Client future has to be driven.
#[must_use]
#[derive(Debug, Clone)]
pub struct ClientHandle {
    rpc_call_chan: mpsc::Sender<ClientCall>,
}

impl ClientHandle {
    /// Invokes an RPC and creates a future representing the RPC's result.
    pub fn call_method<T>(
        &self,
        method: impl Into<String>,
        parameters: &impl serde::Serialize,
    ) -> impl Future<Item = T, Error = Error>
    where
        T: serde::de::DeserializeOwned,
    {
        let (tx, rx) = oneshot::channel();

        let rpc_chan = self.rpc_call_chan.clone();

        future::result(serialize_parameters(parameters))
            .and_then(|params| {
                rpc_chan
                    .send(ClientCall::RpcCall(method.into(), params, tx))
                    .map_err(|_| ErrorKind::Shutdown.into())
            }).then(|_| rx.map_err(|_| ErrorKind::Shutdown))
            .flatten()
            .map(|r| serde_json::from_value(r).chain_err(|| ErrorKind::DeserializeError))
            .flatten()
            .from_err()
    }

    /// Send arbitrary RPC call to Client. Primarily intended to be used from macro
    /// `jsonrpc_client!`.
    pub fn send_client_call<T: serde::de::DeserializeOwned + Send + Sized>(
        &self,
        client_call: Result<ClientCall>,
        rx: oneshot::Receiver<Result<JsonValue>>,
    ) -> impl Future<Item = T, Error = Error> {
        let rpc_chan = self.rpc_call_chan.clone();

        future::result(client_call)
            .and_then(|call| rpc_chan.send(call).map_err(|_| ErrorKind::Shutdown.into()))
            .then(|_| {
                rx.map_err(|_| ErrorKind::Shutdown)
                    .flatten()
                    .map(|r| serde_json::from_value(r).chain_err(|| ErrorKind::DeserializeError))
            }).flatten()
            .from_err()
    }


    /// Sends a notificaiton to the Server.
    pub fn send_notification<P, T>(
        &self,
        method: String,
        parameters: P,
    ) -> impl Future<Item = (), Error = Error>
    where
        P: serde::Serialize,
    {
        let (tx, rx) = oneshot::channel();

        let rpc_chan = self.rpc_call_chan.clone();

        future::result(serialize_parameters(&parameters))
            .and_then(|params| {
                rpc_chan
                    .send(ClientCall::Notification(method, params, tx))
                    .map_err(|_| ErrorKind::Shutdown.into())
            }).then(|_| rx.map_err(|_| Error::from(ErrorKind::Shutdown)))
            .flatten()
    }
}

#[derive(Debug)]
struct IdGenerator {
    next_id: u64,
}

impl IdGenerator {
    fn new() -> IdGenerator {
        IdGenerator { next_id: 1 }
    }

    fn next(&mut self) -> Id {
        let id = Id::Num(self.next_id);
        self.next_id += 1;
        id
    }
}


/// Client is a future that takes an arbitrary transport sink and stream pair and handles JSON-RPC
/// 2.0 messages with a server. This future has to be driven for the messages to be passed around.
/// To send and receive messages, one should use the ClientHandle.
#[derive(Debug)]
pub struct Client<W, R, E>
where
    W: Sink<SinkItem = String, SinkError = E>,
    R: Stream<Item = String, Error = E>,
    E: std::error::Error + Send + 'static,
{
    // request channel
    rpc_call_rx: Peekable<mpsc::Receiver<ClientCall>>,

    // state
    id_generator: IdGenerator,
    shutting_down: bool,
    pending_requests: HashMap<Id, oneshot::Sender<Result<JsonValue>>>,
    pending_payload: Option<String>,
    fatal_error: Option<Error>,

    // transport
    transport_tx: W,
    transport_rx: R,
}

impl<W, R, E> Client<W, R, E>
where
    W: Sink<SinkItem = String, SinkError = E>,
    R: Stream<Item = String, Error = E>,
    E: std::error::Error + Send + 'static,
{
    /// To create a new Client, one must provide a transport sink and stream pair. The transport
    /// sinks are expected to send and receive strings which should hold exactly one JSON
    /// object. If any error is returned by either the sink or the stream, this future will fail,
    /// and all pending requests will be dropped. If the transport stream finishes, this future
    /// will resolve without an error. The client will resolve once all of it's handles and
    /// corresponding futures get resolved.
    pub fn new(transport_tx: W, transport_rx: R) -> (Self, ClientHandle) {
        let (rpc_call_chan, rpc_call_rx) = mpsc::channel(0);
        (
            Client {
                // request channel
                rpc_call_rx: rpc_call_rx.peekable(),

                // state
                id_generator: IdGenerator::new(),
                pending_payload: None,
                shutting_down: false,
                fatal_error: None,
                pending_requests: HashMap::new(),

                // transport
                transport_tx,
                transport_rx,
            },
            ClientHandle { rpc_call_chan },
        )
    }

    fn should_shut_down(&mut self) -> bool {
        self.fatal_error.is_some() || self.shutting_down
    }

    /// Handles incoming RPC requests from handles, drains incoming responses from the transport
    /// stream and drives the transport sink.
    fn handle_messages(&mut self) -> Result<()> {
        // try send a leftover payload
        if let Some(payload) = self.pending_payload.take() {
            self.send_payload(payload)?;
        }
        // drain incoming payload
        self.poll_transport_rx()?;
        // drain incoming rpc requests, only if the writing pipe is ready
        self.poll_rpc_requests()?;
        // poll transport tx to drive sending
        self.poll_transport_tx()?;
        Ok(())
    }

    fn send_payload(&mut self, json_string: String) -> Result<()> {
        ensure!(self.fatal_error.is_none(), ErrorKind::TransportError);
        match self.transport_tx.start_send(json_string) {
            Ok(AsyncSink::Ready) => Ok(()),
            Ok(AsyncSink::NotReady(payload)) => {
                self.pending_payload = Some(payload);
                Ok(())
            }
            Err(e) => Err(e).chain_err(|| ErrorKind::TransportError),
        }
    }

    fn poll_transport_rx(&mut self) -> Result<()> {
        loop {
            match self
                .transport_rx
                .poll()
                .chain_err(|| ErrorKind::TransportError)?
            {
                Async::Ready(Some(new_payload)) => {
                    self.handle_transport_rx_payload(new_payload)?;
                    continue;
                }
                Async::Ready(None) => {
                    trace!("transport receiver shut down, shutting down as well");
                    return Err(ErrorKind::Shutdown.into());
                }
                Async::NotReady => return Ok(()),
            }
        }
    }

    fn handle_transport_rx_payload(&mut self, payload: String) -> Result<()> {
        let response: Output = serde_json::from_str(&payload)
            .chain_err(|| ErrorKind::ResponseError("Failed to deserialize response"))?;
        self.handle_response(response)
    }

    fn handle_response(&mut self, output: Output) -> Result<()> {
        if output.version() != Some(jsonrpc_core::types::Version::V2) {
            return Err(ErrorKind::InvalidVersion.into());
        };
        let (id, result): (Id, Result<JsonValue>) = match output {
            Output::Success(RpcSuccess { result, id, .. }) => (id, Ok(result)),
            Output::Failure(RpcFailure { id, error, .. }) => {
                (id, Err(ErrorKind::JsonRpcError(error).into()))
            }
        };

        match self.pending_requests.remove(&id) {
            Some(completion_chan) => Self::send_rpc_response(&id, completion_chan, result),
            None => trace!("Received response with an invalid id {:?}", id),
        };
        Ok(())
    }

    fn poll_rpc_requests(&mut self) -> Result<()> {
        loop {
            // can only drain incoming RPC calls if the transport is ready to send a payload.  If
            // there's a pending payload present, it must be because the transport wasn't ready to
            // start sending it previously. However, if the incoming calls stream is finished,
            // don't return early, as the future has to be shut down.
            let call_stream_ended = match self.rpc_call_rx.peek() {
                Ok(Async::Ready(None)) => true,
                _ => false,
            };
            if self.pending_payload.is_some() && !call_stream_ended {
                return Ok(());
            }

            // There's no pending payload, so new RPC requests can be processed.
            match self.rpc_call_rx.poll() {
                Ok(Async::NotReady) => return Ok(()),
                Ok(Async::Ready(Some(call))) => {
                    self.handle_rpc_request(call)?;
                    continue;
                }
                Ok(Async::Ready(None)) => {
                    trace!("All client handles and futures dropped, shutting down");
                    return Err(ErrorKind::Shutdown.into());
                }
                Err(_) => {
                    unreachable!("RPC channel returned an error, should never happen");
                }
            }
        }
    }

    fn handle_rpc_request(&mut self, call: ClientCall) -> Result<()> {
        match call {
            ClientCall::RpcCall(method, parameters, completion) => {
                let new_id = self.id_generator.next();
                match serialize_method_request(new_id.clone(), method, &parameters) {
                    Ok(payload) => {
                        self.add_new_call(new_id, completion);
                        self.send_payload(payload)?;
                    }
                    Err(err) => {
                        Self::send_rpc_response(&new_id, completion, Err(err));
                    }
                };
            }
            ClientCall::Notification(method, parameters, completion) => {
                match serialize_notification_request(method, &parameters) {
                    Ok(payload) => {
                        if let Err(_) = completion.send(Ok(())) {
                            trace!("future for notification dopped already");
                        }
                        self.send_payload(payload)?;
                    }
                    Err(e) => {
                        if let Err(_) = completion.send(Err(e)) {
                            trace!("Future for notification already dropped");
                        }
                    }
                }
            } // TODO: add support for subscriptions
        };
        Ok(())
    }

    fn send_rpc_response<T>(id: &Id, chan: oneshot::Sender<Result<T>>, value: Result<T>) {
        if let Err(_) = chan.send(value) {
            trace!("Future for RPC call {:?} dropped already", id);
        }
    }

    fn handle_shutdown(&mut self) -> futures::Poll<(), Error> {
        if let Err(e) = self.poll_transport_rx() {
            trace!(
                "Failed to drain incoming messages from transport whilst shutting down: {}",
                e.description()
            );
        }
        match self
            .transport_tx
            .close()
            .chain_err(|| ErrorKind::TransportError)
        {
            Ok(Async::NotReady) => {
                return Ok(Async::NotReady);
            }
            Err(e) => {
                warn!(
                    "Encountered error whilst shutting down client: {}",
                    e.description()
                );
            }
            _ => (),
        }

        self.fatal_error
            .take()
            .map(|e| Err(e))
            .unwrap_or(Ok(Async::Ready(())))
    }

    fn add_new_call(&mut self, id: Id, completion: oneshot::Sender<Result<JsonValue>>) {
        self.pending_requests.insert(id, completion);
    }

    fn poll_transport_tx(&mut self) -> Result<()> {
        if self.fatal_error.is_none() {
            self.transport_tx
                .poll_complete()
                .chain_err(|| ErrorKind::TransportError)?;
        }
        Ok(())
    }
}

impl<W, R, E> Future for Client<W, R, E>
where
    W: Sink<SinkItem = String, SinkError = E>,
    R: Stream<Item = String, Error = E>,
    E: std::error::Error + Send + 'static,
{
    type Item = ();
    type Error = Error;


    fn poll(&mut self) -> Result<Async<Self::Item>> {
        if !self.should_shut_down() {
            match self.handle_messages() {
                Ok(()) => return Ok(Async::NotReady),
                Err(Error(ErrorKind::Shutdown, _)) => self.shutting_down = true,
                Err(e) => self.fatal_error = Some(e),
            }
        }
        self.handle_shutdown()
    }
}


/// A client call is an operation that the Client can execute with a JSON-RPC 2.0 server.
/// Any client handle can execute these against the corresponding RPC server.
#[derive(Debug)]
pub enum ClientCall {
    /// Invoke an RPC
    RpcCall(String, Option<Params>, oneshot::Sender<Result<JsonValue>>),
    /// Send a notification
    Notification(String, Option<Params>, oneshot::Sender<Result<()>>),
}


/// Creates a JSON-RPC 2.0 request to the given method with the given parameters.
fn serialize_method_request(
    id: Id,
    method: String,
    params: &impl serde::Serialize,
) -> Result<String> {
    let serialized_params = serialize_parameters(params)?;
    let method_call = MethodCall {
        jsonrpc: Some(Version::V2),
        method,
        params: serialized_params,
        id,
    };
    serde_json::to_string(&method_call).chain_err(|| ErrorKind::SerializeError)
}

fn serialize_parameters(params: &impl serde::Serialize) -> Result<Option<Params>> {
    let parameters = match serde_json::to_value(params).chain_err(|| ErrorKind::SerializeError)? {
        JsonValue::Null => None,
        JsonValue::Array(vec) => Some(Params::Array(vec)),
        JsonValue::Object(obj) => Some(Params::Map(obj)),
        value => Some(Params::Array(vec![value])),
    };
    Ok(parameters)
}

/// Creates a JSON-RPC 2.0 notification request to the given method with the given parameters.
fn serialize_notification_request(
    method: String,
    params: &impl serde::Serialize,
) -> Result<String> {
    let serialized_params = serialize_parameters(params)?;
    let notification = Notification {
        jsonrpc: Some(Version::V2),
        method,
        params: serialized_params,
    };
    serde_json::to_string(&notification).chain_err(|| ErrorKind::SerializeError)
}
