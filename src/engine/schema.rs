// ── inline schema generation (no $ref/$defs/$schema) ─────────────────────────

use schemars::{r#gen::SchemaSettings, schema::RootSchema, JsonSchema};

/// Generate a JSON Schema with all subschemas inlined and no `$schema` key.
///
/// The default `schemars::schema_for!` emits `$schema`, `definitions`, and `$ref`
/// which Google's Gemini `responseSchema` rejects. This helper configures
/// schemars to inline everything — the resulting schema is accepted by all
/// three providers (OpenAI, DeepSeek, Google).
pub(crate) fn inline_schema_for<T: JsonSchema>() -> RootSchema {
    let settings = SchemaSettings::draft07().with(|s| {
        s.inline_subschemas = true;
        s.meta_schema = None;
    });
    let gen = settings.into_generator();
    gen.into_root_schema_for::<T>()
}
