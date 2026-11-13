//! Proves the compiler convenience path and the standalone crate share host descriptor types.

use moth::builder::{BuilderDefinition, HostPackage};
use moth::host as facade;
use moth_host as standalone;

#[test]
fn facade_and_standalone_share_host_descriptors() {
    // A descriptor built through `moth::host` is the standalone crate's type.
    let descriptor: standalone::HostFunction = facade::HostFunction::new("has_prefix")
        .parameter(facade::HostType::String)
        .parameter(facade::HostType::String)
        .returns(facade::HostType::Bool)
        .fallible();
    assert_eq!(
        descriptor,
        standalone::HostFunction::new("has_prefix")
            .parameter(standalone::HostType::String)
            .parameter(standalone::HostType::String)
            .returns(standalone::HostType::Bool)
            .fallible(),
    );

    // A descriptor built through the standalone crate is the facade's type.
    let standalone_descriptor = standalone::HostFunction::new("count")
        .parameter(standalone::HostType::optional(standalone::HostType::Byte));
    let facade_descriptor: facade::HostFunction = standalone_descriptor;
    assert_eq!(
        facade_descriptor.parameters(),
        [facade::HostType::optional(facade::HostType::Byte)],
    );

    // The compiler registers the standalone type through its public package builder.
    BuilderDefinition::new("game")
        .host_package(HostPackage::new("@platform").function(descriptor))
        .finish()
        .expect("the builder accepts a descriptor from the standalone crate");
}
