use std::fs;
use std::path::PathBuf;

use openapiv3::OpenAPI;

fn main() {
    let generate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");

    let spec_src = PathBuf::from("openapi.json");

    let file = std::fs::File::open(&spec_src).unwrap();
    let mut spec_json: serde_json::Value =
        serde_json::from_reader(file).expect("Failed to parse openapi spec");
    // This endpoint is causing the generated mock code to fail,
    // and we're not using /metrics/ anyways, so drop it.
    spec_json["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/metrics/")
        .unwrap();
    let spec = serde_json::from_value(spec_json).expect("Failed to parse openapi spec");

    let client = generate_client(&spec);
    let client_dst = generate_dir.join("client.rs");
    fs::write(client_dst, client).unwrap();

    // rerun if the spec changed
    println!("cargo:rerun-if-changed={}", spec_src.display());
}

fn generator() -> progenitor::Generator {
    let mut settings = progenitor::GenerationSettings::default();
    settings.with_derive("PartialEq");
    settings.with_replacement(
        "MessageType",
        "crate::error::MessageType",
        ["Default".parse().unwrap()].into_iter(),
    );
    settings.with_replacement(
        "CatalogStoreConfig",
        "crate::types::CatalogStoreConfig",
        vec![].into_iter(),
    );
    settings.with_replacement(
        "CatalogStoreConfigNixCopy",
        "crate::types::CatalogStoreConfigNixCopy",
        vec![].into_iter(),
    );
    progenitor::Generator::new(&settings)
}

fn generate_client(spec: &OpenAPI) -> String {
    let tokens = generator().generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}
