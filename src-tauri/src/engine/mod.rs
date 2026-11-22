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
    HumanizeResponse, Intensity, RetryChunkRequest, RetryChunkResponse, RewriteMode,
    RewriteOptions,
};
use crate::engine::chunker::{
    build_work_units, split_sentences_safe, trailing_sentences, ChunkJoin, ChunkSpec, WorkUnit,
};
use crate::engine::prompts::{
    build_prompt_bundle, validate_language, ChatMessage, PromptBundle, RetryDirective,
};
use crate::engine::protect::{
    extract_invariants, mask_opaque_tokens, restore_masked_tokens, MaskedTokens, RestoreResult,
};
use crate::engine::reconstructor::Reconstructor;
use crate::engine::segmenter::{count_paragraphs, count_words, sanitize_input, segment_document};
use crate::engine::validator::{
    attempt_score, collect_chunk_warnings, decide_retry, has_identifier_subject_verb,
    is_fatal_chunk_failure, sanitize_model_output, validate_chunk_with_mode, ChunkValidation,
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
    pub min_p: f32,
    pub repeat_penalty: f32,
    pub presence_penalty: f32,
    pub frequency_penalty: f32,
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
            min_p: bundle.sampling.min_p,
            repeat_penalty: bundle.sampling.repeat_penalty,
            presence_penalty: bundle.sampling.presence_penalty,
            frequency_penalty: bundle.sampling.frequency_penalty,
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
    min_p: f32,
    repeat_penalty: f32,
    presence_penalty: f32,
    frequency_penalty: f32,
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
                min_p: req.min_p,
                repeat_penalty: req.repeat_penalty,
                presence_penalty: req.presence_penalty,
                frequency_penalty: req.frequency_penalty,
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
            failed: false,
            error_message: None,
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
            failed: false,
            error_message: None,
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
                failed: false,
                error_message: None,
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
                        failed: false,
                        error_message: None,
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

                let outcome = paraphrase_chunk_with_strategies(
                    client,
                    &chunk,
                    req.mode,
                    req.intensity,
                    &req.options,
                    prev_tail.as_deref(),
                    RetryDirective::None,
                    disable_thinking,
                    cancel,
                    || {
                        emit_progress(HumanizeProgressEvent {
                            request_id: req.request_id.clone(),
                            chunk_index: chunk.chunk_index,
                            chunks_total,
                            state: HumanizeProgressState::Retrying,
                            partial_output: rec.partial_output(),
                            chunk_warnings: vec![],
                        });
                    },
                )
                .await;

                let (best_text, best_val, was_retried) = match outcome {
                    Ok(res) => res,
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
                            failed: false,
                            error_message: None,
                        });
                    }
                    Err(e) => {
                        let msg = e.clone().into_message();
                        emit_progress(HumanizeProgressEvent {
                            request_id: req.request_id.clone(),
                            chunk_index: chunk.chunk_index,
                            chunks_total,
                            state: HumanizeProgressState::Error,
                            partial_output: String::new(),
                            chunk_warnings: vec![msg],
                        });
                        return Err(e);
                    }
                };

                if was_retried {
                    chunks_retried += 1;
                }

                let chunk_warnings = collect_chunk_warnings(&best_val);
                if is_fatal_chunk_failure(&best_val) {
                    let reason = format_chunk_failure_reason(chunk.chunk_index, &best_val, &chunk_warnings);
                    let mut all_warnings = rec.finish().1;
                    all_warnings.extend(chunk_warnings.clone());
                    if all_warnings.is_empty() {
                        all_warnings.push(reason.clone());
                    }
                    emit_progress(HumanizeProgressEvent {
                        request_id: req.request_id.clone(),
                        chunk_index: chunk.chunk_index,
                        chunks_total,
                        state: HumanizeProgressState::Error,
                        partial_output: String::new(),
                        chunk_warnings: vec![reason.clone()],
                    });
                    return Ok(HumanizeResponse {
                        request_id: req.request_id,
                        output_text: String::new(),
                        chunks_total,
                        chunks_completed,
                        chunks_retried,
                        warnings: all_warnings,
                        duration_ms: started.elapsed().as_millis() as u64,
                        cancelled: false,
                        failed: true,
                        error_message: Some(reason),
                    });
                }

                rec.push_piece(&best_text, chunk.join_with_prev);
                rec.push_warnings(chunk_warnings.clone());
                prev_tail = sanitize_prev_context(&best_text, &req.options);
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
        failed: false,
        error_message: None,
    })
}

