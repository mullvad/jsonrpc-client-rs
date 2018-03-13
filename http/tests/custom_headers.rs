extern crate futures;
extern crate hyper;
extern crate jsonrpc_client_core;
extern crate jsonrpc_client_http;
extern crate tokio_core;
extern crate tokio_service;

use std::{io, thread};
use std::sync::{Arc, Mutex};

use futures::future::{Future, FutureResult, IntoFuture};
use futures::sync::oneshot::{self, Sender};
use hyper::{Headers, Request, Response, StatusCode};
use hyper::header::{ContentLength, ContentType, Host};
use hyper::server::Http;
use tokio_core::reactor::Core;
use tokio_service::{NewService, Service};

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
    C: FnOnce(&Headers) -> bool + Send + 'static,
{
    let (tx, rx) = oneshot::channel();

    let (_server, port) = spawn_server(move |request| {
        let test_result = check_headers(request.headers());

        tx.send(test_result).unwrap();
    });

    let transport = HttpTransport::new().unwrap();
    let uri = format!("http://localhost:{}", port);
    let mut transport_handle = transport.handle(&uri).unwrap();

    set_headers(&mut transport_handle);

    let mut reactor = Core::new().unwrap();
    let send_and_check = transport_handle
        .send(Vec::new())
        .map_err(|_| ())
        .and_then(|_| rx.map_err(|_| ()));
    let check_passed = reactor.run(send_and_check).unwrap();

    assert!(check_passed);
}

pub struct CheckService<F> {
    check: Arc<Mutex<Option<F>>>,
}

impl<F> CheckService<F>
where
    F: FnOnce(Request),
{
    pub fn new(check: F) -> Self {
        CheckService {
            check: Arc::new(Mutex::new(Some(check))),
        }
    }
}

impl<F> Service for CheckService<F>
where
    F: FnOnce(Request),
{
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn call(&self, request: Request) -> Self::Future {
        let check = self.check.lock().unwrap().take().unwrap();

        check(request);

        Ok(Response::new().with_status(StatusCode::Ok)).into_future()
    }
}

pub struct ServerShutdownFlag(Option<Sender<()>>);

impl Drop for ServerShutdownFlag {
    fn drop(&mut self) {
        self.0.take().unwrap().send(()).unwrap();
    }
}

pub struct OneNewService<S>(Arc<Mutex<Option<S>>>);

impl<S: Service> OneNewService<S> {
    pub fn new(service: S) -> Self {
        OneNewService(Arc::new(Mutex::new(Some(service))))
    }
}

impl<S: Service> NewService for OneNewService<S> {
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Instance = S;

    fn new_service(&self) -> io::Result<Self::Instance> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "service already created"))
    }
}

fn spawn_server<F>(check: F) -> (ServerShutdownFlag, u16)
where
    F: FnOnce(Request) + Send + 'static,
{
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (started_tx, started_rx) = oneshot::channel();

    thread::spawn(move || {
        let service = CheckService::new(check);
        let address = "127.0.0.1:0".parse().unwrap();
        let server = Http::new()
            .bind(&address, OneNewService::new(service))
            .unwrap();
        let port = server.local_addr().unwrap().port();

        started_tx.send(port).unwrap();
        server.run_until(shutdown_rx.map_err(|_| ())).unwrap();
    });

    let server_shutdown_flag = ServerShutdownFlag(Some(shutdown_tx));
    let server_port = started_rx.wait().unwrap();

    (server_shutdown_flag, server_port)
}
