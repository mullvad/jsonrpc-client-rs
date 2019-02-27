use super::{Error, ErrorKind, OutgoingMessage, Result};
use id_generator::IdGenerator;

use futures::future::Either;
use futures::{
    future, stream,
    sync::{mpsc, oneshot},
    Async, Future, Sink, Stream,
};
pub use jsonrpc_core::types;
use jsonrpc_core::types::{
    Call, Error as RpcError, Failure, Id, MethodCall, Notification, Output, Request, Response,
    Version,
};

use std::collections::{BTreeMap, HashMap};
use std::error;
use std::fmt;
use std::result;


type MethodHandler =
    Box<dyn Fn(MethodCall) -> Box<dyn Future<Item = Output, Error = Error> + Send> + Send>;
type NotificationHandler =
    Box<dyn Fn(Notification) -> Box<dyn Future<Item = (), Error = Error> + Send> + Send>;

type PendingCall = Box<Future<Item = Option<Output>, Error = Error> + Send>;
type DrivableCall = Box<Future<Item = (), Error = Error> + Send>;


/// A callback to be called in response to either a notification or a method call coming in from
/// the server.
pub enum Handler {
    /// Method handler.
    Method(MethodHandler),
    /// Notification handler.
    Notification(NotificationHandler),
}

impl Handler {
    fn description(&self) -> &'static str {
        match self {
            Handler::Method(_) => "method",
            Handler::Notification(_) => "notification",
        }
    }
}

impl fmt::Debug for Handler {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RPC {} handler", self.description())
    }
}

/// An error that is returned when one fails to set a handler
#[derive(Debug)]
pub struct HandlerSettingError {
    /// The handler that wasn't set.
    pub handler: Handler,
    /// Kind of error encountered whilst trying to set handler.
    pub kind: HandlerError,
}

impl fmt::Display for HandlerSettingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Failure to set new {} - {}",
            self.handler.description(),
            self.kind
        )
    }
}

impl HandlerSettingError {
    fn already_exists(handler: Handler) -> Self {
        Self {
            handler,
            kind: HandlerError::AlreadyExists,
        }
    }

    fn shutdown(handler: Handler) -> Self {
        Self {
            handler,
            kind: HandlerError::Shutdown,
        }
    }
}


impl error::Error for HandlerError {}

/// Failure kinds that can occur when managing the server handle.
#[derive(Debug)]
pub enum HandlerError {
    /// Handler already exists
    AlreadyExists,
    /// Handler does not exist
    NoHandler,
    /// Server is already shutting down
    Shutdown,
}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerError::AlreadyExists => write!(
                f,
                "HandlerError - Handler for the given method name is already set"
            ),
            HandlerError::NoHandler => write!(
                f,
                "HandlerError - Handler for the given method name does not exist"
            ),
            HandlerError::Shutdown => write!(f, "HandlerError - RPC client already shut down"),
        }
    }
}

impl error::Error for HandlerSettingError {}


/// A collection callbacks to be executed for specific notification and method JSON-RPC requests.
pub struct Handlers {
    handlers: HashMap<String, Handler>,
}

impl Handlers {
    fn new() -> Self {
        Handlers {
            handlers: HashMap::new(),
        }
    }

    /// Add a handler for a given method or notification.
    pub fn add(
        &mut self,
        method: String,
        handler: Handler,
    ) -> result::Result<(), HandlerSettingError> {
        if self.handlers.contains_key(&method) {
            return Err(HandlerSettingError::already_exists(handler));
        };

        self.handlers.insert(method, handler);
        Ok(())
    }

    /// Remove a handler for a given method or notification.
    pub fn remove(&mut self, method: &str) -> result::Result<Handler, HandlerError> {
        if let Some(handler) = self.handlers.remove(method) {
            Ok(handler)
        } else {
            Err(HandlerError::NoHandler)
        }
    }

    fn shutdown(&mut self) {
        self.handlers.clear();
    }

    fn handle_single_call(&self, call: Call) -> PendingCall {
        match call {
            Call::MethodCall(method_call) => self.handle_method_call(method_call),
            Call::Notification(notification) => self.handle_notification_call(notification),
            Call::Invalid(id) => {
                Box::new(future::ok(Some(failure(id, RpcError::invalid_request()))))
            }
        }
    }

    fn handle_method_call(&self, method_call: MethodCall) -> PendingCall {
        match self.handlers.get(&method_call.method) {
            Some(Handler::Method(handler)) => Box::new(handler(method_call).map(Some)),
            _ => Box::new(future::ok(Some(failure(
                method_call.id,
                RpcError::method_not_found(),
            )))),
        }
    }

    fn handle_notification_call(&self, notification: Notification) -> PendingCall {
        match self.handlers.get(&notification.method) {
            Some(Handler::Notification(handler)) => Box::new(handler(notification).map(|_| None)),
            _ => Box::new(future::ok(None)),
        }
    }
}

impl fmt::Debug for Handlers {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Handlers{{\n")?;
        for (method_name, callback) in self.handlers.iter() {
            write!(f, "\t{} - {}\n", method_name, callback.description())?;
        }

