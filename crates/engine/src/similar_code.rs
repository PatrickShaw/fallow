//! Pure, bounded evaluation of provider-supplied code vectors.
//!
//! This prototype deliberately owns no extraction, model, network, subprocess,
//! or public output behavior. Orchestration validates provider consent and then
//! passes vectors into this deterministic layer.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::fmt;
use std::mem::size_of;

use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};

pub use fallow_types::similar_code::{
    SIMILAR_CODE_EXTRACTION_SEMANTICS_VERSION as EXTRACTION_SEMANTICS_VERSION,
    SimilarCodeFunctionLocation as FunctionLocation, SimilarCodeSourceDigest,
};

/// One extracted function and its provider-supplied vector.
#[derive(Debug, Clone)]
pub struct FunctionVector {
    /// Stable source location for this run.
    pub location: FunctionLocation,
    /// Full SHA-256 digest of the exact extracted function source.
    pub source_sha256: SimilarCodeSourceDigest,
    /// Version of the extraction semantics that produced the function.
    pub extraction_semantics_version: u32,
    /// Dense vector values returned by the provider.
    pub values: Vec<f32>,
}

/// Source identity available before provider inference.
#[derive(Debug, Clone, Copy)]
pub struct SimilarCodeSelectionInput<'a> {
    /// Stable source occurrence for this run.
    pub location: &'a FunctionLocation,
    /// Full SHA-256 digest of the exact source fragment.
    pub source_sha256: SimilarCodeSourceDigest,
}

/// Hard limits for one candidate-evaluation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimilarCodeLimits {
    /// Required vector width.
    pub dimensions: usize,
    /// Maximum functions considered after deterministic sorting.
    pub max_functions: usize,
    /// Maximum pairwise cosine comparisons.
    pub max_comparisons: usize,
    /// Maximum threshold-passing pairs retained before neighbor filtering.
    pub max_candidates: usize,
    /// Maximum retained candidates involving any one function.
    pub max_neighbors_per_function: usize,
    /// Maximum bytes represented by vectors considered in this run.
    pub max_vector_bytes: usize,
}

impl SimilarCodeLimits {
    /// Construct default work limits for a provider-declared vector width.
    #[must_use]
    pub const fn for_dimensions(dimensions: usize) -> Self {
        Self {
            dimensions,
            max_functions: 10_000,
            max_comparisons: 1_000_000,
            max_candidates: 4_096,
            max_neighbors_per_function: 20,
            max_vector_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Why an evaluation omitted otherwise eligible work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SimilarCodeSkipReason {
    /// Functions exceeded the configured function limit.
    FunctionLimit,
    /// Functions exceeded the configured vector-memory limit.
    VectorMemoryLimit,
    /// Pairwise checks exceeded the comparison limit.
    ComparisonLimit,
    /// Threshold-passing pairs exceeded the candidate limit.
    CandidateLimit,
    /// Candidate pairs exceeded a per-function neighbor limit.
    NeighborLimit,
}

/// Omitted-work count for one stable reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimilarCodeSkip {
    /// Stable skip reason.
    pub reason: SimilarCodeSkipReason,
    /// Number of functions, comparisons, or candidates omitted.
    pub count: usize,
}

/// Deterministic bounded corpus chosen before provider inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarCodeCorpusSelection {
    /// Indices into the caller's input slice, in stable occurrence order.
    pub selected_indices: Vec<usize>,
    /// Typed omissions caused by function, vector-memory, or comparison limits.
    pub skipped: Vec<SimilarCodeSkip>,
}

/// Whether all eligible work within the supplied corpus was evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimilarCodeCompletionStatus {
    /// No configured limit omitted eligible work.
    Complete,
    /// One or more configured limits omitted eligible work.
    Partial,
}

/// Machine-readable completion evidence for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarCodeCompletion {
    /// Complete or partial state.
    pub status: SimilarCodeCompletionStatus,
    /// Limits applied to the run.
    pub limits: SimilarCodeLimits,
    /// Functions considered after deterministic limiting.
    pub functions_considered: usize,
    /// Pairwise comparisons actually performed.
    pub comparisons_performed: usize,
    /// Omitted work in stable reason order.
    pub skipped: Vec<SimilarCodeSkip>,
}

/// Candidate verification state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SimilarCodeVerificationStatus {
    /// Provider similarity has not been adjudicated by a human or verifier.
    Unverified,
}

/// One advisory similar-code pair.
#[derive(Debug, Clone, PartialEq)]
pub struct SimilarCodeCandidate {
    /// Snapshot identity over content, occurrences, and extraction semantics.
    pub candidate_id: String,
    /// Content-only identity stable when both functions move without changing.
    pub review_key: String,
    /// Canonically ordered first source location.
    pub left: FunctionLocation,
    /// Canonically ordered second source location.
    pub right: FunctionLocation,
    /// Cosine similarity in the closed range from -1 to 1.
    pub similarity: f64,
    /// Explicit adjudication state. Prototype results always start unverified.
    pub verification_status: SimilarCodeVerificationStatus,
}

