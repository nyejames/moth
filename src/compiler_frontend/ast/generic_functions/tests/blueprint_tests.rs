//! Focused tests for nominal blueprint semantic agreement.
//!
//! WHAT: proves field provenance is carried without changing nominal identity agreement, including
//!       struct and choice payload blueprints.
//! WHY: imported public projections have no authored field range, so their default diagnostic
//!      location must agree with the source declaration lane without replacing its provenance.

use super::*;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;

fn location(line: i32) -> StableSourceLocation {
    let position = crate::compiler_frontend::tokenizer::tokens::CharPosition {
        line_number: line,
        char_column: 1,
    };
    StableSourceLocation {
        scope: vec!["provider.moth".to_owned()].into_boxed_slice(),
        start: position,
        end: position,
    }
}

fn default_location() -> StableSourceLocation {
    StableSourceLocation {
        scope: Box::new([]),
        start: Default::default(),
        end: Default::default(),
    }
}

fn exported_identity() -> ExportedGenericParameterIdentity {
    let origin = OriginTypeId::new(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("blueprint-tests"),
            "provider".to_owned(),
            ModuleRootRole::Normal,
        ),
        "Box".to_owned(),
        OriginTypeCategory::Struct,
    );
    ExportedGenericParameterIdentity::new(
        GenericDeclarationOrigin::nominal_type(origin).expect("struct origins declare generics"),
        0,
        "T".to_owned(),
    )
}

fn field() -> NominalFieldBlueprint {
    NominalFieldBlueprint {
        name: "value".to_owned(),
        field_type: MaterialisationTypeBlueprint::Canonical(CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Int,
        )),
        folded_default: Some(PublicFoldedValue::Int(7)),
        location: location(10),
    }
}

fn struct_blueprint() -> NominalMaterialisationBlueprint {
    NominalMaterialisationBlueprint {
        generic_parameters: vec![NominalGenericParameterBlueprint {
            name: "T".to_owned(),
            exported_identity: None,
            bounds: Box::new([]),
        }]
        .into_boxed_slice(),
        definition: NominalMaterialisationDefinition::Struct {
            fields: vec![field()].into_boxed_slice(),
            const_record: false,
        },
    }
}

fn choice_blueprint() -> NominalMaterialisationBlueprint {
    NominalMaterialisationBlueprint {
        generic_parameters: Box::new([]),
        definition: NominalMaterialisationDefinition::Choice {
            variants: vec![NominalChoiceVariantBlueprint {
                name: "Some".to_owned(),
                tag: 0,
                payload_fields: vec![field()].into_boxed_slice(),
            }]
            .into_boxed_slice(),
        },
    }
}

#[test]
fn nominal_blueprint_agreement_ignores_provenance_but_checks_identity() {
    let source = struct_blueprint();
    let mut provenance_only = source.clone();
    let NominalMaterialisationDefinition::Struct { fields, .. } = &mut provenance_only.definition
    else {
        unreachable!();
    };
    fields[0].location = location(99);
    assert!(
        source == provenance_only,
        "field provenance must not decide nominal blueprint agreement"
    );

    let mut different_name = source.clone();
    let NominalMaterialisationDefinition::Struct { fields, .. } = &mut different_name.definition
    else {
        unreachable!();
    };
    fields[0].name = "other".to_owned();
    assert!(source != different_name);

    let mut different_type = source.clone();
    let NominalMaterialisationDefinition::Struct { fields, .. } = &mut different_type.definition
    else {
        unreachable!();
    };
    fields[0].field_type = MaterialisationTypeBlueprint::Canonical(CanonicalTypeIdentity::Builtin(
        CanonicalBuiltinType::Bool,
    ));
    assert!(source != different_type);

    let mut different_default = source.clone();
    let NominalMaterialisationDefinition::Struct { fields, .. } = &mut different_default.definition
    else {
        unreachable!();
    };
    fields[0].folded_default = Some(PublicFoldedValue::Int(8));
    assert!(source != different_default);

    let mut different_const_record = source.clone();
    let NominalMaterialisationDefinition::Struct { const_record, .. } =
        &mut different_const_record.definition
    else {
        unreachable!();
    };
    *const_record = true;
    assert!(source != different_const_record);

    let mut different_parameter_name = source.clone();
    different_parameter_name.generic_parameters[0].name = "U".to_owned();
    assert!(source != different_parameter_name);

    let mut different_parameter_bounds = source.clone();
    different_parameter_bounds.generic_parameters[0].bounds = vec![CanonicalTraitIdentity::Core(
        crate::compiler_frontend::canonical_type_identity::CanonicalCoreTraitIdentity::Displayable,
    )]
    .into_boxed_slice();
    assert!(source != different_parameter_bounds);

    let mut different_parameter_export = source.clone();
    different_parameter_export.generic_parameters[0].exported_identity = Some(exported_identity());
    assert!(source != different_parameter_export);

    let choice = choice_blueprint();
    let mut choice_provenance_only = choice.clone();
    let NominalMaterialisationDefinition::Choice { variants } =
        &mut choice_provenance_only.definition
    else {
        unreachable!();
    };
    variants[0].payload_fields[0].location = location(101);
    assert!(
        choice == choice_provenance_only,
        "choice payload provenance must not decide nominal blueprint agreement"
    );

    let mut different_variant_name = choice.clone();
    let NominalMaterialisationDefinition::Choice { variants } =
        &mut different_variant_name.definition
    else {
        unreachable!();
    };
    variants[0].name = "None".to_owned();
    assert!(choice != different_variant_name);

    let mut different_variant_tag = choice.clone();
    let NominalMaterialisationDefinition::Choice { variants } =
        &mut different_variant_tag.definition
    else {
        unreachable!();
    };
    variants[0].tag = 1;
    assert!(choice != different_variant_tag);

    let mut different_payload_name = choice.clone();
    let NominalMaterialisationDefinition::Choice { variants } =
        &mut different_payload_name.definition
    else {
        unreachable!();
    };
    variants[0].payload_fields[0].name = "other".to_owned();
    assert!(choice != different_payload_name);
}

#[test]
fn equivalent_nominal_blueprints_keep_authored_provenance_deterministically() {
    let authored = struct_blueprint();
    let mut imported = authored.clone();
    let NominalMaterialisationDefinition::Struct { fields, .. } = &mut imported.definition else {
        unreachable!();
    };
    fields[0].location = default_location();

    let mut imported_first = imported.clone();
    imported_first.merge_provenance_from(&authored);
    let NominalMaterialisationDefinition::Struct {
        fields: imported_first_fields,
        ..
    } = &imported_first.definition
    else {
        unreachable!();
    };
    assert!(imported_first_fields[0].location == location(10));

    let mut authored_first = authored.clone();
    authored_first.merge_provenance_from(&imported);
    let NominalMaterialisationDefinition::Struct {
        fields: authored_first_fields,
        ..
    } = &authored_first.definition
    else {
        unreachable!();
    };
    assert!(authored_first_fields[0].location == location(10));

    let mut later_authored = authored.clone();
    let NominalMaterialisationDefinition::Struct { fields, .. } = &mut later_authored.definition
    else {
        unreachable!();
    };
    fields[0].location = location(99);
    let mut earlier_authored = authored.clone();
    earlier_authored.merge_provenance_from(&later_authored);
    let NominalMaterialisationDefinition::Struct { fields, .. } = &earlier_authored.definition
    else {
        unreachable!();
    };
    assert!(
        fields[0].location == location(10),
        "both authored ranges use a stable tie-break independent of lane order"
    );
}
