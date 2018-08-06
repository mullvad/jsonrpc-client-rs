// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

jsonrpc_client!(
    /// Just an example RPC client to showcase how to use the `jsonrpc_client` macro and what
    /// the resulting structs look like.
    pub struct ExampleRpcClient {
        /// A method without any arguments and with no return value. Can still of course have
        /// lots of side effects on the server where it executes.
        pub fn nullary(&mut self) -> Future<()>;

        /// Send a string to the server and it will presumably echo it back.
        pub fn echo(&mut self, input: &str) -> Future<String>;

        /// Example RPC method named "concat" that takes a `String` and an unsigned integer and
        /// returns a `String`. From the name one could guess it will concatenate the two
        /// arguments. But that of course depends on the server where this call is sent.
        pub fn concat(&mut self, arg0: String, arg1: u64) -> Future<String>;
    }
);
