//! Local-provider orchestration for advisory similar-code discovery.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fallow_engine::source::similar_code::SimilarCodeSourceDigest;

mod cache;
mod protocol;
mod transport;

pub use protocol::SimilarCodeProviderStatus;

const EMBED_BATCH_SIZE: usize = 1;
const EMBED_RUN_TIMEOUT: Duration = Duration::from_mins(15);

/// Classified local-provider failure used by the programmatic runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderError {
    /// The exact companion or its pinned model is not installed yet.
    NotReady(String),
    /// An installed companion failed execution, provenance, or protocol checks.
    Failed(String),
}

impl ProviderError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::NotReady(message) | Self::Failed(message) => message,
        }
    }
}

/// Opaque, validated handle obtained before project source is read.
pub(crate) struct ReadyProvider {
    path: PathBuf,
    pub(crate) status: SimilarCodeProviderStatus,
}

/// One transient source fragment selected for local embedding.
pub(crate) struct EmbeddingInput<'a> {
    pub(crate) source_sha256: SimilarCodeSourceDigest,
    pub(crate) source: &'a str,
}

/// Privacy-safe cache and provider accounting for one embedding pass.
pub(crate) struct EmbeddingResult {
    /// Vectors in input order. Missing entries represent a bounded partial pass.
    pub(crate) vectors: Vec<Option<Vec<f32>>>,
    pub(crate) cache_hits: usize,
    pub(crate) cache_misses: usize,
    pub(crate) cache_writes: usize,
    pub(crate) cache_invalid_entries: usize,
    pub(crate) cache_disabled: bool,
    pub(crate) provider_problem: Option<String>,
    pub(crate) inference_ms: f64,
    pub(crate) truncated_functions: usize,
}

/// Return the installed local provider status.
///
/// # Errors
///
/// Returns a structured message when the trusted sibling is missing, cannot be
/// executed, or reports incompatible provenance.
pub fn status() -> Result<SimilarCodeProviderStatus, String> {
    let sidecar = transport::discover_provider()?;
    let status = transport::provider_status(&sidecar)?;
    validate_status(&status)?;
    Ok(status)
}

/// Download and verify the pinned local model through the installed provider.
///
/// Callers must obtain explicit human confirmation before invoking this API.
/// MCP, NAPI, project configuration, and agent workflows intentionally do not
/// expose this mutation.
///
/// # Errors
///
/// Returns a structured message when setup fails or provenance is invalid.
pub fn setup_local() -> Result<SimilarCodeProviderStatus, String> {
    let sidecar = transport::discover_provider()?;
    let status = transport::setup_provider(&sidecar)?;
    validate_status(&status)?;
    if !status.model_ready {
        return Err("similar-code setup completed without a verified local model".to_owned());
    }
    Ok(status)
}

/// Immutable local provider identity used by public provenance output.
#[must_use]
pub fn provider_identity() -> (&'static str, &'static str, usize, &'static str) {
    (
        protocol::MODEL_ID,
        protocol::MODEL_REVISION,
        protocol::MODEL_DIMENSIONS,
        protocol::MODEL_LICENSE,
    )
}

/// Total bytes downloaded by an explicit local model setup.
#[must_use]
pub fn model_download_bytes() -> u64 {
    protocol::MODEL_ARTIFACTS
        .iter()
        .map(|artifact| artifact.size)
        .sum()
}

/// Resolve and validate the exact local companion before reading project source.
pub(crate) fn ready_provider() -> Result<ReadyProvider, ProviderError> {
    let path = transport::discover_provider().map_err(ProviderError::NotReady)?;
    let status = transport::provider_status(&path).map_err(ProviderError::Failed)?;
    validate_status(&status).map_err(ProviderError::Failed)?;
    if !status.model_ready {
        let problem = status.problem.unwrap_or_else(|| {
            "the pinned local model is not installed; run `fallow similar-code setup --local`"
                .to_owned()
        });
        return Err(ProviderError::NotReady(problem));
    }
    Ok(ReadyProvider { path, status })
}

/// Validate an exact companion path already resolved and signature-verified
/// by an official distribution adapter.
///
/// This exists for in-process hosts such as Node, whose process executable is
/// not the Fallow binary and therefore has no meaningful sibling discovery.
/// The path is never read from project config, PATH, or a model response.
pub(crate) fn ready_provider_from_adapter_path(
    path: &Path,
) -> Result<ReadyProvider, ProviderError> {
    if !path.is_file() {
        return Err(ProviderError::NotReady(format!(
            "verified similar-code companion is unavailable at {}",
            path.display()
        )));
    }
    let path = path.to_path_buf();
    let status = transport::provider_status(&path).map_err(ProviderError::Failed)?;
    validate_status(&status).map_err(ProviderError::Failed)?;
    if !status.model_ready {
        let problem = status.problem.unwrap_or_else(|| {
            "the pinned local model is not installed; run `fallow similar-code setup --local`"
                .to_owned()
        });
        return Err(ProviderError::NotReady(problem));
    }
    Ok(ReadyProvider { path, status })
}

