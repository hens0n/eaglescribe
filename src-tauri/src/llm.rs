//! Local LLM client for Command Mode.
//!
//! Talks to an **on-machine** OpenAI-compatible HTTP API only
//! (Ollama, llama-server, LM Studio, etc.). Default is localhost.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// e.g. `http://127.0.0.1:11434/v1`
    pub base_url: String,
    /// Model id for the local server (e.g. `llama3.2`, `qwen2.5:3b`)
    pub model: String,
    /// Optional API key (Ollama ignores this; some local servers want a dummy value)
    pub api_key: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.2".into(),
            api_key: String::new(),
        }
    }
}

impl LlmConfig {
    pub fn is_configured(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.model.trim().is_empty()
    }

    fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{base}/chat/completions")
        } else if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/v1/chat/completions")
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Option<Vec<ChatChoice>>,
    error: Option<ChatErrorBody>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: Option<ChatMessageOut>,
}

#[derive(Debug, Deserialize)]
struct ChatMessageOut {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatErrorBody {
    message: Option<String>,
}

/// Rewrite or generate text using a local chat-completions endpoint.
pub fn complete(config: &LlmConfig, system: &str, user: &str) -> AppResult<String> {
    if !config.is_configured() {
        return Err(AppError::from(
            "LLM not configured. Set base URL and model (e.g. Ollama at http://127.0.0.1:11434/v1).",
        ));
    }

    let url = config.chat_url();
    let body = json!({
        "model": config.model,
        "temperature": 0.3,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "stream": false,
    });

    let mut req = ureq::post(&url)
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(120));

    if !config.api_key.trim().is_empty() {
        req = req.set(
            "Authorization",
            &format!("Bearer {}", config.api_key.trim()),
        );
    }

    let response = req.send_json(body).map_err(|e| {
        AppError::from(format!(
            "Local LLM request failed ({url}): {e}. Is Ollama/llama-server running?"
        ))
    })?;

    let parsed: ChatResponse = response
        .into_json()
        .map_err(|e| AppError::from(format!("Invalid LLM JSON response: {e}")))?;

    if let Some(err) = parsed.error {
        return Err(AppError::from(format!(
            "LLM error: {}",
            err.message.unwrap_or_else(|| "unknown".into())
        )));
    }

    let content = parsed
        .choices
        .and_then(|c| c.into_iter().next())
        .and_then(|c| c.message)
        .and_then(|m| m.content)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::from("LLM returned empty content"))?;

    Ok(strip_code_fences(&content))
}

/// Drop optional markdown fences models sometimes wrap around the rewrite.
fn strip_code_fences(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest
            .strip_prefix("text")
            .or_else(|| rest.strip_prefix("markdown"))
            .unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim().to_string();
        }
    }
    t.to_string()
}

pub fn build_rewrite_prompt(instruction: &str, selection: &str) -> (String, String) {
    let system = "You are a precise writing assistant running fully locally. \
Follow the user's instruction exactly. Return ONLY the final rewritten text \
with no preamble, quotes, or explanation.";
    let user = if selection.trim().is_empty() {
        format!(
            "Instruction: {instruction}\n\n\
             There is no selected text. Generate the requested content only."
        )
    } else {
        format!(
            "Instruction: {instruction}\n\n\
             Selected text to transform:\n---\n{selection}\n---\n\n\
             Return only the transformed text."
        )
    };
    (system.to_string(), user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_fences() {
        assert_eq!(strip_code_fences("```text\nHello\n```"), "Hello");
        assert_eq!(strip_code_fences("Hello"), "Hello");
    }

    #[test]
    fn chat_url_shapes() {
        let mut c = LlmConfig::default();
        c.base_url = "http://127.0.0.1:11434/v1".into();
        assert!(c.chat_url().ends_with("/v1/chat/completions"));
        c.base_url = "http://127.0.0.1:8080".into();
        assert!(c.chat_url().ends_with("/v1/chat/completions"));
    }
}
