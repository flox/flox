use std::fs;
use std::path::PathBuf;

use openapiv3::OpenAPI;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let generate_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let spec_src = manifest_dir.join("openapi.json");

    // rerun if the spec changes
    println!("cargo:rerun-if-changed={}", spec_src.display());

    let file = std::fs::File::open(spec_src).unwrap();
    let spec = serde_json::from_reader(file).expect("Failed to parse openapi spec");

    let client = generate_client(&spec);
    let client_dst = generate_dir.join("client.rs");
    fs::write(client_dst, client).unwrap();

    let mock = generate_mock(&spec);
    let mock_dst = generate_dir.join("mock.rs");
    fs::write(mock_dst, mock).unwrap();
}

fn generate_client(spec: &OpenAPI) -> String {
    let mut generator = progenitor::Generator::default();
    let tokens = generator.generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}

fn generate_mock(spec: &OpenAPI) -> String {
    let mut generator = progenitor::Generator::default();
    let tokens = generator.httpmock(spec, "crate").unwrap();

    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}
