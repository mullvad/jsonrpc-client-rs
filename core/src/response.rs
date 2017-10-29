// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use {Error, ErrorKind, Result, ResultExt};
use jsonrpc_core;
use jsonrpc_core::types::Id;
use serde;
use serde_json::{self, Value as JsonValue};

/// Parses a binary response into json, extracts the "result" field and tries to deserialize that
/// to the desired type.
pub fn parse<R>(response: &[u8], expected_id: Id) -> Result<R>
where
    for<'de> R: serde::Deserialize<'de>,
{
    let response_json = deserialize_response(response)?;
    let result_json = check_response_and_get_result(response_json, expected_id)?;
    debug!("Received json result: {}", result_json);
    serde_json::from_value::<R>(result_json)
        .chain_err(|| ErrorKind::ResponseError("Not valid for target type"))
}

fn deserialize_response(response: &[u8]) -> Result<JsonValue> {
    serde_json::from_slice(response).chain_err(|| ErrorKind::ResponseError("Not valid json"))
}

/// Validate if response is a valid JSON-RPC 2.0 response object. If it is, it returns the
/// content of the "result" field of that object.
fn check_response_and_get_result(mut response: JsonValue, expected_id: Id) -> Result<JsonValue> {
    let response_map = response.as_object_mut().ok_or_else(|| {
        Error::from_kind(ErrorKind::ResponseError("Not a json object"))
    })?;
    ensure!(
        response_map.get("jsonrpc") == Some(&JsonValue::from("2.0")),
        ErrorKind::ResponseError("Not JSON-RPC 2.0 compatible")
    );
    ensure!(
        response_map.get("id") == serde_json::to_value(expected_id).ok().as_ref(),
        ErrorKind::ResponseError("Response id not equal to request id")
    );
    if let Some(error_json) = response_map.remove("error") {
        let error: jsonrpc_core::Error = serde_json::from_value(error_json)
            .chain_err(|| ErrorKind::ResponseError("Malformed error object"))?;
        bail!(ErrorKind::JsonRpcError(error));
    }
    response_map
        .remove("result")
        .ok_or_else(|| ErrorKind::ResponseError("No \"result\" field").into())
}
