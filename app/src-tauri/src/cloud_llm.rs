//! Cloud LLM backends (Anthropic Claude, Google Gemini) for cleanup and
//! transforms — opt-in alternatives to the local Ollama path.
//!
//! Unlike everything else in this app, a cloud call sends the dictated text
//! (and glossary terms) off the machine. `settings::AiEngine` is the single
//! switch that opts a user into this; the default stays `Local`. Every
//! function here mirrors the `Result<String, String>` contract already used
//! by `transforms::run`/`cleanup::try_clean` so callers can share the same
//! fallback-to-raw-text handling regardless of engine.

use crate::settings::AiEngine;
use crate::settings::StructureMode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Cleanup and transforms run while the user waits, and a failure costs the
/// ~4s local fallback on top — so fail fast rather than holding the pipeline.
const TIMEOUT: Duration = Duration::from_secs(10);
/// `test_key` is a deliberate, user-initiated check with a spinner; it can
/// afford to wait out a slow first response.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const GEMINI_URL_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const MAX_TOKENS: u32 = 2048;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    Claude,
    Gemini,
}

impl CloudProvider {
    /// Keychain account name for this provider's API key (see `secrets.rs`).
    pub fn keychain_account(self) -> &'static str {
        match self {
            CloudProvider::Claude => "anthropic_api_key",
            CloudProvider::Gemini => "gemini_api_key",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CloudProvider::Claude => "Claude",
            CloudProvider::Gemini => "Gemini",
        }
    }

    /// The provider a cloud engine talks to. `Local` has none — a `None`
    /// here means the caller took the wrong branch.
    pub fn from_engine(engine: AiEngine) -> Option<Self> {
        match engine {
            AiEngine::Local => None,
            AiEngine::Claude => Some(CloudProvider::Claude),
            AiEngine::Gemini => Some(CloudProvider::Gemini),
        }
    }
}

/// One system+user turn against a cloud provider. Plain text in, plain text
/// out — no streaming, no tool use; that's a separate concern from cleanup
/// and transforms.
fn run(
    system: &str,
    user: &str,
    provider: CloudProvider,
    model: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err(format!(
            "No {} API key saved — add one in Settings.",
            provider.label()
        ));
    }
    if model.trim().is_empty() {
        return Err(format!(
            "No {} model configured in Settings.",
            provider.label()
        ));
    }
    match provider {
        CloudProvider::Claude => run_claude(system, user, model, api_key, timeout),
        CloudProvider::Gemini => run_gemini(system, user, model, api_key, timeout),
    }
}

#[derive(Serialize)]
struct ClaudeMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ClaudeRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    temperature: f32,
    messages: Vec<ClaudeMessage<'a>>,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    #[serde(default)]
    content: Vec<ClaudeContentBlock>,
}

#[derive(Deserialize)]
struct ClaudeContentBlock {
    #[serde(default)]
    text: String,
}

fn run_claude(
    system: &str,
    user: &str,
    model: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<String, String> {
    let req = ClaudeRequest {
        model,
        max_tokens: MAX_TOKENS,
        system,
        temperature: 0.2,
        messages: vec![ClaudeMessage {
            role: "user",
            content: user,
        }],
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(ANTHROPIC_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&req)
        .send()
        .map_err(|e| format!("Claude unreachable: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!(
            "Claude returned {status}: {}",
            truncate(&body, 200)
        ));
    }
    let body: ClaudeResponse = resp.json().map_err(|e| e.to_string())?;
    let text: String = body.content.into_iter().map(|b| b.text).collect();
    if text.trim().is_empty() {
        return Err("Claude returned empty output.".into());
    }
    Ok(text)
}

#[derive(Serialize)]
struct GeminiPart<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct GeminiContent<'a> {
    parts: Vec<GeminiPart<'a>>,
}

#[derive(Serialize)]
struct GeminiRequest<'a> {
    system_instruction: GeminiContent<'a>,
    contents: Vec<GeminiContent<'a>>,
    #[serde(rename = "generationConfig")]
    generation_config: serde_json::Value,
}

#[derive(Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiRespContent,
}

#[derive(Deserialize)]
struct GeminiRespContent {
    #[serde(default)]
    parts: Vec<GeminiRespPart>,
}

#[derive(Deserialize)]
struct GeminiRespPart {
    #[serde(default)]
    text: String,
}

