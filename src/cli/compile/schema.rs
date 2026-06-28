// ── compile response schemas (derived from Deserialize structs) ───────────────

use serde_json::Value;

use super::parse::{CompileResponse, IncrementalCompileResponse};

pub(super) fn compile_response_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(CompileResponse)).unwrap()
}

pub(super) fn incremental_compile_response_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(IncrementalCompileResponse)).unwrap()
}
