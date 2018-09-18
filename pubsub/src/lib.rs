//! This crate adds support for subscriptions as defined in [here][1].
//!
//! [1]: https://github.com/ethereum/go-ethereum/wiki/RPC-PUB-SUB

extern crate futures;
extern crate jsonrpc_client_core;
extern crate serde;
extern crate serde_json;

#[macro_use]
extern crate serde_derive;
extern crate tokio;
#[macro_use]
extern crate log;

use futures::{sync::mpsc, Async, Future, Poll, Sink, Stream};


use jsonrpc_client_core::server::{types::Params, Handler, Handlers, Server};
use jsonrpc_client_core::{ClientHandle, Error, ErrorKind, ResultExt, Transport};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use tokio::executor::Executor;

#[derive(Debug, Deserialize)]
struct SubscriptionMessage {
    subscription: SubscriptionId,
    result: Value,
}

/// A stream of messages from a subscription.
#[derive(Debug)]
pub struct Subscription<T: serde::de::DeserializeOwned> {
    rx: mpsc::Receiver<Value>,
    id: Option<SubscriptionId>,
    handler_chan: mpsc::UnboundedSender<SubscriberMsg>,
    _marker: PhantomData<T>,
}


impl<T: serde::de::DeserializeOwned> Stream for Subscription<T> {
    type Item = T;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<T>, Error> {
        match self.rx.poll().map_err(|_: ()| ErrorKind::Shutdown)? {
            Async::Ready(Some(v)) => Ok(Async::Ready(Some(
                serde_json::from_value(v).chain_err(|| ErrorKind::DeserializeError)?,
            ))),
            Async::Ready(None) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl<T: serde::de::DeserializeOwned> Drop for Subscription<T> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self
                .handler_chan
                .unbounded_send(SubscriberMsg::RemoveSubscriber(id));
        }
    }
}

/// A subscriber creates new subscriptions.
#[derive(Debug)]
pub struct Subscriber<E: Executor + Clone + Send + 'static> {
    client_handle: ClientHandle,
    handlers: Handlers,
    notification_handlers: BTreeMap<String, mpsc::UnboundedSender<SubscriberMsg>>,
    executor: E,
}


impl<E: Executor + Clone + Send + 'static> Subscriber<E> {
    /// Constructs a new subscriber with the provided executor.
    pub fn new(executor: E, client_handle: ClientHandle, handlers: Handlers) -> Self {
        let notification_handlers = BTreeMap::new();
        Self {
            client_handle,
            handlers,
            notification_handlers,
            executor,
        }
    }

    /// Creates a new subscription with the given method names and parameters. Parameters
    /// `sub_method` and `unsub_method` are only taken into account if this is the first time a
    /// subscription for `notification` has been created in the lifetime of this `Subscriber`.
    pub fn subscribe<T, P>(
        &mut self,
        sub_method: String,
        unsub_method: String,
        notification: String,
        buffer_size: usize,
        sub_parameters: P,
    ) -> impl Future<Item = Subscription<T>, Error = Error>
    where
        T: serde::de::DeserializeOwned + 'static,
        P: serde::Serialize + 'static,
    {
        // Get a channel to an existing notification handler or spawn a new one.
        let chan = self
            .notification_handlers
            .get(&notification)
            .filter(|c| c.is_closed())
            .map(Clone::clone)
            .unwrap_or_else(|| self.spawn_notification_handler(notification.clone(), unsub_method));


        let (sub_tx, sub_rx) = mpsc::channel(buffer_size);

        self.client_handle
            .call_method(sub_method, &sub_parameters)
            .and_then(move |id: SubscriptionId| {
                if let Err(_) =
                    chan.unbounded_send(SubscriberMsg::NewSubscriber(id.clone(), sub_tx))
                {
                    debug!(
                        "Notificaton handler for {} - {} already closed",
                        notification, id
                    );
                };
                Ok(Subscription {
                    rx: sub_rx,
                    id: Some(id),
                    handler_chan: chan.clone(),
                    _marker: PhantomData::<T>,
                })
            })
    }

    fn spawn_notification_handler(
        &mut self,
        notification: String,
        unsub_method: String,
    ) -> mpsc::UnboundedSender<SubscriberMsg> {
        let (notif_tx, notif_rx) = mpsc::unbounded();
        let handler_tx = notif_tx.clone();
        self.handlers.add(
            notification.clone(),
            Handler::Notification(Box::new(move |notification| {
                let tx = handler_tx.clone();
                if let Err(_) =
                    if let Some(msg) = params_to_subscription_message(notification.params) {
                        tx.unbounded_send(msg)
                    } else {
                        trace!(
                            "Received notification with invalid parameters for subscription - {}",
                            notification.method
                        );
                        Ok(())
                    } {
                    error!("Notification handler doesn't exist anymore");
                };
                Box::new(futures::future::ok(()))
            })),
        );

        let notification_handler = NotificationHandler::new(
            notification.clone(),
            self.handlers.clone(),
            self.client_handle.clone(),
            unsub_method,
            notif_rx,
        );

        if let Err(e) = self
            .executor
            .spawn(Box::new(notification_handler.map_err(|_| ())))
        {
            error!("Failed to spawn notification handler - {}", e);
        };

        self.notification_handlers
            .insert(notification.clone(), notif_tx.clone());

        notif_tx
    }
}