fn run_gemini(
    system: &str,
    user: &str,
    model: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<String, String> {
    // The key goes in a header, not `?key=` — a credential in a URL can be
    // captured by intermediaries and TLS-terminating proxies that log paths.
    let url = format!("{GEMINI_URL_BASE}/{model}:generateContent");
    let req = GeminiRequest {
        system_instruction: GeminiContent {
            parts: vec![GeminiPart { text: system }],
        },
        contents: vec![GeminiContent {
            parts: vec![GeminiPart { text: user }],
        }],
        generation_config: serde_json::json!({ "temperature": 0.2 }),
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&url)
        .header("x-goog-api-key", api_key)
        .json(&req)
        .send()
        .map_err(|e| format!("Gemini unreachable: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!(
            "Gemini returned {status}: {}",
            truncate(&body, 200)
        ));
    }
    let body: GeminiResponse = resp.json().map_err(|e| e.to_string())?;
    let text: String = body
        .candidates
        .into_iter()
        .next()
        .map(|c| c.content.parts.into_iter().map(|p| p.text).collect())
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err("Gemini returned empty output.".into());
    }
    Ok(text)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Byte slicing at a fixed offset panics when `max` lands inside a
    // multi-byte character, and this runs on provider error bodies — arbitrary
    // UTF-8 this app does not control. A panic here would destroy the
    // in-flight transcript, so back off to the nearest character boundary.
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Cloud equivalent of `cleanup::clean` — same few-shot system prompt, same
/// word-overlap guard against the model "answering" instead of cleaning, same
/// never-lose-the-transcript fallback. Only the wire call differs.
pub fn clean(
    raw: &str,
    glossary_terms: &str,
    provider: CloudProvider,
    model: &str,
    api_key: &str,
    mode: StructureMode,
) -> crate::cleanup::CleanResult {
    use crate::cleanup::CleanResult;

    let raw = raw.trim();
    if raw.is_empty() {
        return CleanResult {
            text: String::new(),
            used_llm: false,
            note: Some("empty transcript".into()),
        };
    }
    let system = crate::cleanup::build_system_prompt(glossary_terms, mode);
    let user = format!("<transcript>\n{raw}\n</transcript>");
    match run(&system, &user, provider, model, api_key, TIMEOUT) {
        Ok(text) => {
            let cleaned = crate::cleanup::strip_transcript_tags(&text);
            if cleaned.is_empty() {
                CleanResult {
                    text: raw.to_string(),
                    used_llm: false,
                    note: Some("LLM returned empty output; using raw transcript".into()),
                }
            } else if crate::cleanup::word_overlap(&cleaned, raw) < 0.5 {
                CleanResult {
                    text: raw.to_string(),
                    used_llm: false,
                    note: Some("LLM replied instead of cleaning; using raw transcript".into()),
                }
            } else {
                CleanResult {
                    text: cleaned,
                    used_llm: true,
                    note: None,
                }
            }
        }
        Err(e) => CleanResult {
            text: raw.to_string(),
            used_llm: false,
            note: Some(format!("cleanup skipped ({e}); using raw transcript")),
        },
    }
}

/// Cloud equivalent of `transforms::run`.
pub fn run_transform(
    text: &str,
    transform_prompt: &str,
    provider: CloudProvider,
    model: &str,
    api_key: &str,
) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Nothing to transform.".into());
    }
    let system = crate::transforms::system_prompt(transform_prompt);
    let user = format!("<text>\n{text}\n</text>");
    let out = run(&system, &user, provider, model, api_key, TIMEOUT)?;
    let out = crate::transforms::strip_text_tags(&out);
    if out.is_empty() {
        return Err("The model returned empty output.".into());
    }
    Ok(out)
}

/// One tiny round-trip to prove a key and model actually work, so a bad
/// key surfaces at configuration time. Without this, the fallback ladder
/// does its job and a wrong key reads as "dictation quietly got slower and
/// no better".
pub fn test_key(provider: CloudProvider, model: &str, api_key: &str) -> Result<(), String> {
    match run(
        "Reply with the single word: ok.",
        "ping",
        provider,
        model,
        api_key,
        TEST_TIMEOUT,
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            let head = error_head(&e);
            if head.contains(" 401") || head.contains(" 403") {
                Err(format!(
                    "{} rejected the key. Check you pasted all of it, and that the key is enabled for this API.",
                    provider.label()
                ))
            } else if head.contains(" 400") || head.contains(" 404") {
                Err(format!(
                    "{} rejected the model id \"{model}\". Pick a different model.",
                    provider.label()
                ))
            } else {
                Err(e)
            }
        }
    }
}

