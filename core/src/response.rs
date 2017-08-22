// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use {Error, ErrorKind, Result, ResultExt};
use jsonrpc_core;
use serde;
use serde_json;

/// Parses a binary response into json, extracts the "result" field and tries to deserialize that
/// to the desired type.
pub fn parse<R>(response: &[u8], expected_id: u64) -> Result<R>
where
    for<'de> R: serde::Deserialize<'de>,
{
    let response_json = deserialize_response(response)?;
    let result_json = check_response_and_get_result(response_json, expected_id)?;
    debug!("Received json result: {}", result_json);
    serde_json::from_value::<R>(result_json)
        .chain_err(|| ErrorKind::ResponseError("Not valid for target type"))
}

fn deserialize_response(response: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(response).chain_err(|| ErrorKind::ResponseError("Not valid json"))
}

/// Validate if response is a valid JSON-RPC 2.0 response object. If it is, it returns the
/// content of the "result" field of that object.
fn check_response_and_get_result(
    mut response: serde_json::Value,
    expected_id: u64,
) -> Result<serde_json::Value> {
    let response_map = response.as_object_mut().ok_or_else(|| {
        Error::from_kind(ErrorKind::ResponseError("Not a json object"))
    })?;
    ensure!(
        response_map.remove("jsonrpc") == Some(serde_json::Value::String("2.0".into())),
        ErrorKind::ResponseError("Not JSON-RPC 2.0 compatible")
    );
    ensure!(
        response_map.remove("id") == Some(expected_id.into()),
        ErrorKind::ResponseError("Response id not equal to request id")
    );
    if let Some(error_json) = response_map.remove("error") {
        let error = json_value_to_rpc_error(error_json)
            .chain_err(|| ErrorKind::ResponseError("Malformed error object"))?;
        bail!(ErrorKind::JsonRpcError(error));
    }
    response_map
        .remove("result")
        .ok_or_else(|| ErrorKind::ResponseError("No \"result\" field").into())
}

/// Parses a `serde_json::Value` as a JSON-RPC 2.0 error.
fn json_value_to_rpc_error(
    mut error_json: serde_json::Value,
) -> Result<jsonrpc_core::types::error::Error> {
    let map = error_json
        .as_object_mut()
        .ok_or(ErrorKind::ResponseError("Error is not a json object"))?;

    let code = map.remove("code")
        .ok_or_else(|| {
            ErrorKind::ResponseError("Error has no code field").into()
        })
        .and_then(|code| {
            serde_json::from_value(code)
                .chain_err(|| ErrorKind::ResponseError("Malformed code field in error"))
        })?;
    let message = map.get("message")
        .and_then(|v| v.as_str())
        .ok_or(ErrorKind::ResponseError(
            "Error has no message field of string type",
        ))?
        .to_owned();

    Ok(jsonrpc_core::types::error::Error {
        code: code,
        message: message,
        data: map.remove("data"),
    })
}
