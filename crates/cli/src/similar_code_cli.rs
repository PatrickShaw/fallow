//! CLI adapter for opt-in local similar-code discovery.

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use fallow_api::{
    AnalysisOptions, SimilarCodeInspectOptions, SimilarCodeOptions, inspect_similar_code,
    review_similar_code, run_similar_code,
};
use fallow_config::OutputFormat;
use fallow_output::{
    SimilarCodeCacheClearOutput, SimilarCodeCacheClearSchemaVersion, SimilarCodeCompletionStatus,
    SimilarCodeInspectOutput, SimilarCodeOutput, SimilarCodeReviewOutput, SimilarCodeStatusOutput,
    SimilarCodeStatusSchemaVersion, serialize_similar_code_cache_clear_json_output,
    serialize_similar_code_inspect_json_output, serialize_similar_code_json_output,
    serialize_similar_code_review_json_output, serialize_similar_code_status_json_output,
};
use fallow_types::envelope::ToolVersion;

use crate::error::emit_error_with_style;
use crate::json_style::JsonStyle;

#[derive(Subcommand)]
pub enum SimilarCodeSubcommand {
    /// Show exact companion and pinned-model readiness without reading project source.
    Status,
    /// Download and verify the pinned local model after explicit confirmation.
    Setup {
        /// Confirm this is local setup for the first-party pinned model.
        #[arg(long, required = true)]
        local: bool,
        /// Confirm non-interactively. Required when stdin is not a terminal.
        #[arg(long)]
        yes: bool,
    },
    /// Reproduce and inspect one unverified candidate with bounded source evidence.
    Inspect {
        /// Snapshot-stable candidate identity from `fallow similar-code`.
        #[arg(value_name = "CANDIDATE_ID")]
        candidate_id: String,
    },
    /// Join candidate JSON with a separate human or agent verdict document.
    Review {
        /// Raw `fallow similar-code --format json` document.
        #[arg(long, value_name = "PATH")]
        candidates: PathBuf,
        /// Versioned verdict JSON document.
        #[arg(long, value_name = "PATH")]
        verdicts: PathBuf,
        /// Fail closed unless every candidate receives a safely matched verdict.
        #[arg(long)]
        require_verdict_for_each_candidate: bool,
    },
    /// Manage the project-local vector cache. Model artifacts are unaffected.
    Cache {
        #[command(subcommand)]
        subcommand: SimilarCodeCacheSubcommand,
    },
}

#[derive(Subcommand)]
pub enum SimilarCodeCacheSubcommand {
    /// Remove cached vectors after explicit confirmation.
    Clear {
        /// Confirm deletion of derived vectors.
        #[arg(long)]
        yes: bool,
    },
}

pub struct SimilarCodeCliInput<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config_path: Option<&'a Path>,
    pub(crate) allow_remote_extends: bool,
    pub(crate) no_cache: bool,
    pub(crate) threads: usize,
    pub(crate) changed_since: Option<&'a str>,
    pub(crate) diff_file: Option<&'a Path>,
    pub(crate) workspace: Option<&'a [String]>,
    pub(crate) changed_workspaces: Option<&'a str>,
    pub(crate) explain: bool,
    pub(crate) output: OutputFormat,
    pub(crate) json_style: JsonStyle,
    pub(crate) threshold: Option<f64>,
    pub(crate) min_lines: Option<usize>,
    pub(crate) top: Option<usize>,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) subcommand: Option<SimilarCodeSubcommand>,
}

pub fn run(input: SimilarCodeCliInput<'_>) -> ExitCode {
    if !matches!(input.output, OutputFormat::Human | OutputFormat::Json) {
        return failure(
            "`fallow similar-code` supports only human and json output; it is advisory and has no SARIF or CI gate",
            2,
            input.output,
            input.json_style,
        );
    }
    let similar_code_options = options(&input);
    match input.subcommand {
        Some(SimilarCodeSubcommand::Status) => run_status(input.output, input.json_style),
        Some(SimilarCodeSubcommand::Setup { local: _, yes }) => {
            run_setup(yes, input.output, input.json_style)
        }
        Some(SimilarCodeSubcommand::Inspect { candidate_id }) => {
            let options = SimilarCodeInspectOptions {
                similar_code: similar_code_options,
                candidate_id,
            };
            match inspect_similar_code(&options) {
                Ok(output) => emit_inspect(output, input.output, input.json_style),
                Err(error) => failure(
                    &error.message,
                    error.exit_code,
                    input.output,
                    input.json_style,
                ),
            }
        }
        Some(SimilarCodeSubcommand::Review {
            candidates,
            verdicts,
            require_verdict_for_each_candidate,
        }) => run_review(
            &candidates,
            &verdicts,
            require_verdict_for_each_candidate,
            input.output,
            input.json_style,
        ),
        Some(SimilarCodeSubcommand::Cache { subcommand }) => match subcommand {
            SimilarCodeCacheSubcommand::Clear { yes } => run_cache_clear(
                input.root,
                input.config_path,
                input.allow_remote_extends,
                yes,
                input.output,
                input.json_style,
            ),
        },
        None => match run_similar_code(&similar_code_options) {
            Ok(output) => emit_discovery(output, input.output, input.json_style),
            Err(error) => failure(
                &error.message,
                error.exit_code,
                input.output,
                input.json_style,
            ),
        },
    }
}

