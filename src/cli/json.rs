use serde::Serialize;
use serde_json::{json, Value};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonError {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl JsonError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: json!({}),
        }
    }

    pub fn with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: object_or_empty(details),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonWarning {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl JsonWarning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: json!({}),
        }
    }

    pub fn with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: object_or_empty(details),
        }
    }
}

#[derive(Serialize)]
struct SuccessEnvelope<'a, T> {
    schema_version: u32,
    ok: bool,
    command: &'a str,
    data: T,
    warnings: Vec<JsonWarning>,
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    schema_version: u32,
    ok: bool,
    command: &'a str,
    error: JsonError,
    warnings: Vec<JsonWarning>,
}

pub fn success_string<T: Serialize>(
    command: &str,
    data: T,
    warnings: Vec<JsonWarning>,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&SuccessEnvelope {
        schema_version: SCHEMA_VERSION,
        ok: true,
        command,
        data,
        warnings,
    })
}

pub fn error_string(
    command: &str,
    error: JsonError,
    warnings: Vec<JsonWarning>,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&ErrorEnvelope {
        schema_version: SCHEMA_VERSION,
        ok: false,
        command,
        error,
        warnings,
    })
}

fn object_or_empty(value: Value) -> Value {
    if value.is_object() {
        value
    } else {
        json!({})
    }
}

#[cfg(test)]
#[path = "../tests/cli_json_tests.rs"]
mod tests;
