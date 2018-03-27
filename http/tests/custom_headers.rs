extern crate futures;
extern crate hyper;
extern crate jsonrpc_client_core;
extern crate jsonrpc_client_http;
extern crate tokio_service;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use futures::future::{Future, FutureResult, IntoFuture};
use futures::sync::oneshot;
use hyper::{Request, Response, StatusCode};
use hyper::server::Http;
use jsonrpc_client_http::header::{ContentLength, ContentType, Host};
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

    let request = test_custom_headers(set);
    let host = request.headers().get::<Host>().expect("No Host");
    assert_eq!(host.hostname(), hostname);
    assert_eq!(host.port(), port);
}

#[test]
fn set_host_header_twice() {
    let hostname = "dummy.url";
    let port = Some(8081);

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(Host::new("should.be.overwritten.url", None));
        transport.set_header(Host::new(hostname, port));
    };


    let request = test_custom_headers(set);
    let host = request.headers().get::<Host>().expect("No Host");
    assert_eq!(host.hostname(), hostname);
    assert_eq!(host.port(), port);
}

#[test]
fn set_content_type() {
    let content_type = ContentType::xml();
    let expected_content_type = content_type.clone();

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(content_type);
    };


    let request = test_custom_headers(set);
    let content_type = request
        .headers()
        .get::<ContentType>()
        .expect("No ContentType");
    assert_eq!(*content_type, expected_content_type);
}

#[test]
fn set_content_length() {
    let fake_content_length = ContentLength(100);

    let set = move |transport: &mut HttpHandle| {
        transport.set_header(fake_content_length);
    };

    let request = test_custom_headers(set);
    let content_length = request
        .headers()
        .get::<ContentLength>()
        .expect("No ContentLength");
    assert_eq!(*content_length, fake_content_length);
}

fn test_custom_headers<S>(set_headers: S) -> Request
where
    S: FnOnce(&mut HttpHandle),
{
    let server = Server::spawn();

    let transport = HttpTransport::new().standalone().unwrap();
    let uri = format!("http://127.0.0.1:{}", server.port);
    let mut transport_handle = transport.handle(&uri).unwrap();

    set_headers(&mut transport_handle);

    transport_handle.send(Vec::new()).wait().unwrap();
    server
        .requests
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
}

#[derive(Clone)]
pub struct ForwardToChannel {
    sender: mpsc::Sender<Request>,
}

impl ForwardToChannel {
    pub fn new() -> (Self, mpsc::Receiver<Request>) {
        let (sender, receiver) = mpsc::channel();
        let service = ForwardToChannel { sender };

        (service, receiver)
    }
}

impl Service for ForwardToChannel {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn call(&self, request: Request) -> Self::Future {
        let _ = self.sender.send(request);

        Ok(Response::new().with_status(StatusCode::Ok)).into_future()
    }
}

pub struct Server {
    pub port: u16,
    pub requests: mpsc::Receiver<Request>,
    _shutdown_tx: oneshot::Sender<()>,
}

impl Server {
    fn spawn() -> Self {
        let (forward_service, requests) = ForwardToChannel::new();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (port_tx, port_rx) = oneshot::channel();

        thread::spawn(move || {
            let address = "127.0.0.1:0".parse().unwrap();
            let server = Http::new()
                .bind(&address, move || Ok(forward_service.clone()))
                .unwrap();
            let port = server.local_addr().unwrap().port();

            port_tx.send(port).unwrap();
            server.run_until(shutdown_rx.then(|_| Ok(()))).unwrap();
        });

        let port = port_rx.wait().unwrap();

        Self {
            port,
            requests,
            _shutdown_tx: shutdown_tx,
        }
    }
}