fn options(input: &SimilarCodeCliInput<'_>) -> SimilarCodeOptions {
    SimilarCodeOptions {
        analysis: AnalysisOptions {
            root: Some(input.root.to_path_buf()),
            config_path: input.config_path.map(Path::to_path_buf),
            allow_remote_extends: input.allow_remote_extends,
            no_cache: input.no_cache,
            threads: Some(input.threads),
            diff_file: input.diff_file.map(Path::to_path_buf),
            changed_since: input.changed_since.map(str::to_owned),
            workspace: input.workspace.map(<[String]>::to_vec),
            changed_workspaces: input.changed_workspaces.map(str::to_owned),
            explain: input.explain,
            ..AnalysisOptions::default()
        },
        threshold: input.threshold,
        min_lines: input.min_lines,
        top: input.top,
        files: input.files.clone(),
        adapter_provider_path: None,
    }
}

fn run_status(output: OutputFormat, json_style: JsonStyle) -> ExitCode {
    match fallow_api::similar_code::status() {
        Ok(status) => {
            if matches!(output, OutputFormat::Json) {
                return emit_status_json(status_output(status), json_style);
            }
            crate::report::sink::outln!("Similar-code local provider");
            crate::report::sink::outln!("  Companion: {}", status.sidecar_version);
            crate::report::sink::outln!("  Model: {} @ {}", status.model_id, status.model_revision);
            crate::report::sink::outln!("  License: {}", status.license);
            crate::report::sink::outln!(
                "  Ready: {}",
                if status.model_ready { "yes" } else { "no" }
            );
            crate::report::sink::outln!("  Analysis: local and offline");
            if let Some(problem) = status.problem {
                crate::report::sink::outln!("  Next: {problem}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(&error, 3, output, json_style),
    }
}

fn run_setup(yes: bool, output: OutputFormat, json_style: JsonStyle) -> ExitCode {
    let download_bytes = fallow_api::similar_code::model_download_bytes();
    let (model, revision, dimensions, license) = fallow_api::similar_code::provider_identity();
    if !yes {
        if !std::io::stdin().is_terminal() {
            return failure(
                "non-interactive similar-code setup requires `--yes`; agents and MCP tools must not invoke setup",
                2,
                output,
                json_style,
            );
        }
        eprintln!("Fallow will download and verify a local model:");
        eprintln!("  Model: {model} @ {revision}");
        eprintln!("  License: {license}");
        eprintln!("  Dimensions: {dimensions}");
        eprintln!("  Download: {} MiB", download_bytes / (1024 * 1024));
        eprintln!("  Source analysis remains local and offline after setup.");
        eprint!("Type `yes` to continue: ");
        let _ = std::io::stderr().flush();
        let mut confirmation = String::new();
        if std::io::stdin().read_line(&mut confirmation).is_err() || confirmation.trim() != "yes" {
            return failure("similar-code setup cancelled", 2, output, json_style);
        }
    }
    match fallow_api::similar_code::setup_local() {
        Ok(status) => {
            if matches!(output, OutputFormat::Json) {
                return emit_status_json(status_output(status), json_style);
            }
            crate::report::sink::outln!("Similar-code model is ready and verified.");
            crate::report::sink::outln!(
                "Run `fallow similar-code` to discover unverified candidates."
            );
            ExitCode::SUCCESS
        }
        Err(error) => failure(&error, 2, output, json_style),
    }
}

fn run_review(
    candidates: &Path,
    verdicts: &Path,
    require_all: bool,
    output: OutputFormat,
    json_style: JsonStyle,
) -> ExitCode {
    let candidate_json = match std::fs::read(candidates) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                &format!("failed to read {}: {error}", candidates.display()),
                2,
                output,
                json_style,
            );
        }
    };
    let verdict_json = match std::fs::read(verdicts) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                &format!("failed to read {}: {error}", verdicts.display()),
                2,
                output,
                json_style,
            );
        }
    };
    match review_similar_code(&candidate_json, &verdict_json, require_all) {
        Ok(review) => emit_review(review, output, json_style),
        Err(error) => failure(&error.message, error.exit_code, output, json_style),
    }
}

