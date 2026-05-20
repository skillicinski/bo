// ── compile response schemas ──────────────────────────────────────────────────

use serde_json::{json, Value};

pub(super) fn incremental_compile_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "updated_branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "slug": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["slug", "title", "body", "leaves"],
                    "additionalProperties": false
                }
            },
            "new_branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["title", "body", "leaves"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["updated_branches", "new_branches"],
        "additionalProperties": false
    })
}

pub(super) fn compile_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Human-readable concept name"
                        },
                        "body": {
                            "type": "string",
                            "description": "Markdown body describing the concept across the collection"
                        },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filenames (with .md) of leaves this concept appears in"
                        }
                    },
                    "required": ["title", "body", "leaves"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["branches"],
        "additionalProperties": false
    })
}
