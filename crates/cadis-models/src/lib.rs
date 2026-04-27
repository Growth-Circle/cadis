//! Model provider adapters for CADIS.

use std::env;
use std::error::Error;
use std::fmt;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use reqwest::StatusCode;

const CODEX_CLI_DEFAULT_BIN: &str = "codex";
const CODEX_CLI_TIMEOUT: Duration = Duration::from_secs(300);
const CONFIGURED_MODEL: &str = "configured";
const ECHO_MODEL: &str = "cadis-local-fallback";
const CODEX_CLI_PLAN_MODEL: &str = "chatgpt-plan";

/// Streaming-oriented model provider interface used by the runtime.
pub trait ModelProvider: Send + Sync {
    /// Provider label.
    fn name(&self) -> &str;

    /// Returns model output split into display deltas.
    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError>;

    /// Returns model output plus normalized provider/model invocation metadata.
    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        let deltas = self.chat(request.prompt)?;
        Ok(ModelResponse {
            deltas,
            invocation: ModelInvocation {
                requested_model: request.selected_model.map(ToOwned::to_owned),
                effective_provider: self.name().to_owned(),
                effective_model: request
                    .selected_model
                    .unwrap_or(CONFIGURED_MODEL)
                    .to_owned(),
                fallback: false,
                fallback_reason: None,
            },
        })
    }

    /// Streams normalized provider events. Providers without native streaming use chunk simulation.
    fn stream_chat(
        &self,
        request: ModelRequest<'_>,
        callback: &mut ModelStreamCallback<'_>,
    ) -> Result<ModelResponse, ModelError> {
        match self.chat_with_request(request) {
            Ok(response) => stream_response(response, callback),
            Err(error) => fail_stream(callback, error),
        }
    }
}

/// Callback used by providers to stream normalized model events.
pub type ModelStreamCallback<'a> =
    dyn FnMut(ModelStreamEvent) -> Result<ModelStreamControl, ModelError> + 'a;

/// Callback control returned to a provider after each streamed event.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ModelStreamControl {
    /// Continue streaming more provider events.
    #[default]
    Continue,
    /// Stop the provider request and return a normalized cancellation error.
    Cancel,
}

/// Model request context supplied by the runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelRequest<'a> {
    /// Prompt text sent to the selected provider.
    pub prompt: &'a str,
    /// Optional provider/model ID selected on the routed agent.
    pub selected_model: Option<&'a str>,
}

impl<'a> ModelRequest<'a> {
    /// Creates a model request without an explicit selected model.
    pub fn new(prompt: &'a str) -> Self {
        Self {
            prompt,
            selected_model: None,
        }
    }

    /// Adds an optional selected model ID.
    pub fn with_selected_model(mut self, selected_model: Option<&'a str>) -> Self {
        self.selected_model = selected_model;
        self
    }
}

/// Normalized provider/model invocation metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelInvocation {
    /// Requested provider/model ID, when supplied by an agent.
    pub requested_model: Option<String>,
    /// Provider that actually served the request.
    pub effective_provider: String,
    /// Model that actually served the request.
    pub effective_model: String,
    /// Whether the response came from a fallback provider.
    pub fallback: bool,
    /// Redacted fallback reason, when fallback occurred.
    pub fallback_reason: Option<String>,
}

/// Completed model response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResponse {
    /// Display deltas in order.
    pub deltas: Vec<String>,
    /// Provider/model metadata for this response.
    pub invocation: ModelInvocation,
}

/// Normalized streaming event emitted by model providers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelStreamEvent {
    /// Provider invocation started.
    Started(ModelInvocation),
    /// Provider emitted a text delta.
    Delta(String),
    /// Provider invocation completed.
    Completed(ModelInvocation),
    /// Provider invocation failed.
    Failed(ModelFailure),
    /// Provider invocation was cancelled before completion.
    Cancelled(ModelInvocation),
}

/// Structured model failure metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelFailure {
    /// Stable error code.
    pub code: String,
    /// Redacted human-readable message.
    pub message: String,
    /// Whether retrying may help.
    pub retryable: bool,
    /// Provider/model metadata when resolution happened before failure.
    pub invocation: Option<ModelInvocation>,
}

/// Model provider error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelError {
    code: String,
    message: String,
    retryable: bool,
    invocation: Option<Box<ModelInvocation>>,
}

impl ModelError {
    /// Creates a redaction-ready provider error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: "model_error".to_owned(),
            message: message.into(),
            retryable: true,
            invocation: None,
        }
    }

    /// Creates a structured provider error.
    pub fn with_code(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            invocation: None,
        }
    }

    /// Creates a normalized provider cancellation error.
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::with_code("model_cancelled", message, false)
    }

    /// Attaches provider/model invocation metadata.
    pub fn with_invocation(mut self, invocation: ModelInvocation) -> Self {
        self.invocation = Some(Box::new(invocation));
        self
    }

    /// Returns the stable error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether retrying may help.
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    /// Returns provider/model invocation metadata when available.
    pub fn invocation(&self) -> Option<&ModelInvocation> {
        self.invocation.as_deref()
    }

    /// Returns whether this error represents provider-boundary cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.code == "model_cancelled"
    }

    fn with_invocation_if_missing(mut self, invocation: &ModelInvocation) -> Self {
        if self.invocation.is_none() {
            self.invocation = Some(Box::new(invocation.clone()));
        }
        self
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ModelError {}

fn provider_http_error(provider: &str, status: StatusCode) -> ModelError {
    let is_auth_error = status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN;
    let (code, retryable) = if is_auth_error {
        ("model_auth_failed", false)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        ("provider_rate_limited", true)
    } else if status == StatusCode::NOT_FOUND {
        ("model_not_found", false)
    } else if status.is_server_error() {
        ("provider_unavailable", true)
    } else if status.is_client_error() {
        ("model_request_rejected", false)
    } else {
        ("provider_http_error", false)
    };

    ModelError::with_code(
        code,
        format!("{provider} returned HTTP status {status}"),
        retryable,
    )
}

fn emit_stream_event(
    callback: &mut ModelStreamCallback<'_>,
    event: ModelStreamEvent,
    invocation: &ModelInvocation,
) -> Result<ModelStreamControl, ModelError> {
    callback(event).map_err(|error| error.with_invocation_if_missing(invocation))
}

fn stream_response(
    response: ModelResponse,
    callback: &mut ModelStreamCallback<'_>,
) -> Result<ModelResponse, ModelError> {
    if emit_stream_event(
        callback,
        ModelStreamEvent::Started(response.invocation.clone()),
        &response.invocation,
    )? == ModelStreamControl::Cancel
    {
        return cancel_stream(callback, &response.invocation);
    }

    for delta in &response.deltas {
        if emit_stream_event(
            callback,
            ModelStreamEvent::Delta(delta.clone()),
            &response.invocation,
        )? == ModelStreamControl::Cancel
        {
            return cancel_stream(callback, &response.invocation);
        }
    }

    if emit_stream_event(
        callback,
        ModelStreamEvent::Completed(response.invocation.clone()),
        &response.invocation,
    )? == ModelStreamControl::Cancel
    {
        return cancel_stream(callback, &response.invocation);
    }

    Ok(response)
}

fn cancel_stream(
    callback: &mut ModelStreamCallback<'_>,
    invocation: &ModelInvocation,
) -> Result<ModelResponse, ModelError> {
    let _ = emit_stream_event(
        callback,
        ModelStreamEvent::Cancelled(invocation.clone()),
        invocation,
    )?;
    Err(ModelError::cancelled("model request was cancelled").with_invocation(invocation.clone()))
}

fn fail_stream(
    callback: &mut ModelStreamCallback<'_>,
    error: ModelError,
) -> Result<ModelResponse, ModelError> {
    if error.is_cancelled() {
        if let Some(invocation) = error.invocation.as_deref() {
            let _ = callback(ModelStreamEvent::Cancelled(invocation.clone()))?;
        }
        return Err(error);
    }

    let _ = callback(ModelStreamEvent::Failed(ModelFailure {
        code: error.code.clone(),
        message: error.message.clone(),
        retryable: error.retryable,
        invocation: error.invocation.as_deref().cloned(),
    }))?;
    Err(error)
}

/// Conservative provider readiness exposed by the model catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderReadiness {
    /// Provider is expected to be usable without known additional setup.
    Ready,
    /// Provider entry is a local fallback rather than a real model provider.
    Fallback,
    /// Provider requires credentials, login, or a local service.
    RequiresConfiguration,
    /// Provider is known unavailable.
    Unavailable,
}