fn run_cache_clear(
    root: &Path,
    config_path: Option<&Path>,
    allow_remote_extends: bool,
    yes: bool,
    output: OutputFormat,
    json_style: JsonStyle,
) -> ExitCode {
    if !yes {
        return failure(
            "`fallow similar-code cache clear` requires `--yes`; downloaded model artifacts are not removed",
            2,
            output,
            json_style,
        );
    }
    match fallow_api::similar_code::clear_project_cache(root, config_path, allow_remote_extends) {
        Ok(removed) => {
            if matches!(output, OutputFormat::Json) {
                let result = SimilarCodeCacheClearOutput {
                    schema_version: SimilarCodeCacheClearSchemaVersion::V1,
                    version: ToolVersion(env!("CARGO_PKG_VERSION").to_owned()),
                    removed,
                    model_removed: false,
                };
                match serialize_similar_code_cache_clear_json_output(
                    result,
                    crate::output_runtime::current_root_envelope_mode(),
                ) {
                    Ok(value) => emit_json(&value, json_style),
                    Err(error) => failure(
                        &format!("failed to serialize similar-code cache output: {error}"),
                        2,
                        output,
                        json_style,
                    ),
                }
            } else {
                crate::report::sink::outln!(
                    "Similar-code vector cache {}. The local model was not removed.",
                    if removed {
                        "cleared"
                    } else {
                        "was already empty"
                    }
                );
                ExitCode::SUCCESS
            }
        }
        Err(error) => failure(&error, 2, output, json_style),
    }
}

fn status_output(
    status: fallow_api::similar_code::SimilarCodeProviderStatus,
) -> SimilarCodeStatusOutput {
    SimilarCodeStatusOutput {
        schema_version: SimilarCodeStatusSchemaVersion::V1,
        version: ToolVersion(env!("CARGO_PKG_VERSION").to_owned()),
        protocol_version: status.protocol_version,
        companion_version: status.sidecar_version,
        model_ready: status.model_ready,
        model_id: status.model_id,
        model_revision: status.model_revision,
        dimensions: u32::try_from(status.dimensions).unwrap_or(u32::MAX),
        max_tokens: u32::try_from(status.max_tokens).unwrap_or(u32::MAX),
        license: status.license,
        cache_dir: status.cache_dir,
        download_bytes: status.download_bytes,
        analysis_offline: status.analysis_offline,
        integrity_verified: status.integrity_verified,
        problem: status.problem,
        downloaded: status.downloaded,
    }
}

fn emit_status_json(output: SimilarCodeStatusOutput, style: JsonStyle) -> ExitCode {
    match serialize_similar_code_status_json_output(
        output,
        crate::output_runtime::current_root_envelope_mode(),
    ) {
        Ok(value) => emit_json(&value, style),
        Err(error) => failure(
            &format!("failed to serialize similar-code status output: {error}"),
            2,
            OutputFormat::Json,
            style,
        ),
    }
}

fn emit_discovery(output: SimilarCodeOutput, format: OutputFormat, style: JsonStyle) -> ExitCode {
    crate::telemetry::note_result_count(output.candidates.len());
    if matches!(format, OutputFormat::Json) {
        return match serialize_similar_code_json_output(
            output,
            crate::output_runtime::current_root_envelope_mode(),
        ) {
            Ok(value) => emit_json(&value, style),
            Err(error) => failure(
                &format!("failed to serialize similar-code output: {error}"),
                2,
                format,
                style,
            ),
        };
    }
    crate::report::sink::outln!("Similar-code candidates (unverified)");
    crate::report::sink::outln!(
        "Model score is cosine similarity, not a probability or refactor verdict."
    );
    if output.candidates.is_empty() {
        if output.completion.status == SimilarCodeCompletionStatus::Complete {
            crate::report::sink::outln!("No candidates met the configured threshold.");
        } else {
            crate::report::sink::outln!(
                "No candidates were returned, but the run was partial. Inspect completion in JSON before concluding absence."
            );
        }
    }
    for candidate in &output.candidates {
        crate::report::sink::outln!("");
        crate::report::sink::outln!(
            "{}  score {:.4}  {:?}",
            candidate.candidate_id,
            candidate.similarity,
            candidate.similarity_band
        );
        crate::report::sink::outln!(
            "  {}:{}  {}",
            candidate.left.path,
            candidate.left.start_line,
            candidate.left.name
        );
        crate::report::sink::outln!(
            "  {}:{}  {}",
            candidate.right.path,
            candidate.right.start_line,
            candidate.right.name
        );
        crate::report::sink::outln!(
            "  Inspect: {}",
            inspect_command(
                output.generation.threshold,
                output.generation.min_lines,
                &candidate.candidate_id
            )
        );
    }
    crate::report::sink::outln!("");
    if output.completion.status == SimilarCodeCompletionStatus::Complete {
        crate::report::sink::outln!("Completion: Complete");
    } else {
        crate::report::sink::outln!(
            "Completion: Partial (use --format json to inspect skipped work)"
        );
    }
    ExitCode::SUCCESS
}

