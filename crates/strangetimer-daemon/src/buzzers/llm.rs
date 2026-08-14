use std::time::Duration;

use serde::Serialize;
use strangetimer_core::model::LlmPromptSource;

use super::audio;

const OLLAMA_ENDPOINT: &str = "http://localhost:11434/api/generate";

/// Ask a local Ollama instance to generate a completion and print it to
/// stdout. If Ollama is unreachable, fall back to the built-in chime and
/// log a warning — a buzzer that silently does nothing is worse than a
/// beep.
pub async fn fire_llm(model: &str, prompt: &LlmPromptSource) {
    let prompt_text = match resolve_prompt(prompt).await {
        Ok(t) => t,
        Err(e) => {
            warn!("cannot read LLM prompt for model {model}: {e}");
            return;
        }
    };

    let client = reqwest::Client::new();
    let request = OllamaRequest {
        model,
        prompt: &prompt_text,
        stream: false,
    };

    let response = client
        .post(OLLAMA_ENDPOINT)
        .json(&request)
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            warn!("Ollama unavailable ({e}) — falling back to default audio");
            audio::fire_audio(None);
            return;
        }
    };

    let body: OllamaResponse = match response.json().await {
        Ok(b) => b,
        Err(e) => {
            warn!(
                "Ollama returned an unexpected response ({e}) — \
                 falling back to default audio"
            );
            audio::fire_audio(None);
            return;
        }
    };

    println!("{}", body.response);
}

async fn resolve_prompt(prompt: &LlmPromptSource) -> anyhow::Result<String> {
    match prompt {
        LlmPromptSource::Inline(s) => Ok(s.clone()),
        LlmPromptSource::File(p) => {
            let text = tokio::fs::read_to_string(p).await?;
            Ok(text)
        }
    }
}

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(serde::Deserialize)]
struct OllamaResponse {
    response: String,
}