/// Results and boundedness evidence for one candidate-evaluation run.
#[derive(Debug, Clone, PartialEq)]
pub struct SimilarCodeEvaluation {
    /// Candidates ranked by similarity and then stable source identity.
    pub candidates: Vec<SimilarCodeCandidate>,
    /// Machine-readable completion evidence.
    pub completion: SimilarCodeCompletion,
}

/// Invalid input rejected before candidate output is produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SimilarCodeError {
    /// The configured vector width was zero.
    ZeroDimensions,
    /// The similarity threshold was non-finite or outside -1 through 1.
    InvalidThreshold,
    /// A vector used an unexpected extraction version.
    ExtractionVersion {
        /// Source location of the invalid vector.
        location: FunctionLocation,
        /// Expected extraction version.
        expected: u32,
        /// Actual extraction version.
        actual: u32,
    },
    /// A vector width differed from the configured width.
    DimensionMismatch {
        /// Source location of the invalid vector.
        location: FunctionLocation,
        /// Expected vector width.
        expected: usize,
        /// Actual vector width.
        actual: usize,
    },
    /// A vector contained NaN or infinity.
    NonFiniteVector {
        /// Source location of the invalid vector.
        location: FunctionLocation,
    },
    /// A vector had zero magnitude and cannot be compared by cosine similarity.
    ZeroMagnitudeVector {
        /// Source location of the invalid vector.
        location: FunctionLocation,
    },
    /// More than one vector claimed the same stable function identity.
    DuplicateFunctionIdentity {
        /// Source location shared by the duplicate inputs.
        location: FunctionLocation,
    },
    /// Preselected vector count did not match the source selection contract.
    SelectionLengthMismatch {
        /// Number of vectors expected from selected source indices.
        expected: usize,
        /// Number of provider vectors supplied.
        actual: usize,
    },
}

impl fmt::Display for SimilarCodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimensions => formatter.write_str("similar-code dimensions must be positive"),
            Self::InvalidThreshold => {
                formatter.write_str("similar-code threshold must be finite and between -1 and 1")
            }
            Self::ExtractionVersion {
                location,
                expected,
                actual,
            } => write!(
                formatter,
                "{}:{} uses extraction version {actual}, expected {expected}",
                location.file, location.start_line
            ),
            Self::DimensionMismatch {
                location,
                expected,
                actual,
            } => write!(
                formatter,
                "{}:{} has {actual} vector dimensions, expected {expected}",
                location.file, location.start_line
            ),
            Self::NonFiniteVector { location } => write!(
                formatter,
                "{}:{} contains a non-finite vector value",
                location.file, location.start_line
            ),
            Self::ZeroMagnitudeVector { location } => write!(
                formatter,
                "{}:{} has a zero-magnitude vector",
                location.file, location.start_line
            ),
            Self::DuplicateFunctionIdentity { location } => write!(
                formatter,
                "{}:{}:{} has duplicate similar-code vector input",
                location.file, location.start_line, location.start_column_utf8
            ),
            Self::SelectionLengthMismatch { expected, actual } => write!(
                formatter,
                "similar-code selection expected {expected} vectors, received {actual}"
            ),
        }
    }
}

impl std::error::Error for SimilarCodeError {}

/// Identity for one vector-cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VectorCacheKey {
    /// Full SHA-256 digest of the exact function source.
    pub function_source_sha256: SimilarCodeSourceDigest,
    /// Extraction-semantics version.
    pub extraction_semantics_version: u32,
    /// Stable model identifier.
    pub model_id: String,
    /// Immutable model revision or artifact digest.
    pub model_revision: String,
    /// Vector width.
    pub dimensions: usize,
    /// Digest of provider parameters that influence vector output.
    pub provider_parameter_digest: u64,
}

/// Separate FIFO cache with a bounded vector-payload budget.
///
/// Source fragments are never stored. The byte budget covers vector values,
/// while map, queue, key, and allocator overhead remain normal process memory.
#[derive(Debug)]
pub struct SimilarCodeVectorCache {
    max_payload_bytes: usize,
    used_payload_bytes: usize,
    entries: FxHashMap<VectorCacheKey, Box<[f32]>>,
    insertion_order: VecDeque<VectorCacheKey>,
}

impl SimilarCodeVectorCache {
    /// Create an empty cache with a vector-payload byte budget.
    #[must_use]
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_payload_bytes: max_bytes,
            used_payload_bytes: 0,
            entries: FxHashMap::default(),
            insertion_order: VecDeque::new(),
        }
    }

    /// Return a cached vector without changing eviction order.
    #[must_use]
    pub fn get(&self, key: &VectorCacheKey) -> Option<&[f32]> {
        self.entries.get(key).map(AsRef::as_ref)
    }

    /// Insert a vector, evicting oldest entries until it fits.
    ///
    /// Returns `false` when the vector itself exceeds the cache budget or its
    /// width does not match the cache key.
    pub fn insert(&mut self, key: VectorCacheKey, values: Vec<f32>) -> bool {
        if values.len() != key.dimensions
            || values.iter().any(|value| !value.is_finite())
            || values.iter().all(|value| is_zero(*value))
        {
            return false;
        }
        let bytes = vector_bytes(values.len());
        if bytes > self.max_payload_bytes {
            return false;
        }

        if let Some(existing) = self.entries.get_mut(&key) {
            self.used_payload_bytes = self
                .used_payload_bytes
                .saturating_sub(vector_bytes(existing.len()))
                .saturating_add(bytes);
            *existing = values.into_boxed_slice();
            return true;
        }

        while self.used_payload_bytes.saturating_add(bytes) > self.max_payload_bytes {
            let Some(oldest) = self.insertion_order.pop_front() else {
                return false;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.used_payload_bytes = self
                    .used_payload_bytes
                    .saturating_sub(vector_bytes(removed.len()));
            }
        }

        self.used_payload_bytes = self.used_payload_bytes.saturating_add(bytes);
        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, values.into_boxed_slice());
        true
    }

    /// Bytes represented by cached vector payloads.
    #[must_use]
    pub const fn used_payload_bytes(&self) -> usize {
        self.used_payload_bytes
    }

    /// Number of cached vectors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug, Clone)]
