use hyper::Body;
use hyper::client::{Client, Connect, HttpConnector};
use std::io;
use tokio_core::reactor::Handle;

/// Trait for types able to produce Hyper `Client`s for use in `HttpTransport`.
pub trait ClientCreator: Send + 'static {
    /// The connector type inside the `Client` created by this type.
    type Connect: Connect;

    /// The error emitted by this type in case creating the `Client` failed.
    type Error: ::std::error::Error + Send;

    /// Tries to create a Hyper `Client` based on the given Tokio `Handle`.
    fn create(&self, handle: &Handle) -> Result<Client<Self::Connect, Body>, Self::Error>;
}

/// Default `Client` creator that defaults to creating a standard `Client` with just
/// `hyper::Client::new(handle)`.
#[derive(Debug, Default)]
pub struct DefaultClient;

impl ClientCreator for DefaultClient {
    type Connect = HttpConnector;
    type Error = io::Error;

    fn create(&self, handle: &Handle) -> Result<Client<HttpConnector, Body>, io::Error> {
        Ok(Client::new(handle))
    }
}

impl<C, E, F> ClientCreator for F
where
    C: Connect,
    E: ::std::error::Error + Send,
    F: Fn(&Handle) -> Result<Client<C, Body>, E>,
    F: Send + 'static,
{
    type Connect = C;
    type Error = E;

    fn create(&self, handle: &Handle) -> Result<Client<C, Body>, E> {
        (self)(handle)
    }
}


#[cfg(feature = "tls")]
mod tls {
    use super::*;
    use hyper_tls::HttpsConnector;
    use native_tls::Error;

    /// Number of threads in the thread pool doing DNS resolutions.
    /// Since DNS is resolved via blocking syscall they must be run on separate threads.
    static DNS_THREADS: usize = 2;

    /// Default `Client` creator for TLS enabled clients. Creates a Hyper `Client` based on
    /// `hyper_tls::HttpsConnector`.
    #[derive(Debug, Default)]
    pub struct DefaultTlsClient;

    impl ClientCreator for DefaultTlsClient {
        type Connect = HttpsConnector<HttpConnector>;
        type Error = Error;

        fn create(
            &self,
            handle: &Handle,
        ) -> Result<Client<HttpsConnector<HttpConnector>, Body>, Error> {
            let connector = HttpsConnector::new(DNS_THREADS, handle)?;
            let client = Client::configure().connector(connector).build(handle);
            Ok(client)
        }
    }
}

#[cfg(feature = "tls")]
pub use self::tls::*;