fn inspect_command(threshold: f64, min_lines: u64, candidate_id: &str) -> String {
    format!(
        "fallow similar-code --threshold {threshold} --min-lines {min_lines} inspect {candidate_id}"
    )
}

fn emit_inspect(
    output: SimilarCodeInspectOutput,
    format: OutputFormat,
    style: JsonStyle,
) -> ExitCode {
    crate::telemetry::note_result_count(1);
    if matches!(format, OutputFormat::Json) {
        return match serialize_similar_code_inspect_json_output(
            output,
            crate::output_runtime::current_root_envelope_mode(),
        ) {
            Ok(value) => emit_json(&value, style),
            Err(error) => failure(
                &format!("failed to serialize similar-code inspect output: {error}"),
                2,
                format,
                style,
            ),
        };
    }
    crate::report::sink::outln!("Similar-code inspection (still unverified)");
    crate::report::sink::outln!(
        "{}  score {:.4}",
        output.candidate.candidate_id,
        output.candidate.similarity
    );
    crate::report::sink::outln!(
        "\n{}:{}  {}",
        output.candidate.left.path,
        output.candidate.left.start_line,
        output.candidate.left.name
    );
    if let Some(source) = output.packet.left.source_window {
        crate::report::sink::outln!("{source}");
    }
    crate::report::sink::outln!(
        "\n{}:{}  {}",
        output.candidate.right.path,
        output.candidate.right.start_line,
        output.candidate.right.name
    );
    if let Some(source) = output.packet.right.source_window {
        crate::report::sink::outln!("{source}");
    }
    crate::report::sink::outln!(
        "\nDecide candidate_worthy, behaviorally_equivalent, and refactor_safe separately."
    );
    ExitCode::SUCCESS
}

fn emit_review(
    output: SimilarCodeReviewOutput,
    format: OutputFormat,
    style: JsonStyle,
) -> ExitCode {
    crate::telemetry::note_result_count(output.candidates.len());
    if matches!(format, OutputFormat::Json) {
        return match serialize_similar_code_review_json_output(
            output,
            crate::output_runtime::current_root_envelope_mode(),
        ) {
            Ok(value) => emit_json(&value, style),
            Err(error) => failure(
                &format!("failed to serialize similar-code review output: {error}"),
                2,
                format,
                style,
            ),
        };
    }
    crate::report::sink::outln!("Similar-code review");
    for candidate in output.candidates {
        crate::report::sink::outln!(
            "{}  {:?}  {:?}",
            candidate.candidate.candidate_id,
            candidate.verdict_match,
            candidate.outcome
        );
    }
    ExitCode::SUCCESS
}

fn emit_json(value: &serde_json::Value, style: JsonStyle) -> ExitCode {
    match style.serialize(&value) {
        Ok(json) => {
            crate::report::sink::outln!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => failure(
            &format!("failed to serialize similar-code JSON: {error}"),
            2,
            OutputFormat::Json,
            style,
        ),
    }
}

fn failure(message: &str, code: u8, output: OutputFormat, style: JsonStyle) -> ExitCode {
    emit_error_with_style(message, code, output, style)
}

#[cfg(test)]
mod tests {
    use super::inspect_command;

    #[test]
    fn inspect_command_preserves_effective_discovery_parameters() {
        assert_eq!(
            inspect_command(0.6, 7, "similar-code:candidate:v1:abc"),
            "fallow similar-code --threshold 0.6 --min-lines 7 inspect similar-code:candidate:v1:abc"
        );
    }
}
