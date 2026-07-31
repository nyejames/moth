import @sub { ExportedStruct }
import @sub { ExportedChoice }
import @sub { ExportedAlias }

instance = ExportedStruct("hello")
variant = ExportedChoice::VariantA
aliased_value ExportedAlias = "world"

choice_label ~= "unknown"
if variant is:
    VariantA => choice_label = "VariantA"
    VariantB(payload) => choice_label = "VariantB"
;

io.line([: module_facade_unprefixed_type_export struct=[instance.value] choice=[choice_label] alias=[aliased_value]])