/// Embed selected source fragments using the persistent source-digest cache.
pub(crate) fn embed_selected(
    provider: &ReadyProvider,
    cache_dir: &Path,
    no_cache: bool,
    inputs: &[EmbeddingInput<'_>],
) -> Result<EmbeddingResult, ProviderError> {
    let mut cache = cache::VectorCache::load(cache_dir, no_cache);
    let cache_invalid_entries = usize::from(cache.load_state == cache::CacheLoadState::Corrupt);
    let cache_disabled = cache.load_state == cache::CacheLoadState::Disabled;
    let mut vectors = Vec::with_capacity(inputs.len());
    let mut misses = Vec::new();
    let mut cache_hits = 0usize;
    for (index, input) in inputs.iter().enumerate() {
        if let Some(values) = cache.get(&input.source_sha256) {
            vectors.push(Some(values.to_vec()));
            cache_hits += 1;
        } else {
            vectors.push(None);
            misses.push(index);
        }
    }

    let cache_misses = misses.len();
    let mut cache_writes = 0usize;
    let mut inference_ms = 0.0f64;
    let mut provider_problem = None;
    let mut truncated_functions = 0usize;
    if !misses.is_empty() {
        let started = Instant::now();
        let mut session =
            transport::ProviderSession::spawn(&provider.path).map_err(ProviderError::Failed)?;
        for batch in misses.chunks(EMBED_BATCH_SIZE) {
            if started.elapsed() >= EMBED_RUN_TIMEOUT {
                provider_problem =
                    Some("similar-code embedding stopped at the 15-minute run limit".to_owned());
                break;
            }
            let request = batch
                .iter()
                .map(|index| {
                    let key = u32::try_from(*index).map_err(|_| {
                        ProviderError::Failed(
                            "similar-code input index exceeded protocol capacity".to_owned(),
                        )
                    })?;
                    Ok((key, inputs[*index].source))
                })
                .collect::<Result<Vec<_>, ProviderError>>()?;
            match session.embed(&request) {
                Ok(response) => {
                    inference_ms += response.timing.inference_ms;
                    truncated_functions += response.completion.truncated_functions;
                    if response.status != protocol::EmbedCompletionStatus::Complete {
                        provider_problem = Some(transport::embed_problem(&response));
                    }
                    for vector in response.vectors {
                        let index = usize::try_from(vector.key).map_err(|_| {
                            ProviderError::Failed(
                                "similar-code provider returned an invalid vector key".to_owned(),
                            )
                        })?;
                        cache.insert(inputs[index].source_sha256, vector.values.clone());
                        vectors[index] = Some(vector.values);
                        cache_writes += 1;
                    }
                }
                Err(error) => {
                    provider_problem = Some(error);
                    break;
                }
            }
        }
    }
    cache.save().map_err(ProviderError::Failed)?;

    Ok(EmbeddingResult {
        vectors,
        cache_hits,
        cache_misses,
        cache_writes,
        cache_invalid_entries,
        cache_disabled,
        provider_problem,
        inference_ms,
        truncated_functions,
    })
}

/// Remove the model-specific vector cache. Model artifacts remain installed.
fn clear_vector_cache(cache_dir: &Path) -> Result<bool, String> {
    cache::clear(cache_dir)
}

/// Remove only persisted similar-code vectors for one project.
///
/// The downloaded model is user-level state and is never removed here.
///
/// # Errors
///
/// Returns an error when config resolution or cache removal fails.
pub fn clear_project_cache(
    root: &Path,
    config_path: Option<&Path>,
    allow_remote_extends: bool,
) -> Result<bool, String> {
    let project = fallow_engine::project_config::config_for_project_with_load_options(
        root,
        config_path,
        fallow_config::ConfigLoadOptions {
            allow_remote_extends,
        },
    )
    .map_err(|error| format!("failed to load config: {error}"))?;
    clear_vector_cache(&project.config.cache_dir)
}

pub(crate) fn model_artifact_sha256() -> &'static str {
    protocol::MODEL_ARTIFACTS
        .first()
        .map_or("", |artifact| artifact.sha256)
}

pub(crate) fn parameter_sha256() -> String {
    let digest = cache::parameter_digest();
    digest.iter().fold(
        String::with_capacity(digest.len().saturating_mul(2)),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

pub(crate) const fn embedding_batch_size() -> usize {
    EMBED_BATCH_SIZE
}

fn validate_status(status: &SimilarCodeProviderStatus) -> Result<(), String> {
    if status.protocol_version != protocol::WIRE_PROTOCOL_VERSION
        || status.sidecar_version != env!("CARGO_PKG_VERSION")
        || status.model_id != protocol::MODEL_ID
        || status.model_revision != protocol::MODEL_REVISION
        || status.dimensions != protocol::MODEL_DIMENSIONS
        || status.max_tokens != protocol::MODEL_MAX_TOKENS
        || status.license != protocol::MODEL_LICENSE
        || status.download_bytes != model_download_bytes()
        || !status.analysis_offline
        || (status.model_ready && !status.integrity_verified)
    {
        return Err(
            "similar-code companion provenance does not match this Fallow release".to_owned(),
        );
    }
    Ok(())
}
