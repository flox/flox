use std::fs;
use std::path::PathBuf;

use openapiv3::OpenAPI;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let generate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");

    let spec_src = manifest_dir.join("openapi.json");

    let file = std::fs::File::open(&spec_src).unwrap();
    let spec = serde_json::from_reader(file).expect("Failed to parse openapi spec");

    let client = generate_client(&spec);
    let client_dst = generate_dir.join("client.rs");
    fs::write(client_dst, client).unwrap();

    // rerun if the spec changed
    println!("cargo:rerun-if-changed={}", spec_src.display());
}

fn generate_client(spec: &OpenAPI) -> String {
    let mut settings = progenitor::GenerationSettings::default();
    settings.with_derive("PartialEq");
    let mut generator = progenitor::Generator::new(&settings);
    let tokens = generator.generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}
