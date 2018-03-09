// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/// The main macro of this crate. Generates JSON-RPC 2.0 client structs with automatic serialization
/// and deserialization. Method calls get correct types automatically.
#[macro_export]
macro_rules! jsonrpc_client {
    (
        $(#[$struct_attr:meta])*
        pub struct $struct_name:ident {$(
            $(#[$attr:meta])*
            pub fn $method:ident(&mut $selff:ident $(, $arg_name:ident: $arg_ty:ty)*)
                -> RpcRequest<$return_ty:ty>;
        )*}
    ) => (
        $(#[$struct_attr])*
        pub struct $struct_name<T: $crate::Transport> {
            transport: T,
            timeout: Option<::std::time::Duration>,
        }

        impl<T: $crate::Transport> $struct_name<T> {
            /// Creates a new RPC client backed by the given transport implementation.
            pub fn new(transport: T) -> Self {
                $struct_name {
                    transport,
                    timeout: None,
                }
            }

            #[allow(dead_code)]
            /// Configures the timeout for remote procedure calls.
            pub fn set_timeout(&mut self, timeout: Option<::std::time::Duration>) {
                self.timeout = timeout;
            }

            $(
                $(#[$attr])*
                pub fn $method(&mut $selff $(, $arg_name: $arg_ty)*)
                    -> $crate::RpcRequest<$return_ty, T::Future>
                {
                    let method = String::from(stringify!($method));
                    let params = expand_params!($($arg_name,)*);
                    let timeout = $selff.timeout.clone();
                    $crate::call_method(&mut $selff.transport, method, params, timeout)
                }
            )*
        }
    )
}

/// Expands a variable list of parameters into its serializable form. Is needed to make the params
/// of a nullary method equal to `[]` instead of `()` and thus make sure it serializes to `[]`
/// instead of `null`.
#[doc(hidden)]
#[macro_export]
macro_rules! expand_params {
    () => ([] as [(); 0]);
    ($($arg_name:ident,)+) => (($($arg_name,)+))
}