/// The part of a provider error before the response body.
///
/// `run_claude` / `run_gemini` format failures as
/// `"{label} returned {status}: {body}"`. Matching status codes against the
/// whole string would read digits out of the body — a 429 rate-limit body
/// mentioning "400" would be reported to the user as a bad model id. Splitting
/// at the first `": "` keeps the match on the status only; `StatusCode`'s
/// Display never contains a colon, so the head is exactly
/// `"{label} returned {status}"`.
pub(crate) fn error_head(e: &str) -> &str {
    e.split_once(": ").map(|(head, _)| head).unwrap_or(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_errors_without_network() {
        let err = run(
            "system",
            "user",
            CloudProvider::Claude,
            "claude-sonnet-5",
            "",
            TIMEOUT,
        )
        .unwrap_err();
        assert!(err.contains("API key"));
    }

    #[test]
    fn missing_model_errors_without_network() {
        let err = run(
            "system",
            "user",
            CloudProvider::Gemini,
            "",
            "some-key",
            TIMEOUT,
        )
        .unwrap_err();
        assert!(err.contains("model"));
    }

    #[test]
    fn empty_selection_errors_without_network() {
        assert!(run_transform(
            "   ",
            "do something",
            CloudProvider::Claude,
            "claude-sonnet-5",
            "sk-x"
        )
        .is_err());
    }

    #[test]
    fn empty_transcript_short_circuits_without_network() {
        let r = clean(
            "   ",
            "- SKU",
            CloudProvider::Claude,
            "claude-sonnet-5",
            "sk-x",
            StructureMode::None,
        );
        assert_eq!(r.text, "");
        assert!(!r.used_llm);
    }

    #[test]
    fn truncate_respects_max_len() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_does_not_split_a_multibyte_character() {
        // Provider error bodies are arbitrary UTF-8. Cutting mid-character used
        // to panic, which would destroy the in-flight transcript on the cloud
        // failure path.
        let s = "aaaé"; // 'é' occupies bytes 3..5, so len() == 5
        assert_eq!(truncate(s, 4), "aaa…"); // byte 4 is inside 'é' — must back off
                                            // A cut exactly on a boundary keeps everything up to it.
        assert_eq!(truncate(s, 3), "aaa…");
        // Multi-byte content that fits is returned whole.
        assert_eq!(truncate("héllo", 100), "héllo");
        // Emoji are 4 bytes; cutting inside one must not panic either.
        let e = "ab🙂"; // len() == 6, emoji at bytes 2..6
        assert_eq!(truncate(e, 4), "ab…");
        assert_eq!(truncate(e, 2), "ab…");
    }

    #[test]
    fn engine_maps_to_provider() {
        assert_eq!(
            CloudProvider::from_engine(AiEngine::Claude),
            Some(CloudProvider::Claude)
        );
        assert_eq!(
            CloudProvider::from_engine(AiEngine::Gemini),
            Some(CloudProvider::Gemini)
        );
        // Local has no provider — callers must not reach the cloud path.
        assert_eq!(CloudProvider::from_engine(AiEngine::Local), None);
    }

    #[test]
    fn keychain_accounts_are_distinct_and_stable() {
        // These strings are the Keychain lookup keys. Changing one orphans
        // every already-stored key, so they are pinned by test.
        assert_eq!(
            CloudProvider::Claude.keychain_account(),
            "anthropic_api_key"
        );
        assert_eq!(CloudProvider::Gemini.keychain_account(), "gemini_api_key");
    }

    #[test]
    fn test_key_rejects_a_missing_key_without_network() {
        let err = test_key(CloudProvider::Gemini, "gemini-3.5-flash-lite", "").unwrap_err();
        assert!(err.contains("API key"), "unexpected error: {err}");
    }

    /// Two separate guarantees, neither of which is "the prompt `clean` sends
    /// is the shared one" — proving that would need a fake transport, which is
    /// not worth building for a five-line function with one call site. What this
    /// DOES pin: a keyless cloud call preserves the transcript, and the shared
    /// builder still carries the agreement rule the cloud path inherits from it.
    /// If the rule is ever deleted, the cloud path silently loses it, and this
    /// fires. If someone forks a separate cloud prompt, this will NOT catch it —
    /// the single `build_system_prompt` call site in `clean` is the only thing
    /// enforcing that, and it is a review concern.
    #[test]
    fn keyless_call_preserves_transcript_and_shared_prompt_has_the_rule() {
        let raw = "we need to merge these two account";
        // An empty key fails before any network call.
        let r = clean(
            raw,
            "- SKU",
            CloudProvider::Gemini,
            "gemini-3.5-flash-lite",
            "",
            StructureMode::None,
        );
        assert_eq!(r.text, raw, "the transcript must survive a keyless call");
        assert!(!r.used_llm);
        assert!(r.note.is_some(), "a failure must explain itself");
        let shared = crate::cleanup::build_system_prompt("- SKU", StructureMode::None);
        assert!(
            shared.contains("subject-verb agreement"),
            "the shared prompt lost the agreement rule — the cloud path inherits this"
        );
    }

    #[test]
    fn error_head_ignores_the_response_body() {
        // A rate-limit body mentioning 400 must not be read as a bad model id.
        assert_eq!(
            error_head("Gemini returned 429 Too Many Requests: {\"retryAfter\":400}"),
            "Gemini returned 429 Too Many Requests"
        );
        // A status with no body still matches.
        assert_eq!(error_head("Claude unreachable"), "Claude unreachable");
        assert!(error_head("Claude returned 401 Unauthorized: {\"x\":1}").contains(" 401"));
    }
}
