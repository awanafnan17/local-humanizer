pub mod chunker;
pub mod prompts;
pub mod protect;
pub mod reconstructor;
pub mod segmenter;
pub mod validator;

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::commands::{
    build_loopback_http_client, HumanizeProgressEvent, HumanizeProgressState, HumanizeRequest,
    HumanizeResponse, RetryChunkRequest, RetryChunkResponse,
};
use crate::engine::chunker::{build_work_units, trailing_sentences, ChunkJoin, ChunkSpec, WorkUnit};
use crate::engine::prompts::{build_prompt_bundle, validate_language, ChatMessage, PromptBundle, RetryDirective};
use crate::engine::protect::extract_invariants;
use crate::engine::reconstructor::Reconstructor;
use crate::engine::segmenter::{count_paragraphs, count_words, sanitize_input, segment_document};
use crate::engine::validator::{
    attempt_score, collect_chunk_warnings, decide_retry, sanitize_model_output, validate_chunk,
    RetryAction,
};
use crate::runtime::ServerEndpoint;

const INFERENCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_CANCEL_HISTORY: usize = 64;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub enum EngineError {
    Cancelled,
    Transport(String),
    InvalidInput(String),
}

impl EngineError {
    pub fn into_message(self) -> String {
        match self {
            EngineError::Cancelled => "Request cancelled".to_string(),
            EngineError::Transport(m) | EngineError::InvalidInput(m) => m,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub repeat_penalty: f32,
    pub max_tokens: u32,
    pub disable_thinking: bool,
}

impl ChatCompletionRequest {
    pub fn from_bundle(bundle: &PromptBundle) -> Self {
        Self {
            messages: bundle.messages.clone(),
            temperature: bundle.sampling.temperature,
            top_p: bundle.sampling.top_p,
            top_k: bundle.sampling.top_k,
            repeat_penalty: bundle.sampling.repeat_penalty,
            max_tokens: bundle.sampling.max_tokens,
            disable_thinking: bundle.disable_thinking,
        }
    }
}

pub trait InferenceClient: Send + Sync {
    fn chat_completion<'a>(
        &'a self,
        req: ChatCompletionRequest,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<String, EngineError>>;
}

#[derive(Debug, Serialize)]
struct LlamaChatPayload {
    messages: Vec<ChatMessage>,
    temperature: f32,
    top_p: f32,
    top_k: u32,
    repeat_penalty: f32,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<ChatTemplateKwargs>,
}

#[derive(Debug, Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

#[derive(Debug, Deserialize)]
struct LlamaChatResponse {
    choices: Vec<LlamaChatChoice>,
}

#[derive(Debug, Deserialize)]
struct LlamaChatChoice {
    message: LlamaChatMsg,
}

#[derive(Debug, Deserialize)]
struct LlamaChatMsg {
    content: Option<String>,
}

pub struct LlamaServerClient {
    endpoint: ServerEndpoint,
    http: reqwest::Client,
}

impl LlamaServerClient {
    pub fn new(endpoint: ServerEndpoint) -> Result<Self, String> {
        let http = build_loopback_http_client()?;
        Ok(Self { endpoint, http })
    }
}

impl InferenceClient for LlamaServerClient {
    fn chat_completion<'a>(
        &'a self,
        req: ChatCompletionRequest,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<String, EngineError>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let url = format!("{}/v1/chat/completions", self.endpoint.base_url());
            let auth = format!("Bearer {}", self.endpoint.bearer_token);
            let payload = LlamaChatPayload {
                messages: req.messages,
                temperature: req.temperature,
                top_p: req.top_p,
                top_k: req.top_k,
                repeat_penalty: req.repeat_penalty,
                max_tokens: req.max_tokens,
                stream: false,
                chat_template_kwargs: if req.disable_thinking {
                    Some(ChatTemplateKwargs {
                        enable_thinking: false,
                    })
                } else {
                    None
                },
            };

            let send_fut = self
                .http
                .post(&url)
                .header(reqwest::header::AUTHORIZATION, auth)
                .timeout(INFERENCE_REQUEST_TIMEOUT)
                .json(&payload)
                .send();

            let resp = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                res = send_fut => res.map_err(|e| EngineError::Transport(format!("Inference request failed: {e}")))?,
            };

            let status = resp.status();
            let body_fut = resp.text();
            let body = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                res = body_fut => res.map_err(|e| EngineError::Transport(format!("Failed reading inference response: {e}")))?,
            };

