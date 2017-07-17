// Copyright 2017 Amagicom AB.

// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


#[macro_use]
extern crate error_chain;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate jsonrpc_client_core;
extern crate tokio_core;

use futures::Stream;
use futures::future::Future;
use hyper::{Client, Request, StatusCode, Uri};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;
use tokio_core::reactor::{Core, Handle};

use jsonrpc_client_core::Transport;

use std::str::FromStr;

error_chain! {
    errors {
        TransportError
        HttpError(http_code: StatusCode) {
            description("Http error. Server did not return 200 OK")
            display("Http error. Status code {}", http_code)
        }
    }
    foreign_links {
        Hyper(hyper::Error);
        Uri(hyper::error::UriError);
    }
}

/// A [`hyper`](https://crates.io/crates/hyper) based HTTP transport implementation.
pub struct HttpTransport {
    core: Core,
    client: hyper::Client<HttpsConnector<HttpConnector>, hyper::Body>,
    uri: Uri,
}

impl HttpTransport {
    /// Tries to create a new HTTP transport instance from the given URI.
    pub fn new(uri: &str) -> Result<Self> {
        let uri = Uri::from_str(uri)?;
        let core = tokio_core::reactor::Core::new()
            .chain_err(|| ErrorKind::TransportError)?;
        let client = Self::create_client(core.handle())?;
        Ok(HttpTransport { core, client, uri })
    }

    fn create_client(handle: Handle) -> Result<Client<HttpsConnector<HttpConnector>, hyper::Body>> {
        let https_connector = HttpsConnector::new(1, &handle)
            .chain_err(|| ErrorKind::TransportError)?;
        Ok(
            Client::configure()
                .connector(https_connector)
                .build(&handle),
        )
    }

    fn create_request(&self, body: Vec<u8>) -> Request {
        let mut request = hyper::Request::new(hyper::Method::Post, self.uri.clone());
        request
            .headers_mut()
            .set(hyper::header::ContentType::json());
        request
            .headers_mut()
            .set(hyper::header::ContentLength(body.len() as u64));
        request.set_body(body);
        request
    }
}

impl Transport<Error> for HttpTransport {
    fn send(&mut self, json_data: &[u8]) -> Result<Vec<u8>> {
        let request = self.create_request(json_data.to_owned());

        let response_future = self.client
            .request(request)
            .map_err(|e| e.into())
            .and_then(|res: hyper::Response| {
                let result: Result<hyper::Response> = if res.status() != hyper::StatusCode::Ok {
                    Err(ErrorKind::HttpError(res.status()).into())
                } else {
                    Ok(res)
                };
                futures::future::result(result)
            })
            .and_then(|res: hyper::Response| {
                res.body().concat2().map_err(|e| e.into())
            });

        // Run the network main loop to process the future and get the response body.
        let json_response = self.core.run(response_future)?;
        Ok(json_response.to_vec())
    }
}
