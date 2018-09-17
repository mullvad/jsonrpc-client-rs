use super::{Error, ErrorKind, OutgoingMessage, Result};
use id_generator::IdGenerator;

use futures::future::Either;
use futures::{future, stream, sync::mpsc, Async, Future, Sink, Stream};
pub use jsonrpc_core::types;
use jsonrpc_core::types::{
    Call, Error as RpcError, Failure, Id, MethodCall, Notification, Output, Request, Response,
    Version,
};

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock};


type MethodHandler =
    Box<dyn Fn(MethodCall) -> Box<Future<Item = Output, Error = Error> + Send> + Send + Sync>;
type NotificationHandler =
    Box<dyn Fn(Notification) -> Box<Future<Item = (), Error = Error> + Send> + Send + Sync>;


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

/// A collection callbacks to be executed for specific notification and method JSON-RPC requests.
#[derive(Clone)]
pub struct Handlers {
    handlers: Arc<RwLock<HashMap<String, Handler>>>,
}

impl Handlers {
    fn new() -> Self {
        Handlers {
            handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a handler for a given method or notification.
    pub fn add(&self, method: String, handler: Handler) {
        let mut map = self.handlers.write().unwrap();
        map.insert(method, handler);
    }

    /// Remove a handler for a given method or notification.
    pub fn remove(&self, method: &str) {
        let mut map = self.handlers.write().unwrap();
        map.remove(method);
    }

    fn handle_single_call(&mut self, call: Call) -> PendingCall {
        match call {
            Call::MethodCall(method_call) => self.handle_method_call(method_call),
            Call::Notification(notification) => self.handle_notification_call(notification),
            Call::Invalid(id) => {
                Box::new(future::ok(Some(failure(id, RpcError::invalid_request()))))
            }
        }
    }

    fn handle_method_call(&self, method_call: MethodCall) -> PendingCall {
        let handlers = self.handlers.read().unwrap();

        match handlers.get(&method_call.method) {
            Some(Handler::Method(handler)) => Box::new(handler(method_call).map(Some)),
            _ => Box::new(future::ok(Some(failure(
                method_call.id,
                RpcError::method_not_found(),
            )))),
        }
    }

    fn handle_notification_call(&self, notification: Notification) -> PendingCall {
        let handlers = self.handlers.read().unwrap();
        match handlers.get(&notification.method) {
            Some(Handler::Notification(handler)) => Box::new(handler(notification).map(|_| None)),
            _ => Box::new(future::ok(None)),
        }
    }
}


impl fmt::Debug for Handlers {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Handlers{{\n")?;
        match self.handlers.read() {
            Ok(handlers) => {
                for (method_name, callback) in handlers.iter() {
                    write!(f, "\t{} - {}\n", method_name, callback.description())?;
                }
            }
            Err(e) => {
                write!(f, "ERROR - POISONED MUTEX  - {}\n", e)?;
            }
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

/// Default server implementation.
pub struct Server {
    handlers: Handlers,
    pending_futures: BTreeMap<u64, DrivableCall>,
    id_generator: IdGenerator,
}

type PendingCall = Box<Future<Item = Option<Output>, Error = Error> + Send>;
type DrivableCall = Box<Future<Item = (), Error = Error> + Send>;

impl Server {
    /// Constructs a new server.
    pub fn new() -> Self {
        Self {
            handlers: Handlers::new(),
            pending_futures: BTreeMap::new(),
            id_generator: IdGenerator::new(),
        }
    }

    fn push_future(&mut self, driveable_future: DrivableCall) {
        self.pending_futures
            .insert(self.id_generator.next_int(), driveable_future);
    }

    /// Access handler collection to add or remove handlers.
    pub fn get_handlers(&self) -> Handlers {
        self.handlers.clone()
    }
}

impl Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Result<Async<()>> {
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


impl ServerHandler for Server {
    fn process_request(&mut self, request: Request, sink: mpsc::Sender<OutgoingMessage>) {
        let driveable_future = match request {
            Request::Single(req) => {
                let driveable_future =
                    self.handlers
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
                        .map(|call| self.handlers.handle_single_call(call)),
                ).filter_map(|maybe_response| maybe_response)
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
        self.push_future(Box::new(driveable_future));
    }
}

/// The core client is able to use whatever server that implements this trait. The handler is
/// expected to be asynchronous, so it has to implement the Future trait as well.
pub trait ServerHandler: Future<Item = (), Error = Error> {
    /// Handles a request coming in from the server, optionally sending a result back to the
    /// server.
    fn process_request(&mut self, Request, sender: mpsc::Sender<OutgoingMessage>);
}
