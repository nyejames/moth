use super::super::file_reference_resolution::{
    SingleFileReferenceOutcome, SingleFileReferenceResolver, stable_case_mismatch,
};
use super::super::resource_inputs::ResourceInputRegistry;
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::InvalidCompileTimePathReason;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReference, PreparedFileReferenceClass,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;

#[test]
fn case_mismatch_evidence_is_sorted_and_unicode_lowercased() {
    let mut ambiguous = vec!["data".to_owned(), "Data".to_owned()];
    assert_eq!(
        stable_case_mismatch("DATA", &mut ambiguous),
        Some("Data".to_owned())
    );

    let mut unicode = vec!["Éclair".to_owned()];
    assert_eq!(
        stable_case_mismatch("éCLAIR", &mut unicode),
        Some("Éclair".to_owned())
    );
}

#[test]
fn synthetic_physical_resolution_cache_reuses_settled_outcome() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let resource_directory = temp.path().join("assets");
    fs::create_dir_all(&resource_directory).expect("resource directory should be created");
    let resource_path = resource_directory.join("logo.svg");
    fs::write(&resource_path, "resource").expect("resource should be created");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let mut resource_inputs = ResourceInputRegistry::new();
    let canonical_root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let mut resolver = SingleFileReferenceResolver::new(
        canonical_root.clone(),
        &source_file_kinds,
        &mut resource_inputs,
    );
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork.try_intern_portable_path("assets/logo.svg", &mut strings).expect("test path fits");
    let path_syntax_id = path_syntax.push(
        path.clone(),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ResourceFile,
    };

    let owner = canonical_root.join("main.moth");
    let first = resolver
        .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
        .expect("first resource reference should resolve");
    let second = resolver
        .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
        .expect("repeated resource reference should resolve");

    assert!(
        matches!(first.outcome, SingleFileReferenceOutcome::Resource { .. }),
        "first outcome: {:?}",
        first.outcome
    );
    assert!(matches!(
        second.outcome,
        SingleFileReferenceOutcome::Resource { .. }
    ));
    assert_eq!(resource_inputs.records().len(), 1);
}

#[test]
fn synthetic_moth_value_skips_physical_resolution_for_missing_target() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork.try_intern_portable_path("missing.moth", &mut strings).expect("test path fits");
    let path_syntax_id = path_syntax.push(
        path.clone(),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::SourceKindNoFileValue,
    };

    let resolved = resolver
        .resolve(
            &root.join("main.moth"),
            &path_syntax,
            &reference,
            &mut strings,
            &path_fork,
        )
        .expect("a .moth value should be identified without a physical target");

    assert!(matches!(
        resolved.outcome,
        SingleFileReferenceOutcome::IdentifiedSourceKind
    ));
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[test]
fn synthetic_not_a_directory_is_a_typed_path_failure() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let blocker = temp.path().join("not_a_directory");
    fs::write(&blocker, "regular file").expect("blocker file should be created");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver = SingleFileReferenceResolver::new(
        fs::canonicalize(temp.path()).expect("test root should canonicalize"),
        &source_file_kinds,
        &mut resource_inputs,
    );
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork.try_intern_portable_path("not_a_directory/value.mtf", &mut strings).expect("test path fits");
    let path_syntax_id = path_syntax.push(
        path.clone(),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };
    let resolved = resolver
        .resolve(
            &temp.path().join("main.moth"),
            &path_syntax,
            &reference,
            &mut strings,
            &path_fork,
        )
        .expect("NotADirectory should remain a typed path outcome");
    let SingleFileReferenceOutcome::Diagnostic(diagnostic) = resolved.outcome else {
        panic!("NotADirectory should remain a typed path diagnostic");
    };
    assert_eq!(
        diagnostic.primary_span,
        Some(reference.span),
        "synthetic file-reference diagnostics must retain the authored source span"
    );
    assert!(matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
            reason: InvalidCompileTimePathReason::TargetNotRegular,
            ..
        }
    ));
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_in_owner_dangling_alias_watches_resolved_physical_prefix() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let real = root.join("real");
    fs::create_dir_all(&real).expect("real directory should be created");
    symlink(real.join("missing-target"), root.join("alias"))
        .expect("dangling in-owner alias should be created");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork.try_intern_portable_path("alias/leaf.svg", &mut strings).expect("test path fits");
    let path_syntax_id = path_syntax.push(
        path.clone(),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ResourceFile,
    };

    let owner = root.join("main.moth");
    for _ in 0..2 {
        let resolved = resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
            .expect("in-owner dangling alias should settle");
        assert!(matches!(
            resolved.outcome,
            SingleFileReferenceOutcome::Diagnostic(diagnostic)
                if matches!(
                    diagnostic.payload,
                    crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                        reason: InvalidCompileTimePathReason::MissingTarget,
                        ..
                    }
                )
        ));
    }

    assert_eq!(resource_inputs.missing_watch_interests().len(), 1);
    assert_eq!(
        resource_inputs.missing_watch_interests()[0].canonical_path(),
        real.join("missing-target")
    );
}