struct ScoredPair {
    left: usize,
    right: usize,
    similarity: f64,
}

#[derive(Debug, Clone, Copy)]
struct SelectedVector {
    index: usize,
    inverse_norm: f64,
}

impl PartialEq for ScoredPair {
    fn eq(&self, other: &Self) -> bool {
        self.left == other.left
            && self.right == other.right
            && self.similarity.to_bits() == other.similarity.to_bits()
    }
}

impl Eq for ScoredPair {}

impl PartialOrd for ScoredPair {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredPair {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .similarity
            .total_cmp(&self.similarity)
            .then_with(|| self.left.cmp(&other.left))
            .then_with(|| self.right.cmp(&other.right))
    }
}

/// Validate vectors without generating pair candidates.
pub fn validate_function_vectors(
    vectors: &[FunctionVector],
    dimensions: usize,
    extraction_semantics_version: u32,
) -> Result<(), SimilarCodeError> {
    if dimensions == 0 {
        return Err(SimilarCodeError::ZeroDimensions);
    }

    for vector in vectors {
        let _inverse_norm = validate_vector(vector, dimensions, extraction_semantics_version)?;
    }
    Ok(())
}

/// Evaluate provider-supplied vectors with bounded brute-force cosine search.
pub fn evaluate_similar_code(
    vectors: &[FunctionVector],
    threshold: f64,
    limits: SimilarCodeLimits,
    extraction_semantics_version: u32,
) -> Result<SimilarCodeEvaluation, SimilarCodeError> {
    if limits.dimensions == 0 {
        return Err(SimilarCodeError::ZeroDimensions);
    }
    if !threshold.is_finite() || !(-1.0..=1.0).contains(&threshold) {
        return Err(SimilarCodeError::InvalidThreshold);
    }

    let mut skips = FxHashMap::default();
    let selected = select_vectors(vectors, limits, extraction_semantics_version, &mut skips)?;
    let (ranked, comparisons_performed) =
        score_pairs(vectors, &selected, threshold, limits, &mut skips);
    let candidates = build_candidates(
        vectors,
        &selected,
        ranked,
        limits.max_neighbors_per_function,
        extraction_semantics_version,
        &mut skips,
    );

    let mut skipped = skips
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(reason, count)| SimilarCodeSkip { reason, count })
        .collect::<Vec<_>>();
    skipped.sort_by_key(|skip| skip.reason);

    Ok(SimilarCodeEvaluation {
        candidates,
        completion: SimilarCodeCompletion {
            status: if skipped.is_empty() {
                SimilarCodeCompletionStatus::Complete
            } else {
                SimilarCodeCompletionStatus::Partial
            },
            limits,
            functions_considered: selected.len(),
            comparisons_performed,
            skipped,
        },
    })
}

/// Evaluate only the vectors produced for a prior source-corpus selection.
///
/// `vectors` must follow `selection.selected_indices` order. The regular
/// evaluator runs the same selection helper again over this bounded subset,
/// while this wrapper preserves the pre-inference omissions in completion
/// evidence.
pub fn evaluate_selected_similar_code(
    vectors: &[FunctionVector],
    selection: &SimilarCodeCorpusSelection,
    threshold: f64,
    limits: SimilarCodeLimits,
    extraction_semantics_version: u32,
) -> Result<SimilarCodeEvaluation, SimilarCodeError> {
    if vectors.len() != selection.selected_indices.len() {
        return Err(SimilarCodeError::SelectionLengthMismatch {
            expected: selection.selected_indices.len(),
            actual: vectors.len(),
        });
    }

    let mut evaluation =
        evaluate_similar_code(vectors, threshold, limits, extraction_semantics_version)?;
    let mut skipped = evaluation
        .completion
        .skipped
        .drain(..)
        .map(|skip| (skip.reason, skip.count))
        .collect::<FxHashMap<_, _>>();
    for skip in &selection.skipped {
        record_skip(&mut skipped, skip.reason, skip.count);
    }
    evaluation.completion.skipped = skipped
        .into_iter()
        .map(|(reason, count)| SimilarCodeSkip { reason, count })
        .collect();
    evaluation
        .completion
        .skipped
        .sort_by_key(|skip| skip.reason);
    if !evaluation.completion.skipped.is_empty() {
        evaluation.completion.status = SimilarCodeCompletionStatus::Partial;
    }
    Ok(evaluation)
}