        write!(f, "}}")
    }
}

fn failure(id: Id, error: RpcError) -> Output {
    Output::Failure(Failure {
        jsonrpc: Some(Version::V2),
        error,
        id,
    })
}

#[derive(Debug)]
enum HandlerMsg {
    AddHandler(
        String,
        Handler,
        oneshot::Sender<result::Result<(), HandlerSettingError>>,
    ),
    RemoveHandler(
        String,
        oneshot::Sender<result::Result<Handler, HandlerError>>,
    ),
}

/// Dynamically sets and unsets new handlers for method calls and notifications
#[derive(Debug, Clone)]
pub struct ServerHandle {
    tx: mpsc::Sender<HandlerMsg>,
}

impl ServerHandle {
    /// Sets a new handler
    pub fn add(
        &self,
        name: String,
        handler: Handler,
    ) -> impl Future<Item = (), Error = HandlerSettingError> {
        let (result_tx, result_rx) = oneshot::channel();
        self.tx
            .clone()
            .send(HandlerMsg::AddHandler(name, handler, result_tx))
            .map_err(|e| {
                let handler = match e.into_inner() {
                    HandlerMsg::AddHandler(_, handler, _) => handler,
                    _ => unreachable!(),
                };
                HandlerSettingError::shutdown(handler)
            })
            // since the channel is not buffered, if the message is received, the only circumstance
            // when receiving a result back would fail is if a panic occurs while adding the
            // handler.
            .and_then(|_| result_rx.map_err(|_| unreachable!()))
            .and_then(|r| future::result(r))
    }

    /// Unsets a handler
    pub fn remove(&self, name: String) -> impl Future<Item = Handler, Error = HandlerError> {
        let (result_tx, result_rx) = oneshot::channel();
        self.tx
            .clone()
            .send(HandlerMsg::RemoveHandler(name, result_tx))
            .map_err(|_| HandlerError::Shutdown)
            .and_then(|_| result_rx.map_err(|_| HandlerError::Shutdown))
            .and_then(|r| future::result(r))
    }
}


/// Default server implementation.
pub struct Server {
    handler_chan: Option<mpsc::Receiver<HandlerMsg>>,
    handler_map: Handlers,
    pending_futures: BTreeMap<u64, DrivableCall>,
    id_generator: IdGenerator,
}

impl Server {
    /// Constructs a new server.
    pub fn new() -> (Self, ServerHandle) {
        let (tx, rx) = mpsc::channel(0);
        (
            Self {
                handler_map: Handlers::new(),
                pending_futures: BTreeMap::new(),
                id_generator: IdGenerator::new(),
                handler_chan: Some(rx),
            },
            ServerHandle { tx },
        )
    }

    fn push_future(&mut self, driveable_future: DrivableCall) {
        self.pending_futures
            .insert(self.id_generator.next_int(), driveable_future);
    }

    fn drain_handler_chan(&mut self) {
        if let Some(mut chan) = self.handler_chan.take() {
            loop {
                match chan.poll() {
                    Ok(Async::Ready(Some(msg))) => match msg {
                        HandlerMsg::AddHandler(name, handler, tx) => {
                            let _ = tx.send(self.handler_map.add(name, handler));
                        }
                        HandlerMsg::RemoveHandler(name, tx) => {
                            let _ = tx.send(self.handler_map.remove(&name));
                        }
                    },
                    Ok(Async::NotReady) => {
                        self.handler_chan = Some(chan);
                        return;
                    }
                    Err(_) => unreachable!(),
                    Ok(Async::Ready(None)) => {
                        return;
                    }
                }
            }
        };
    }
}

impl Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Result<Async<()>> {
        self.drain_handler_chan();
        let polled: Result<Vec<(u64, Async<()>)>> = self
            .pending_futures
            .iter_mut()
            .map(|(key, fut)| Ok((*key, fut.poll()?)))
            .collect();
        for key in polled?.iter().filter_map(|(key, res)| match res {
            Async::Ready(_) => Some(key),
            _ => None,
        }) {
            self.pending_futures.remove(key);
        }

        Ok(Async::NotReady)
    }
}

/// Once the server is dropped, all the handlers should be destroyed.
impl Drop for Server {
    fn drop(&mut self) {
        self.handler_map.shutdown();
    }
}


impl ServerHandler for Server {
    fn process_request(
        &mut self,
        request: Request,
        sink: mpsc::Sender<OutgoingMessage>,
    ) -> Result<()> {
        let mut driveable_future = match request {
            Request::Single(req) => {
                let driveable_future =
                    self.handler_map
                        .handle_single_call(req)
                        .and_then(move |output| match output {
                            Some(response) => Either::A(
                                sink.send(OutgoingMessage::Response(Response::Single(response)))
                                    .map_err(|_| ErrorKind::Shutdown.into())
                                    .map(|_| ()),
                            ),
                            None => Either::B(future::ok(())),
                        });
                Either::A(driveable_future)
            }

            Request::Batch(requests) => {
                let fut = stream::futures_unordered(
                    requests
                        .into_iter()
                        .map(|call| self.handler_map.handle_single_call(call)),
                )
                .filter_map(|maybe_response| maybe_response)
                .collect()
                .and_then(|results| {
                    if results.len() > 0 {
                        Either::A(
                            sink.send(OutgoingMessage::Response(Response::Batch(results)))
                                .map_err(|_| ErrorKind::Shutdown.into())
                                .map(|_| ()),
                        )
                    } else {
                        Either::B(future::ok(()))
                    }
                });
                Either::B(fut)
            }
        };
        // eagerly drive the new future to register it with the current future execution context.
        // otherwise, once it's ready, the currently executing future won't be notified.
        match driveable_future.poll() {
            Ok(Async::NotReady) => self.push_future(Box::new(driveable_future)),
            Err(e) => self.push_future(Box::new(future::err(e))),
            Ok(Async::Ready(())) => (),
        };
        Ok(())
    }
}

/// The core client is able to use whatever server that implements this trait. The handler is
/// expected to be asynchronous, so it has to implement the Future trait as well.
pub trait ServerHandler: Future<Item = (), Error = Error> {
    /// Handles a request coming in from the server, optionally sending a result back to the
    /// server.
    fn process_request(&mut self, Request, sender: mpsc::Sender<OutgoingMessage>) -> Result<()>;
}
