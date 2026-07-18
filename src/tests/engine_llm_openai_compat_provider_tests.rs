use super::*;

#[test]
fn custom_constructor_appends_chat_completions() {
    let provider = OpenAiCompatProvider::custom("sk-test", "https://api.example.com/v1");
    assert_eq!(
        provider.base_url,
        "https://api.example.com/v1/chat/completions"
    );
}

#[test]
fn custom_constructor_trims_trailing_slash() {
    let provider = OpenAiCompatProvider::custom("sk-test", "https://api.example.com/v1/");
    assert_eq!(
        provider.base_url,
        "https://api.example.com/v1/chat/completions"
    );
}