            if !status.is_success() {
                return Err(EngineError::Transport(format!(
                    "llama-server returned HTTP {status}"
                )));
            }

            let parsed: LlamaChatResponse = serde_json::from_str(&body)
                .map_err(|e| EngineError::Transport(format!("Invalid llama-server JSON response: {e}")))?;
            let content = parsed
                .choices
                .into_iter()
                .next()
                .and_then(|c| c.message.content)
                .ok_or_else(|| {
                    EngineError::Transport("llama-server response missing choices[0].message.content".to_string())
                })?;

            Ok(content)
        })
    }
}

#[derive(Default)]
struct CancelRegistry {
    active: HashMap<String, CancellationToken>,
    recently_cancelled: VecDeque<String>,
}

pub struct EngineCoordinator {
    inference_gate: tokio::sync::Mutex<()>,
    cancels: Mutex<CancelRegistry>,
}

impl Default for EngineCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineCoordinator {
    pub fn new() -> Self {
        Self {
            inference_gate: tokio::sync::Mutex::new(()),
            cancels: Mutex::new(CancelRegistry::default()),
        }
    }

    pub fn register_request(&self, request_id: &str) -> RequestCancelGuard<'_> {
        let token = CancellationToken::new();
        if let Ok(mut reg) = self.cancels.lock() {
            if reg.recently_cancelled.iter().any(|id| id == request_id) {
                token.cancel();
            }
            reg.active.insert(request_id.to_string(), token.clone());
        }
        RequestCancelGuard {
            coordinator: self,
            request_id: request_id.to_string(),
            token,
        }
    }

    pub fn cancel_request(&self, request_id: &str) {
        if let Ok(mut reg) = self.cancels.lock() {
            if let Some(tok) = reg.active.get(request_id) {
                tok.cancel();
            }
            if !reg.recently_cancelled.iter().any(|id| id == request_id) {
                if reg.recently_cancelled.len() >= MAX_CANCEL_HISTORY {
                    reg.recently_cancelled.pop_front();
                }
                reg.recently_cancelled.push_back(request_id.to_string());
            }
        }
    }

    pub fn cancel_all_active(&self) {
        if let Ok(reg) = self.cancels.lock() {
            for tok in reg.active.values() {
                tok.cancel();
            }
        }
    }

    pub async fn acquire_gate(&self, cancel: &CancellationToken) -> Result<tokio::sync::MutexGuard<'_, ()>, EngineError> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(EngineError::Cancelled),
            guard = self.inference_gate.lock() => {
                if cancel.is_cancelled() {
                    Err(EngineError::Cancelled)
                } else {
                    Ok(guard)
                }
            }
        }
    }
}

pub struct RequestCancelGuard<'a> {
    coordinator: &'a EngineCoordinator,
    request_id: String,
    pub token: CancellationToken,
}

impl Drop for RequestCancelGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut reg) = self.coordinator.cancels.lock() {
            reg.active.remove(&self.request_id);
        }
    }
}