/// Select a deterministic fair corpus before provider inference.
///
/// Selection uses only normalized source occurrence and full source digest.
/// The hash-ranked subset is then returned in stable occurrence order. Every
/// pair in the selected corpus fits inside `max_comparisons`, so callers can
/// embed only `selected_indices` and the evaluator can exhaustively compare the
/// resulting vectors.
pub fn select_similar_code_corpus(
    functions: &[SimilarCodeSelectionInput<'_>],
    limits: SimilarCodeLimits,
) -> Result<SimilarCodeCorpusSelection, SimilarCodeError> {
    if limits.dimensions == 0 {
        return Err(SimilarCodeError::ZeroDimensions);
    }

    let occurrence_identities = functions
        .iter()
        .map(|function| occurrence_identity_for_location(function.location))
        .collect::<Vec<_>>();
    let mut identity_order = (0..functions.len()).collect::<Vec<_>>();
    identity_order.sort_by(|&left, &right| {
        occurrence_identities[left]
            .cmp(&occurrence_identities[right])
            .then_with(|| {
                functions[left]
                    .source_sha256
                    .cmp(&functions[right].source_sha256)
            })
    });
    for duplicate in identity_order.windows(2) {
        if occurrence_identities[duplicate[0]] == occurrence_identities[duplicate[1]] {
            return Err(SimilarCodeError::DuplicateFunctionIdentity {
                location: functions[duplicate[0]].location.clone(),
            });
        }
    }

    let memory_function_limit = limits
        .max_vector_bytes
        .checked_div(vector_bytes(limits.dimensions))
        .unwrap_or(0);
    let function_limit = functions.len().min(limits.max_functions);
    let memory_considered = function_limit.min(memory_function_limit);
    let considered = functions_within_comparison_budget(memory_considered, limits.max_comparisons);

    let selection_keys = functions.iter().map(selection_key).collect::<Vec<_>>();
    let mut order = (0..functions.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| {
        selection_keys[left]
            .cmp(&selection_keys[right])
            .then_with(|| {
                functions[left]
                    .source_sha256
                    .cmp(&functions[right].source_sha256)
            })
            .then_with(|| occurrence_identities[left].cmp(&occurrence_identities[right]))
    });
    order.truncate(considered);
    order.sort_by(|&left, &right| {
        occurrence_identities[left]
            .cmp(&occurrence_identities[right])
            .then_with(|| {
                functions[left]
                    .source_sha256
                    .cmp(&functions[right].source_sha256)
            })
    });

    let mut skips = FxHashMap::default();
    record_skip(
        &mut skips,
        SimilarCodeSkipReason::FunctionLimit,
        functions.len().saturating_sub(function_limit),
    );
    record_skip(
        &mut skips,
        SimilarCodeSkipReason::VectorMemoryLimit,
        function_limit.saturating_sub(memory_considered),
    );
    record_skip(
        &mut skips,
        SimilarCodeSkipReason::ComparisonLimit,
        pair_count(memory_considered).saturating_sub(pair_count(considered)),
    );
    let mut skipped = skips
        .into_iter()
        .map(|(reason, count)| SimilarCodeSkip { reason, count })
        .collect::<Vec<_>>();
    skipped.sort_by_key(|skip| skip.reason);

    Ok(SimilarCodeCorpusSelection {
        selected_indices: order,
        skipped,
    })
}

fn select_vectors(
    vectors: &[FunctionVector],
    limits: SimilarCodeLimits,
    extraction_semantics_version: u32,
    skips: &mut FxHashMap<SimilarCodeSkipReason, usize>,
) -> Result<Vec<SelectedVector>, SimilarCodeError> {
    let functions = vectors
        .iter()
        .map(|vector| SimilarCodeSelectionInput {
            location: &vector.location,
            source_sha256: vector.source_sha256,
        })
        .collect::<Vec<_>>();
    let selection = select_similar_code_corpus(&functions, limits)?;
    for skip in selection.skipped {
        record_skip(skips, skip.reason, skip.count);
    }
    selection
        .selected_indices
        .into_iter()
        .map(|index| {
            let inverse_norm = validate_vector(
                &vectors[index],
                limits.dimensions,
                extraction_semantics_version,
            )?;
            Ok(SelectedVector {
                index,
                inverse_norm,
            })
        })
        .collect::<Result<Vec<_>, SimilarCodeError>>()
}

fn score_pairs(
    vectors: &[FunctionVector],
    selected: &[SelectedVector],
    threshold: f64,
    limits: SimilarCodeLimits,
    skips: &mut FxHashMap<SimilarCodeSkipReason, usize>,
) -> (Vec<ScoredPair>, usize) {
    let possible_comparisons = pair_count(selected.len());
    debug_assert!(possible_comparisons <= limits.max_comparisons);
    let mut comparisons_performed = 0usize;
    let mut retained = BinaryHeap::with_capacity(limits.max_candidates.min(possible_comparisons));

    for left in 0..selected.len() {
        for right in left + 1..selected.len() {
            comparisons_performed += 1;
            let similarity = cosine_similarity(
                &vectors[selected[left].index],
                &vectors[selected[right].index],
                selected[left].inverse_norm,
                selected[right].inverse_norm,
            );
            if similarity < threshold {
                continue;
            }
            retain_pair(
                &mut retained,
                ScoredPair {
                    left,
                    right,
                    similarity,
                },
                limits.max_candidates,
                skips,
            );
        }
    }

    let mut ranked = retained.into_vec();
    ranked.sort_by(|left, right| {
        right
            .similarity
            .total_cmp(&left.similarity)
            .then_with(|| left.left.cmp(&right.left))
            .then_with(|| left.right.cmp(&right.right))
    });
    (ranked, comparisons_performed)
}

fn retain_pair(
    retained: &mut BinaryHeap<ScoredPair>,
    pair: ScoredPair,
    max_candidates: usize,
    skips: &mut FxHashMap<SimilarCodeSkipReason, usize>,
) {
    if max_candidates == 0 {
        record_skip(skips, SimilarCodeSkipReason::CandidateLimit, 1);
        return;
    }
    if retained.len() < max_candidates {
        retained.push(pair);
        return;
    }
    if retained.peek().is_some_and(|worst| pair < *worst) {
        retained.pop();
        retained.push(pair);
    }
    record_skip(skips, SimilarCodeSkipReason::CandidateLimit, 1);
}

fn build_candidates(
    vectors: &[FunctionVector],
    selected: &[SelectedVector],
    ranked: Vec<ScoredPair>,
    max_neighbors: usize,
    extraction_semantics_version: u32,
    skips: &mut FxHashMap<SimilarCodeSkipReason, usize>,
) -> Vec<SimilarCodeCandidate> {
    let mut neighbors = vec![0usize; selected.len()];
    let mut candidates = Vec::with_capacity(ranked.len());
    for pair in ranked {
        if neighbors[pair.left] >= max_neighbors || neighbors[pair.right] >= max_neighbors {
            record_skip(skips, SimilarCodeSkipReason::NeighborLimit, 1);
            continue;
        }
        neighbors[pair.left] += 1;
        neighbors[pair.right] += 1;

        let left = &vectors[selected[pair.left].index];
        let right = &vectors[selected[pair.right].index];
        let (left, right) = canonical_pair(left, right);
        candidates.push(SimilarCodeCandidate {
            candidate_id: candidate_id(left, right, extraction_semantics_version),
            review_key: review_key(left, right, extraction_semantics_version),
            left: normalized_location(&left.location),
            right: normalized_location(&right.location),
            similarity: pair.similarity,
            verification_status: SimilarCodeVerificationStatus::Unverified,
        });
    }
    candidates
}

fn validate_vector(
    vector: &FunctionVector,
    dimensions: usize,
    extraction_semantics_version: u32,
) -> Result<f64, SimilarCodeError> {
    if vector.extraction_semantics_version != extraction_semantics_version {
        return Err(SimilarCodeError::ExtractionVersion {
            location: vector.location.clone(),
            expected: extraction_semantics_version,
            actual: vector.extraction_semantics_version,
        });
    }
    if vector.values.len() != dimensions {
        return Err(SimilarCodeError::DimensionMismatch {
            location: vector.location.clone(),
            expected: dimensions,
            actual: vector.values.len(),
        });
    }
    if vector.values.iter().any(|value| !value.is_finite()) {
        return Err(SimilarCodeError::NonFiniteVector {
            location: vector.location.clone(),
        });
    }
    if vector.values.iter().all(|value| is_zero(*value)) {
        return Err(SimilarCodeError::ZeroMagnitudeVector {
            location: vector.location.clone(),
        });
    }
    let squared_norm = vector.values.iter().fold(0.0, |norm, &value| {
        let value = f64::from(value);
        value.mul_add(value, norm)
    });
    Ok(squared_norm.sqrt().recip())
}

fn cosine_similarity(
    left: &FunctionVector,
    right: &FunctionVector,
    left_inverse_norm: f64,
    right_inverse_norm: f64,
) -> f64 {
    let mut dot = 0.0;
    for (&left_value, &right_value) in left.values.iter().zip(&right.values) {
        let left_value = f64::from(left_value);
        let right_value = f64::from(right_value);
        dot = left_value.mul_add(right_value, dot);
    }
    (dot * left_inverse_norm * right_inverse_norm).clamp(-1.0, 1.0)
}

const fn is_zero(value: f32) -> bool {
    value.to_bits().trailing_zeros() >= 31
}

fn candidate_id(
    left: &FunctionVector,
    right: &FunctionVector,
    extraction_semantics_version: u32,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fallow:similar-code:candidate-snapshot:v1\0");
    hasher.update(extraction_semantics_version.to_be_bytes());
    update_snapshot_identity(&mut hasher, left);
    update_snapshot_identity(&mut hasher, right);
    format_digest_id("similar-code:candidate:v1:", hasher.finalize().as_ref())
}

fn review_key(
    left: &FunctionVector,
    right: &FunctionVector,
    extraction_semantics_version: u32,
) -> String {
    let (first, second) = if left.source_sha256 <= right.source_sha256 {
        (left.source_sha256, right.source_sha256)
    } else {
        (right.source_sha256, left.source_sha256)
    };
    let mut hasher = Sha256::new();
    hasher.update(b"fallow:similar-code:review-key:v1\0");
    hasher.update(extraction_semantics_version.to_be_bytes());
    hasher.update(first.as_bytes());
    hasher.update(second.as_bytes());
    format_digest_id("similar-code:review:v1:", hasher.finalize().as_ref())
}

fn canonical_pair<'a>(
    left: &'a FunctionVector,
    right: &'a FunctionVector,
) -> (&'a FunctionVector, &'a FunctionVector) {
    let left_occurrence = occurrence_identity(left);
    let right_occurrence = occurrence_identity(right);
    if (left.source_sha256, left_occurrence) <= (right.source_sha256, right_occurrence) {
        (left, right)
    } else {
        (right, left)
    }
}