/// Metadata for one model catalog entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCatalogEntry {
    /// Provider name clients can select.
    pub provider: String,
    /// Model identifier clients can select.
    pub model: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Capability labels.
    pub capabilities: Vec<String>,
    /// Conservative readiness state.
    pub readiness: ProviderReadiness,
    /// Provider expected to serve requests for this entry.
    pub effective_provider: String,
    /// Model expected to serve requests for this entry.
    pub effective_model: String,
    /// Whether this entry is a local fallback rather than a real provider.
    pub fallback: bool,
}

/// Runtime configuration used to build the client-visible provider catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCatalogConfig {
    /// Default provider from `[model].provider`.
    pub default_provider: String,
    /// Configured Ollama model.
    pub ollama_model: String,
    /// Configured OpenAI model.
    pub openai_model: String,
    /// Whether an OpenAI API key is present in the daemon environment.
    pub openai_api_key_configured: bool,
}

impl ModelCatalogConfig {
    /// Creates catalog config from daemon model settings.
    pub fn new(
        default_provider: impl Into<String>,
        ollama_model: impl Into<String>,
        openai_model: impl Into<String>,
        openai_api_key_configured: bool,
    ) -> Self {
        Self {
            default_provider: normalize_provider_id(&default_provider.into()).to_owned(),
            ollama_model: catalog_model_or_configured(ollama_model.into()),
            openai_model: catalog_model_or_configured(openai_model.into()),
            openai_api_key_configured,
        }
    }
}

impl Default for ModelCatalogConfig {
    fn default() -> Self {
        Self::new("auto", "llama3.2", "gpt-5.2", false)
    }
}

/// Returns the conservative built-in provider catalog using default settings.
pub fn provider_catalog() -> Vec<ProviderCatalogEntry> {
    provider_catalog_for_config(&ModelCatalogConfig::default())
}

/// Returns the conservative built-in provider catalog for daemon settings.
pub fn provider_catalog_for_config(config: &ModelCatalogConfig) -> Vec<ProviderCatalogEntry> {
    let default_provider = normalize_provider_id(&config.default_provider);
    let auto_readiness = if default_provider == "auto" {
        ProviderReadiness::Fallback
    } else {
        ProviderReadiness::RequiresConfiguration
    };
    let openai_readiness = if config.openai_api_key_configured {
        ProviderReadiness::Ready
    } else {
        ProviderReadiness::RequiresConfiguration
    };

    vec![
        ProviderCatalogEntry {
            provider: "auto".to_owned(),
            model: config.ollama_model.clone(),
            display_name: format!("Auto (Ollama {}, then local fallback)", config.ollama_model),
            capabilities: vec!["streaming".to_owned(), "local_fallback".to_owned()],
            readiness: auto_readiness,
            effective_provider: "ollama".to_owned(),
            effective_model: config.ollama_model.clone(),
            fallback: true,
        },
        ProviderCatalogEntry {
            provider: "ollama".to_owned(),
            model: config.ollama_model.clone(),
            display_name: format!("Ollama {}", config.ollama_model),
            capabilities: vec!["streaming".to_owned(), "local_model".to_owned()],
            readiness: ProviderReadiness::RequiresConfiguration,
            effective_provider: "ollama".to_owned(),
            effective_model: config.ollama_model.clone(),
            fallback: false,
        },
        ProviderCatalogEntry {
            provider: "codex-cli".to_owned(),
            model: "chatgpt-plan".to_owned(),
            display_name: "Codex CLI (ChatGPT Plus/Pro login)".to_owned(),
            capabilities: vec![
                "codex_cli".to_owned(),
                "chatgpt_login".to_owned(),
                "read_only_sandbox".to_owned(),
            ],
            readiness: ProviderReadiness::RequiresConfiguration,
            effective_provider: "codex-cli".to_owned(),
            effective_model: "chatgpt-plan".to_owned(),
            fallback: false,
        },
        ProviderCatalogEntry {
            provider: "openai".to_owned(),
            model: config.openai_model.clone(),
            display_name: format!("OpenAI {}", config.openai_model),
            capabilities: vec!["api_key".to_owned(), "streaming".to_owned()],
            readiness: openai_readiness,
            effective_provider: "openai".to_owned(),
            effective_model: config.openai_model.clone(),
            fallback: false,
        },
        ProviderCatalogEntry {
            provider: "echo".to_owned(),
            model: "cadis-local-fallback".to_owned(),
            display_name: "CADIS local fallback".to_owned(),
            capabilities: vec!["offline".to_owned()],
            readiness: ProviderReadiness::Fallback,
            effective_provider: "echo".to_owned(),
            effective_model: "cadis-local-fallback".to_owned(),
            fallback: true,
        },
    ]
}

/// Local provider that requires no network or credentials.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EchoProvider;

impl ModelProvider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        let response = format!(
            "CADIS runtime is online. I received: {prompt}\n\nConfigure Ollama in ~/.cadis/config.toml to use a local model for real assistant responses."
        );
        Ok(chunk_text(&response))
    }

    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        Ok(ModelResponse {
            deltas: self.chat(request.prompt)?,
            invocation: ModelInvocation {
                requested_model: request.selected_model.map(ToOwned::to_owned),
                effective_provider: "echo".to_owned(),
                effective_model: ECHO_MODEL.to_owned(),
                fallback: false,
                fallback_reason: None,
            },
        })
    }
}

/// Ollama local model provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OllamaProvider {
    endpoint: String,
    model: String,
}

impl OllamaProvider {
    /// Creates an Ollama provider.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
            model: model.into(),
        }
    }
}

