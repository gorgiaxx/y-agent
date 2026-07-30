//! Stable headless invocation surface for automation and A2A callers.

use std::fmt;
use std::future::Future;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use y_core::provider::{ThinkingConfig, ThinkingEffort};
use y_service::chat_types::{OperationMode, TurnCancellationToken};
use y_service::{
    AutomationRunRequest, AutomationRunService, ChatService, TurnError, TurnEventSender,
};

use crate::wire::AppServices;

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// User-callable built-in or configured agent. Omit for ordinary chat.
    #[arg(long, conflicts_with_all = ["chat", "session", "continue_last"])]
    pub agent: Option<String>,

    /// Explicitly create an ordinary chat session (the default).
    #[arg(long, conflicts_with_all = ["agent", "session", "continue_last"])]
    pub chat: bool,

    /// Public `ses_...` reference, legacy raw ID, unique prefix, or title.
    #[arg(long, value_name = "SESSION", conflicts_with = "continue_last")]
    pub session: Option<String>,

    /// Resume the most recent session in the selected workspace.
    #[arg(long = "continue", short = 'c', conflicts_with = "session")]
    pub continue_last: bool,

    /// Searchable name for a newly created session.
    #[arg(long = "name", conflicts_with_all = ["session", "continue_last"])]
    pub session_name: Option<String>,

    /// Provider identifier override.
    #[arg(long)]
    pub provider: Option<String>,

    /// Exact model identifier override.
    #[arg(long)]
    pub model: Option<String>,

    /// Skill to activate for this turn. Repeat for multiple skills.
    #[arg(long = "skill", action = clap::ArgAction::Append)]
    pub skills: Vec<String>,

    /// Knowledge collection to query. Repeat for multiple collections.
    #[arg(long = "knowledge", action = clap::ArgAction::Append)]
    pub knowledge: Vec<String>,

    /// Reasoning effort override.
    #[arg(long, value_enum, default_value_t = ThinkingArg::Default)]
    pub thinking: ThinkingArg,

    /// Turn orchestration strategy.
    #[arg(long = "mode", value_enum, default_value_t = ExecutionModeArg::Fast)]
    pub execution_mode: ExecutionModeArg,

    /// Approval and permission behavior for the turn.
    #[arg(long, value_enum, default_value_t = PermissionArg::Default)]
    pub permission: PermissionArg,

    /// Output contract. `jsonl` emits the session reference before execution.
    #[arg(long, value_enum, default_value_t = RunOutputFormat::Text)]
    pub format: RunOutputFormat,

    /// Cancel the turn after this many seconds, preserving resumable state.
    #[arg(
        long = "timeout",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub timeout_seconds: Option<u64>,

    /// Workspace used for tools and workspace-scoped resume.
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Prompt text. Piped stdin is used when omitted and appended when present.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub prompt: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThinkingArg {
    Default,
    Low,
    Medium,
    High,
    Max,
}

impl ThinkingArg {
    fn to_config(self) -> Option<ThinkingConfig> {
        let effort = match self {
            Self::Default => return None,
            Self::Low => ThinkingEffort::Low,
            Self::Medium => ThinkingEffort::Medium,
            Self::High => ThinkingEffort::High,
            Self::Max => ThinkingEffort::Max,
        };
        Some(ThinkingConfig { effort })
    }
}

impl fmt::Display for ThinkingArg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.to_possible_value().expect("value enum").get_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExecutionModeArg {
    Fast,
    Plan,
    Loop,
    Auto,
}

impl fmt::Display for ExecutionModeArg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.to_possible_value().expect("value enum").get_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionArg {
    Default,
    AutoReview,
    FullAccess,
}

impl PermissionArg {
    fn to_operation_mode(self) -> Option<OperationMode> {
        match self {
            Self::Default => None,
            Self::AutoReview => Some(OperationMode::AutoReview),
            Self::FullAccess => Some(OperationMode::FullAccess),
        }
    }
}

impl fmt::Display for PermissionArg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Default => "default",
            Self::AutoReview => "auto_review",
            Self::FullAccess => "full_access",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunOutputFormat {
    Text,
    Json,
    Jsonl,
}