fn update_snapshot_identity(hasher: &mut Sha256, vector: &FunctionVector) {
    hasher.update(vector.source_sha256.as_bytes());
    let occurrence = occurrence_identity(vector);
    hasher.update(
        u64::try_from(occurrence.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(occurrence.as_bytes());
}

fn selection_key(function: &SimilarCodeSelectionInput<'_>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"fallow:similar-code:corpus-selection:v1\0");
    hasher.update(function.source_sha256.as_bytes());
    let occurrence = occurrence_identity_for_location(function.location);
    hasher.update(
        u64::try_from(occurrence.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(occurrence.as_bytes());
    hasher.finalize().into()
}

fn format_digest_id(prefix: &str, digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(prefix.len().saturating_add(digest.len() * 2));
    output.push_str(prefix);
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn occurrence_identity(vector: &FunctionVector) -> String {
    occurrence_identity_for_location(&vector.location)
}

fn occurrence_identity_for_location(location: &FunctionLocation) -> String {
    let location = normalized_location(location);
    let path = location.file;
    format!(
        "{}:{path}:{}:{}:{}:{}:{}:{}",
        path.len(),
        location.start_byte,
        location.end_byte,
        location.start_line,
        location.start_column_utf8,
        location.end_line,
        location.end_column_utf8
    )
}

fn normalized_location(location: &FunctionLocation) -> FunctionLocation {
    FunctionLocation {
        file: location.file.replace('\\', "/"),
        start_byte: location.start_byte,
        end_byte: location.end_byte,
        start_line: location.start_line,
        start_column_utf8: location.start_column_utf8,
        end_line: location.end_line,
        end_column_utf8: location.end_column_utf8,
    }
}

fn record_skip(
    skips: &mut FxHashMap<SimilarCodeSkipReason, usize>,
    reason: SimilarCodeSkipReason,
    count: usize,
) {
    if count > 0 {
        let value = skips.entry(reason).or_default();
        *value = value.saturating_add(count);
    }
}

const fn vector_bytes(dimensions: usize) -> usize {
    dimensions.saturating_mul(size_of::<f32>())
}

const fn pair_count(functions: usize) -> usize {
    functions.saturating_mul(functions.saturating_sub(1)) / 2
}

fn functions_within_comparison_budget(max_functions: usize, max_comparisons: usize) -> usize {
    let mut low = 0usize;
    let mut high = max_functions;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let middle_wide = u128::try_from(middle).unwrap_or(u128::MAX);
        let comparisons = middle_wide.saturating_mul(middle_wide.saturating_sub(1)) / 2;
        if comparisons <= u128::try_from(max_comparisons).unwrap_or(u128::MAX) {
            low = middle;
        } else {
            high = middle.saturating_sub(1);
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIMENSIONS: usize = 256;

    fn digest(seed: u64) -> SimilarCodeSourceDigest {
        let mut bytes = [0; 32];
        bytes[24..].copy_from_slice(&seed.to_be_bytes());
        SimilarCodeSourceDigest::new(bytes)
    }

    fn vector(file: &str, hash: u64, first: f32, second: f32) -> FunctionVector {
        let mut values = vec![0.0; DIMENSIONS];
        values[0] = first;
        values[1] = second;
        FunctionVector {
            location: FunctionLocation {
                file: file.into(),
                start_byte: 0,
                end_byte: 100,
                start_line: 1,
                start_column_utf8: 0,
                end_line: 10,
                end_column_utf8: 1,
            },
            source_sha256: digest(hash),
            extraction_semantics_version: EXTRACTION_SEMANTICS_VERSION,
            values,
        }
    }

    fn limits() -> SimilarCodeLimits {
        SimilarCodeLimits {
            dimensions: DIMENSIONS,
            max_functions: 100,
            max_comparisons: 100,
            max_candidates: 100,
            max_neighbors_per_function: 100,
            max_vector_bytes: 100 * DIMENSIONS * size_of::<f32>(),
        }
    }

    #[test]
    fn recorded_vectors_find_only_the_similar_pair() {
        let result = evaluate_similar_code(
            &[
                vector("src/a.ts", 0x10, 1.0, 0.0),
                vector("src/b.ts", 0x20, 0.98, 0.02),
                vector("src/c.ts", 0x30, 0.0, 1.0),
            ],
            0.95,
            limits(),
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();

        assert_eq!(result.candidates.len(), 1);
        assert!(
            result.candidates[0]
                .candidate_id
                .starts_with("similar-code:candidate:v1:")
        );
        assert!(
            result.candidates[0]
                .review_key
                .starts_with("similar-code:review:v1:")
        );
        assert_eq!(
            result.candidates[0].verification_status,
            SimilarCodeVerificationStatus::Unverified
        );
        assert_eq!(
            result.completion.status,
            SimilarCodeCompletionStatus::Complete
        );
    }

    #[test]
    fn candidate_identity_and_order_ignore_input_order() {
        let forward = vec![
            vector("src/a.ts", 0x30, 1.0, 0.0),
            vector("src/b.ts", 0x10, 1.0, 0.0),
            vector("src/c.ts", 0x20, 1.0, 0.0),
        ];
        let mut reverse = forward.clone();
        reverse.reverse();

        let forward =
            evaluate_similar_code(&forward, 0.9, limits(), EXTRACTION_SEMANTICS_VERSION).unwrap();
        let reverse =
            evaluate_similar_code(&reverse, 0.9, limits(), EXTRACTION_SEMANTICS_VERSION).unwrap();

        assert_eq!(forward, reverse);
    }

    #[test]
    fn candidate_identity_and_output_normalize_path_separators() {
        let unix = evaluate_similar_code(
            &[
                vector("src/a.ts", 0x10, 1.0, 0.0),
                vector("src/b.ts", 0x20, 1.0, 0.0),
            ],
            0.9,
            limits(),
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();
        let windows = evaluate_similar_code(
            &[
                vector("src\\a.ts", 0x10, 1.0, 0.0),
                vector("src\\b.ts", 0x20, 1.0, 0.0),
            ],
            0.9,
            limits(),
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();

        assert_eq!(unix, windows);
    }

    #[test]
    fn repeated_content_at_distinct_locations_has_unique_candidate_ids() {
        let result = evaluate_similar_code(
            &[
                vector("src/a.ts", 1, 1.0, 0.0),
                vector("src/b.ts", 1, 1.0, 0.0),
                vector("src/c.ts", 1, 1.0, 0.0),
            ],
            0.9,
            limits(),
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();
        let ids = result
            .candidates
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<rustc_hash::FxHashSet<_>>();
        let review_keys = result
            .candidates
            .iter()
            .map(|candidate| candidate.review_key.as_str())
            .collect::<rustc_hash::FxHashSet<_>>();

        assert_eq!(ids.len(), result.candidates.len());
        assert_eq!(review_keys.len(), 1);
    }

    #[test]
    fn review_key_survives_moves_while_candidate_id_tracks_snapshot() {
        let original = evaluate_similar_code(
            &[
                vector("src/a.ts", 1, 1.0, 0.0),
                vector("src/b.ts", 2, 1.0, 0.0),
            ],
            0.9,
            limits(),
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();
        let moved = evaluate_similar_code(
            &[
                vector("packages/core/a.ts", 1, 1.0, 0.0),
                vector("packages/core/b.ts", 2, 1.0, 0.0),
            ],
            0.9,
            limits(),
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();

        assert_ne!(
            original.candidates[0].candidate_id,
            moved.candidates[0].candidate_id
        );
        assert_eq!(
            original.candidates[0].review_key,
            moved.candidates[0].review_key
        );
    }

    #[test]
    fn comparison_budget_selects_a_stable_fair_corpus_and_checks_every_pair() {
        let vectors = (0..6)
            .map(|index| vector(&format!("src/{index}.ts"), index, 1.0, 0.0))
            .collect::<Vec<_>>();
        let mut bounded = limits();
        bounded.max_functions = vectors.len();
        bounded.max_comparisons = 3;
        bounded.max_candidates = 3;

        let inputs = vectors
            .iter()
            .map(|vector| SimilarCodeSelectionInput {
                location: &vector.location,
                source_sha256: vector.source_sha256,
            })
            .collect::<Vec<_>>();
        let corpus = select_similar_code_corpus(&inputs, bounded).unwrap();
        let expected = corpus
            .selected_indices
            .iter()
            .copied()
            .map(|index| vectors[index].location.file.as_str())
            .collect::<rustc_hash::FxHashSet<_>>();

        let direct =
            evaluate_similar_code(&vectors, 0.9, bounded, EXTRACTION_SEMANTICS_VERSION).unwrap();
        let embedded = corpus
            .selected_indices
            .iter()
            .map(|&index| vectors[index].clone())
            .collect::<Vec<_>>();
        let result = evaluate_selected_similar_code(
            &embedded,
            &corpus,
            0.9,
            bounded,
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();
        let selected = result
            .candidates
            .iter()
            .flat_map(|candidate| [candidate.left.file.as_str(), candidate.right.file.as_str()])
            .collect::<rustc_hash::FxHashSet<_>>();

        assert_eq!(result.completion.functions_considered, 3);
        assert_eq!(result.completion.comparisons_performed, 3);
        assert_eq!(selected, expected);
        assert_eq!(result.completion.skipped, corpus.skipped);
        assert_eq!(result, direct);
    }

    #[test]
    fn invalid_vectors_fail_closed() {
        let mut wrong_dimensions = vector("src/a.ts", 1, 1.0, 0.0);
        wrong_dimensions.values.pop();
        assert!(matches!(
            validate_function_vectors(
                &[wrong_dimensions],
                DIMENSIONS,
                EXTRACTION_SEMANTICS_VERSION
            ),
            Err(SimilarCodeError::DimensionMismatch { .. })
        ));

        let mut non_finite = vector("src/a.ts", 1, 1.0, 0.0);
        non_finite.values[0] = f32::NAN;
        assert!(matches!(
            validate_function_vectors(&[non_finite], DIMENSIONS, EXTRACTION_SEMANTICS_VERSION),
            Err(SimilarCodeError::NonFiniteVector { .. })
        ));

        let mut zero = vector("src/a.ts", 1, 1.0, 0.0);
        zero.values.fill(0.0);
        assert!(matches!(
            validate_function_vectors(&[zero], DIMENSIONS, EXTRACTION_SEMANTICS_VERSION),
            Err(SimilarCodeError::ZeroMagnitudeVector { .. })
        ));
    }

    #[test]
    fn limits_report_partial_work_with_stable_reasons() {
        let mut bounded = limits();
        bounded.max_functions = 3;
        bounded.max_comparisons = 1;
        bounded.max_candidates = 1;
        bounded.max_neighbors_per_function = 0;
        let result = evaluate_similar_code(
            &[
                vector("src/d.ts", 4, 1.0, 0.0),
                vector("src/c.ts", 3, 1.0, 0.0),
                vector("src/b.ts", 2, 1.0, 0.0),
                vector("src/a.ts", 1, 1.0, 0.0),
            ],
            0.9,
            bounded,
            EXTRACTION_SEMANTICS_VERSION,
        )
        .unwrap();

        assert!(result.candidates.is_empty());
        assert_eq!(
            result.completion.status,
            SimilarCodeCompletionStatus::Partial
        );
        assert_eq!(
            result.completion.skipped,
            vec![
                SimilarCodeSkip {
                    reason: SimilarCodeSkipReason::FunctionLimit,
                    count: 1,
                },
                SimilarCodeSkip {
                    reason: SimilarCodeSkipReason::ComparisonLimit,
                    count: 2,
                },
                SimilarCodeSkip {
                    reason: SimilarCodeSkipReason::NeighborLimit,
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn pathological_candidate_limit_does_not_preallocate_without_pairs() {
        let mut untrusted = limits();
        untrusted.max_functions = usize::MAX;
        untrusted.max_comparisons = usize::MAX;
        untrusted.max_candidates = usize::MAX;

        let result =
            evaluate_similar_code(&[], 0.9, untrusted, EXTRACTION_SEMANTICS_VERSION).unwrap();

        assert!(result.candidates.is_empty());
        assert_eq!(
            result.completion.status,
            SimilarCodeCompletionStatus::Complete
        );
    }

    #[test]
    fn duplicate_stable_function_identity_fails_independent_of_input_order() {
        let first = vector("src/a.ts", 1, 1.0, 0.0);
        let mut conflicting = first.clone();
        conflicting.source_sha256 = digest(2);
        conflicting.values[0] = 0.5;
        conflicting.values[1] = 0.5;

        for vectors in [
            vec![first.clone(), conflicting.clone()],
            vec![conflicting, first],
        ] {
            assert!(matches!(
                evaluate_similar_code(&vectors, 0.9, limits(), EXTRACTION_SEMANTICS_VERSION,),
                Err(SimilarCodeError::DuplicateFunctionIdentity { .. })
            ));
        }
    }

    #[test]
    fn vector_cache_is_separate_bounded_and_revision_keyed() {
        let key = |hash, revision: &str| VectorCacheKey {
            function_source_sha256: digest(hash),
            extraction_semantics_version: EXTRACTION_SEMANTICS_VERSION,
            model_id: "fixture-model".to_string(),
            model_revision: revision.to_string(),
            dimensions: 2,
            provider_parameter_digest: 7,
        };
        let mut cache = SimilarCodeVectorCache::new(2 * 2 * size_of::<f32>());

        assert!(cache.insert(key(1, "model@a"), vec![1.0, 0.0]));
        assert!(!cache.insert(key(9, "model@a"), vec![f32::NAN, 0.0]));
        assert!(!cache.insert(key(9, "model@a"), vec![0.0, -0.0]));
        assert!(cache.insert(key(2, "model@a"), vec![0.0, 1.0]));
        assert!(cache.insert(key(3, "model@b"), vec![0.5, 0.5]));

        assert!(cache.get(&key(1, "model@a")).is_none());
        assert_eq!(cache.get(&key(2, "model@a")), Some([0.0, 1.0].as_slice()));
        assert_eq!(cache.get(&key(3, "model@b")), Some([0.5, 0.5].as_slice()));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.used_payload_bytes(), 4 * size_of::<f32>());
    }
}