impl ModelProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| {
                ModelError::with_code(
                    "provider_client_error",
                    format!("failed to create Ollama client: {error}"),
                    false,
                )
            })?;

        let response = client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&OllamaGenerateRequest {
                model: self.model.clone(),
                prompt: prompt.to_owned(),
                stream: false,
            })
            .send()
            .map_err(|_| {
                ModelError::with_code("provider_unavailable", "Ollama request failed", true)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(provider_http_error("Ollama", status));
        }

        let body = response.json::<OllamaGenerateResponse>().map_err(|error| {
            ModelError::with_code(
                "provider_response_invalid",
                format!("Ollama response was invalid: {error}"),
                false,
            )
        })?;

        if let Some(error) = body.error {
            return Err(ModelError::with_code(
                "model_request_rejected",
                format!("Ollama error: {error}"),
                false,
            ));
        }

        Ok(chunk_text(body.response.as_deref().unwrap_or_default()))
    }

    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        let invocation = ModelInvocation {
            requested_model: request.selected_model.map(ToOwned::to_owned),
            effective_provider: "ollama".to_owned(),
            effective_model: self.model.clone(),
            fallback: false,
            fallback_reason: None,
        };
        self.chat(request.prompt)
            .map(|deltas| ModelResponse {
                deltas,
                invocation: invocation.clone(),
            })
            .map_err(|error| error.with_invocation(invocation))
    }

    fn stream_chat(
        &self,
        request: ModelRequest<'_>,
        callback: &mut ModelStreamCallback<'_>,
    ) -> Result<ModelResponse, ModelError> {
        let invocation = ModelInvocation {
            requested_model: request.selected_model.map(ToOwned::to_owned),
            effective_provider: "ollama".to_owned(),
            effective_model: self.model.clone(),
            fallback: false,
            fallback_reason: None,
        };

        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                return fail_stream(
                    callback,
                    ModelError::with_code(
                        "provider_client_error",
                        format!("failed to create Ollama client: {error}"),
                        false,
                    )
                    .with_invocation(invocation),
                );
            }
        };

        let response = match client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&OllamaGenerateRequest {
                model: self.model.clone(),
                prompt: request.prompt.to_owned(),
                stream: true,
            })
            .send()
        {
            Ok(response) => response,
            Err(_) => {
                return fail_stream(
                    callback,
                    ModelError::with_code("provider_unavailable", "Ollama request failed", true)
                        .with_invocation(invocation),
                );
            }
        };

        let status = response.status();
        if !status.is_success() {
            return fail_stream(
                callback,
                provider_http_error("Ollama", status).with_invocation(invocation),
            );
        }

        if emit_stream_event(
            callback,
            ModelStreamEvent::Started(invocation.clone()),
            &invocation,
        )? == ModelStreamControl::Cancel
        {
            return cancel_stream(callback, &invocation);
        }

        let mut deltas = Vec::new();
        for line in BufReader::new(response).lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    return fail_stream(
                        callback,
                        ModelError::with_code(
                            "provider_response_invalid",
                            format!("failed to read Ollama stream: {error}"),
                            true,
                        )
                        .with_invocation(invocation),
                    );
                }
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let chunk = match serde_json::from_str::<OllamaGenerateResponse>(line) {
                Ok(chunk) => chunk,
                Err(error) => {
                    return fail_stream(
                        callback,
                        ModelError::with_code(
                            "provider_response_invalid",
                            format!("Ollama stream chunk was invalid: {error}"),
                            false,
                        )
                        .with_invocation(invocation),
                    );
                }
            };

            if let Some(error) = chunk.error {
                return fail_stream(
                    callback,
                    ModelError::with_code(
                        "model_request_rejected",
                        format!("Ollama error: {error}"),
                        false,
                    )
                    .with_invocation(invocation),
                );
            }

            if let Some(delta) = chunk.response.filter(|delta| !delta.is_empty()) {
                deltas.push(delta.clone());
                if emit_stream_event(callback, ModelStreamEvent::Delta(delta), &invocation)?
                    == ModelStreamControl::Cancel
                {
                    return cancel_stream(callback, &invocation);
                }
            }

            if chunk.done.unwrap_or(false) {
                break;
            }
        }

        if deltas.is_empty() {
            deltas.push(String::new());
        }

        let response = ModelResponse {
            deltas,
            invocation: invocation.clone(),
        };
        if emit_stream_event(
            callback,
            ModelStreamEvent::Completed(invocation.clone()),
            &invocation,
        )? == ModelStreamControl::Cancel
        {
            return cancel_stream(callback, &invocation);
        }
        Ok(response)
    }
}

/// OpenAI Chat Completions provider.
pub struct OpenAiProvider {
    base_url: String,
    model: String,
    api_key: String,
}

impl OpenAiProvider {
    /// Creates an OpenAI provider.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            model: model.into(),
            api_key: api_key.into(),
        }
    }
}

impl fmt::Debug for OpenAiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl ModelProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        if self.api_key.trim().is_empty() {
            return Err(ModelError::with_code(
                "model_auth_missing",
                "OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY",
                false,
            ));
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| {
                ModelError::with_code(
                    "provider_client_error",
                    "failed to create OpenAI client",
                    false,
                )
            })?;

        let response = client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&OpenAiChatRequest {
                model: self.model.clone(),
                messages: vec![OpenAiChatMessage {
                    role: "user",
                    content: prompt,
                }],
                stream: false,
            })
            .send()
            .map_err(|_| {
                ModelError::with_code("provider_unavailable", "OpenAI request failed", true)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(provider_http_error("OpenAI", status));
        }

        let body = response.json::<OpenAiChatResponse>().map_err(|_| {
            ModelError::with_code(
                "provider_response_invalid",
                "OpenAI response was invalid",
                false,
            )
        })?;
        let content = body
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .unwrap_or_default();

        if content.is_empty() {
            return Err(ModelError::with_code(
                "provider_response_empty",
                "OpenAI response was empty",
                true,
            ));
        }

        Ok(chunk_text(content))
    }

    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        let invocation = ModelInvocation {
            requested_model: request.selected_model.map(ToOwned::to_owned),
            effective_provider: "openai".to_owned(),
            effective_model: self.model.clone(),
            fallback: false,
            fallback_reason: None,
        };
        self.chat(request.prompt)
            .map(|deltas| ModelResponse {
                deltas,
                invocation: invocation.clone(),
            })
            .map_err(|error| error.with_invocation(invocation))
    }

    fn stream_chat(
        &self,
        request: ModelRequest<'_>,
        callback: &mut ModelStreamCallback<'_>,
    ) -> Result<ModelResponse, ModelError> {
        let invocation = ModelInvocation {
            requested_model: request.selected_model.map(ToOwned::to_owned),
            effective_provider: "openai".to_owned(),
            effective_model: self.model.clone(),
            fallback: false,
            fallback_reason: None,
        };

        if self.api_key.trim().is_empty() {
            return fail_stream(
                callback,
                ModelError::with_code(
                    "model_auth_missing",
                    "OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY",
                    false,
                )
                .with_invocation(invocation),
            );
        }

        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
        {
            Ok(client) => client,
            Err(_) => {
                return fail_stream(
                    callback,
                    ModelError::with_code(
                        "provider_client_error",
                        "failed to create OpenAI client",
                        false,
                    )
                    .with_invocation(invocation),
                );
            }
        };

        let response = match client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&OpenAiChatRequest {
                model: self.model.clone(),
                messages: vec![OpenAiChatMessage {
                    role: "user",
                    content: request.prompt,
                }],
                stream: true,
            })
            .send()
        {
            Ok(response) => response,
            Err(_) => {
                return fail_stream(
                    callback,
                    ModelError::with_code("provider_unavailable", "OpenAI request failed", true)
                        .with_invocation(invocation),
                );
            }
        };

        let status = response.status();
        if !status.is_success() {
            return fail_stream(
                callback,
                provider_http_error("OpenAI", status).with_invocation(invocation),
            );
        }

        if emit_stream_event(
            callback,
            ModelStreamEvent::Started(invocation.clone()),
            &invocation,
        )? == ModelStreamControl::Cancel
        {
            return cancel_stream(callback, &invocation);
        }

        let mut deltas = Vec::new();
        for line in BufReader::new(response).lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    return fail_stream(
                        callback,
                        ModelError::with_code(
                            "provider_response_invalid",
                            format!("failed to read OpenAI stream: {error}"),
                            true,
                        )
                        .with_invocation(invocation),
                    );
                }
            };
            let line = line.trim();
            if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
                continue;
            }
            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }

            let chunk = match serde_json::from_str::<OpenAiChatStreamChunk>(data) {
                Ok(chunk) => chunk,
                Err(error) => {
                    return fail_stream(
                        callback,
                        ModelError::with_code(
                            "provider_response_invalid",
                            format!("OpenAI stream chunk was invalid: {error}"),
                            false,
                        )
                        .with_invocation(invocation),
                    );
                }
            };

            for choice in chunk.choices {
                if let Some(delta) = choice.delta.content.filter(|delta| !delta.is_empty()) {
                    deltas.push(delta.clone());
                    if emit_stream_event(callback, ModelStreamEvent::Delta(delta), &invocation)?
                        == ModelStreamControl::Cancel
                    {
                        return cancel_stream(callback, &invocation);
                    }
                }
            }
        }

        if deltas.is_empty() {
            return fail_stream(
                callback,
                ModelError::with_code("provider_response_empty", "OpenAI response was empty", true)
                    .with_invocation(invocation),
            );
        }

        let response = ModelResponse {
            deltas,
            invocation: invocation.clone(),
        };
        if emit_stream_event(
            callback,
            ModelStreamEvent::Completed(invocation.clone()),
            &invocation,
        )? == ModelStreamControl::Cancel
        {
            return cancel_stream(callback, &invocation);
        }
        Ok(response)
    }
}

