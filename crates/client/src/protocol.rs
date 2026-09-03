//! One response line to one typed response, with no I/O: the function the
//! fuzz target hits.

use grid_wire::errors::WireError;
use serde_json::Value;

use crate::error::{Protocol, RpcError};

/// One decoded response line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The `id` member as sent. Matching it against the outstanding request
    /// is the caller's job: this function sees bytes, not a connection.
    pub id: Value,
    /// Exactly one of `result` and `error`.
    pub body: Body,
}

/// The one body a response carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// The `result` member.
    Result(Value),
    /// The `error` member.
    Error(RpcError),
}

/// Decode one newline-terminated response line.
///
/// Total over arbitrary bytes: every malformed input is a [`Protocol`]
/// variant naming what was wrong, and nothing panics. Keys other than the
/// four JSON-RPC 2.0 names are ignored, so an additive envelope change is
/// not a fault.
///
/// # Errors
/// [`Protocol::Truncated`] when the line has no trailing newline;
/// [`Protocol::NotUtf8`], [`Protocol::NotJson`], [`Protocol::NotObject`] for
/// bytes that are not a JSON object; [`Protocol::BadJsonrpc`] when `jsonrpc`
/// is not `"2.0"`; [`Protocol::MissingId`] when there is no `id`;
/// [`Protocol::BothBodies`] and [`Protocol::NoBody`] when `result` and
/// `error` are not exactly one; [`Protocol::BadError`] when `error` is not
/// `{code: i32, message: string}`.
pub fn decode_line(line: &[u8]) -> Result<Response, Protocol> {
    let Some(body) = line.strip_suffix(b"\n") else {
        return Err(Protocol::Truncated);
    };
    let text = std::str::from_utf8(body).map_err(|_| Protocol::NotUtf8)?;
    let value: Value = serde_json::from_str(text).map_err(|_| Protocol::NotJson)?;
    let Value::Object(mut fields) = value else {
        return Err(Protocol::NotObject);
    };
    if fields.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(Protocol::BadJsonrpc(fields.remove("jsonrpc")));
    }
    let id = fields.remove("id").ok_or(Protocol::MissingId)?;
    let body = match (fields.remove("result"), fields.remove("error")) {
        (Some(_), Some(_)) => return Err(Protocol::BothBodies),
        (None, None) => return Err(Protocol::NoBody),
        (Some(result), None) => Body::Result(result),
        (None, Some(error)) => Body::Error(rpc_error(error)?),
    };
    Ok(Response { id, body })
}

fn rpc_error(error: Value) -> Result<RpcError, Protocol> {
    let Value::Object(mut fields) = error else {
        return Err(Protocol::BadError);
    };
    let code = fields
        .get("code")
        .and_then(Value::as_i64)
        .and_then(|c| i32::try_from(c).ok())
        .ok_or(Protocol::BadError)?;
    let Some(Value::String(message)) = fields.remove("message") else {
        return Err(Protocol::BadError);
    };
    Ok(RpcError {
        code,
        wire: WireError::from_code(code),
        message,
        data: fields.remove("data"),
    })
}
