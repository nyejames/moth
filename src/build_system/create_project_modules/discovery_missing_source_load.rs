//! Missing-source loading for Stage 0 discovery.
//!
//! WHAT: `load_missing_sources` runs serially below `STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES`.
//!       Larger batches use `load_missing_sources_parallel`, which runs on Rayon.
//!       Both paths sort results by `missing_source_load_input_index`.
//! WHY:  small projects and mostly-cached modules stay serial, while markdown-heavy modules
//!       overlap filesystem reads. `Stage0SerialSourceLoadCount` and
//!       `Stage0ParallelSourceLoadCount` record which path ran.
use super::*;
pub(super) fn load_missing_sources(
    missing_sources: Vec<MissingSourceFile>,
) -> Vec<MissingSourceLoadResult> {
    if missing_sources.is_empty() {
        return Vec::new();
    }

    if missing_sources.len() < STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES {
        add_frontend_counter(
            FrontendCounter::Stage0SerialSourceLoadCount,
            missing_sources.len(),
        );
        return load_missing_sources_serial(missing_sources);
    }

    add_frontend_counter(
        FrontendCounter::Stage0ParallelSourceLoadCount,
        missing_sources.len(),
    );
    load_missing_sources_parallel(missing_sources)
}

fn load_missing_sources_serial(
    missing_sources: Vec<MissingSourceFile>,
) -> Vec<MissingSourceLoadResult> {
    let mut load_results = missing_sources
        .into_iter()
        .map(
            |missing| match read_source_code(&missing.source_file.path) {
                Ok(source_code) => MissingSourceLoadResult::Loaded(LoadedMissingSourceFile {
                    input_index: missing.input_index,
                    source_code,
                }),
                Err(error) => MissingSourceLoadResult::Failed(SourceReadFailure {
                    input_index: missing.input_index,
                    path: missing.source_file.path,
                    error,
                }),
            },
        )
        .collect::<Vec<_>>();
    load_results.sort_by_key(missing_source_load_input_index);
    load_results
}

fn load_missing_sources_parallel(
    missing_sources: Vec<MissingSourceFile>,
) -> Vec<MissingSourceLoadResult> {
    let mut load_results = missing_sources
        .into_par_iter()
        .map(
            |missing| match read_source_code(&missing.source_file.path) {
                Ok(source_code) => MissingSourceLoadResult::Loaded(LoadedMissingSourceFile {
                    input_index: missing.input_index,
                    source_code,
                }),
                Err(error) => MissingSourceLoadResult::Failed(SourceReadFailure {
                    input_index: missing.input_index,
                    path: missing.source_file.path,
                    error,
                }),
            },
        )
        .collect::<Vec<_>>();

    load_results.sort_by_key(missing_source_load_input_index);

    load_results
}
