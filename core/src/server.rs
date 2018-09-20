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
use std::error;
use std::fmt;
use std::result;
use std::sync::{atomic, Arc, RwLock};


type MethodHandler =
    Box<dyn Fn(MethodCall) -> Box<dyn Future<Item = Output, Error = Error> + Send> + Send + Sync>;
type NotificationHandler =
    Box<dyn Fn(Notification) -> Box<dyn Future<Item = (), Error = Error> + Send> + Send + Sync>;

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
    pub kind: HandleError,
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
            kind: HandleError::AlreadyExists,
        }
    }

    fn handler_crash(handler: Handler) -> Self {
        Self {
            handler,
            kind: HandleError::HandlerCrash,
        }
    }

    fn shutdown(handler: Handler) -> Self {
        Self {
            handler,
            kind: HandleError::Shutdown,
        }
    }
}


impl error::Error for HandleError {}

/// Failure kinds that can occur when managing the server handle.
#[derive(Debug)]
pub enum HandleError {
    /// Handler already exists
    AlreadyExists,
    /// Handler does not exist
    NoHandler,
    /// Server is already shutting down
    Shutdown,
    /// A callback paniced!
    HandlerCrash,
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandleError::AlreadyExists => write!(
                f,
                "HandlerError - Handler for the given method name is already set"
            ),
            HandleError::NoHandler => write!(
                f,
                "HandlerError - Handler for the given method name does not exist"
            ),
            HandleError::Shutdown => write!(f, "HandlerError - RPC client already shut down"),
            HandleError::HandlerCrash => {
                write!(f, "HandleError - a method handler panicked, shutting down")
            }
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
            return Err(HandlerSettingError::shutdown(handler));
        };
        let mut map = match self.handlers.write() {
            Ok(map) => map,
            Err(_) => return Err(HandlerSettingError::handler_crash(handler)),
        };

        if map.contains_key(&method) {
            return Err(HandlerSettingError::already_exists(handler));
        };

        map.insert(method, handler);
        Ok(())
    }

    /// Remove a handler for a given method or notification.
    pub fn remove(&self, method: &str) -> result::Result<(), HandleError> {
        if self.is_shutdown.load(atomic::Ordering::SeqCst) {
            return Err(HandleError::Shutdown);
        };

        let mut map = self.handlers.write().unwrap();

        if map.remove(method).is_some() {
            Ok(())
        } else {
            Err(HandleError::NoHandler)
        }
    }

    fn shutdown(&mut self) {
        self.is_shutdown.store(true, atomic::Ordering::SeqCst);
        match self.handlers.write() {
            Ok(mut map) => {
                map.clear();
            }
            Err(e) => {
                error!("ServerHandle mutex is poisoned - {}", e);
            }
        }
    }

    fn is_poisoned(&self) -> bool {
        self.handlers.is_poisoned()
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
        if self.handlers.is_poisoned() {
            return Err(ErrorKind::Shutdown.into());
        }
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
    fn process_request(
        &mut self,
        request: Request,
        sink: mpsc::Sender<OutgoingMessage>,
    ) -> Result<()> {
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