/// Provider backed by the official Codex CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexCliProvider {
    command: CodexCliCommand,
    timeout: Duration,
}

impl CodexCliProvider {
    /// Creates a Codex CLI provider from CADIS environment overrides.
    pub fn from_env() -> Result<Self, ModelError> {
        Self::from_env_with_model(None)
    }

    /// Creates a Codex CLI provider with an optional per-request model override.
    pub fn from_env_with_model(model_override: Option<&str>) -> Result<Self, ModelError> {
        let mut command = CodexCliCommand::from_env()?;
        if let Some(model) = normalize_optional_model(model_override) {
            command.model = Some(model);
        }
        Ok(Self {
            command,
            timeout: CODEX_CLI_TIMEOUT,
        })
    }

    fn effective_model(&self) -> String {
        self.command
            .model
            .clone()
            .unwrap_or_else(|| CODEX_CLI_PLAN_MODEL.to_owned())
    }
}

impl ModelProvider for CodexCliProvider {
    fn name(&self) -> &str {
        "codex-cli"
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        let output = run_codex_exec(&self.command, prompt, self.timeout)?;
        if !output.status.success() {
            return Err(ModelError::with_code(
                "codex_cli_failed",
                format_codex_failure(&output),
                false,
            ));
        }

        Ok(chunk_text(output.stdout.trim()))
    }

    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        let invocation = ModelInvocation {
            requested_model: request.selected_model.map(ToOwned::to_owned),
            effective_provider: "codex-cli".to_owned(),
            effective_model: self.effective_model(),
            fallback: false,
            fallback_reason: None,
        };
        self.chat(request.prompt)
            .map(|deltas| ModelResponse {
                deltas,
                invocation: invocation.clone(),
            })
            .map_err(|error| error.with_invocation(invocation))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CodexCliCommand {
    bin: String,
    model: Option<String>,
    extra_args: Vec<String>,
}

impl CodexCliCommand {
    fn from_env() -> Result<Self, ModelError> {
        let bin = env::var("CADIS_CODEX_BIN")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CODEX_CLI_DEFAULT_BIN.to_owned());

        let model = env::var("CADIS_CODEX_MODEL")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let extra_args = env::var("CADIS_CODEX_EXTRA_ARGS")
            .ok()
            .map(|value| parse_extra_args(&value))
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            bin,
            model,
            extra_args,
        })
    }

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "exec".to_owned(),
            "--color".to_owned(),
            "never".to_owned(),
            "--skip-git-repo-check".to_owned(),
            "--ephemeral".to_owned(),
            "--sandbox".to_owned(),
            "read-only".to_owned(),
        ];

        if let Some(model) = &self.model {
            args.push("--model".to_owned());
            args.push(model.clone());
        }

        args.extend(self.extra_args.iter().cloned());
        args
    }
}

#[derive(Debug)]
struct CodexCliOutput {
    status: ExitStatus,
    stdout: String,
}

fn run_codex_exec(
    command: &CodexCliCommand,
    prompt: &str,
    timeout: Duration,
) -> Result<CodexCliOutput, ModelError> {
    let mut child = Command::new(&command.bin)
        .args(command.args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                ModelError::with_code(
                    "codex_cli_missing",
                    format!(
                    "Codex CLI binary '{}' was not found. Install the official Codex CLI, run `codex login`, or set CADIS_CODEX_BIN.",
                    command.bin
                    ),
                    false,
                )
            } else {
                ModelError::with_code(
                    "codex_cli_start_failed",
                    format!("failed to start Codex CLI '{}': {error}", command.bin),
                    false,
                )
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(prompt.as_bytes()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ModelError::with_code(
                    "codex_cli_io_error",
                    format!("failed to send prompt to Codex CLI: {error}"),
                    false,
                ));
            }
        }
    }

    let stdout = child.stdout.take().ok_or_else(|| {
        ModelError::with_code(
            "codex_cli_io_error",
            "failed to capture Codex CLI stdout",
            false,
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ModelError::with_code(
            "codex_cli_io_error",
            "failed to capture Codex CLI stderr",
            false,
        )
    })?;
    let stdout_reader = read_pipe(stdout);
    let stderr_reader = read_pipe(stderr);

    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            ModelError::with_code(
                "codex_cli_wait_failed",
                format!("failed to wait for Codex CLI: {error}"),
                false,
            )
        })? {
            let stderr = join_pipe(stderr_reader, "stderr")?;
            drop(stderr);
            return Ok(CodexCliOutput {
                status,
                stdout: join_pipe(stdout_reader, "stdout")?,
            });
        }

        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_pipe(stdout_reader, "stdout");
            let _ = join_pipe(stderr_reader, "stderr");
            return Err(ModelError::with_code(
                "codex_cli_timeout",
                format!(
                    "Codex CLI timed out after {} seconds while running `{} exec`.",
                    timeout.as_secs(),
                    command.bin
                ),
                true,
            ));
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn read_pipe<R>(mut reader: R) -> thread::JoinHandle<Result<String, std::io::Error>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output)?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    })
}

fn join_pipe(
    handle: thread::JoinHandle<Result<String, std::io::Error>>,
    label: &str,
) -> Result<String, ModelError> {
    handle
        .join()
        .map_err(|_| {
            ModelError::with_code(
                "codex_cli_io_error",
                format!("Codex CLI {label} reader panicked"),
                false,
            )
        })?
        .map_err(|error| {
            ModelError::with_code(
                "codex_cli_io_error",
                format!("failed to read Codex CLI {label}: {error}"),
                false,
            )
        })
}

fn parse_extra_args(input: &str) -> Result<Vec<String>, ModelError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = input.chars();

    while let Some(character) = chars.next() {
        match (character, quote) {
            ('\\', Some('\'')) => current.push(character),
            ('\\', _) => {
                if let Some(next) = chars.next() {
                    current.push(next);
                } else {
                    current.push(character);
                }
            }
            ('\'' | '"', None) => quote = Some(character),
            ('\'' | '"', Some(active_quote)) if character == active_quote => quote = None,
            (character, None) if character.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            (character, _) => current.push(character),
        }
    }

    if let Some(active_quote) = quote {
        return Err(ModelError::with_code(
            "codex_cli_args_invalid",
            format!("CADIS_CODEX_EXTRA_ARGS has an unterminated {active_quote} quote"),
            false,
        ));
    }

    if !current.is_empty() {
        args.push(current);
    }

    validate_codex_extra_args(&args)?;
    Ok(args)
}

fn validate_codex_extra_args(args: &[String]) -> Result<(), ModelError> {
    let blocked = [
        "--dangerously-bypass-approvals-and-sandbox",
        "--full-auto",
        "--sandbox",
        "-s",
        "--add-dir",
        "--cd",
        "-C",
        "--config",
        "-c",
        "--profile",
        "-p",
    ];
    for arg in args {
        if blocked
            .iter()
            .any(|blocked| arg == blocked || arg.starts_with(&format!("{blocked}=")))
        {
            return Err(ModelError::with_code(
                "codex_cli_args_unsafe",
                format!("CADIS_CODEX_EXTRA_ARGS contains unsupported unsafe option: {arg}"),
                false,
            ));
        }
    }
    Ok(())
}

fn format_codex_failure(output: &CodexCliOutput) -> String {
    format!(
        "Codex CLI exited with status {}. Run `codex login` with the official CLI before selecting provider \"codex-cli\".",
        output.status
    )
}

/// Provider that tries Ollama first and falls back to local echo.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoProvider {
    ollama: OllamaProvider,
    echo: EchoProvider,
}