pub async fn humanize_document_impl<C, F>(
    client: &C,
    coordinator: &EngineCoordinator,
    req: HumanizeRequest,
    disable_thinking: bool,
    mut emit_progress: F,
) -> Result<HumanizeResponse, EngineError>
where
    C: InferenceClient,
    F: FnMut(HumanizeProgressEvent),
{
    let started = Instant::now();
    if req.request_id.trim().is_empty() {
        return Err(EngineError::InvalidInput(
            "request_id cannot be empty".to_string(),
        ));
    }
    validate_language(&req.options.language).map_err(EngineError::InvalidInput)?;

    let cancel_guard = coordinator.register_request(&req.request_id);
    let cancel = &cancel_guard.token;

    let sanitized = sanitize_input(&req.text);
    if sanitized.trim().is_empty() {
        emit_progress(HumanizeProgressEvent {
            request_id: req.request_id.clone(),
            chunk_index: 0,
            chunks_total: 0,
            state: HumanizeProgressState::Segmenting,
            partial_output: String::new(),
            chunk_warnings: vec![],
        });
        emit_progress(HumanizeProgressEvent {
            request_id: req.request_id.clone(),
            chunk_index: 0,
            chunks_total: 0,
            state: HumanizeProgressState::Completed,
            partial_output: String::new(),
            chunk_warnings: vec!["Input text is empty.".to_string()],
        });
        return Ok(HumanizeResponse {
            request_id: req.request_id,
            output_text: String::new(),
            chunks_total: 0,
            chunks_completed: 0,
            chunks_retried: 0,
            warnings: vec!["Input text is empty.".to_string()],
            duration_ms: started.elapsed().as_millis() as u64,
            cancelled: false,
        });
    }

    let segments = segment_document(&sanitized);
    let units = build_work_units(&segments, &req.options);
    let chunks_total = units
        .iter()
        .filter(|u| matches!(u, WorkUnit::ParaphraseChunk(_)))
        .count();

    emit_progress(HumanizeProgressEvent {
        request_id: req.request_id.clone(),
        chunk_index: 0,
        chunks_total,
        state: HumanizeProgressState::Segmenting,
        partial_output: String::new(),
        chunk_warnings: vec![],
    });

    if chunks_total == 0 {
        let mut rec = Reconstructor::new();
        for u in &units {
            if let WorkUnit::Verbatim {
                text,
                join_with_prev,
                ..
            } = u
            {
                rec.push_piece(text, *join_with_prev);
            }
        }
        let (output_text, warnings) = rec.finish();
        emit_progress(HumanizeProgressEvent {
            request_id: req.request_id.clone(),
            chunk_index: 0,
            chunks_total: 0,
            state: HumanizeProgressState::Completed,
            partial_output: output_text.clone(),
            chunk_warnings: warnings.clone(),
        });
        return Ok(HumanizeResponse {
            request_id: req.request_id,
            output_text,
            chunks_total: 0,
            chunks_completed: 0,
            chunks_retried: 0,
            warnings,
            duration_ms: started.elapsed().as_millis() as u64,
            cancelled: false,
        });
    }

    let _gate = match coordinator.acquire_gate(cancel).await {
        Ok(g) => g,
        Err(EngineError::Cancelled) => {
            emit_progress(HumanizeProgressEvent {
                request_id: req.request_id.clone(),
                chunk_index: 0,
                chunks_total,
                state: HumanizeProgressState::Cancelled,
                partial_output: String::new(),
                chunk_warnings: vec![],
            });
            return Ok(HumanizeResponse {
                request_id: req.request_id,
                output_text: String::new(),
                chunks_total,
                chunks_completed: 0,
                chunks_retried: 0,
                warnings: vec!["Paraphrasing cancelled.".to_string()],
                duration_ms: started.elapsed().as_millis() as u64,
                cancelled: true,
            });
        }
        Err(e) => return Err(e),
    };

    let mut rec = Reconstructor::new();
    let mut prev_tail: Option<String> = None;
    let mut chunks_completed = 0usize;
    let mut chunks_retried = 0usize;

    for unit in units {
        match unit {
            WorkUnit::Verbatim {
                text,
                join_with_prev,
                ..
            } => {
                rec.push_piece(&text, join_with_prev);
            }
            WorkUnit::ParaphraseChunk(chunk) => {
                if cancel.is_cancelled() {
                    let partial = rec.partial_output();
                    emit_progress(HumanizeProgressEvent {
                        request_id: req.request_id.clone(),
                        chunk_index: chunk.chunk_index,
                        chunks_total,
                        state: HumanizeProgressState::Cancelled,
                        partial_output: partial.clone(),
                        chunk_warnings: vec![],
                    });
                    return Ok(HumanizeResponse {
                        request_id: req.request_id,
                        output_text: partial,
                        chunks_total,
                        chunks_completed,
                        chunks_retried,
                        warnings: vec!["Paraphrasing cancelled by user.".to_string()],
                        duration_ms: started.elapsed().as_millis() as u64,
                        cancelled: true,
                    });
                }

                emit_progress(HumanizeProgressEvent {
                    request_id: req.request_id.clone(),
                    chunk_index: chunk.chunk_index,
                    chunks_total,
                    state: HumanizeProgressState::Inferring,
                    partial_output: rec.partial_output(),
                    chunk_warnings: vec![],
                });

                let bundle1 = build_prompt_bundle(
                    &chunk.text,
                    chunk.word_count,
                    req.mode,
                    req.intensity,
                    &req.options,
                    prev_tail.as_deref(),
                    &RetryDirective::None,
                    disable_thinking,
                    false,
                )
                .map_err(EngineError::InvalidInput)?;

                let raw1 = match client
                    .chat_completion(ChatCompletionRequest::from_bundle(&bundle1), cancel)
                    .await
                {
                    Ok(txt) => txt,
                    Err(EngineError::Cancelled) => {
                        let partial = rec.partial_output();
                        emit_progress(HumanizeProgressEvent {
                            request_id: req.request_id.clone(),
                            chunk_index: chunk.chunk_index,
                            chunks_total,
                            state: HumanizeProgressState::Cancelled,
                            partial_output: partial.clone(),
                            chunk_warnings: vec![],
                        });
                        return Ok(HumanizeResponse {
                            request_id: req.request_id,
                            output_text: partial,
                            chunks_total,
                            chunks_completed,
                            chunks_retried,
                            warnings: vec!["Paraphrasing cancelled by user.".to_string()],
                            duration_ms: started.elapsed().as_millis() as u64,
                            cancelled: true,
                        });
                    }
                    Err(e) => {
                        let msg = e.clone().into_message();
                        emit_progress(HumanizeProgressEvent {
                            request_id: req.request_id.clone(),
                            chunk_index: chunk.chunk_index,
                            chunks_total,
                            state: HumanizeProgressState::Error,
                            partial_output: rec.partial_output(),
                            chunk_warnings: vec![msg],
                        });
                        return Err(e);
                    }
                };

                let san1 = sanitize_model_output(
                    &raw1,
                    prev_tail.as_deref(),
                    &bundle1.delimiter_tag,
                );
                let val1 = validate_chunk(&chunk, &san1.text, &req.options);
                let think_trunc1 = san1.think_leaked && (val1.empty_flag || val1.truncation_flag);

                emit_progress(HumanizeProgressEvent {
                    request_id: req.request_id.clone(),
                    chunk_index: chunk.chunk_index,
                    chunks_total,
                    state: HumanizeProgressState::Validating,
                    partial_output: rec.partial_output(),
                    chunk_warnings: vec![],
                });

                let mut best_text = san1.text;
                let mut best_val = val1;

                let retry_action = decide_retry(&best_val, think_trunc1);
                if retry_action != RetryAction::Accept {
                    chunks_retried += 1;
                    emit_progress(HumanizeProgressEvent {
                        request_id: req.request_id.clone(),
                        chunk_index: chunk.chunk_index,
                        chunks_total,
                        state: HumanizeProgressState::Retrying,
                        partial_output: rec.partial_output(),
                        chunk_warnings: vec![],
                    });

                    let bundle2 = build_prompt_bundle(
                        &chunk.text,
                        chunk.word_count,
                        req.mode,
                        req.intensity,
                        &req.options,
                        prev_tail.as_deref(),
                        &retry_action.to_directive(),
                        disable_thinking,
                        false,
                    )
                    .map_err(EngineError::InvalidInput)?;

                    match client
                        .chat_completion(ChatCompletionRequest::from_bundle(&bundle2), cancel)
                        .await
                    {
                        Ok(raw2) => {
                            let san2 = sanitize_model_output(
                                &raw2,
                                prev_tail.as_deref(),
                                &bundle2.delimiter_tag,
                            );
                            let val2 = validate_chunk(&chunk, &san2.text, &req.options);
                            if attempt_score(&val2) > attempt_score(&best_val) {
                                best_text = san2.text;
                                best_val = val2;
                            }
                        }
                        Err(EngineError::Cancelled) => {
                            let partial = rec.partial_output();
                            emit_progress(HumanizeProgressEvent {
                                request_id: req.request_id.clone(),
                                chunk_index: chunk.chunk_index,
                                chunks_total,
                                state: HumanizeProgressState::Cancelled,
                                partial_output: partial.clone(),
                                chunk_warnings: vec![],
                            });
                            return Ok(HumanizeResponse {
                                request_id: req.request_id,
                                output_text: partial,
                                chunks_total,
                                chunks_completed,
                                chunks_retried,
                                warnings: vec!["Paraphrasing cancelled by user.".to_string()],
                                duration_ms: started.elapsed().as_millis() as u64,
                                cancelled: true,
                            });
                        }
                        Err(_) => {
                            // Keep best_text from Attempt 1 if retry call errored transiently unless fatal
                        }
                    }
                }

                // Fallback if model returned completely empty even after retry: retain source with warning
                let mut chunk_warnings = collect_chunk_warnings(&best_val);
                if best_text.trim().is_empty() {
                    best_text = chunk.text.clone();
                    if chunk_warnings.is_empty() {
                        chunk_warnings.push(format!(
                            "Chunk {} returned empty output; preserved original chunk.",
                            chunk.chunk_index + 1
                        ));
                    }
                }

                rec.push_piece(&best_text, chunk.join_with_prev);
                rec.push_warnings(chunk_warnings.clone());
                prev_tail = Some(trailing_sentences(&best_text, 2));
                chunks_completed += 1;

                emit_progress(HumanizeProgressEvent {
                    request_id: req.request_id.clone(),
                    chunk_index: chunk.chunk_index,
                    chunks_total,
                    state: HumanizeProgressState::Validating,
                    partial_output: rec.partial_output(),
                    chunk_warnings,
                });
            }
        }
    }

    let (output_text, warnings) = rec.finish();
    emit_progress(HumanizeProgressEvent {
        request_id: req.request_id.clone(),
        chunk_index: chunks_total,
        chunks_total,
        state: HumanizeProgressState::Completed,
        partial_output: output_text.clone(),
        chunk_warnings: vec![],
    });

    Ok(HumanizeResponse {
        request_id: req.request_id,
        output_text,
        chunks_total,
        chunks_completed,
        chunks_retried,
        warnings,
        duration_ms: started.elapsed().as_millis() as u64,
        cancelled: false,
    })
}

