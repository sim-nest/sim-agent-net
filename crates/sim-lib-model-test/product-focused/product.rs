#[path = "../src/product.rs"]
mod product_impl;

use product_impl::{EXIT_INCOMPLETE, EXIT_NOT_FOUND, EXIT_OK, EXIT_USAGE, OUTPUT_SCHEMA, run_command};

#[test]
fn product_is_loadable_and_offline_projections_do_not_contact_providers() {
    let library = product_impl::ModelTestLib;
    let _ = library;
    assert_eq!([EXIT_OK, EXIT_USAGE, EXIT_NOT_FOUND, EXIT_INCOMPLETE], [0, 2, 3, 4]);
    for verb in ["packs", "status", "verify", "report", "decide", "pick"] {
        let output = run_command(&["model-test".into(), verb.into(), "--machine".into()]).unwrap();
        assert!(output.contains(OUTPUT_SCHEMA));
        assert!(output.contains("\"provider_contact\":\"none\""));
    }
}