impl AutoProvider {
    /// Creates the automatic desktop provider.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            ollama: OllamaProvider::new(endpoint, model),
            echo: EchoProvider,
        }
    }
}

impl ModelProvider for AutoProvider {
    fn name(&self) -> &str {
        "auto"
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        Ok(self.chat_with_request(ModelRequest::new(prompt))?.deltas)
    }

    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        let requested_model = request.selected_model.map(ToOwned::to_owned);
        let primary_invocation = ModelInvocation {
            requested_model: requested_model.clone(),
            effective_provider: "ollama".to_owned(),
            effective_model: self.ollama.model.clone(),
            fallback: false,
            fallback_reason: None,
        };
        match self.ollama.chat(request.prompt) {
            Ok(deltas) => Ok(ModelResponse {
                deltas,
                invocation: primary_invocation,
            }),
            Err(error) => {
                let response = format!(
                    "CADIS runtime is online, but Ollama is not ready ({error}).\n\nI received: {}\n\nStart Ollama or set [model].provider = \"echo\" in ~/.cadis/config.toml for an explicit local fallback.",
                    request.prompt
                );
                Ok(ModelResponse {
                    deltas: chunk_text(&response),
                    invocation: ModelInvocation {
                        requested_model,
                        effective_provider: "echo".to_owned(),
                        effective_model: ECHO_MODEL.to_owned(),
                        fallback: true,
                        fallback_reason: Some(format!("ollama unavailable: {error}")),
                    },
                })
            }
        }
    }

    fn stream_chat(
        &self,
        request: ModelRequest<'_>,
        callback: &mut ModelStreamCallback<'_>,
    ) -> Result<ModelResponse, ModelError> {
        let requested_model = request.selected_model.map(ToOwned::to_owned);
        let primary_request =
            ModelRequest::new(request.prompt).with_selected_model(request.selected_model);
        let mut native_stream_started = false;
        let stream_result = self.ollama.stream_chat(primary_request, &mut |event| {
            match &event {
                ModelStreamEvent::Started(_)
                | ModelStreamEvent::Delta(_)
                | ModelStreamEvent::Completed(_)
                | ModelStreamEvent::Cancelled(_) => native_stream_started = true,
                ModelStreamEvent::Failed(_) => {}
            }

            if matches!(event, ModelStreamEvent::Failed(_)) && !native_stream_started {
                return Ok(ModelStreamControl::Continue);
            }

            callback(event)
        });

        match stream_result {
            Ok(response) => Ok(response),
            Err(error) if error.is_cancelled() => Err(error),
            Err(error) if !native_stream_started => {
                let response = format!(
                    "CADIS runtime is online, but Ollama is not ready ({error}).\n\nI received: {}\n\nStart Ollama or set [model].provider = \"echo\" in ~/.cadis/config.toml for an explicit local fallback.",
                    request.prompt
                );
                stream_response(
                    ModelResponse {
                        deltas: chunk_text(&response),
                        invocation: ModelInvocation {
                            requested_model,
                            effective_provider: "echo".to_owned(),
                            effective_model: ECHO_MODEL.to_owned(),
                            fallback: true,
                            fallback_reason: Some(format!("ollama unavailable: {error}")),
                        },
                    },
                    callback,
                )
            }
            Err(error) => Err(error),
        }
    }
}

/// Builds a provider from a provider label.
pub fn provider_from_config(
    provider: &str,
    ollama_endpoint: &str,
    ollama_model: &str,
    openai_base_url: &str,
    openai_model: &str,
    openai_api_key: Option<&str>,
) -> Box<dyn ModelProvider> {
    Box::new(RoutingModelProvider::new(ModelRouterConfig {
        default_provider: normalize_provider_id(provider).to_owned(),
        ollama_endpoint: ollama_endpoint.to_owned(),
        ollama_model: ollama_model.to_owned(),
        openai_base_url: openai_base_url.to_owned(),
        openai_model: openai_model.to_owned(),
        openai_api_key: openai_api_key.map(ToOwned::to_owned),
    }))
}

/// Provider router backed by the configured built-in providers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingModelProvider {
    config: ModelRouterConfig,
}

impl RoutingModelProvider {
    /// Creates a provider router.
    pub fn new(config: ModelRouterConfig) -> Self {
        Self { config }
    }

    fn selection(&self, selected_model: Option<&str>) -> ModelSelection {
        ModelSelection::parse(selected_model, &self.config.default_provider)
    }

    fn unsupported_provider_error(&self, selection: &ModelSelection) -> ModelError {
        let requested = selection.requested_model.clone();
        ModelError::with_code(
            "unsupported_model_provider",
            format!(
                "model provider '{}' is not supported by this CADIS build",
                selection.provider
            ),
            false,
        )
        .with_invocation(ModelInvocation {
            requested_model: requested,
            effective_provider: self.config.default_provider.clone(),
            effective_model: CONFIGURED_MODEL.to_owned(),
            fallback: false,
            fallback_reason: None,
        })
    }
}

impl ModelProvider for RoutingModelProvider {
    fn name(&self) -> &str {
        &self.config.default_provider
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        Ok(self.chat_with_request(ModelRequest::new(prompt))?.deltas)
    }

    fn chat_with_request(&self, request: ModelRequest<'_>) -> Result<ModelResponse, ModelError> {
        let selection = self.selection(request.selected_model);
        let model_request = ModelRequest::new(request.prompt)
            .with_selected_model(selection.requested_model.as_deref());

        match selection.provider.as_str() {
            "auto" => {
                let model = selection
                    .model
                    .as_deref()
                    .unwrap_or(&self.config.ollama_model);
                AutoProvider::new(&self.config.ollama_endpoint, model).chat_with_request(
                    model_request.with_selected_model(selection.requested_model.as_deref()),
                )
            }
            "ollama" => {
                let model = selection
                    .model
                    .as_deref()
                    .unwrap_or(&self.config.ollama_model);
                OllamaProvider::new(&self.config.ollama_endpoint, model)
                    .chat_with_request(model_request)
            }
            "openai" => {
                let model = selection
                    .model
                    .as_deref()
                    .unwrap_or(&self.config.openai_model);
                match self.config.openai_api_key.as_deref() {
                    Some(api_key) => {
                        OpenAiProvider::new(&self.config.openai_base_url, model, api_key)
                            .chat_with_request(model_request)
                    }
                    None => missing_openai_key_response(model_request, model),
                }
            }
            "codex-cli" => {
                match CodexCliProvider::from_env_with_model(selection.model.as_deref()) {
                    Ok(provider) => provider.chat_with_request(model_request),
                    Err(error) => Err(error.with_invocation(ModelInvocation {
                        requested_model: selection.requested_model,
                        effective_provider: "codex-cli".to_owned(),
                        effective_model: selection
                            .model
                            .unwrap_or_else(|| CODEX_CLI_PLAN_MODEL.to_owned()),
                        fallback: false,
                        fallback_reason: None,
                    })),
                }
            }
            "echo" => EchoProvider.chat_with_request(model_request),
            _ => Err(self.unsupported_provider_error(&selection)),
        }
    }