pub async fn retry_chunk_impl<C>(
    client: &C,
    coordinator: &EngineCoordinator,
    req: RetryChunkRequest,
    disable_thinking: bool,
) -> Result<RetryChunkResponse, EngineError>
where
    C: InferenceClient,
{
    if req.request_id.trim().is_empty() {
        return Err(EngineError::InvalidInput(
            "request_id cannot be empty".to_string(),
        ));
    }
    validate_language(&req.options.language).map_err(EngineError::InvalidInput)?;

    let sanitized = sanitize_input(&req.original_chunk);
    if sanitized.trim().is_empty() {
        return Err(EngineError::InvalidInput(
            "original_chunk cannot be empty".to_string(),
        ));
    }

    let cancel_guard = coordinator.register_request(&req.request_id);
    let cancel = &cancel_guard.token;
    let _gate = coordinator.acquire_gate(cancel).await?;

    let wc = count_words(&sanitized);
    let chunk = ChunkSpec {
        chunk_index: req.chunk_index,
        text: sanitized.clone(),
        word_count: wc,
        paragraph_count: count_paragraphs(&sanitized).max(1),
        join_with_prev: ChunkJoin::First,
        invariants: extract_invariants(&sanitized, &req.options),
    };

    let ctx_opt = if req.context_before.trim().is_empty() {
        None
    } else {
        Some(req.context_before.trim())
    };

    let initial_directive = if wc >= 8 {
        RetryDirective::EchoRetry
    } else {
        RetryDirective::None
    };

    let bundle1 = build_prompt_bundle(
        &chunk.text,
        chunk.word_count,
        req.mode,
        req.intensity,
        &req.options,
        ctx_opt,
        &initial_directive,
        disable_thinking,
        false,
    )
    .map_err(EngineError::InvalidInput)?;

    let raw1 = client
        .chat_completion(ChatCompletionRequest::from_bundle(&bundle1), cancel)
        .await?;
    let san1 = sanitize_model_output(&raw1, ctx_opt, &bundle1.delimiter_tag);
    let val1 = validate_chunk(&chunk, &san1.text, &req.options);
    let think_trunc1 = san1.think_leaked && (val1.empty_flag || val1.truncation_flag);

    let mut best_text = san1.text;
    let mut best_val = val1;

    let retry_action = decide_retry(&best_val, think_trunc1);
    if retry_action != RetryAction::Accept {
        let bundle2 = build_prompt_bundle(
            &chunk.text,
            chunk.word_count,
            req.mode,
            req.intensity,
            &req.options,
            ctx_opt,
            &retry_action.to_directive(),
            disable_thinking,
            false,
        )
        .map_err(EngineError::InvalidInput)?;

        if let Ok(raw2) = client
            .chat_completion(ChatCompletionRequest::from_bundle(&bundle2), cancel)
            .await
        {
            let san2 = sanitize_model_output(&raw2, ctx_opt, &bundle2.delimiter_tag);
            let val2 = validate_chunk(&chunk, &san2.text, &req.options);
            if attempt_score(&val2) > attempt_score(&best_val) {
                best_text = san2.text;
                best_val = val2;
            }
        }
    }

    let mut warnings = collect_chunk_warnings(&best_val);
    if best_text.trim().is_empty() {
        best_text = chunk.text;
        warnings.push("Retry returned empty output; preserved original chunk.".to_string());
    }

    Ok(RetryChunkResponse {
        request_id: req.request_id,
        chunk_index: req.chunk_index,
        rewritten_chunk: best_text,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Intensity, RewriteMode, RewriteOptions, VoiceOption};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn opts() -> RewriteOptions {
        RewriteOptions {
            preserve_length: true,
            preserve_formatting: true,
            preserve_citations: true,
            preserve_quotes: true,
            language: "en".to_string(),
            voice: VoiceOption::Auto,
        }
    }

    struct MockClient<F>
    where
        F: Fn(usize, &ChatCompletionRequest) -> Result<String, EngineError> + Send + Sync,
    {
        calls: AtomicUsize,
        delay: Duration,
        responder: F,
    }

    impl<F> InferenceClient for MockClient<F>
    where
        F: Fn(usize, &ChatCompletionRequest) -> Result<String, EngineError> + Send + Sync,
    {
        fn chat_completion<'a>(
            &'a self,
            req: ChatCompletionRequest,
            cancel: &'a CancellationToken,
        ) -> BoxFuture<'a, Result<String, EngineError>> {
            Box::pin(async move {
                let idx = self.calls.fetch_add(1, Ordering::SeqCst);
                if !self.delay.is_zero() {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                        _ = tokio::time::sleep(self.delay) => {}
                    }
                }
                if cancel.is_cancelled() {
                    return Err(EngineError::Cancelled);
                }
                (self.responder)(idx, &req)
            })
        }
    }

    fn extract_source_from_user_msg(content: &str) -> String {
        let start = content.find("<SOURCE_TEXT_DATA").unwrap();
        let after_open = content[start..].find('\n').unwrap() + start + 1;
        let end = content[after_open..].find("\n</SOURCE_TEXT_DATA").unwrap() + after_open;
        content[after_open..end].to_string()
    }

    #[tokio::test]
    async fn empty_and_whitespace_inputs_return_zero_chunks_without_calling_model() {
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, _| panic!("must not call LLM on empty input"),
        };
        let coord = EngineCoordinator::new();
        let mut events = Vec::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "empty-1".to_string(),
                text: "   \n\t\r\n   ".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |ev| events.push(ev),
        )
        .await
        .unwrap();

        assert_eq!(resp.chunks_total, 0);
        assert_eq!(resp.output_text, "");
        assert!(!resp.cancelled);
        assert_eq!(client.calls.load(Ordering::SeqCst), 0);
        assert_eq!(events.last().unwrap().state, HumanizeProgressState::Completed);
    }

    #[tokio::test]
    async fn pipeline_retries_echo_with_higher_temp_and_preserves_verbatim_blocks() {
        let mut paras = Vec::new();
        let nouns = ["throughput", "latency", "jitter", "cache", "scheduler", "memory", "queue", "storage"];
        let verbs = ["improved", "stabilized", "scaled", "converged", "adapted", "balanced", "persisted", "accelerated"];
        for p in 0..18 {
            let mut s = format!("Paragraph {p} documents $1,250 savings across 12 regional centers in 2026. ");
            for j in 0..8 {
                s.push_str(&format!(
                    "Indicator {p}-{j} showed {} {} during cycle {}. ",
                    nouns[j],
                    verbs[(p + j) % verbs.len()],
                    100 + p * 10 + j
                ));
            }
            paras.push(s.trim().to_string());
        }
        paras.insert(9, "```rust\nfn keep_intact() {\n\n    println!(\"ok\");\n}\n```".to_string());
        let doc = paras.join("\n\n");

        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |call_idx, req| {
                let src = extract_source_from_user_msg(&req.messages[1].content);
                if call_idx == 0 {
                    // Attempt 1 echoes verbatim
                    return Ok(src);
                }
                if call_idx == 1 {
                    // Attempt 2 must receive elevated temp=0.72 and anti-copy directive
                    assert!((req.temperature - 0.72).abs() < 0.01);
                    assert!(req.messages[0].content.contains("ANTI-COPY"));
                }
                let rewritten = src
                    .replace("documents", "records")
                    .replace("savings across", "cost reductions across")
                    .replace("regional centers", "local hubs")
                    .replace("showed", "demonstrated")
                    .replace("during cycle", "throughout phase");
                Ok(rewritten)
            },
        };

        let coord = EngineCoordinator::new();
        let mut events = Vec::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "doc-3000".to_string(),
                text: doc,
                mode: RewriteMode::Professional,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |ev| events.push(ev),
        )
        .await
        .unwrap();

        assert!(!resp.cancelled);
        assert_eq!(resp.chunks_completed, resp.chunks_total);
        assert!(resp.chunks_total >= 5);
        assert_eq!(resp.chunks_retried, 1);
        assert!(resp.output_text.contains("```rust\nfn keep_intact() {\n\n    println!(\"ok\");\n}\n```"));
        assert_eq!(events.first().unwrap().state, HumanizeProgressState::Segmenting);
        assert_eq!(events.last().unwrap().state, HumanizeProgressState::Completed);
    }

    #[tokio::test]
    async fn persistent_echo_emits_mandatory_warning() {
        let src = "Local execution keeps every confidential manuscript strictly inside the user workspace without touching remote endpoints.";
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, req| Ok(extract_source_from_user_msg(&req.messages[1].content)),
        };
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "echo-warn".to_string(),
                text: src.to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(resp.chunks_retried, 1);
        assert!(
            resp.warnings.iter().any(|w| w.contains("echoed")),
            "expected non-silent echo warning, got {:?}",
            resp.warnings
        );
    }

    #[tokio::test]
    async fn mid_flight_and_pre_cancellation_work_cleanly_without_placeholders() {
        let client = Arc::new(MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::from_millis(80),
            responder: |_, req| {
                let src = extract_source_from_user_msg(&req.messages[1].content);
                Ok(src.replace("sample", "example"))
            },
        });
        let coord = Arc::new(EngineCoordinator::new());

        // Pre-cancel test
        coord.cancel_request("pre-cancel-1");
        let pre_resp = humanize_document_impl(
            client.as_ref(),
            &coord,
            HumanizeRequest {
                request_id: "pre-cancel-1".to_string(),
                text: "This is a sample paragraph with enough words to process.".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Light,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert!(pre_resp.cancelled);

        // Live mid-flight cancel test
        let c_clone = Arc::clone(&coord);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            c_clone.cancel_request("live-cancel-1");
        });
        let live_resp = humanize_document_impl(
            client.as_ref(),
            &coord,
            HumanizeRequest {
                request_id: "live-cancel-1".to_string(),
                text: "This is a sample paragraph with enough words to process cleanly.".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Light,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert!(live_resp.cancelled);
        assert!(!live_resp.output_text.contains("__LH_PROT_"));

        // Subsequent request succeeds cleanly
        let next_resp = humanize_document_impl(
            client.as_ref(),
            &coord,
            HumanizeRequest {
                request_id: "after-cancel".to_string(),
                text: "This is a sample sentence with ten clear words inside.".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Light,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert!(!next_resp.cancelled);
        assert!(next_resp.output_text.contains("example"));
    }

    #[tokio::test]
    async fn retry_chunk_recovers_single_chunk_with_context() {
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, req| {
                assert!(req.messages[0].content.contains("Preceding context here."));
                Ok("Fresh phrasing confirming 42 nodes operated at 99.5% uptime.".to_string())
            },
        };
        let coord = EngineCoordinator::new();
        let resp = retry_chunk_impl(
            &client,
            &coord,
            RetryChunkRequest {
                request_id: "retry-1".to_string(),
                chunk_index: 2,
                original_chunk: "Original phrasing noting 42 nodes achieved 99.5% uptime.".to_string(),
                context_before: "Preceding context here.".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
        )
        .await
        .unwrap();

        assert_eq!(resp.chunk_index, 2);
        assert!(resp.rewritten_chunk.contains("42"));
        assert!(resp.rewritten_chunk.contains("99.5%"));
    }

    #[tokio::test]
    async fn punctuation_only_document_preserves_verbatim_without_warnings() {
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, _| panic!("must not call LLM on punctuation-only separator"),
        };
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "punct-1".to_string(),
                text: "---".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(resp.chunks_total, 0);
        assert_eq!(resp.chunks_retried, 0);
        assert_eq!(resp.output_text, "---");
        assert!(resp.warnings.is_empty(), "unexpected warnings: {:?}", resp.warnings);
        assert_eq!(client.calls.load(Ordering::SeqCst), 0);
    }
}
