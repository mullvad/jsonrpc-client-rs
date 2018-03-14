extern crate futures;
extern crate hyper;
extern crate jsonrpc_client_core;
extern crate jsonrpc_client_http;
extern crate tokio_core;
extern crate tokio_service;

use std::fmt::Display;
use std::sync::Arc;
use std::thread;

use futures::Stream;
use futures::future::{Future, FutureResult, IntoFuture};
use futures::sync::{mpsc, oneshot};
use hyper::{Headers, Request, Response, StatusCode};
use hyper::header::{ContentLength, ContentType, Header, Host};
use hyper::server::Http;
use tokio_core::reactor::Core;
use tokio_service::Service;

use jsonrpc_client_core::Transport;
use jsonrpc_client_http::{HttpHandle, HttpTransport};

#[test]
fn set_host_header() {
    let hostname = "dummy.url";
    let port = Some(8081);

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(Host::new(hostname, port));
    };

    let check = move |headers: &Headers| {
        if let Some(host) = headers.get::<Host>() {
            host.hostname() == hostname && host.port() == port
        } else {
            false
        }
    };

    test_custom_headers(set, check);
}

#[test]
fn set_host_header_twice() {
    let hostname = "dummy.url";
    let port = Some(8081);

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(Host::new("should.be.overwritten.url", None));
        transport.set_header(Host::new(hostname, port));
    };

    let check = move |headers: &Headers| {
        if let Some(host) = headers.get::<Host>() {
            host.hostname() == hostname && host.port() == port
        } else {
            false
        }
    };

    test_custom_headers(set, check);
}

#[test]
fn set_content_type() {
    let content_type = ContentType::xml();
    let expected_content_type = content_type.clone();

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(content_type);
    };

    let check = move |headers: &Headers| {
        if let Some(content_type) = headers.get::<ContentType>() {
            *content_type == expected_content_type
        } else {
            false
        }
    };

    test_custom_headers(set, check);
}

#[test]
fn set_content_length() {
    let fake_content_length = ContentLength(100);

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(fake_content_length);
    };

    let check = move |headers: &Headers| {
        if let Some(content_length) = headers.get::<ContentLength>() {
            *content_length == fake_content_length
        } else {
            false
        }
    };

    test_custom_headers(set, check);
}

fn test_custom_headers<S, C>(set_headers: S, check_headers: C)
where
    S: FnOnce(&mut HttpHandle),
    C: Fn(&Headers) -> bool + Send + Sync + 'static,
{
    let (check_service, check_results) = CheckService::new(check_headers);
    let check_result = check_results
        .into_future()
        .map(|(result, _)| result.unwrap_or(false))
        .map_err(|_| "failure to receive test result from other thread".to_string());

    let (_server, port) = spawn_server(check_service);

    let transport = HttpTransport::new().unwrap();
    let uri = format!("http://127.0.0.1:{}", port);
    let mut transport_handle = transport.handle(&uri).unwrap();

    set_headers(&mut transport_handle);

    let mut reactor = Core::new().unwrap();
    let send_and_check = transport_handle
        .send(Vec::new())
        .map_err(|err| err.to_string())
        .and_then(|_| check_result);
    let check_passed = reactor.run(send_and_check).unwrap();

    assert!(check_passed);
}

pub struct CheckService<F>
where
    F: Fn(&Headers) -> bool,
{
    check: Arc<F>,
    sender: mpsc::Sender<bool>,
}

impl<F> CheckService<F>
where
    F: Fn(&Headers) -> bool,
{
    pub fn new(check: F) -> (Self, mpsc::Receiver<bool>) {
        let (sender, receiver) = mpsc::channel(0);

        let check_service = CheckService {
            check: Arc::new(check),
            sender,
        };

        (check_service, receiver)
    }
}

impl<F> Clone for CheckService<F>
where
    F: Fn(&Headers) -> bool,
{
    fn clone(&self) -> Self {
        CheckService {
            check: self.check.clone(),
            sender: self.sender.clone(),
        }
    }
}

impl<F> Service for CheckService<F>
where
    F: Fn(&Headers) -> bool,
{
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn call(&self, request: Request) -> Self::Future {
        let mut sender = self.sender.clone();
        let _ = sender.try_send((self.check)(request.headers()));

        Ok(Response::new().with_status(StatusCode::Ok)).into_future()
    }
}

pub struct ServerHandle(oneshot::Sender<()>);

fn spawn_server<F>(service: CheckService<F>) -> (ServerHandle, u16)
where
    F: Fn(&Headers) -> bool + Send + Sync + 'static,
{
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (started_tx, started_rx) = oneshot::channel();

    thread::spawn(move || {
        let address = "127.0.0.1:0".parse().unwrap();
        let server = Http::new()
            .bind(&address, move || Ok(service.clone()))
            .unwrap();
        let port = server.local_addr().unwrap().port();

        started_tx.send(port).unwrap();
        server.run_until(shutdown_rx.then(|_| Ok(()))).unwrap();
    });

    let server_handle = ServerHandle(shutdown_tx);
    let server_port = started_rx.wait().unwrap();

    (server_handle, server_port)
}