#[cfg(unix)]
#[test]
fn synthetic_dangling_alias_resolves_nested_symlink_targets_before_parent_segments() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let outside_temp = tempfile::tempdir().expect("outside root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let outside_root =
        fs::canonicalize(outside_temp.path()).expect("outside root should canonicalize");
    fs::create_dir_all(root.join("inside")).expect("inside directory should be created");
    symlink(&outside_root, root.join("inside/redirect"))
        .expect("nested external target should be created");
    symlink("inside/redirect/..", root.join("alias"))
        .expect("nested dangling alias should be created");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork.try_intern_portable_path("alias/missing.svg", &mut strings).expect("test path fits");
    let path_syntax_id = path_syntax.push(
        path.clone(),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ResourceFile,
    };

    let resolved = resolver
        .resolve(
            &root.join("main.moth"),
            &path_syntax,
            &reference,
            &mut strings,
            &path_fork,
        )
        .expect("nested dangling alias should settle");
    assert!(matches!(
        resolved.outcome,
        SingleFileReferenceOutcome::Diagnostic(diagnostic)
            if matches!(
                diagnostic.payload,
                crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                    reason: InvalidCompileTimePathReason::EscapesSymlink,
                    ..
                }
            )
    ));
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_dangling_symlink_cycles_and_excessive_depth_are_infrastructure_errors() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    symlink(root.join("cycle-b"), root.join("cycle-a"))
        .expect("first cycle link should be created");
    symlink(root.join("cycle-a"), root.join("cycle-b"))
        .expect("second cycle link should be created");
    for index in 0..=41 {
        let target = if index == 41 {
            root.join("missing-target")
        } else {
            root.join(format!("deep-{}", index + 1))
        };
        symlink(target, root.join(format!("deep-{index}")))
            .expect("deep symlink should be created");
    }

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();

    let mut resolve = |resolver: &mut SingleFileReferenceResolver<'_>,
                   path: &str,
                   strings: &mut StringTable,
                   path_syntax: &mut PathSyntaxTable| {
        let authored_path = path_fork.try_intern_portable_path(path, strings).expect("test path fits");
        let path_syntax_id = path_syntax.push(
            authored_path.clone(),
            SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        );
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class: PreparedFileReferenceClass::ResourceFile,
        };
        resolver.resolve(&root.join("main.moth"), path_syntax, &reference, strings, &path_fork)
    };

    assert!(
        resolve(
            &mut resolver,
            "cycle-a/leaf.svg",
            &mut strings,
            &mut path_syntax
        )
        .is_err()
    );
    assert!(
        resolve(
            &mut resolver,
            "deep-0/leaf.svg",
            &mut strings,
            &mut path_syntax
        )
        .is_err()
    );
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_rejects_support_and_missing_symlink_boundaries_without_watches() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let outside_temp = tempfile::tempdir().expect("outside root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let support_root = root.join("support");
    let child_root = root.join("child");
    let outside_root =
        fs::canonicalize(outside_temp.path()).expect("outside root should canonicalize");
    fs::create_dir_all(&support_root).expect("support root should be created");
    fs::create_dir_all(&child_root).expect("child root should be created");
    fs::write(support_root.join("+support.moth"), "support root")
        .expect("support root file should be created");
    fs::write(child_root.join("@child.moth"), "child root")
        .expect("child root file should be created");
    fs::write(support_root.join("existing.mtf"), "template")
        .expect("support content should be created");
    fs::write(support_root.join("existing.svg"), "resource")
        .expect("support resource should be created");
    symlink(&support_root, root.join("support-alias")).expect("support alias should be created");
    symlink(&child_root, root.join("child-alias")).expect("child alias should be created");
    symlink(&outside_root, root.join("outside-alias")).expect("outside alias should be created");
    symlink(
        outside_root.join("missing-target"),
        root.join("dangling-external"),
    )
    .expect("dangling external alias should be created");
    symlink(
        child_root.join("missing-target"),
        root.join("dangling-child"),
    )
    .expect("dangling child alias should be created");
    symlink(
        support_root.join("missing-target"),
        root.join("dangling-support"),
    )
    .expect("dangling support alias should be created");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();

    let mut resolve = |resolver: &mut SingleFileReferenceResolver<'_>,
                   path: &str,
                   class: PreparedFileReferenceClass,
                   strings: &mut StringTable,
                   path_syntax: &mut PathSyntaxTable| {
        let authored_path = path_fork.try_intern_portable_path(path, strings).expect("test path fits");
        let path_syntax_id = path_syntax.push(
            authored_path.clone(),
            SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        );
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class,
        };
        resolver
            .resolve(&root.join("main.moth"), path_syntax, &reference, strings, &path_fork)
            .expect("synthetic reference should settle")
    };

    for (path, class) in [
        (
            "support/existing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
        (
            "support/existing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "support/missing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
        (
            "support/missing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "support-alias/missing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
        (
            "child-alias/missing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "dangling-child/missing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "dangling-support/missing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
    ] {
        let resolved = resolve(&mut resolver, path, class, &mut strings, &mut path_syntax);
        assert!(
            matches!(
                resolved.outcome,
                SingleFileReferenceOutcome::Diagnostic(diagnostic)
                    if matches!(
                        diagnostic.payload,
                        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                            reason: InvalidCompileTimePathReason::EscapesModuleBoundary,
                            ..
                        }
                    )
            ),
            "expected module-boundary rejection for {path}"
        );
    }

    for path in [
        "outside-alias/missing.mtf",
        "outside-alias/missing.svg",
        "dangling-external/missing.mtf",
        "dangling-external/missing.svg",
    ] {
        let resolved = resolve(
            &mut resolver,
            path,
            PreparedFileReferenceClass::ResourceFile,
            &mut strings,
            &mut path_syntax,
        );
        assert!(
            matches!(
                resolved.outcome,
                SingleFileReferenceOutcome::Diagnostic(diagnostic)
                    if matches!(
                        diagnostic.payload,
                        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                            reason: InvalidCompileTimePathReason::EscapesSymlink,
                            ..
                        }
                    )
            ),
            "expected symlink escape rejection for {path}"
        );
    }
    assert!(resource_inputs.missing_watch_interests().is_empty());
}
