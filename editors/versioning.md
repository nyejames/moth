# Editor release identities

Public releases follow Obvious Versioning (ObVer), as specified by the project lead:

    YYYY[-MM[-DD]].MINOR[.MICRO][-LABEL]

The first unlabelled official release is MAJOR.0. The date is UTC and Gregorian. A public breaking change requires a greater date-based major with counters reset. A compatible feature increments MINOR and resets MICRO, and compatible fixes increment MICRO. Omit zero MICRO. An official changed source revision requires a new numeric version. A major can remain in use across calendar years. Use the current month or day precision only when a new major needs a distinct date at that precision. No additional official major on the same fully qualified day.

Compare parsed year, month, day, minor and micro, using zero for absent fields. Labels have no precedence. A label is lowercase ASCII snake_case starting with a letter. Published full identifiers are immutable and unique within the project. Exact selection includes the presence or absence of the label. Labelled development builds need not advance the official history. No ObVer version ranges or compatibility promises for labelled builds.

The editor package programme and shared grammars use ObVer publicly. Keep a release manifest with the full canonical ObVer identifier, exact source revision, grammar ABI, syntax compatibility, and per-package registry versions when there is a real release consumer. Internal Rust crates and their compiler crate dependency constraints use SemVer. npm, editor extension manifests and Tree-sitter metadata use SemVer wherever required. A registry version is not an ObVer comparator or build selector. Do not derive compatibility from an accidental overlap between valid spellings. Track an explicit mapping and ensure registry upgrades remain valid, including date-qualified and labelled builds.

Development manifests use private, unpublished SemVer placeholder versions. They do not declare a public ObVer release or select a release date. Choose publication identities and implement any mapping tooling at the release milestone. Keep the public compatibility contract for editor versions and compiler syntax separate from the compiler's Rust crate versions.
