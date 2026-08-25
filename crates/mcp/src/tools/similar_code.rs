use crate::params::{FindSimilarCodeParams, InspectSimilarCodeParams};

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, ContentBlock};

use super::{push_global, push_remote_extends, push_str_flag, run_tool, validation_error_body};

/// Run local semantic candidate discovery without exposing setup mutation.
pub async fn run_find_similar_code(
    binary: &str,
    params: FindSimilarCodeParams,
) -> Result<CallToolResult, McpError> {
    match build_find_similar_code_args(&params) {
        Ok(args) => run_tool(binary, "find_similar_code", &args).await,
        Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)])),
    }
}

/// Reproduce one candidate and return its bounded evidence packet.
pub async fn run_inspect_similar_code(
    binary: &str,
    params: InspectSimilarCodeParams,
) -> Result<CallToolResult, McpError> {
    match build_inspect_similar_code_args(&params) {
        Ok(args) => run_tool(binary, "inspect_similar_code", &args).await,
        Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)])),
    }
}

/// Build CLI arguments for `find_similar_code`.
pub fn build_find_similar_code_args(params: &FindSimilarCodeParams) -> Result<Vec<String>, String> {
    build_args(params, None)
}

/// Build CLI arguments for `inspect_similar_code`.
pub fn build_inspect_similar_code_args(
    params: &InspectSimilarCodeParams,
) -> Result<Vec<String>, String> {
    if params.candidate_id.trim().is_empty() {
        return Err(validation_error_body("candidate_id must not be empty"));
    }
    let common = FindSimilarCodeParams {
        root: params.root.clone(),
        config: params.config.clone(),
        allow_remote_extends: params.allow_remote_extends,
        workspace: params.workspace.clone(),
        changed_since: params.changed_since.clone(),
        changed_workspaces: params.changed_workspaces.clone(),
        paths: params.paths.clone(),
        threshold: params.threshold,
        min_lines: params.min_lines,
        top: params.top,
        no_cache: params.no_cache,
        threads: params.threads,
    };
    build_args(&common, Some(params.candidate_id.trim()))
}

fn build_args(
    params: &FindSimilarCodeParams,
    candidate_id: Option<&str>,
) -> Result<Vec<String>, String> {
    if has_value(params.workspace.as_deref()) && has_value(params.changed_workspaces.as_deref()) {
        return Err(validation_error_body(
            "workspace and changed_workspaces are mutually exclusive for similar-code tools",
        ));
    }
    if params
        .threshold
        .is_some_and(|threshold| !threshold.is_finite() || !(0.0..=1.0).contains(&threshold))
    {
        return Err(validation_error_body(
            "threshold must be a finite number from 0 through 1",
        ));
    }
    if params.min_lines == Some(0) {
        return Err(validation_error_body("min_lines must be greater than zero"));
    }
    if params.top == Some(0) {
        return Err(validation_error_body("top must be greater than zero"));
    }

    let mut args = vec![
        "similar-code".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        "--quiet".to_owned(),
    ];
    push_global(
        &mut args,
        params.root.as_deref(),
        params.config.as_deref(),
        params.no_cache,
        params.threads,
    );
    push_remote_extends(&mut args, params.allow_remote_extends);
    push_str_flag(&mut args, "--workspace", params.workspace.as_deref());
    push_str_flag(
        &mut args,
        "--changed-since",
        params.changed_since.as_deref(),
    );
    push_str_flag(
        &mut args,
        "--changed-workspaces",
        params.changed_workspaces.as_deref(),
    );
    if let Some(paths) = &params.paths {
        for path in paths {
            if path.trim().is_empty() {
                return Err(validation_error_body("paths entries must not be empty"));
            }
            args.extend(["--file".to_owned(), path.clone()]);
        }
    }
    if let Some(threshold) = params.threshold {
        args.extend(["--threshold".to_owned(), threshold.to_string()]);
    }
    if let Some(min_lines) = params.min_lines {
        args.extend(["--min-lines".to_owned(), min_lines.to_string()]);
    }
    if let Some(top) = params.top {
        args.extend(["--top".to_owned(), top.to_string()]);
    }
    if let Some(candidate_id) = candidate_id {
        args.extend(["inspect".to_owned(), candidate_id.to_owned()]);
    }
    Ok(args)
}

fn has_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}