impl fmt::Display for RunOutputFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.to_possible_value().expect("value enum").get_name())
    }
}

#[derive(Serialize)]
struct SessionStarted<'a> {
    schema_version: u8,
    #[serde(rename = "type")]
    kind: &'static str,
    session_id: &'a str,
    raw_session_id: &'a str,
    resumed: bool,
}

#[derive(Serialize)]
struct RunResult<'a> {
    schema_version: u8,
    #[serde(rename = "type")]
    kind: &'static str,
    session_id: &'a str,
    raw_session_id: &'a str,
    resumed: bool,
    content: &'a str,
    model: &'a str,
    provider_id: Option<&'a str>,
    input_tokens: u64,
    output_tokens: u64,
    tool_calls: Vec<RunToolCall<'a>>,
}

#[derive(Serialize)]
struct RunToolCall<'a> {
    name: &'a str,
    success: bool,
}

#[derive(Serialize)]
struct RunFailed<'a> {
    schema_version: u8,
    #[serde(rename = "type")]
    kind: &'static str,
    session_id: &'a str,
    raw_session_id: &'a str,
    error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    Completed,
    TimedOut,
    Interrupted(TerminationSignal),
}

impl RunOutcome {
    pub(crate) fn exit_code_value(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::TimedOut => 124,
            Self::Interrupted(signal) => signal.exit_code(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminationSignal {
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

impl TerminationSignal {
    fn name(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            #[cfg(unix)]
            Self::Terminate => "SIGTERM",
        }
    }

    fn exit_code(self) -> u8 {
        match self {
            Self::Interrupt => 130,
            #[cfg(unix)]
            Self::Terminate => 143,
        }
    }
}

enum SignalAwareResult<T> {
    Completed(Result<T, TurnError>),
    TimedOut {
        timeout: std::time::Duration,
        result: Result<T, TurnError>,
    },
    Interrupted {
        signal: TerminationSignal,
        result: Result<T, TurnError>,
    },
}

#[derive(Serialize)]
struct RunInterrupted<'a> {
    schema_version: u8,
    #[serde(rename = "type")]
    kind: &'static str,
    session_id: &'a str,
    raw_session_id: &'a str,
    signal: &'static str,
    exit_code: u8,
}

#[derive(Serialize)]
struct RunTimedOut<'a> {
    schema_version: u8,
    #[serde(rename = "type")]
    kind: &'static str,
    session_id: &'a str,
    raw_session_id: &'a str,
    timeout_seconds: u64,
    exit_code: u8,
}

pub async fn run(services: &AppServices, args: RunArgs) -> Result<RunOutcome> {
    let prompt = resolve_prompt(&args.prompt)?;
    let workspace = match args.cwd {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let agent_id = args.agent.and_then(|agent| match agent.as_str() {
        "default" | "chat" => None,
        _ => Some(agent),
    });
    let prepared = AutomationRunService::prepare(
        services,
        AutomationRunRequest {
            session_target: args.session,
            continue_last: args.continue_last,
            session_name: args.session_name,
            agent_id,
            user_input: prompt,
            workspace,
            provider_id: args.provider,
            model: args.model,
            skills: (!args.skills.is_empty()).then_some(args.skills),
            knowledge_collections: (!args.knowledge.is_empty()).then_some(args.knowledge),
            thinking: args.thinking.to_config(),
            plan_mode: Some(args.execution_mode.to_string()),
            operation_mode: args.permission.to_operation_mode(),
        },
    )
    .await?;

    announce_session(
        args.format,
        &prepared.session_reference,
        prepared.turn.session_id.as_str(),
        prepared.resumed,
    )?;

    let cancellation = TurnCancellationToken::new();
    let (progress, mut progress_receiver) = TurnEventSender::channel();
    let progress_drain =
        tokio::spawn(async move { while progress_receiver.recv().await.is_some() {} });
    let execution = Box::pin(cancel_on_signal_or_timeout(
        ChatService::execute_turn_with_progress(
            services,
            &prepared.turn.as_turn_input(),
            progress,
            Some(cancellation.clone()),
        ),
        wait_for_termination_signal(),
        args.timeout_seconds.map(std::time::Duration::from_secs),
        cancellation,
    ))
    .await;
    progress_drain
        .await
        .map_err(|error| anyhow!("turn progress drain failed: {error}"))?;

    let result = match execution {
        SignalAwareResult::Completed(Ok(result)) => result,
        SignalAwareResult::Completed(Err(error)) => {
            if let Some(failure) = structured_failure(
                args.format,
                &prepared.session_reference,
                prepared.turn.session_id.as_str(),
                error.to_string(),
            ) {
                write_json_line(&failure)?;
                std::io::stdout().flush()?;
            }
            return Err(anyhow!(error));
        }
        SignalAwareResult::Interrupted {
            signal,
            result: turn_result,
        } => {
            if let Err(error) = turn_result {
                if !matches!(error, TurnError::Cancelled) {
                    tracing::warn!(%error, signal = signal.name(), "turn failed while cancelling");
                }
            }
            announce_interruption(
                args.format,
                &prepared.session_reference,
                prepared.turn.session_id.as_str(),
                signal,
            )?;
            return Ok(RunOutcome::Interrupted(signal));
        }
        SignalAwareResult::TimedOut {
            timeout,
            result: turn_result,
        } => {
            if let Err(error) = turn_result {
                if !matches!(error, TurnError::Cancelled) {
                    tracing::warn!(%error, "turn failed while applying deadline");
                }
            }
            announce_timeout(
                args.format,
                &prepared.session_reference,
                prepared.turn.session_id.as_str(),
                timeout.as_secs(),
            )?;
            return Ok(RunOutcome::TimedOut);
        }
    };

    let tool_calls = result
        .tool_calls_executed
        .iter()
        .map(|call| RunToolCall {
            name: call.name.as_str(),
            success: call.success,
        })
        .collect();
    let output = RunResult {
        schema_version: 1,
        kind: "run_completed",
        session_id: &prepared.session_reference,
        raw_session_id: prepared.turn.session_id.as_str(),
        resumed: prepared.resumed,
        content: &result.content,
        model: &result.model,
        provider_id: result.provider_id.as_deref(),
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        tool_calls,
    };

    match args.format {
        RunOutputFormat::Text => println!("{}", result.content),
        RunOutputFormat::Json | RunOutputFormat::Jsonl => write_json_line(&output)?,
    }
    std::io::stdout().flush()?;
    Ok(RunOutcome::Completed)
}

async fn cancel_on_signal_or_timeout<T, TurnFuture, SignalFuture>(
    turn: TurnFuture,
    signal: SignalFuture,
    timeout: Option<std::time::Duration>,
    cancellation: TurnCancellationToken,
) -> SignalAwareResult<T>
where
    TurnFuture: Future<Output = Result<T, TurnError>>,
    SignalFuture: Future<Output = TerminationSignal>,
{
    tokio::pin!(turn);
    tokio::pin!(signal);
    let deadline = async move {
        match timeout {
            Some(duration) => {
                tokio::time::sleep(duration).await;
                duration
            }
            None => std::future::pending::<std::time::Duration>().await,
        }
    };
    tokio::pin!(deadline);
    tokio::select! {
        biased;
        result = &mut turn => SignalAwareResult::Completed(result),
        signal = &mut signal => {
            cancellation.cancel();
            let result = turn.await;
            SignalAwareResult::Interrupted { signal, result }
        }
        timeout = &mut deadline => {
            cancellation.cancel();
            let result = turn.await;
            SignalAwareResult::TimedOut { timeout, result }
        }
    }
}

async fn wait_for_termination_signal() -> TerminationSignal {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    () = wait_for_ctrl_c() => TerminationSignal::Interrupt,
                    _ = terminate.recv() => TerminationSignal::Terminate,
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to register SIGTERM handler");
                wait_for_ctrl_c().await;
                TerminationSignal::Interrupt
            }
        }
    }

    #[cfg(not(unix))]
    {
        wait_for_ctrl_c().await;
        TerminationSignal::Interrupt
    }
}

