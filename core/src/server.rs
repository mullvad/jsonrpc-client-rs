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
use std::sync::{Arc, RwLock, atomic};
use std::result;
use std::error;


type MethodHandler =
    Box<dyn Fn(MethodCall) -> Box<dyn Future<Item = Output, Error = Error> + Send> + Send + Sync>;
type NotificationHandler =
    Box<dyn Fn(Notification) -> Box<dyn Future<Item = (), Error = Error> + Send> + Send + Sync>;


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

/// A failure to set a handler
#[derive(Debug)]
pub enum HandlerSettingError {
    /// Can't set handler because a handler for the given method name is already set
    AlreadyExists(Handler),
    /// Server is already shutting down
    Shutdown(Handler),
}

impl fmt::Display for HandlerSettingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerSettingError::AlreadyExists(_) => write!(f, "HandlerSettingError - Handler for the given method name is already set"),
            HandlerSettingError::Shutdown(_) => write!(f, "HandlerSettingError - RPC client already shut down"),
        }
    }
}
impl error::Error for HandlerRemovalError {}

/// A failure to remove a handler
#[derive(Debug)]
pub enum HandlerRemovalError{
    /// Handler does not exist
    NoHandler,
    /// Server is already shutting down
    Shutdown,
}

impl fmt::Display for HandlerRemovalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerRemovalError::NoHandler => write!(f, "HandlerError - Handler for the given method name is already set"),
            HandlerRemovalError::Shutdown => write!(f, "HandlerError - RPC client already shut down"),
        }
    }
}

impl error::Error for HandlerSettingError {}



/// A collection callbacks to be executed for specific notification and method JSON-RPC requests.
#[derive(Clone)]
pub struct ServerHandle {
    handlers: Arc<RwLock<HashMap<String, Handler>>>,
    is_shutdown: Arc<atomic::AtomicBool>,
}

impl ServerHandle {
    fn new() -> Self {
        ServerHandle {
            handlers: Arc::new(RwLock::new(HashMap::new())),
            is_shutdown: Arc::new(false.into()),
        }
    }

    /// Add a handler for a given method or notification.
    pub fn add(&self, method: String, handler: Handler) -> result::Result<(), HandlerSettingError> {
        if self.is_shutdown.load(atomic::Ordering::SeqCst) {
            return Err(HandlerSettingError::Shutdown(handler));
        };
        let mut map = self.handlers.write().unwrap();

        if map.contains_key(&method) {
            return Err(HandlerSettingError::AlreadyExists(handler));
        };

        map.insert(method, handler);
        Ok(())
    }

    /// Remove a handler for a given method or notification.
    pub fn remove(&self, method: &str) -> result::Result<(), HandlerRemovalError> {
        if self.is_shutdown.load(atomic::Ordering::SeqCst) {
            return Err(HandlerRemovalError::Shutdown);
        };

        let mut map = self.handlers.write().unwrap();

        if map.remove(method).is_some() {
            Ok(())
        } else {
            Err(HandlerRemovalError::NoHandler)
        }
    }

    fn shutdown(&mut self) {
        self.is_shutdown.store(true, atomic::Ordering::SeqCst);
        match self.handlers.write() {
            Ok(mut map) => {
                map.clear();
            },
            Err(e) => {
                error!("Handler mutex is poisoned - {}", e);
            }
        }
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


impl fmt::Debug for ServerHandle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ServerHandle{{\n")?;
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
    handlers: ServerHandle,
    pending_futures: BTreeMap<u64, DrivableCall>,
    id_generator: IdGenerator,
}

type PendingCall = Box<Future<Item = Option<Output>, Error = Error> + Send>;
type DrivableCall = Box<Future<Item = (), Error = Error> + Send>;

impl Server {
    /// Constructs a new server.
    pub fn new() -> Self {
        Self {
            handlers: ServerHandle::new(),
            pending_futures: BTreeMap::new(),
            id_generator: IdGenerator::new(),
        }
    }

    fn push_future(&mut self, driveable_future: DrivableCall) {
        self.pending_futures
            .insert(self.id_generator.next_int(), driveable_future);
    }

    /// Access handler collection to add or remove handlers.
    pub fn get_handle(&self) -> ServerHandle {
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

/// Once the server is dropped, all the handlers should be destroyed.
impl Drop for Server {
    fn drop(&mut self) {
        self.handlers.shutdown();
    }
}


impl ServerHandler for Server {
    fn process_request(&mut self, request: Request, sink: mpsc::Sender<OutgoingMessage>) {
        let mut driveable_future = match request {
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
        // eagerly drive the new future to register it with the current future execution context.
        // otherwise, once it's ready, the currently executing future won't be notified.
        match driveable_future.poll() {
            Ok(Async::NotReady) => self.push_future(Box::new(driveable_future)),
            Err(e) => self.push_future(Box::new(future::err(e))),
            Ok(Async::Ready(())) => (),
        };
    }
}

/// The core client is able to use whatever server that implements this trait. The handler is
/// expected to be asynchronous, so it has to implement the Future trait as well.
pub trait ServerHandler: Future<Item = (), Error = Error> {
    /// Handles a request coming in from the server, optionally sending a result back to the
    /// server.
    fn process_request(&mut self, Request, sender: mpsc::Sender<OutgoingMessage>);
}
