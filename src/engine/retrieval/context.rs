// retrieval/context — model budget + source-context assembly.

use super::{DocKind, RetrievalError, RetrievedDoc, MAX_COMPLETION_TOKENS};
use crate::engine::llm::Model;

const DEPTH_TOP_K: usize = 5;
const PROMPT_OVERHEAD_TOKENS: usize = 4096;
const MIN_SOURCE_WORDS: usize = 1000;
const TOKENS_TO_WORDS_NUMERATOR: usize = 3;
const TOKENS_TO_WORDS_DENOMINATOR: usize = 4;

pub fn compute_context_budget(model: &Model) -> Result<usize, RetrievalError> {
    compute_context_budget_from_tokens(model.as_str(), model.context_tokens())
}

pub fn compute_context_budget_from_tokens(
    model: &str,
    context_tokens: usize,
) -> Result<usize, RetrievalError> {
    let reserved_tokens = PROMPT_OVERHEAD_TOKENS + MAX_COMPLETION_TOKENS as usize;
    if context_tokens <= reserved_tokens {
        return Err(RetrievalError::ContextBudgetExhausted {
            model: model.to_string(),
            context_tokens,
            reserved_tokens,
        });
    }

    let source_words = ((context_tokens - reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR)
        / TOKENS_TO_WORDS_DENOMINATOR;

    if source_words < MIN_SOURCE_WORDS {
        return Err(RetrievalError::ContextBudgetExhausted {
            model: model.to_string(),
            context_tokens,
            reserved_tokens,
        });
    }

    Ok(source_words)
}

/// Assemble LLM context from retrieved documents (leaves and branches).
/// Returns (context_string, consulted_count).
pub fn assemble_context(docs: &[RetrievedDoc], source_word_budget: usize) -> (String, usize) {
    let mut context = String::new();
    let mut word_budget = source_word_budget;
    let mut consulted = 0;

    // Breadth tier: all retrieved docs get summary context
    context.push_str("## Available sources\n\n");
    for doc in docs {
        // Branches are synthesized concept pages with no URL; leaves are raw
        // sources collected from a URL. The label tells the LLM which is which.
        let origin = match doc.kind {
            DocKind::Leaf => format!("({})", doc.url),
            DocKind::Branch => "(branch)".to_string(),
        };
        let entry = format!(
            "- [[{}]] — {} {}
  Summary: {}\n\n",
            doc.slug, doc.title, origin, doc.summary
        );
        let words = entry.split_whitespace().count();
        if words > word_budget {
            break;
        }
        context.push_str(&entry);
        word_budget = word_budget.saturating_sub(words);
    }

    // Depth tier: top-k get full body
    let depth_count = docs.len().min(DEPTH_TOP_K);
    if depth_count > 0 {
        context.push_str("## Full source content\n\n");
    }
    for doc in docs.iter().take(depth_count) {
        let body_words: Vec<&str> = doc.body.split_whitespace().collect();
        let usable_words = body_words.len().min(word_budget);
        if usable_words == 0 {
            break;
        }
        let truncated_body: String = body_words[..usable_words].join(" ");

        let entry = format!(
            "### [[{}]] — {}\n\n{}\n\n",
            doc.slug, doc.title, truncated_body
        );
        let entry_words = entry.split_whitespace().count();
        context.push_str(&entry);
        word_budget = word_budget.saturating_sub(entry_words);
        consulted += 1;
    }

    (context, consulted)
}

#[cfg(test)]
#[path = "../../tests/engine_retrieval/context_tests.rs"]
mod tests;
