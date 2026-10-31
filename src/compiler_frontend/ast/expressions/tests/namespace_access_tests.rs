//! Namespace value-position member selection tests.

use super::*;
use crate::compiler_frontend::external_packages::{
    ExternalConstantId, ExternalSymbolId, ExternalTypeId,
};
use crate::compiler_frontend::headers::binding_environment::{
    NamespaceTypeMember, NamespaceValueMember,
};

#[test]
fn value_position_leaf_excludes_type_and_child_namespace_candidates() {
    let mut string_table = StringTable::new();
    let package = string_table.intern("@test");
    let value_name = string_table.intern("PI");
    let type_name = string_table.intern("Point");
    let child_name = string_table.intern("tools");
    let mut record = NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package));
    record.value_members.insert(
        value_name,
        NamespaceValueMember::ExternalSymbol(ExternalSymbolId::Constant(ExternalConstantId(1))),
    );
    record.type_members.insert(
        type_name,
        NamespaceTypeMember::ExternalSymbol(ExternalSymbolId::Type(ExternalTypeId(2))),
    );
    record.child_namespaces.insert(
        child_name,
        NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package)),
    );

    let candidates = record.member_names_for_role(NamespaceMemberRole::Value);
    assert!(candidates.contains(&value_name));
    assert!(
        !candidates.contains(&type_name),
        "final value members must not suggest types"
    );
    assert!(
        !candidates.contains(&child_name),
        "final value members must not suggest child namespaces"
    );
}
