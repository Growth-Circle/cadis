//! Model provider adapters for CADIS.

use std::env;
use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const CODEX_CLI_DEFAULT_BIN: &str = "codex";
const CODEX_CLI_TIMEOUT: Duration = Duration::from_secs(300);

/// Streaming-oriented model provider interface used by the runtime.
pub trait ModelProvider: Send + Sync {
    /// Provider label.
    fn name(&self) -> &str;

    /// Returns model output split into display deltas.
    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError>;
}

/// Model provider error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelError {
    message: String,
}

impl ModelError {
    /// Creates a redaction-ready provider error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ModelError {}

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
            .map_err(|error| ModelError::new(format!("failed to create Ollama client: {error}")))?;

        let response = client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&OllamaGenerateRequest {
                model: self.model.clone(),
                prompt: prompt.to_owned(),
                stream: false,
            })
            .send()
            .map_err(|error| ModelError::new(format!("Ollama request failed: {error}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ModelError::new(format!(
                "Ollama returned HTTP status {status}"
            )));
        }

        let body = response
            .json::<OllamaGenerateResponse>()
            .map_err(|error| ModelError::new(format!("Ollama response was invalid: {error}")))?;

        if let Some(error) = body.error {
            return Err(ModelError::new(format!("Ollama error: {error}")));
        }

        Ok(chunk_text(body.response.as_deref().unwrap_or_default()))
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
            return Err(ModelError::new(
                "OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY",
            ));
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| ModelError::new("failed to create OpenAI client"))?;

        let response = client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&OpenAiChatRequest {
                model: self.model.clone(),
                messages: vec![OpenAiChatMessage {
                    role: "user",
                    content: prompt,
                }],
            })
            .send()
            .map_err(|_| ModelError::new("OpenAI request failed"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ModelError::new(format!(
                "OpenAI returned HTTP status {status}"
            )));
        }

        let body = response
            .json::<OpenAiChatResponse>()
            .map_err(|_| ModelError::new("OpenAI response was invalid"))?;
        let content = body
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .unwrap_or_default();

        if content.is_empty() {
            return Err(ModelError::new("OpenAI response was empty"));
        }

        Ok(chunk_text(content))
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
        Ok(Self {
            command: CodexCliCommand::from_env()?,
            timeout: CODEX_CLI_TIMEOUT,
        })
    }
}

impl ModelProvider for CodexCliProvider {
    fn name(&self) -> &str {
        "codex-cli"
    }

    fn chat(&self, prompt: &str) -> Result<Vec<String>, ModelError> {
        let output = run_codex_exec(&self.command, prompt, self.timeout)?;
        if !output.status.success() {
            return Err(ModelError::new(format_codex_failure(&output)));
        }

        Ok(chunk_text(output.stdout.trim()))
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
            if error.kind() == std::io::ErrorKind::NotFound {
                ModelError::new(format!(
                    "Codex CLI binary '{}' was not found. Install the official Codex CLI, run `codex login`, or set CADIS_CODEX_BIN.",
                    command.bin
                ))
            } else {
                ModelError::new(format!("failed to start Codex CLI '{}': {error}", command.bin))
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(prompt.as_bytes()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ModelError::new(format!(
                    "failed to send prompt to Codex CLI: {error}"
                )));
            }
        }
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ModelError::new("failed to capture Codex CLI stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ModelError::new("failed to capture Codex CLI stderr"))?;
    let stdout_reader = read_pipe(stdout);
    let stderr_reader = read_pipe(stderr);

    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| ModelError::new(format!("failed to wait for Codex CLI: {error}")))?
        {
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
            return Err(ModelError::new(format!(
                "Codex CLI timed out after {} seconds while running `{} exec`.",
                timeout.as_secs(),
                command.bin
            )));
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
        .map_err(|_| ModelError::new(format!("Codex CLI {label} reader panicked")))?
        .map_err(|error| ModelError::new(format!("failed to read Codex CLI {label}: {error}")))
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
        return Err(ModelError::new(format!(
            "CADIS_CODEX_EXTRA_ARGS has an unterminated {active_quote} quote"
        )));
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
            return Err(ModelError::new(format!(
                "CADIS_CODEX_EXTRA_ARGS contains unsupported unsafe option: {arg}"
            )));
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
        match self.ollama.chat(prompt) {
            Ok(deltas) => Ok(deltas),
            Err(error) => {
                let response = format!(
                    "CADIS runtime is online, but Ollama is not ready ({error}).\n\nI received: {prompt}\n\nStart Ollama or set [model].provider = \"echo\" in ~/.cadis/config.toml for an explicit local fallback."
                );
                Ok(chunk_text(&response))
            }
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
    match provider {
        "ollama" => Box::new(OllamaProvider::new(ollama_endpoint, ollama_model)),
        "openai" => match openai_api_key {
            Some(api_key) => Box::new(OpenAiProvider::new(
                openai_base_url,
                openai_model,
                api_key.to_owned(),
            )),
            None => Box::new(MissingOpenAiKeyProvider),
        },
        "codex" | "codex-cli" => match CodexCliProvider::from_env() {
            Ok(provider) => Box::new(provider),
            Err(error) => Box::new(ErrorProvider::new("codex-cli", error)),
        },
        "echo" | "dev-echo" => Box::<EchoProvider>::default(),
        _ => Box::new(AutoProvider::new(ollama_endpoint, ollama_model)),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ErrorProvider {
    name: String,
    error: ModelError,
}

impl ErrorProvider {
    fn new(name: impl Into<String>, error: ModelError) -> Self {
        Self {
            name: name.into(),
            error,
        }
    }
}

impl ModelProvider for ErrorProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat(&self, _prompt: &str) -> Result<Vec<String>, ModelError> {
        Err(self.error.clone())
    }
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
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest<'a> {
    model: String,
    messages: Vec<OpenAiChatMessage<'a>>,
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MissingOpenAiKeyProvider;

impl ModelProvider for MissingOpenAiKeyProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn chat(&self, _prompt: &str) -> Result<Vec<String>, ModelError> {
        Err(ModelError::new(
            "OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_provider_returns_deltas() {
        let deltas = EchoProvider.chat("hello").expect("echo should not fail");

        assert!(!deltas.is_empty());
        assert!(deltas.join("").contains("hello"));
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
        assert_eq!(
            provider.chat("hello").expect_err("missing key should fail"),
            ModelError::new("OpenAI provider requires OPENAI_API_KEY or CADIS_OPENAI_API_KEY")
        );
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

        assert!(error.to_string().contains("Codex CLI binary"));
    }
}
