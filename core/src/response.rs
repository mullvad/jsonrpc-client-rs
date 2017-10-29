// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use {ErrorKind, Result, ResultExt};
use jsonrpc_core::types::{Id, Version, Output};
use serde;
use serde_json;

/// Parses a binary response into json, extracts the "result" field and tries to deserialize that
/// to the desired type.
pub fn parse<R>(response_raw: &[u8], expected_id: Id) -> Result<R>
where
    R: serde::de::DeserializeOwned,
{
    let response: Output = serde_json::from_slice(response_raw)
        .chain_err(|| ErrorKind::ResponseError("Not valid json"))?;
    match response {
        Output::Success(success) => {
            check_response(success.jsonrpc, success.id, expected_id)?;
            debug!("Received json result: {}", success.result);
            serde_json::from_value::<R>(success.result)
                .chain_err(|| ErrorKind::ResponseError("Not valid for target type"))
        },
        Output::Failure(failure) => {
            check_response(failure.jsonrpc, failure.id, expected_id)?;
            Err(ErrorKind::JsonRpcError(failure.error).into())
        },
    }
}

/// Validate if response is a valid JSON-RPC 2.0 response object with the correct Id.
fn check_response(version: Option<Version>, id: Id, expected_id: Id) -> Result<()> {
    ensure!(
        version == Some(Version::V2),
        ErrorKind::ResponseError("Not JSON-RPC 2.0 compatible")
    );
    ensure!(
        id == expected_id,
        ErrorKind::ResponseError("Response id not equal to request id")
    );
    Ok(())
}