    fn stream_chat(
        &self,
        request: ModelRequest<'_>,
        callback: &mut ModelStreamCallback<'_>,
    ) -> Result<ModelResponse, ModelError> {
        let selection = self.selection(request.selected_model);
        let model_request = ModelRequest::new(request.prompt)
            .with_selected_model(selection.requested_model.as_deref());

        match selection.provider.as_str() {
            "auto" => {
                let model = selection
                    .model
                    .as_deref()
                    .unwrap_or(&self.config.ollama_model);
                AutoProvider::new(&self.config.ollama_endpoint, model).stream_chat(
                    model_request.with_selected_model(selection.requested_model.as_deref()),
                    callback,
                )
            }
            "ollama" => {
                let model = selection
                    .model
                    .as_deref()
                    .unwrap_or(&self.config.ollama_model);
                OllamaProvider::new(&self.config.ollama_endpoint, model)
                    .stream_chat(model_request, callback)
            }
            "openai" => {
                let model = selection
                    .model
                    .as_deref()
                    .unwrap_or(&self.config.openai_model);
                match self.config.openai_api_key.as_deref() {
                    Some(api_key) => {
                        OpenAiProvider::new(&self.config.openai_base_url, model, api_key)
                            .stream_chat(model_request, callback)
                    }
                    None => fail_stream(callback, missing_openai_key_error(model_request, model)),
                }
            }
            "codex-cli" => {
                match CodexCliProvider::from_env_with_model(selection.model.as_deref()) {
                    Ok(provider) => provider.stream_chat(model_request, callback),
                    Err(error) => fail_stream(
                        callback,
                        error.with_invocation(ModelInvocation {
                            requested_model: selection.requested_model,
                            effective_provider: "codex-cli".to_owned(),
                            effective_model: selection
                                .model
                                .unwrap_or_else(|| CODEX_CLI_PLAN_MODEL.to_owned()),
                            fallback: false,
                            fallback_reason: None,
                        }),
                    ),
                }
            }
            "echo" => EchoProvider.stream_chat(model_request, callback),
            _ => fail_stream(callback, self.unsupported_provider_error(&selection)),
        }
    }
}

/// Configuration used by the provider router.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRouterConfig {
    /// Default provider from `[model].provider`.
    pub default_provider: String,
    /// Ollama HTTP endpoint.
    pub ollama_endpoint: String,
    /// Default Ollama model.
    pub ollama_model: String,
    /// OpenAI-compatible API base URL.
    pub openai_base_url: String,
    /// Default OpenAI model.
    pub openai_model: String,
    /// Optional OpenAI API key from the environment.
    pub openai_api_key: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelSelection {
    requested_model: Option<String>,
    provider: String,
    model: Option<String>,
}

impl ModelSelection {
    fn parse(selected_model: Option<&str>, default_provider: &str) -> Self {
        let default_provider = normalize_provider_id(default_provider).to_owned();
        let requested_model = selected_model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let Some(requested) = requested_model.clone() else {
            return Self {
                requested_model,
                provider: default_provider,
                model: None,
            };
        };
        let requested = requested.as_str();

        if let Some((provider, model)) = requested
            .split_once('/')
            .or_else(|| requested.split_once(':'))
        {
            return Self {
                requested_model,
                provider: normalize_provider_id(provider).to_owned(),
                model: normalize_optional_model(Some(model)),
            };
        }

        let normalized = normalize_provider_id(requested);
        if is_known_provider(normalized) {
            return Self {
                requested_model,
                provider: normalized.to_owned(),
                model: None,
            };
        }

        Self {
            requested_model,
            provider: default_provider,
            model: normalize_optional_model(Some(requested)),
        }
    }
}

fn missing_openai_key_response(
    request: ModelRequest<'_>,
    effective_model: &str,
) -> Result<ModelResponse, ModelError> {
    Err(missing_openai_key_error(request, effective_model))
}

fn missing_openai_key_error(request: ModelRequest<'_>, effective_model: &str) -> ModelError {
    let invocation = ModelInvocation {
        requested_model: request.selected_model.map(ToOwned::to_owned),
        effective_provider: "openai".to_owned(),
        effective_model: effective_model.to_owned(),
        fallback: false,
        fallback_reason: None,
    };
    ModelError::with_code(
        "model_auth_missing",
        "OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY",
        false,
    )
    .with_invocation(invocation)
}

fn normalize_provider_id(provider: &str) -> &str {
    match provider.trim() {
        "codex" => "codex-cli",
        "dev-echo" => "echo",
        "" => "auto",
        value => value,
    }
}

fn is_known_provider(provider: &str) -> bool {
    matches!(
        provider,
        "auto" | "ollama" | "openai" | "codex-cli" | "echo"
    )
}

fn catalog_model_or_configured(model: String) -> String {
    let model = model.trim();
    if model.is_empty() {
        CONFIGURED_MODEL.to_owned()
    } else {
        model.to_owned()
    }
}

fn normalize_optional_model(model: Option<&str>) -> Option<String> {
    model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| {
            !matches!(
                *value,
                CONFIGURED_MODEL | ECHO_MODEL | CODEX_CLI_PLAN_MODEL | "default"
            )
        })
        .map(ToOwned::to_owned)
}

fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for word in text.split_inclusive(' ') {
        current.push_str(word);
        if current.len() >= 96 {
            chunks.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: Option<String>,
    error: Option<String>,
    done: Option<bool>,
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest<'a> {
    model: String,
    messages: Vec<OpenAiChatMessage<'a>>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatStreamChunk {
    choices: Vec<OpenAiChatStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatStreamChoice {
    delta: OpenAiChatStreamDelta,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatStreamDelta {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct DeterministicStreamingProvider {
        deltas: Vec<String>,
    }

    impl DeterministicStreamingProvider {
        fn new(deltas: &[&str]) -> Self {
            Self {
                deltas: deltas.iter().map(|delta| (*delta).to_owned()).collect(),
            }
        }

        fn invocation(request: ModelRequest<'_>) -> ModelInvocation {
            ModelInvocation {
                requested_model: request.selected_model.map(ToOwned::to_owned),
                effective_provider: "deterministic-stream".to_owned(),
                effective_model: "deterministic-test-model".to_owned(),
                fallback: false,
                fallback_reason: None,
            }
        }
    }

    fn start_http_response_server(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test HTTP listener should bind");
        let addr = listener
            .local_addr()
            .expect("test HTTP listener should have an address");
        let status = status.to_owned();
        let content_type = content_type.to_owned();
        let body = body.to_owned();
        let (request_tx, request_rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test HTTP client should connect");
            let mut request = String::new();
            let mut content_length = 0usize;
            {
                let mut reader = BufReader::new(
                    stream
                        .try_clone()
                        .expect("test HTTP stream should clone for reading"),
                );
                loop {
                    let mut line = String::new();
                    let bytes = reader
                        .read_line(&mut line)
                        .expect("test HTTP header should read");
                    if bytes == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                    request.push_str(&line);
                }
                if content_length > 0 {
                    let mut body_bytes = vec![0u8; content_length];
                    reader
                        .read_exact(&mut body_bytes)
                        .expect("test HTTP request body should read");
                    request.push_str(&String::from_utf8_lossy(&body_bytes));
                }
            }
            request_tx
                .send(request)
                .expect("test HTTP request should send");
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test HTTP response should write");
        });

        (format!("http://{addr}"), request_rx)
    }

    impl ModelProvider for DeterministicStreamingProvider {
        fn name(&self) -> &str {
            "deterministic-stream"
        }

        fn chat(&self, _prompt: &str) -> Result<Vec<String>, ModelError> {
            panic!("deterministic streaming provider should emit deltas through stream_chat")
        }

        fn chat_with_request(
            &self,
            request: ModelRequest<'_>,
        ) -> Result<ModelResponse, ModelError> {
            Ok(ModelResponse {
                deltas: self.deltas.clone(),
                invocation: Self::invocation(request),
            })
        }

        fn stream_chat(
            &self,
            request: ModelRequest<'_>,
            callback: &mut ModelStreamCallback<'_>,
        ) -> Result<ModelResponse, ModelError> {
            let response = self.chat_with_request(request)?;
            if emit_stream_event(
                callback,
                ModelStreamEvent::Started(response.invocation.clone()),
                &response.invocation,
            )? == ModelStreamControl::Cancel
            {
                return cancel_stream(callback, &response.invocation);
            }

            for delta in &response.deltas {
                if emit_stream_event(
                    callback,
                    ModelStreamEvent::Delta(delta.clone()),
                    &response.invocation,
                )? == ModelStreamControl::Cancel
                {
                    return cancel_stream(callback, &response.invocation);
                }
            }

            if emit_stream_event(
                callback,
                ModelStreamEvent::Completed(response.invocation.clone()),
                &response.invocation,
            )? == ModelStreamControl::Cancel
            {
                return cancel_stream(callback, &response.invocation);
            }

            Ok(response)
        }
    }

    #[test]
    fn echo_provider_returns_deltas() {
        let deltas = EchoProvider.chat("hello").expect("echo should not fail");

        assert!(!deltas.is_empty());
        assert!(deltas.join("").contains("hello"));
    }

    #[test]
    fn provider_catalog_marks_fallback_entries() {
        let catalog = provider_catalog();

        let echo = catalog
            .iter()
            .find(|entry| entry.provider == "echo")
            .expect("echo fallback should be listed");
        assert_eq!(echo.readiness, ProviderReadiness::Fallback);
        assert!(echo.fallback);
        assert_eq!(echo.effective_provider, "echo");

        let ollama = catalog
            .iter()
            .find(|entry| entry.provider == "ollama")
            .expect("ollama should be listed");
        assert_eq!(ollama.readiness, ProviderReadiness::RequiresConfiguration);
        assert!(!ollama.fallback);
    }

    #[test]
    fn provider_catalog_uses_configured_models_and_openai_key_readiness() {
        let catalog = provider_catalog_for_config(&ModelCatalogConfig::new(
            "openai",
            "qwen2.5-coder",
            "gpt-5.4",
            true,
        ));

        let auto = catalog
            .iter()
            .find(|entry| entry.provider == "auto")
            .expect("auto should be listed");
        assert_eq!(auto.model, "qwen2.5-coder");
        assert_eq!(auto.effective_model, "qwen2.5-coder");
        assert_eq!(auto.readiness, ProviderReadiness::RequiresConfiguration);
        assert!(auto.fallback);

        let openai = catalog
            .iter()
            .find(|entry| entry.provider == "openai")
            .expect("openai should be listed");
        assert_eq!(openai.model, "gpt-5.4");
        assert_eq!(openai.effective_model, "gpt-5.4");
        assert_eq!(openai.readiness, ProviderReadiness::Ready);
        assert!(!openai.fallback);
    }

    #[test]
    fn chunk_text_keeps_full_content() {
        let text = "one two three";

        assert_eq!(chunk_text(text).join(""), text);
    }

    #[test]
    fn openai_provider_requires_key_without_network() {
        let provider = provider_from_config(
            "openai",
            "http://127.0.0.1:11434",
            "llama3.2",
            "https://api.openai.com/v1",
            "gpt-5.2",
            None,
        );

        assert_eq!(provider.name(), "openai");
        let error = provider.chat("hello").expect_err("missing key should fail");
        assert_eq!(error.code(), "model_auth_missing");
        assert_eq!(
            error.message(),
            "OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY"
        );
        assert_eq!(
            error
                .invocation()
                .map(|invocation| invocation.effective_provider.as_str()),
            Some("openai")
        );
    }

    #[test]
    fn ollama_connection_failure_has_structured_error() {
        let provider = OllamaProvider::new("http://127.0.0.1:1", "llama3.2");

        let error = provider
            .chat_with_request(
                ModelRequest::new("hello").with_selected_model(Some("ollama/llama3.2")),
            )
            .expect_err("closed localhost port should fail");

        assert_eq!(error.code(), "provider_unavailable");
        assert!(error.retryable());
        assert_eq!(
            error
                .invocation()
                .map(|invocation| invocation.effective_provider.as_str()),
            Some("ollama")
        );
    }

    #[test]
    fn native_stream_failures_emit_failed_event() {
        let provider = OllamaProvider::new("http://127.0.0.1:1", "llama3.2");
        let mut events = Vec::new();

        let error = provider
            .stream_chat(ModelRequest::new("hello"), &mut |event| {
                events.push(event);
                Ok(ModelStreamControl::Continue)
            })
            .expect_err("closed localhost port should fail");

        assert_eq!(error.code(), "provider_unavailable");
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Failed(failure)) if failure.code == "provider_unavailable"
        ));
    }

    #[test]
    fn auto_stream_falls_back_only_before_native_stream_starts() {
        let provider = AutoProvider::new("http://127.0.0.1:1", "llama3.2");
        let mut events = Vec::new();

        let response = provider
            .stream_chat(ModelRequest::new("hello"), &mut |event| {
                events.push(event);
                Ok(ModelStreamControl::Continue)
            })
            .expect("auto should fall back when Ollama cannot connect");

        assert!(response.invocation.fallback);
        assert_eq!(response.invocation.effective_provider, "echo");
        assert!(!events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Failed(_))));

        let invalid_body = "not-json\n";
        let (endpoint, _request_rx) =
            start_http_response_server("200 OK", "application/x-ndjson", invalid_body);
        let provider = AutoProvider::new(endpoint, "llama3.2");
        let mut events = Vec::new();

        let error = provider
            .stream_chat(ModelRequest::new("hello"), &mut |event| {
                events.push(event);
                Ok(ModelStreamControl::Continue)
            })
            .expect_err("auto should not mix fallback after native stream starts");

        assert_eq!(error.code(), "provider_response_invalid");
        assert!(events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Started(_))));
        assert!(events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Failed(_))));
    }

    #[test]
    fn http_status_errors_are_structured() {
        let auth_error = provider_http_error("OpenAI", StatusCode::UNAUTHORIZED);
        assert_eq!(auth_error.code(), "model_auth_failed");
        assert!(!auth_error.retryable());

        let rate_limit = provider_http_error("OpenAI", StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(rate_limit.code(), "provider_rate_limited");
        assert!(rate_limit.retryable());

        let server_error = provider_http_error("Ollama", StatusCode::BAD_GATEWAY);
        assert_eq!(server_error.code(), "provider_unavailable");
        assert!(server_error.retryable());
    }

    #[test]
    fn router_uses_per_agent_echo_model_selection() {
        let provider = provider_from_config(
            "openai",
            "http://127.0.0.1:11434",
            "llama3.2",
            "https://api.openai.com/v1",
            "gpt-5.2",
            None,
        );

        let response = provider
            .chat_with_request(
                ModelRequest::new("hello").with_selected_model(Some("echo/cadis-local-fallback")),
            )
            .expect("echo selection should not require OpenAI credentials");

        assert_eq!(
            response.invocation.requested_model.as_deref(),
            Some("echo/cadis-local-fallback")
        );
        assert_eq!(response.invocation.effective_provider, "echo");
        assert_eq!(response.invocation.effective_model, ECHO_MODEL);
        assert!(response.deltas.join("").contains("hello"));
    }

    #[test]
    fn simulated_stream_emits_ordered_delta_callbacks() {
        let provider = provider_from_config(
            "echo",
            "http://127.0.0.1:11434",
            "llama3.2",
            "https://api.openai.com/v1",
            "gpt-5.2",
            None,
        );
        let mut events = Vec::new();

        let response = provider
            .stream_chat(ModelRequest::new("stream please"), &mut |event| {
                events.push(event);
                Ok(ModelStreamControl::Continue)
            })
            .expect("echo stream should succeed");

        assert!(matches!(events.first(), Some(ModelStreamEvent::Started(_))));
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Completed(_))
        ));
        let streamed = events
            .into_iter()
            .filter_map(|event| match event {
                ModelStreamEvent::Delta(delta) => Some(delta),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(streamed, response.deltas.join(""));
    }

    #[test]
    fn ollama_native_stream_emits_http_deltas_in_order() {
        let stream_body = concat!(
            r#"{"response":"hel","done":false}"#,
            "\n",
            r#"{"response":"lo","done":true}"#,
            "\n"
        );
        let (endpoint, request_rx) =
            start_http_response_server("200 OK", "application/x-ndjson", stream_body);
        let provider = OllamaProvider::new(endpoint, "llama3.2");
        let mut events = Vec::new();

        let response = provider
            .stream_chat(
                ModelRequest::new("hello").with_selected_model(Some("ollama/llama3.2")),
                &mut |event| {
                    events.push(event);
                    Ok(ModelStreamControl::Continue)
                },
            )
            .expect("Ollama native stream should succeed");

        let request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("test server should capture request");
        assert!(request.contains("POST /api/generate"));
        assert!(request.contains(r#""stream":true"#));
        assert_eq!(response.deltas, vec!["hel", "lo"]);
        assert_eq!(
            response.invocation.requested_model.as_deref(),
            Some("ollama/llama3.2")
        );
        assert!(matches!(events.first(), Some(ModelStreamEvent::Started(_))));
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Completed(_))
        ));
        let streamed = events
            .into_iter()
            .filter_map(|event| match event {
                ModelStreamEvent::Delta(delta) => Some(delta),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(streamed, vec!["hel", "lo"]);
    }

    #[test]
    fn openai_native_stream_parses_sse_delta_content() {
        let stream_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (base_url, request_rx) =
            start_http_response_server("200 OK", "text/event-stream", stream_body);
        let provider = OpenAiProvider::new(base_url, "gpt-test", "sk-test");
        let mut events = Vec::new();

        let response = provider
            .stream_chat(
                ModelRequest::new("hello").with_selected_model(Some("openai/gpt-test")),
                &mut |event| {
                    events.push(event);
                    Ok(ModelStreamControl::Continue)
                },
            )
            .expect("OpenAI native stream should succeed");

        let request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("test server should capture request");
        assert!(request.contains("POST /chat/completions"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test"));
        assert!(request.contains(r#""stream":true"#));
        assert_eq!(response.deltas, vec!["hel", "lo"]);
        assert_eq!(response.invocation.effective_provider, "openai");
        let streamed = events
            .into_iter()
            .filter_map(|event| match event {
                ModelStreamEvent::Delta(delta) => Some(delta),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(streamed, vec!["hel", "lo"]);
    }

    #[test]
    fn routing_provider_uses_native_openai_stream_path() {
        let stream_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"route\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (base_url, _request_rx) =
            start_http_response_server("200 OK", "text/event-stream", stream_body);
        let provider = provider_from_config(
            "echo",
            "http://127.0.0.1:11434",
            "llama3.2",
            &base_url,
            "gpt-test",
            Some("sk-test"),
        );
        let mut events = Vec::new();

        let response = provider
            .stream_chat(
                ModelRequest::new("hello").with_selected_model(Some("openai/gpt-test")),
                &mut |event| {
                    events.push(event);
                    Ok(ModelStreamControl::Continue)
                },
            )
            .expect("router should use selected OpenAI native stream");

        assert_eq!(response.deltas, vec!["route"]);
        assert_eq!(
            response.invocation.requested_model.as_deref(),
            Some("openai/gpt-test")
        );
        assert_eq!(response.invocation.effective_provider, "openai");
        assert!(events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Delta(delta) if delta == "route")));
    }

    #[test]
    fn native_stream_callback_cancellation_stops_ollama() {
        let stream_body = concat!(
            r#"{"response":"first","done":false}"#,
            "\n",
            r#"{"response":"second","done":true}"#,
            "\n"
        );
        let (endpoint, _request_rx) =
            start_http_response_server("200 OK", "application/x-ndjson", stream_body);
        let provider = OllamaProvider::new(endpoint, "llama3.2");
        let mut events = Vec::new();

        let error = provider
            .stream_chat(ModelRequest::new("hello"), &mut |event| {
                let should_cancel = matches!(event, ModelStreamEvent::Delta(_));
                events.push(event);
                if should_cancel {
                    Ok(ModelStreamControl::Cancel)
                } else {
                    Ok(ModelStreamControl::Continue)
                }
            })
            .expect_err("callback cancellation should stop native Ollama stream");

        assert_eq!(error.code(), "model_cancelled");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ModelStreamEvent::Delta(_)))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Cancelled(_))
        ));
    }

    #[test]
    fn provider_can_emit_token_deltas_through_callback() {
        let provider = DeterministicStreamingProvider::new(&["hel", "lo", " stream"]);
        let mut events = Vec::new();

        let response = provider
            .stream_chat(
                ModelRequest::new("ignored by deterministic provider")
                    .with_selected_model(Some("deterministic-stream/test")),
                &mut |event| {
                    events.push(event);
                    Ok(ModelStreamControl::Continue)
                },
            )
            .expect("streaming provider should succeed");

        assert_eq!(response.deltas, vec!["hel", "lo", " stream"]);
        assert_eq!(
            response.invocation.requested_model.as_deref(),
            Some("deterministic-stream/test")
        );
        assert!(matches!(events.first(), Some(ModelStreamEvent::Started(_))));
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Completed(_))
        ));
        let streamed = events
            .into_iter()
            .filter_map(|event| match event {
                ModelStreamEvent::Delta(delta) => Some(delta),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(streamed, vec!["hel", "lo", " stream"]);
    }

    #[test]
    fn callback_cancellation_emits_cancelled_and_stops_streaming() {
        let provider = DeterministicStreamingProvider::new(&["first", "second", "third"]);
        let mut events = Vec::new();

        let error = provider
            .stream_chat(
                ModelRequest::new("cancel after first delta"),
                &mut |event| {
                    let should_cancel = matches!(event, ModelStreamEvent::Delta(_));
                    events.push(event);
                    if should_cancel {
                        Ok(ModelStreamControl::Cancel)
                    } else {
                        Ok(ModelStreamControl::Continue)
                    }
                },
            )
            .expect_err("callback cancellation should stop the provider");

        assert_eq!(error.code(), "model_cancelled");
        assert!(!error.retryable());
        assert_eq!(
            error
                .invocation()
                .map(|invocation| invocation.effective_provider.as_str()),
            Some("deterministic-stream")
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ModelStreamEvent::Delta(_)))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Cancelled(_))
        ));
        assert!(!events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Completed(_))));
    }

    #[test]
    fn openai_debug_redacts_api_key() {
        let provider = OpenAiProvider::new(
            "https://api.openai.com/v1",
            "gpt-5.2",
            "sk-testsecretvalue123456",
        );
        let debug = format!("{provider:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-testsecretvalue123456"));
    }

    #[test]
    fn codex_cli_args_use_read_only_ephemeral_exec() {
        let command = CodexCliCommand {
            bin: "codex".to_owned(),
            model: Some("gpt-5.4".to_owned()),
            extra_args: vec!["--search".to_owned()],
        };
        let args = command.args();

        assert_eq!(args[0], "exec");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "read-only"]));
        assert!(args.iter().any(|arg| arg == "--ephemeral"));
        assert!(args.windows(2).any(|pair| pair == ["--model", "gpt-5.4"]));
        assert!(args.iter().any(|arg| arg == "--search"));
    }

    #[test]
    fn codex_cli_extra_args_support_quotes() {
        let args = parse_extra_args(r#"--search "safe phrase""#).expect("args should parse");

        assert_eq!(args, vec!["--search", "safe phrase"]);
    }

    #[test]
    fn codex_cli_rejects_unsafe_extra_args() {
        let error =
            parse_extra_args("--sandbox danger-full-access").expect_err("unsafe arg should fail");

        assert_eq!(error.code(), "codex_cli_args_unsafe");
        assert!(!error.retryable());
        assert!(error
            .to_string()
            .contains("unsupported unsafe option: --sandbox"));
    }

    #[test]
    fn codex_cli_missing_binary_has_clear_error() {
        let command = CodexCliCommand {
            bin: "/definitely/not/codex".to_owned(),
            model: None,
            extra_args: Vec::new(),
        };
        let error =
            run_codex_exec(&command, "hello", Duration::from_millis(10)).expect_err("missing bin");

        assert_eq!(error.code(), "codex_cli_missing");
        assert!(!error.retryable());
        assert!(error.to_string().contains("Codex CLI binary"));
    }
}
