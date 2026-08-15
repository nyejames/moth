//! `@core/prelude` package registration.
//!
//! WHAT: registers the bare prelude symbols that are available without
//! explicit dependency clauses in every module.
//! WHY: the prelude defines the minimal universal surface; builders must
//! provide it, but the actual implementations live in specific core packages
//! such as `@core/io`.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{CORE_IO_PACKAGE_PATH, IO_NAMESPACE_NAME};

pub fn register_core_prelude(registry: &mut ExternalPackageRegistry) {
    registry
        .register_prelude_namespace_alias(IO_NAMESPACE_NAME, CORE_IO_PACKAGE_PATH)
        .expect("prelude namespace alias registration should not collide");
}
