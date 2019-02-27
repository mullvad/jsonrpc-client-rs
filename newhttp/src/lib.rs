use futures::{
    future,
    sync::{mpsc, oneshot},
    Future, Poll, Sink, Stream,
};
pub use hyper;
use hyper::{client::connect::Connect, Body, Client, Request, Uri};
use std::time::Duration;

mod timeout;

pub struct HttpTransport {
    future: Box<dyn Future<Item = (), Error = ()> + Send>,
    handle_tx: mpsc::UnboundedSender<(Request<Body>, oneshot::Sender<Result<Vec<u8>, Error>>)>,
}

impl HttpTransport {
    pub fn new<C: Connect + 'static>(
        client: Client<C, hyper::Body>,
        timeout: Option<Duration>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded();
        Self {
            future: Box::new(Self::create_request_processing_future(rx, client, timeout)),
            handle_tx: tx,
        }
    }

    pub fn handle(&self, uri: Uri) -> HttpHandle {
        HttpHandle {
            request_tx: self.handle_tx.clone(),
            uri,
        }
    }

    fn create_request_processing_future<C: Connect + 'static>(
        request_rx: mpsc::UnboundedReceiver<(
            Request<Body>,
            oneshot::Sender<Result<Vec<u8>, Error>>,
        )>,
        client: Client<C, hyper::Body>,
        timeout: Option<Duration>,
    ) -> impl Future<Item = (), Error = ()> + Send {
        request_rx.for_each(move |(request, response_tx)| {
            log::trace!("Sending request to {}", request.uri());
            let request = client.request(request);
            timeout::OptionalTimeout::new(request, timeout)
                .map_err(|e| {
                    if e.is_inner() {
                        Error::HyperError(e.into_inner().unwrap())
                    } else if e.is_timer() {
                        Error::TimerError(e.into_timer().unwrap())
                    } else {
                        Error::TimeoutError
                    }
                })
                .and_then(|response| {
                    let status = response.status();
                    log::debug!("Got response with status {}", status);
                    if status == hyper::StatusCode::OK {
                        Ok(response)
                    } else {
                        Err(Error::HttpResponseError(status))
                    }
                })
                .and_then(|response| {
                    response
                        .into_body()
                        .concat2()
                        .map_err(|e| Error::HyperError(e))
                })
                .map(|response_body| response_body.to_vec())
                .then(move |response_result| {
                    if response_tx.send(response_result).is_err() {
                        log::warn!("Unable to send response back to caller");
                    }
                    Ok(()) as Result<(), ()>
                })
        })
    }
}

impl Future for HttpTransport {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.future.poll()
    }
}

pub struct HttpHandle {
    request_tx: mpsc::UnboundedSender<(Request<Body>, oneshot::Sender<Result<Vec<u8>, Error>>)>,
    uri: Uri,
}

impl HttpHandle {
    /// Creates a Hyper POST request with JSON content type and the given body data.
    fn create_request(&self, body: Vec<u8>) -> Request<Body> {
        let mut request = hyper::Request::post(self.uri.clone());
        request.header(hyper::header::CONTENT_TYPE, "application/json");
        request.header(hyper::header::CONTENT_LENGTH, body.len());
        request
            .body(Body::from(body))
            .expect("Failed to create HTTP request")
    }

    fn send(&self, json_data: Vec<u8>) -> impl Future<Item = Vec<u8>, Error = Error> + Send {
        let request = self.create_request(json_data);
        let (response_tx, response_rx) = oneshot::channel();
        future::result(self.request_tx.unbounded_send((request, response_tx)))
            .map_err(|_e| Error::TransportClosedError)
            .and_then(move |_| response_rx.map_err(|_e| Error::TransportClosedError))
            .flatten()
    }
}

impl jsonrpc_client_core::Transport for HttpHandle {
    type Error = Error;
    type Sink = Box<dyn Sink<SinkItem = String, SinkError = Self::Error> + Send>;
    type Stream = Box<dyn Stream<Item = String, Error = Self::Error> + Send>;

    fn io_pair(self) -> (Self::Sink, Self::Stream) {
        let (tx, rx) = mpsc::channel(0);
        let sink = tx
            .sink_map_err(|_| Error::TransportClosedError)
            .with(move |json: String| self.send(json.into_bytes()));
        let stream = rx
            .map_err(|_| Error::TransportClosedError)
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| Error::ParseBodyError(e)));
        (Box::new(sink), Box::new(stream))
    }
}

#[derive(Debug, err_derive::Error)]
pub enum Error {
    #[error(display = "The HTTP transport processor has stopped")]
    TransportClosedError,

    #[error(display = "Error in HTTP client")]
    HyperError(#[error(cause)] hyper::Error),

    #[error(display = "Error in timeout timer")]
    TimerError(#[error(cause)] tokio_timer::Error),

    #[error(display = "Request timed out")]
    TimeoutError,

    #[error(display = "Unexpected HTTP status code in response: {}", _0)]
    HttpResponseError(hyper::StatusCode),

    #[error(display = "Response body is not valid UTF-8")]
    ParseBodyError(#[error(cause)] std::string::FromUtf8Error),
}