fn sanitize_prev_context(accepted_output: &str, opts: &RewriteOptions) -> Option<String> {
    let tail = trailing_sentences(accepted_output, 1);
    if tail.trim().is_empty() {
        return None;
    }
    let inv = extract_invariants(&tail, opts);
    let mut cleaned = tail;
    for span in inv
        .urls
        .iter()
        .chain(inv.emails.iter())
        .chain(inv.dois.iter())
        .chain(inv.citations.iter())
        .chain(inv.quoted_strings.iter())
    {
        if !span.is_empty() {
            cleaned = cleaned.replace(span, "");
        }
    }
    let compact: Vec<&str> = cleaned.split_whitespace().collect();
    if compact.len() < 4 {
        return None;
    }
    let capped = if compact.len() > 30 {
        compact[compact.len() - 30..].join(" ")
    } else {
        compact.join(" ")
    };
    Some(capped)
}

fn format_chunk_failure_reason(
    chunk_index: usize,
    val: &ChunkValidation,
    warnings: &[String],
) -> String {
    let display_idx = chunk_index + 1;
    let detail = if val.empty_flag {
        "the model returned empty output across all 3 rewriting strategies"
    } else if val.polarity_mismatch_flag {
        val.polarity_reason
            .as_deref()
            .unwrap_or("critical negation or polarity meaning drifted")
    } else if val.hallucination_flag {
        "ungrounded numbers, citations, or identifiers were introduced"
    } else if val.leaked_placeholder || !val.duplicated_invariants.is_empty() {
        "protected URL/DOI/email placeholders or quotes were duplicated or leaked"
    } else if val.malformed_syntax_flag {
        "a URL or DOI was placed as a grammatical sentence subject"
    } else if !val.missing_invariants.is_empty() {
        warnings
            .iter()
            .find(|w| w.contains("altered or omitted protected item"))
            .map(String::as_str)
            .unwrap_or("required citations, quotes, or numerical values were omitted or altered")
    } else if val.verbatim_echo_flag {
        "generated phrasing remained too close to the original source across all 3 rewriting strategies"
    } else if val.dangling_end {
        "generated output ended mid-sentence"
    } else if !warnings.is_empty() {
        warnings[0].as_str()
    } else {
        "chunk failed validation checks"
    };
    format!(
        "Paraphrasing stopped at chunk {display_idx} because {detail}. Your original text has been kept intact—please retry or adjust mode/intensity."
    )
}

fn build_decomposition_groups(text: &str) -> Vec<String> {
    let sents = split_sentences_safe(text);
    if sents.len() <= 2 {
        return sents;
    }
    let mut groups: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < sents.len() {
        let cur = &sents[i];
        let cur_words = count_words(cur);
        let remaining_after_merge = sents.len().saturating_sub(i + 2);
        if cur_words < 10 && i + 1 < sents.len() && groups.len() + 1 + remaining_after_merge >= 2 {
            let combined = format!("{cur} {}", sents[i + 1]);
            if count_words(&combined) <= 45 {
                groups.push(combined);
                i += 2;
                continue;
            }
        }
        groups.push(cur.clone());
        i += 1;
    }
    groups
}

fn normalize_single_paragraph_if_needed(
    text: String,
    chunk: &ChunkSpec,
    options: &RewriteOptions,
) -> String {
    if options.preserve_formatting && chunk.paragraph_count == 1 && text.contains("\n\n") {
        let joined = text
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        return joined;
    }
    text
}

fn translate_directive_for_masked(dir: RetryDirective, masked: &MaskedTokens) -> RetryDirective {
    if masked.replacements.is_empty() {
        return dir;
    }
    match dir {
        RetryDirective::None => RetryDirective::None,
        RetryDirective::EchoRetry { hint } => RetryDirective::EchoRetry { hint },
        RetryDirective::StrictInvariant {
            missing_items,
            polarity_hint,
            hallucination_hint,
            length_hint,
        } => RetryDirective::StrictInvariant {
            missing_items: masked.translate_missing_items(&missing_items),
            polarity_hint,
            hallucination_hint,
            length_hint,
        },
        RetryDirective::Combined {
            missing_items,
            polarity_hint,
            hallucination_hint,
            length_hint,
            structural_pivot,
        } => RetryDirective::Combined {
            missing_items: masked.translate_missing_items(&missing_items),
            polarity_hint,
            hallucination_hint,
            length_hint,
            structural_pivot,
        },
        RetryDirective::DecompositionRetry {
            missing_items,
            polarity_hint,
        } => RetryDirective::DecompositionRetry {
            missing_items: masked.translate_missing_items(&missing_items),
            polarity_hint,
        },
    }
}