async fn wait_for_ctrl_c() {
    loop {
        match tokio::signal::ctrl_c().await {
            Ok(()) => return,
            Err(error) => {
                tracing::warn!(%error, "failed to listen for Ctrl-C; retrying");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

fn resolve_prompt(parts: &[String]) -> Result<String> {
    let positional = parts.join(" ");
    if std::io::stdin().is_terminal() {
        return (!positional.trim().is_empty())
            .then_some(positional)
            .ok_or_else(|| anyhow!("no prompt provided; pass text or pipe stdin"));
    }

    let mut piped = String::new();
    std::io::stdin().read_to_string(&mut piped)?;
    match (positional.trim().is_empty(), piped.trim().is_empty()) {
        (true, true) => Err(anyhow!("no prompt provided; pass text or pipe stdin")),
        (false, true) => Ok(positional),
        (true, false) => Ok(piped),
        (false, false) => Ok(format!("{positional}\n{piped}")),
    }
}

fn announce_session(
    format: RunOutputFormat,
    session_reference: &str,
    raw_session_id: &str,
    resumed: bool,
) -> Result<()> {
    if format == RunOutputFormat::Jsonl {
        write_json_line(&SessionStarted {
            schema_version: 1,
            kind: "session_started",
            session_id: session_reference,
            raw_session_id,
            resumed,
        })?;
        std::io::stdout().flush()?;
    } else {
        eprintln!("Session: {session_reference}");
        std::io::stderr().flush()?;
    }
    Ok(())
}

fn write_json_line(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn structured_failure<'a>(
    format: RunOutputFormat,
    session_reference: &'a str,
    raw_session_id: &'a str,
    error: impl Into<String>,
) -> Option<RunFailed<'a>> {
    matches!(format, RunOutputFormat::Json | RunOutputFormat::Jsonl).then(|| RunFailed {
        schema_version: 1,
        kind: "run_failed",
        session_id: session_reference,
        raw_session_id,
        error: error.into(),
    })
}

fn announce_interruption(
    format: RunOutputFormat,
    session_reference: &str,
    raw_session_id: &str,
    signal: TerminationSignal,
) -> Result<()> {
    if let Some(interrupted) =
        structured_interruption(format, session_reference, raw_session_id, signal)
    {
        write_json_line(&interrupted)?;
        std::io::stdout().flush()?;
    } else {
        eprintln!(
            "Run interrupted by {}. Session: {session_reference}",
            signal.name()
        );
        eprintln!("Resume with: yagent run --session {session_reference} -- \"Continue\"");
        std::io::stderr().flush()?;
    }
    Ok(())
}

fn structured_interruption<'a>(
    format: RunOutputFormat,
    session_reference: &'a str,
    raw_session_id: &'a str,
    signal: TerminationSignal,
) -> Option<RunInterrupted<'a>> {
    matches!(format, RunOutputFormat::Json | RunOutputFormat::Jsonl).then(|| RunInterrupted {
        schema_version: 1,
        kind: "run_interrupted",
        session_id: session_reference,
        raw_session_id,
        signal: signal.name(),
        exit_code: signal.exit_code(),
    })
}

fn announce_timeout(
    format: RunOutputFormat,
    session_reference: &str,
    raw_session_id: &str,
    timeout_seconds: u64,
) -> Result<()> {
    if let Some(timed_out) =
        structured_timeout(format, session_reference, raw_session_id, timeout_seconds)
    {
        write_json_line(&timed_out)?;
        std::io::stdout().flush()?;
    } else {
        eprintln!("Run timed out after {timeout_seconds}s. Session: {session_reference}");
        eprintln!("Resume with: yagent run --session {session_reference} -- \"Continue\"");
        std::io::stderr().flush()?;
    }
    Ok(())
}

fn structured_timeout<'a>(
    format: RunOutputFormat,
    session_reference: &'a str,
    raw_session_id: &'a str,
    timeout_seconds: u64,
) -> Option<RunTimedOut<'a>> {
    matches!(format, RunOutputFormat::Json | RunOutputFormat::Jsonl).then(|| RunTimedOut {
        schema_version: 1,
        kind: "run_timed_out",
        session_id: session_reference,
        raw_session_id,
        timeout_seconds,
        exit_code: 124,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        cancel_on_signal_or_timeout, structured_failure, structured_interruption,
        structured_timeout, RunOutcome, RunOutputFormat, SignalAwareResult, TerminationSignal,
    };
    use y_service::chat_types::{TurnCancellationToken, TurnError};

    #[test]
    fn test_structured_formats_emit_a_terminal_failure_record() {
        assert!(structured_failure(RunOutputFormat::Text, "ses_1", "1", "failed").is_none());

        for format in [RunOutputFormat::Json, RunOutputFormat::Jsonl] {
            let failure = structured_failure(format, "ses_1", "1", "failed")
                .expect("structured output should include a failure record");
            let value = serde_json::to_value(failure).expect("failure record should serialize");
            assert_eq!(value["type"], "run_failed");
            assert_eq!(value["session_id"], "ses_1");
        }
    }

    #[tokio::test]
    async fn test_cancel_on_signal_cancels_turn_and_classifies_interrupt() {
        let cancellation = TurnCancellationToken::new();
        let turn_cancellation = cancellation.clone();
        let turn = async move {
            turn_cancellation.cancelled().await;
            Err::<(), _>(TurnError::Cancelled)
        };

        let outcome = cancel_on_signal_or_timeout(
            turn,
            async { TerminationSignal::Interrupt },
            None,
            cancellation.clone(),
        )
        .await;

        assert!(cancellation.is_cancelled());
        assert!(matches!(
            outcome,
            SignalAwareResult::Interrupted {
                signal: TerminationSignal::Interrupt,
                result: Err(TurnError::Cancelled),
            }
        ));
    }

    #[tokio::test]
    async fn test_timeout_cancels_turn_and_classifies_deadline() {
        let cancellation = TurnCancellationToken::new();
        let turn_cancellation = cancellation.clone();
        let turn = async move {
            turn_cancellation.cancelled().await;
            Err::<(), _>(TurnError::Cancelled)
        };

        let outcome = cancel_on_signal_or_timeout(
            turn,
            std::future::pending::<TerminationSignal>(),
            Some(std::time::Duration::ZERO),
            cancellation.clone(),
        )
        .await;

        assert!(cancellation.is_cancelled());
        assert!(matches!(
            outcome,
            SignalAwareResult::TimedOut {
                timeout: std::time::Duration::ZERO,
                result: Err(TurnError::Cancelled),
            }
        ));
    }

    #[test]
    fn test_interrupted_run_uses_conventional_signal_exit_codes() {
        assert_eq!(RunOutcome::Completed.exit_code_value(), 0);
        assert_eq!(RunOutcome::TimedOut.exit_code_value(), 124);
        assert_eq!(
            RunOutcome::Interrupted(TerminationSignal::Interrupt).exit_code_value(),
            130
        );
        #[cfg(unix)]
        assert_eq!(
            RunOutcome::Interrupted(TerminationSignal::Terminate).exit_code_value(),
            143
        );
    }

    #[test]
    fn test_structured_formats_emit_a_terminal_interruption_record() {
        assert!(structured_interruption(
            RunOutputFormat::Text,
            "ses_1",
            "1",
            TerminationSignal::Interrupt,
        )
        .is_none());

        for format in [RunOutputFormat::Json, RunOutputFormat::Jsonl] {
            let interrupted =
                structured_interruption(format, "ses_1", "1", TerminationSignal::Interrupt)
                    .expect("structured output should include an interruption record");
            let value =
                serde_json::to_value(interrupted).expect("interruption record should serialize");
            assert_eq!(value["type"], "run_interrupted");
            assert_eq!(value["session_id"], "ses_1");
            assert_eq!(value["signal"], "SIGINT");
            assert_eq!(value["exit_code"], 130);
        }
    }

    #[test]
    fn test_structured_formats_emit_a_terminal_timeout_record() {
        assert!(structured_timeout(RunOutputFormat::Text, "ses_1", "1", 45).is_none());

        for format in [RunOutputFormat::Json, RunOutputFormat::Jsonl] {
            let timed_out = structured_timeout(format, "ses_1", "1", 45)
                .expect("structured output should include a timeout record");
            let value = serde_json::to_value(timed_out).expect("timeout record should serialize");
            assert_eq!(value["type"], "run_timed_out");
            assert_eq!(value["session_id"], "ses_1");
            assert_eq!(value["timeout_seconds"], 45);
            assert_eq!(value["exit_code"], 124);
        }
    }
}
