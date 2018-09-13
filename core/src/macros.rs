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
                -> Future<$return_ty:ty>;
        )*}
    ) => (
        $(#[$struct_attr])*
        pub struct $struct_name {
            client: $crate::ClientHandle,
        }

        impl $struct_name {
            /// Creates a new RPC client backed by the given transport implementation.
            pub fn new(client: $crate::ClientHandle) -> Self {
                $struct_name { client }
            }

            $(
                $(#[$attr])*
                pub fn $method(&mut $selff $(, $arg_name: $arg_ty)*)
                    -> impl $crate::Future<Item = $return_ty, Error = $crate::Error> + 'static
                {
                    let method = String::from(stringify!($method));
                    let raw_params = expand_params!($($arg_name,)*);
                    let params = $crate::serialize_parameters(&raw_params);
                    let (tx, rx) = $crate::oneshot::channel();
                    let client_call = params.map(|p| $crate::OutgoingMessaage::RpcCall(method, p, tx));
                    $selff.client.send_client_call(client_call, rx)
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