fn apply_restore_and_syntax_checks(
    val: &mut ChunkValidation,
    masked_src: &str,
    masked_out: &str,
    rest: &RestoreResult,
) {
    if !rest.duplicated_placeholders.is_empty() {
        val.leaked_placeholder = true;
        for ph in &rest.duplicated_placeholders {
            let item = format!("placeholder:{ph}");
            if !val.duplicated_invariants.contains(&item) {
                val.duplicated_invariants.push(item);
            }
        }
        val.placeholder_error_reason = Some(format!(
            "Duplicated placeholder token(s) {}—use each placeholder exactly once.",
            rest.duplicated_placeholders.join(", ")
        ));
    }
    if !val.empty_flag
        && !has_identifier_subject_verb(masked_src)
        && has_identifier_subject_verb(masked_out)
    {
        val.malformed_syntax_flag = true;
    }
}

#[allow(clippy::too_many_arguments)]
async fn paraphrase_chunk_with_strategies<C, F>(
    client: &C,
    chunk: &ChunkSpec,
    mode: RewriteMode,
    intensity: Intensity,
    options: &RewriteOptions,
    prev_tail: Option<&str>,
    initial_directive: RetryDirective,
    disable_thinking: bool,
    cancel: &CancellationToken,
    mut on_retry_start: F,
) -> Result<(String, ChunkValidation, bool), EngineError>
where
    C: InferenceClient,
    F: FnMut(),
{
    // Keep exact quotes unmasked in `"..."` so the model understands their clause semantics;
    // mask only opaque URLs, emails, and DOIs.
    let masked = mask_opaque_tokens(&chunk.text);
    let prompt_text = masked.masked_text.as_str();
    let prompt_words = count_words(prompt_text);

    // Strategy 1: Primary mode + intensity prompt
    let dir1 = translate_directive_for_masked(initial_directive, &masked);
    let bundle1 = build_prompt_bundle(
        prompt_text,
        prompt_words,
        mode,
        intensity,
        options,
        prev_tail,
        &dir1,
        disable_thinking,
        false,
    )
    .map_err(EngineError::InvalidInput)?;

    let raw1 = client
        .chat_completion(ChatCompletionRequest::from_bundle(&bundle1), cancel)
        .await?;
    let san1 = sanitize_model_output(&raw1, prev_tail, &bundle1.delimiter_tag);
    let norm1 = normalize_single_paragraph_if_needed(san1.text, chunk, options);
    let rest1 = restore_masked_tokens(&norm1, &masked, &chunk.text);
    let mut val1 = validate_chunk_with_mode(chunk, &rest1.text, options, mode, intensity);
    apply_restore_and_syntax_checks(&mut val1, prompt_text, &norm1, &rest1);
    let think_trunc1 = san1.think_leaked && (val1.empty_flag || val1.truncation_flag);

    let mut best_text = rest1.text;
    let mut best_val = val1;

    let action1 = decide_retry(&best_val, think_trunc1);
    if action1 == RetryAction::Accept {
        return Ok((best_text, best_val, false));
    }

    // Strategy 2: Structural Pivot Retry with remediation hints
    on_retry_start();
    let dir2 = translate_directive_for_masked(action1.to_directive(), &masked);
    let bundle2 = build_prompt_bundle(
        prompt_text,
        prompt_words,
        mode,
        intensity,
        options,
        prev_tail,
        &dir2,
        disable_thinking,
        false,
    )
    .map_err(EngineError::InvalidInput)?;

    let mut think_trunc2 = false;
    match client
        .chat_completion(ChatCompletionRequest::from_bundle(&bundle2), cancel)
        .await
    {
        Ok(raw2) => {
            let san2 = sanitize_model_output(&raw2, prev_tail, &bundle2.delimiter_tag);
            let norm2 = normalize_single_paragraph_if_needed(san2.text, chunk, options);
            let rest2 = restore_masked_tokens(&norm2, &masked, &chunk.text);
            let mut val2 = validate_chunk_with_mode(chunk, &rest2.text, options, mode, intensity);
            apply_restore_and_syntax_checks(&mut val2, prompt_text, &norm2, &rest2);
            think_trunc2 = san2.think_leaked && (val2.empty_flag || val2.truncation_flag);
            if attempt_score(&val2) > attempt_score(&best_val) {
                best_text = rest2.text;
                best_val = val2;
            }
        }
        Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
        Err(_) => {}
    }

    let action2 = decide_retry(&best_val, think_trunc2);
    if action2 == RetryAction::Accept {
        return Ok((best_text, best_val, true));
    }

    // Strategy 3: Sentence / clause-group decomposition fallback
    let groups = build_decomposition_groups(prompt_text);
    if groups.len() >= 2 {
        let mut sub_parts: Vec<String> = Vec::with_capacity(groups.len());
        let mut sub_tail = prev_tail.map(str::to_string);
        let mut decomp_ok = true;
        let mut all_sub_units_valid = true;

        for grp in &groups {
            if cancel.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let grp_words = count_words(grp);
            let grp_orig = restore_masked_tokens(grp, &masked, &chunk.text).text;
            let sub_invariants = extract_invariants(&grp_orig, options);
            let grp_inv = sub_invariants
                .enforced_items(options)
                .into_iter()
                .map(|(cat, val, _)| format!("{cat}:{val}"))
                .collect::<Vec<_>>();
            let sub_chunk = ChunkSpec {
                chunk_index: chunk.chunk_index,
                text: grp_orig.clone(),
                word_count: count_words(&grp_orig),
                paragraph_count: 1,
                join_with_prev: ChunkJoin::First,
                invariants: sub_invariants,
            };
            let pol_hint = best_val.polarity_reason.clone();
            let dir3 = translate_directive_for_masked(
                RetryDirective::DecompositionRetry {
                    missing_items: grp_inv,
                    polarity_hint: pol_hint,
                },
                &masked,
            );
            let bundle3 = build_prompt_bundle(
                grp,
                grp_words,
                mode,
                intensity,
                options,
                sub_tail.as_deref(),
                &dir3,
                disable_thinking,
                false,
            )
            .map_err(EngineError::InvalidInput)?;

            let mut best_sub_text: String;
            let mut best_sub_val: Option<ChunkValidation>;
            let sub_think_trunc: bool;

            match client
                .chat_completion(ChatCompletionRequest::from_bundle(&bundle3), cancel)
                .await
            {
                Ok(raw_sub) => {
                    let san_sub = sanitize_model_output(
                        &raw_sub,
                        sub_tail.as_deref(),
                        &bundle3.delimiter_tag,
                    );
                    let rest_sub = restore_masked_tokens(&san_sub.text, &masked, &grp_orig);
                    let mut val_sub =
                        validate_chunk_with_mode(&sub_chunk, &rest_sub.text, options, mode, intensity);
                    apply_restore_and_syntax_checks(&mut val_sub, grp, &san_sub.text, &rest_sub);
                    sub_think_trunc =
                        san_sub.think_leaked && (val_sub.empty_flag || val_sub.truncation_flag);
                    best_sub_text = rest_sub.text;
                    best_sub_val = Some(val_sub);
                }
                Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
                Err(_) => {
                    decomp_ok = false;
                    break;
                }
            }

            if let Some(mut cur_sub_val) = best_sub_val {
                let sub_action = decide_retry(&cur_sub_val, sub_think_trunc);
                let needs_sub_retry = !matches!(sub_action, RetryAction::Accept)
                    && (cur_sub_val.empty_flag
                        || cur_sub_val.verbatim_echo_flag
                        || !cur_sub_val.missing_invariants.is_empty()
                        || cur_sub_val.polarity_mismatch_flag
                        || cur_sub_val.hallucination_flag
                        || cur_sub_val.leaked_placeholder
                        || !cur_sub_val.duplicated_invariants.is_empty()
                        || cur_sub_val.malformed_syntax_flag);
                if needs_sub_retry {
                    let sub_retry_dir =
                        translate_directive_for_masked(sub_action.to_directive(), &masked);
                    let sub_retry_bundle = build_prompt_bundle(
                        grp,
                        grp_words,
                        mode,
                        intensity,
                        options,
                        sub_tail.as_deref(),
                        &sub_retry_dir,
                        disable_thinking,
                        false,
                    )
                    .map_err(EngineError::InvalidInput)?;

                    match client
                        .chat_completion(
                            ChatCompletionRequest::from_bundle(&sub_retry_bundle),
                            cancel,
                        )
                        .await
                    {
                        Ok(raw_sub2) => {
                            let san_sub2 = sanitize_model_output(
                                &raw_sub2,
                                sub_tail.as_deref(),
                                &sub_retry_bundle.delimiter_tag,
                            );
                            let rest_sub2 =
                                restore_masked_tokens(&san_sub2.text, &masked, &grp_orig);
                            let mut val_sub2 = validate_chunk_with_mode(
                                &sub_chunk,
                                &rest_sub2.text,
                                options,
                                mode,
                                intensity,
                            );
                            apply_restore_and_syntax_checks(
                                &mut val_sub2,
                                grp,
                                &san_sub2.text,
                                &rest_sub2,
                            );
                            if attempt_score(&val_sub2) > attempt_score(&cur_sub_val) {
                                best_sub_text = rest_sub2.text;
                                cur_sub_val = val_sub2;
                            }
                        }
                        Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
                        Err(_) => {}
                    }

                    // If a sub-unit still dropped a protected item (e.g. a word inside a long quote
                    // or a citation like [12]), run one focused StrictInvariant recovery pass.
                    if is_fatal_chunk_failure(&cur_sub_val)
                        && (!cur_sub_val.missing_invariants.is_empty()
                            || cur_sub_val.leaked_placeholder
                            || !cur_sub_val.duplicated_invariants.is_empty()
                            || cur_sub_val.malformed_syntax_flag)
                    {
                        let repair_action = decide_retry(&cur_sub_val, false);
                        let repair_dir = translate_directive_for_masked(
                            match repair_action {
                                RetryAction::CombinedRetry {
                                    missing,
                                    polarity_hint,
                                    hallucination_hint,
                                    length_hint,
                                    ..
                                } => RetryDirective::StrictInvariant {
                                    missing_items: missing,
                                    polarity_hint,
                                    hallucination_hint,
                                    length_hint,
                                },
                                other => other.to_directive(),
                            },
                            &masked,
                        );
                        if let Ok(repair_bundle) = build_prompt_bundle(
                            grp,
                            grp_words,
                            mode,
                            intensity,
                            options,
                            sub_tail.as_deref(),
                            &repair_dir,
                            disable_thinking,
                            false,
                        ) {
                            match client
                                .chat_completion(
                                    ChatCompletionRequest::from_bundle(&repair_bundle),
                                    cancel,
                                )
                                .await
                            {
                                Ok(raw_sub3) => {
                                    let san_sub3 = sanitize_model_output(
                                        &raw_sub3,
                                        sub_tail.as_deref(),
                                        &repair_bundle.delimiter_tag,
                                    );
                                    let rest_sub3 =
                                        restore_masked_tokens(&san_sub3.text, &masked, &grp_orig);
                                    let mut val_sub3 = validate_chunk_with_mode(
                                        &sub_chunk,
                                        &rest_sub3.text,
                                        options,
                                        mode,
                                        intensity,
                                    );
                                    apply_restore_and_syntax_checks(
                                        &mut val_sub3,
                                        grp,
                                        &san_sub3.text,
                                        &rest_sub3,
                                    );
                                    if attempt_score(&val_sub3) > attempt_score(&cur_sub_val) {
                                        best_sub_text = rest_sub3.text;
                                        cur_sub_val = val_sub3;
                                    }
                                }
                                Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
                                Err(_) => {}
                            }
                        }
                    }
                }
                best_sub_val = Some(cur_sub_val);
            }

            if best_sub_val
                .as_ref()
                .is_none_or(|v| is_fatal_chunk_failure(v))
            {
                all_sub_units_valid = false;
            }

            if best_sub_text.trim().is_empty() {
                decomp_ok = false;
                break;
            }
            sub_tail = Some(trailing_sentences(&best_sub_text, 1));
            sub_parts.push(best_sub_text);
        }

        if decomp_ok && !sub_parts.is_empty() {
            let combined =
                normalize_single_paragraph_if_needed(sub_parts.join(" "), chunk, options);
            let mut val3 = validate_chunk_with_mode(chunk, &combined, options, mode, intensity);
            if all_sub_units_valid
                && val3.verbatim_echo_flag
                && val3.sequence_similarity <= 0.84
                && chunk.invariants.enforced_items(options).len() >= 3
            {
                val3.verbatim_echo_flag = false;
            }
            if attempt_score(&val3) > attempt_score(&best_val) {
                best_text = combined;
                best_val = val3;
            }
        }
    } else {
        let dir3 = translate_directive_for_masked(
            RetryDirective::DecompositionRetry {
                missing_items: best_val.missing_invariants.clone(),
                polarity_hint: best_val.polarity_reason.clone(),
            },
            &masked,
        );
        let bundle3 = build_prompt_bundle(
            prompt_text,
            prompt_words,
            mode,
            intensity,
            options,
            prev_tail,
            &dir3,
            disable_thinking,
            false,
        )
        .map_err(EngineError::InvalidInput)?;

        match client
            .chat_completion(ChatCompletionRequest::from_bundle(&bundle3), cancel)
            .await
        {
            Ok(raw3) => {
                let san3 = sanitize_model_output(&raw3, prev_tail, &bundle3.delimiter_tag);
                let norm3 = normalize_single_paragraph_if_needed(san3.text, chunk, options);
                let rest3 = restore_masked_tokens(&norm3, &masked, &chunk.text);
                let mut val3 =
                    validate_chunk_with_mode(chunk, &rest3.text, options, mode, intensity);
                apply_restore_and_syntax_checks(&mut val3, prompt_text, &norm3, &rest3);
                if attempt_score(&val3) > attempt_score(&best_val) {
                    best_text = rest3.text;
                    best_val = val3;
                }
            }
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(_) => {}
        }
    }

    Ok((best_text, best_val, true))
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

    let initial_directive = if wc >= 6 {
        RetryDirective::EchoRetry { hint: None }
    } else {
        RetryDirective::None
    };

    let (best_text, best_val, _) = paraphrase_chunk_with_strategies(
        client,
        &chunk,
        req.mode,
        req.intensity,
        &req.options,
        ctx_opt,
        initial_directive,
        disable_thinking,
        cancel,
        || {},
    )
    .await?;

    let warnings = collect_chunk_warnings(&best_val);
    if is_fatal_chunk_failure(&best_val) {
        let reason = format_chunk_failure_reason(req.chunk_index, &best_val, &warnings);
        return Err(EngineError::InvalidInput(reason));
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
        for p in 0..18 {
            let s = format!(
                "Paragraph {p} documents $1,250 savings across 12 regional centers in 2026. \
                 Indicator {p} showed throughput stabilized during cycle {}. \
                 Regional operators confirmed steady delivery across local hubs.",
                100 + p
            );
            paras.push(s);
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
                    // Attempt 2 must receive elevated temp >= 0.80, presence_penalty >= 0.55, and structural pivot directive
                    assert!(req.temperature >= 0.80, "temp={}", req.temperature);
                    assert!(req.presence_penalty >= 0.55, "pres={}", req.presence_penalty);
                    assert!(req.messages[0].content.contains("STRUCTURAL PIVOT REQUIRED"));
                }
                // Produce genuine structural rewrite preserving numbers/currency/identifiers
                let p_num = src
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("0")
                    .to_string();
                let cyc = src
                    .split("cycle ")
                    .nth(1)
                    .and_then(|s| s.split('.').next())
                    .unwrap_or("100");
                Ok(format!(
                    "In 2026, 12 regional centers realized $1,250 in cost reductions according to Paragraph {p_num}. \
                     During phase {cyc}, telemetry from Indicator {p_num} demonstrated consistent throughput stability. \
                     Steady delivery throughout local hubs was subsequently verified by field operators."
                ))
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
        assert!(!resp.failed);
        assert_eq!(resp.chunks_completed, resp.chunks_total);
        assert_eq!(resp.chunks_total, 18);
        assert_eq!(resp.chunks_retried, 1);
        assert!(resp.output_text.contains("```rust\nfn keep_intact() {\n\n    println!(\"ok\");\n}\n```"));
        assert_eq!(events.first().unwrap().state, HumanizeProgressState::Segmenting);
        assert_eq!(events.last().unwrap().state, HumanizeProgressState::Completed);
    }

    #[tokio::test]
    async fn persistent_echo_runs_three_strategies_and_fails_explicitly_without_passthrough() {
        let src = "Local execution keeps every confidential manuscript strictly inside the user workspace without touching remote endpoints.";
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, req| Ok(extract_source_from_user_msg(&req.messages[1].content)),
        };
        let coord = EngineCoordinator::new();
        let mut events = Vec::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "echo-fail".to_string(),
                text: src.to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |ev| events.push(ev),
        )
        .await
        .unwrap();

        assert!(resp.failed, "persistent echo must fail explicitly, never report success");
        assert!(!resp.cancelled);
        assert!(resp.output_text.is_empty(), "must NOT silently return original text");
        assert!(resp.error_message.as_deref().unwrap_or("").contains("chunk 1"));
        assert_eq!(resp.chunks_retried, 1);
        assert_eq!(client.calls.load(Ordering::SeqCst), 3, "must attempt all 3 strategies");
        assert_eq!(events.last().unwrap().state, HumanizeProgressState::Error);
    }

    #[tokio::test]
    async fn strategy_three_sentence_decomposition_rescues_stubborn_multi_sentence_chunk() {
        let src = "Engineers monitored server latency across twelve clusters. Operators prevented outages without external cloud tools.";
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |call_idx, req| {
                let inner = extract_source_from_user_msg(&req.messages[1].content);
                if call_idx < 2 {
                    // Strategy 1 & Strategy 2 produce shallow 1-to-1 synonym swap
                    return Ok("Engineers tracked server latency across twelve clusters. Operators avoided outages without external cloud tools.".to_string());
                }
                // Strategy 3 decomposes into individual sentences with CLAUSE-LEVEL RECASTING
                assert!(req.messages[0].content.contains("CLAUSE-LEVEL RECASTING"));
                if inner.contains("latency") {
                    Ok("Across twelve clusters, server latency remained under close observation by engineers.".to_string())
                } else {
                    Ok("Without relying on external cloud services, operators successfully prevented service outages.".to_string())
                }
            },
        };
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "strat3-rescue".to_string(),
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

        assert!(!resp.failed);
        assert!(!resp.cancelled);
        assert_eq!(resp.chunks_retried, 1);
        assert!(resp.output_text.contains("Across twelve clusters"));
        assert!(resp.output_text.contains("Without relying on external cloud services"));
    }

    #[tokio::test]
    async fn mid_flight_and_pre_cancellation_work_cleanly_without_placeholders() {
        let client = Arc::new(MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::from_millis(80),
            responder: |_, _| {
                Ok("Ten clear words appear inside this concise example sentence.".to_string())
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
        assert!(!next_resp.failed);
        assert!(next_resp.output_text.contains("example"));
    }

    #[tokio::test]
    async fn retry_chunk_recovers_single_chunk_with_context_and_fails_on_persistent_echo() {
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, req| {
                assert!(req.messages[0].content.contains("Preceding context here."));
                Ok("Achieving 99.5% uptime across 42 nodes, the cluster outperformed earlier runs.".to_string())
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

        // Persistent echo on retry_chunk must return Err, never original chunk
        let echo_client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, req| Ok(extract_source_from_user_msg(&req.messages[1].content)),
        };
        let err_res = retry_chunk_impl(
            &echo_client,
            &coord,
            RetryChunkRequest {
                request_id: "retry-fail".to_string(),
                chunk_index: 0,
                original_chunk: "Original phrasing noting 42 nodes achieved 99.5% uptime.".to_string(),
                context_before: String::new(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
        )
        .await;
        assert!(err_res.is_err(), "persistent echo in retry_chunk must return Err");
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

    #[tokio::test]
    async fn masks_and_restores_doi_url_email_across_retries_and_rejects_corrupted_placeholder() {
        let src = "## 1. Audit Summary\n\nAudit findings documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4 (contact qa@example.org) confirmed 18.4% lower latency across regional nodes.";
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |call_idx, req| {
                let inner = extract_source_from_user_msg(&req.messages[1].content);
                assert!(inner.contains("[[URL_1]]"), "URL must be masked in prompt: {inner}");
                assert!(inner.contains("[[DOI_1]]"), "DOI must be masked in prompt: {inner}");
                assert!(inner.contains("[[EMAIL_1]]"), "EMAIL must be masked in prompt: {inner}");
                assert!(!inner.contains("10.1038/s41586-024-07123-4"));
                if call_idx == 0 {
                    // Attempt 1 drops [[DOI_1]] and leaks [[DOI_2]]
                    Ok("Across regional nodes, latency dropped by 18.4% as logged at [[URL_1]] and [[DOI_2]] via [[EMAIL_1]].".to_string())
                } else {
                    // Retry must ask for [[DOI_1]] instead of raw DOI
                    assert!(req.messages[0].content.contains("[[DOI_1]]"));
                    Ok("Across regional nodes, latency declined by 18.4%, according to records at [[URL_1]] and [[DOI_1]] (inquiries: [[EMAIL_1]]).".to_string())
                }
            },
        };
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "mask-restore-1".to_string(),
                text: src.to_string(),
                mode: RewriteMode::Academic,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();

        assert!(!resp.failed);
        assert_eq!(resp.chunks_total, 1);
        assert_eq!(resp.chunks_retried, 1);
        assert!(resp.output_text.starts_with("## 1. Audit Summary\n\n"));
        assert!(resp.output_text.contains("https://example.org/audits/2025"));
        assert!(resp.output_text.contains("10.1038/s41586-024-07123-4"));
        assert!(resp.output_text.contains("qa@example.org"));
        assert!(!resp.output_text.contains("[[URL_"));
        assert!(!resp.output_text.contains("[[DOI_"));
        assert!(!resp.output_text.contains("[[EMAIL_"));
    }

    #[tokio::test]
    async fn strategy_three_sub_unit_retry_rescues_stubborn_sentence_in_multi_sentence_chunk() {
        let src = "First, operators audited backup telemetry across twelve regional hubs. Second, engineers verified failover switches without external cloud endpoints.";
        let client = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |call_idx, req| {
                let inner = extract_source_from_user_msg(&req.messages[1].content);
                if call_idx < 2 {
                    // Attempts 1 & 2 echo near-verbatim
                    return Ok("First, operators reviewed backup telemetry across twelve regional hubs. Second, engineers checked failover switches without external cloud endpoints.".to_string());
                }
                if inner.contains("telemetry") {
                    // Sentence 1 paraphrases cleanly on first try
                    Ok("Across twelve regional hubs, operators began by auditing backup telemetry.".to_string())
                } else if call_idx == 3 {
                    // Sentence 2 echoes on first Strategy 3 pass
                    Ok("Second, engineers checked failover switches without external cloud endpoints.".to_string())
                } else {
                    // Sub-unit retry rescues Sentence 2
                    Ok("Without relying on external cloud endpoints, engineers next verified every failover switch.".to_string())
                }
            },
        };
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            HumanizeRequest {
                request_id: "sub-retry-1".to_string(),
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

        assert!(!resp.failed);
        assert_eq!(resp.chunks_retried, 1);
        assert!(resp.output_text.contains("Without relying on external cloud endpoints"));
    }

    #[tokio::test]
    async fn keeps_quotes_unmasked_rejects_duplicated_placeholder_and_fails_on_persistent_dropped_citation() {
        let src = "On January 14, 2025, researchers at Northbridge Labs reported 18.4% lower latency (Smith & Patel, 2024, pp. 42–45). According to the audit [12], \"exact quoted statements must remain unaltered during automated processing\" as documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4.";

        // 1) Quotes remain unmasked, duplicated [[DOI_1]] + bad grammar triggers retry and recovers cleanly
        let client_recover = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |call_idx, req| {
                let inner = extract_source_from_user_msg(&req.messages[1].content);
                assert!(inner.contains("\"exact quoted statements must remain unaltered during automated processing\""), "quotes must stay unmasked: {inner}");
                assert!(!inner.contains("[[QUOTE_"), "must not mask quotes into [[QUOTE_1]]: {inner}");
                assert!(inner.contains("[[URL_1]]"));
                assert!(inner.contains("[[DOI_1]]"));
                if call_idx == 0 {
                    // Attempt 1 duplicates [[DOI_1]], uses it as subject, and drops [12]
                    Ok("On January 14, 2025, Northbridge Labs lowered latency by 18.4% (Smith & Patel, 2024, pp. 42–45). [[DOI_1]] documents \"exact quoted statements must remain unaltered during automated processing\" at [[URL_1]] and [[DOI_1]].".to_string())
                } else {
                    // Retry prompt must explicitly name missing [12]
                    assert!(req.messages[0].content.contains("[12]"));
                    assert!(req.messages[1].content.contains("[12]"));
                    Ok("Achieving an 18.4% latency drop, Northbridge Labs reported the finding on January 14, 2025 (Smith & Patel, 2024, pp. 42–45). As documented at [[URL_1]] and [[DOI_1]], the audit [12] notes that \"exact quoted statements must remain unaltered during automated processing\".".to_string())
                }
            },
        };
        let coord = EngineCoordinator::new();
        let ok_resp = humanize_document_impl(
            &client_recover,
            &coord,
            HumanizeRequest {
                request_id: "quote-unmask-recover".to_string(),
                text: src.to_string(),
                mode: RewriteMode::Academic,
                intensity: Intensity::Balanced,
                options: opts(),
            },
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert!(!ok_resp.failed, "expected recovery, got {:?} {:?}", ok_resp.error_message, ok_resp.warnings);
        assert_eq!(ok_resp.chunks_retried, 1);
        assert!(ok_resp.output_text.contains("[12]"));
        assert_eq!(ok_resp.output_text.matches("10.1038/s41586-024-07123-4").count(), 1);

        // 2) Persistent dropped [12] across all strategies MUST fail explicitly (never failed=false)
        let client_drop = MockClient {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
            responder: |_, req| {
                let inner = extract_source_from_user_msg(&req.messages[1].content);
                if inner.contains("Northbridge Labs") && !inner.contains("[[URL_1]]") {
                    Ok("Achieving an 18.4% latency drop, Northbridge Labs reported the finding on January 14, 2025 (Smith & Patel, 2024, pp. 42–45).".to_string())
                } else if inner.contains("[[URL_1]]") && !inner.contains("Northbridge Labs") {
                    Ok("As documented at [[URL_1]] and [[DOI_1]], the audit notes that \"exact quoted statements must remain unaltered during automated processing\".".to_string())
                } else {
                    Ok("Achieving an 18.4% latency drop, Northbridge Labs reported the finding on January 14, 2025 (Smith & Patel, 2024, pp. 42–45). As documented at [[URL_1]] and [[DOI_1]], the audit notes that \"exact quoted statements must remain unaltered during automated processing\".".to_string())
                }
            },
        };
        let fail_resp = humanize_document_impl(
            &client_drop,
            &coord,
            HumanizeRequest {
                request_id: "drop-cite-fatal".to_string(),
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
        assert!(fail_resp.failed, "persistent dropped [12] must be fatal");
        assert!(fail_resp.output_text.is_empty());
        assert!(fail_resp.error_message.as_deref().unwrap_or("").contains("[12]"));
    }
}
