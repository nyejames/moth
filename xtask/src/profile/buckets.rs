//! Profile owner bucket mapping.
//!
//! WHAT: Maps function names to Moth source-owner buckets based on
//! module prefix patterns. Each bucket has a label and suggested source
//! paths for agent-directed investigation.
//!
//! WHY: Bucket mapping turns raw profiler function names into actionable
//! source-ownership hints. Agents can read bucket labels and paths to
//! know which directory to inspect first.
//!
//! # What this module owns
//! - `ProfileOwnerBucket` definition with label, prefixes, and paths
//! - `match_owner_bucket()` that maps a function name to a bucket
//! - Initial bucket definitions for Moth compiler modules
//!
//! # What this module does NOT own
//! - Profile parsing (see `parse.rs`)
//! - Hotspot extraction and filtering (see `hotspots.rs`)
//! - Artifact writing (see `artifacts.rs`)

/// An owner bucket that maps function-name prefixes to source paths.
///
/// WHAT: Defines a logical grouping of compiler functions (e.g., "AST",
/// "HIR") with the module prefixes that identify them and the source
/// directories an agent should inspect.
///
/// WHY: Named buckets with suggested paths let the hotspot summary
/// direct investigation without manual prefix matching.
#[derive(Debug, Clone)]
pub(crate) struct ProfileOwnerBucket {
    /// Human-readable label for this bucket (e.g., "Tokenization", "AST").
    pub(crate) label: &'static str,
    /// Function-name prefixes that belong to this bucket.
    pub(crate) prefixes: &'static [&'static str],
    /// Suggested source paths relative to the repo root.
    pub(crate) suggested_paths: &'static [&'static str],
}

/// Result of matching a function name to an owner bucket.
///
/// WHAT: Contains the bucket label and suggested paths for one function.
/// WHY: A named struct makes the match result explicit in the hotspot output.
#[derive(Debug, Clone)]
pub(crate) struct ProfileOwnerBucketMatch {
    /// Bucket label (e.g., "Tokenization", "std", "unknown").
    pub(crate) label: String,
    /// Suggested source paths relative to the repo root.
    pub(crate) suggested_paths: Vec<String>,
}

/// All defined owner buckets, ordered by specificity.
///
/// The order matters: more specific prefixes should come before general
/// ones so the first match is the most relevant bucket.
///
/// WHAT: Moth compiler module buckets plus fallback buckets for
/// std, core, alloc, rayon, samply/profiler, unknown, and other.
///
/// WHY: These buckets cover the Moth codebase structure and common
/// third-party function patterns seen in Samply profiles.
fn owner_buckets() -> Vec<ProfileOwnerBucket> {
    vec![
        // Moth compiler modules (ordered by specificity)
        ProfileOwnerBucket {
            label: "Tokenization",
            prefixes: &["moth::compiler_frontend::tokenizer"],
            suggested_paths: &["src/compiler_frontend/tokenizer/"],
        },
        ProfileOwnerBucket {
            label: "Header parsing",
            prefixes: &["moth::compiler_frontend::headers"],
            suggested_paths: &["src/compiler_frontend/headers/"],
        },
        ProfileOwnerBucket {
            label: "Dependency sorting",
            prefixes: &["moth::compiler_frontend::module_dependencies"],
            suggested_paths: &["src/compiler_frontend/module_dependencies.rs"],
        },
        ProfileOwnerBucket {
            label: "AST",
            prefixes: &["moth::compiler_frontend::ast"],
            suggested_paths: &["src/compiler_frontend/ast/"],
        },
        ProfileOwnerBucket {
            label: "HIR",
            prefixes: &["moth::compiler_frontend::hir"],
            suggested_paths: &["src/compiler_frontend/hir/"],
        },
        ProfileOwnerBucket {
            label: "Borrow validation",
            prefixes: &["moth::compiler_frontend::analysis::borrow_checker"],
            suggested_paths: &["src/compiler_frontend/analysis/borrow_checker/"],
        },
        ProfileOwnerBucket {
            label: "Build system",
            prefixes: &["moth::build_system"],
            suggested_paths: &["src/build_system/"],
        },
        ProfileOwnerBucket {
            label: "JS backend",
            prefixes: &["moth::backends::js"],
            suggested_paths: &["src/backends/js/"],
        },
        ProfileOwnerBucket {
            label: "Wasm backend",
            prefixes: &["moth::backends::wasm"],
            suggested_paths: &["src/backends/wasm/"],
        },
        ProfileOwnerBucket {
            label: "HTML project builder",
            prefixes: &["moth::projects::html_project"],
            suggested_paths: &["src/projects/html_project/"],
        },
        // Third-party and runtime fallback buckets
        ProfileOwnerBucket {
            label: "std",
            prefixes: &["std::"],
            suggested_paths: &[],
        },
        ProfileOwnerBucket {
            label: "core",
            prefixes: &["core::"],
            suggested_paths: &[],
        },
        ProfileOwnerBucket {
            label: "alloc",
            prefixes: &["alloc::"],
            suggested_paths: &[],
        },
        ProfileOwnerBucket {
            label: "rayon",
            prefixes: &["rayon::"],
            suggested_paths: &[],
        },
        ProfileOwnerBucket {
            label: "samply/profiler",
            prefixes: &["samply_", "profiler_"],
            suggested_paths: &[],
        },
    ]
}

/// Match a function name to the most specific owner bucket.
///
/// WHAT: Iterates through the ordered bucket list and returns the first
/// bucket whose prefix matches the function name. Falls back to "unknown"
/// for empty names and "other" for unmatched non-empty names.
///
/// WHY: The first-match strategy ensures more specific Moth modules
/// (e.g., AST) take priority over general fallbacks (e.g., moth::).
pub(crate) fn match_owner_bucket(function_name: &str) -> ProfileOwnerBucketMatch {
    if function_name.trim().is_empty() || function_name == "unknown" {
        return ProfileOwnerBucketMatch {
            label: "unknown".to_string(),
            suggested_paths: Vec::new(),
        };
    }

    for bucket in owner_buckets() {
        for prefix in bucket.prefixes {
            if function_name.starts_with(prefix) {
                return ProfileOwnerBucketMatch {
                    label: bucket.label.to_string(),
                    suggested_paths: bucket
                        .suggested_paths
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                };
            }
        }
    }

    ProfileOwnerBucketMatch {
        label: "other".to_string(),
        suggested_paths: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "buckets_tests.rs"]
mod tests;
