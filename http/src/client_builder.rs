use hyper::Body;
use hyper::client::{Client, Connect, HttpConnector};
use std::io;
use tokio_core::reactor::Handle;

/// Trait for types able to produce Hyper `Client`s for use in `HttpTransport`.
pub trait ClientBuilder: Send + 'static {
    /// The connector type inside the `Client` created by this builder.
    type Connect: Connect;

    /// The error emitted by this builder in case creating the `Client` failed.
    type Error: ::std::error::Error + Send;

    /// Tries to create a Hyper `Client` based on the given Tokio `Handle`.
    fn build(&self, handle: &Handle) -> Result<Client<Self::Connect, Body>, Self::Error>;
}

/// Default `Client` builder that defaults to creating a standard `Client` with just
/// `hyper::Client::new(handle)`.
#[derive(Default)]
pub struct DefaultClientBuilder;

impl ClientBuilder for DefaultClientBuilder {
    type Connect = HttpConnector;
    type Error = io::Error;

    fn build(&self, handle: &Handle) -> Result<Client<HttpConnector, Body>, io::Error> {
        Ok(Client::new(handle))
    }
}

impl<C, E, F> ClientBuilder for F
where
    C: Connect,
    E: ::std::error::Error + Send,
    F: Fn(&Handle) -> Result<Client<C, Body>, E>,
    F: Send + 'static,
{
    type Connect = C;
    type Error = E;

    fn build(&self, handle: &Handle) -> Result<Client<C, Body>, E> {
        (self)(handle)
    }
}


#[cfg(feature = "tls")]
mod tls {
    use super::*;
    use hyper_tls::HttpsConnector;
    use native_tls::Error;

    /// Default `Client` builder for TLS enabled clients. Creates a Hyper `Client` based on
    /// `hyper_tls::HttpsConnector`.
    #[derive(Default)]
    pub struct DefaultTlsClientBuilder;

    impl ClientBuilder for DefaultTlsClientBuilder {
        type Connect = HttpsConnector<HttpConnector>;
        type Error = Error;

        fn build(
            &self,
            handle: &Handle,
        ) -> Result<Client<HttpsConnector<HttpConnector>, Body>, Error> {
            let connector = HttpsConnector::new(2, handle)?;
            let client = Client::configure().connector(connector).build(handle);
            Ok(client)
        }
    }
}

#[cfg(feature = "tls")]
pub use self::tls::*;