fn params_to_subscription_message(params: Option<Params>) -> Option<SubscriberMsg> {
    params
        .and_then(|p| p.parse().ok())
        .map(SubscriberMsg::NewMessage)
}


#[derive(Ord, PartialOrd, Eq, PartialEq, Clone, Debug, Deserialize)]
#[serde(untagged)]
enum SubscriptionId {
    Num(u64),
    String(String),
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SubscriptionId::Num(n) => write!(f, "{}", n),
            SubscriptionId::String(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Debug)]
enum SubscriberMsg {
    NewMessage(SubscriptionMessage),
    NewSubscriber(SubscriptionId, mpsc::Sender<Value>),
    RemoveSubscriber(SubscriptionId),
}

// A single notification can receive messages for different subscribers for the same notification.
struct NotificationHandler {
    notification: String,
    subscribers: BTreeMap<SubscriptionId, mpsc::Sender<Value>>,
    messages: mpsc::UnboundedReceiver<SubscriberMsg>,
    unsub_method: String,
    client_handle: ClientHandle,
    current_future: Option<Box<Future<Item = (), Error = ()> + Send>>,
    server_handlers: Handlers,
    should_shut_down: bool,
}

impl Drop for NotificationHandler {
    fn drop(&mut self) {
        self.server_handlers.remove(&self.notification);
    }
}

impl NotificationHandler {
    fn new(
        notification: String,
        server_handlers: Handlers,
        client_handle: ClientHandle,
        unsub_method: String,
        messages: mpsc::UnboundedReceiver<SubscriberMsg>,
    ) -> Self {
        Self {
            notification,
            messages,
            server_handlers,
            unsub_method,
            subscribers: BTreeMap::new(),
            client_handle,
            current_future: None,
            should_shut_down: false,
        }
    }

    fn handle_new_subscription(&mut self, id: SubscriptionId, chan: mpsc::Sender<Value>) {
        self.subscribers.insert(id, chan);
    }

    fn handle_removal(&mut self, id: SubscriptionId) {
        if let None = self.subscribers.remove(&id) {
            debug!("Removing non-existant subscriber - {}", &id);
        };

        let fut = self
            .client_handle
            .call_method(self.unsub_method.clone(), &[0u8; 0])
            .map(|_r: bool| ())
            .map_err(|e| trace!("Failed to unsubscribe - {}", e));

        self.should_shut_down = self.subscribers.len() < 1;
        self.current_future = Some(Box::new(fut));
    }

    fn handle_new_message(&mut self, id: SubscriptionId, message: Value) {
        match self.subscribers.get(&id) {
            Some(chan) => {
                let fut = chan
                    .clone()
                    .send(message)
                    .map_err(move |_| trace!("Subscriber already gone: {}", id))
                    .map(|_| ());

                self.current_future = Some(Box::new(fut));
            }
            None => trace!("Received message for non existant subscription - {}", id),
        }
    }

    fn ready_for_next_connection(&mut self) -> bool {
        match self.current_future.take() {
            None => true,
            Some(mut fut) => match fut.poll() {
                Ok(Async::NotReady) => false,
                _ => true,
            },
        }
    }
}

impl Future for NotificationHandler {
    type Item = ();
    type Error = ();
    fn poll(&mut self) -> Poll<(), ()> {
        while self.ready_for_next_connection() {
            match self.messages.poll()? {
                Async::NotReady => {
                    break;
                }
                Async::Ready(None) => {
                    return Ok(Async::Ready(()));
                }
                Async::Ready(Some(SubscriberMsg::NewMessage(msg))) => {
                    self.handle_new_message(msg.subscription, msg.result);
                }

                Async::Ready(Some(SubscriberMsg::NewSubscriber(id, chan))) => {
                    self.handle_new_subscription(id, chan);
                }

                Async::Ready(Some(SubscriberMsg::RemoveSubscriber(id))) => {
                    self.handle_removal(id);
                }
            }
        }

        if self.should_shut_down {
            trace!("shutting down notification handler");
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }
}

/// A trait for constructing the usual client handles with coupled `Subscriber` structs.
pub trait SubscriberTransport: Transport {
    /// Constructs a new client, client handle and a subscriber.
    fn subscriber_client<E: Executor + Clone + Send>(
        self,
        executor: E,
    ) -> (
        jsonrpc_client_core::Client<Self, Server>,
        ClientHandle,
        Subscriber<E>,
    );
}


/// Subscriber transport trait allows one to create a client future, a subscriber and a client
/// handle from a valid JSON-RPC transport.
impl<T: Transport> SubscriberTransport for T {
    /// Constructs a new client, client handle and a subscriber.
    fn subscriber_client<E: Executor + Clone + Send>(
        self,
        executor: E,
    ) -> (
        jsonrpc_client_core::Client<Self, Server>,
        ClientHandle,
        Subscriber<E>,
    ) {
        let server = Server::new();
        let handlers = server.get_handlers();
        let (client, client_handle) = self.with_server(server);
        let subscriber = Subscriber::new(executor, client_handle.clone(), handlers);
        (client, client_handle, subscriber)
    }
}

// TODO: once negative trait bounds are fully implemented, uncomment the lines below
// extern crate jsonrpc_client_http;
// impl !SubscriberTransport for jsonrpc_client_http::HttpHandle {}
