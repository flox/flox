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
    settings.with_replacement(
        "MessageType",
        "crate::error::MessageType",
        ["Default".parse().unwrap()].into_iter(),
    );
    settings.with_pre_hook_mut(ADD_SENTRY_HEADERS_SRC.parse().unwrap());
    let mut generator = progenitor::Generator::new(&settings);
    let tokens = generator.generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}

const ADD_SENTRY_HEADERS_SRC: &str = "|request: &mut reqwest::Request| {
    if let Some(span) = sentry::configure_scope(|scope| scope.get_span()) {
        let headers = request.headers_mut();
        for (k, v) in span.iter_headers() {
            headers.insert(k, HeaderValue::from_str(&v).unwrap());
        }
    }
}";
