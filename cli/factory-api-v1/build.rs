use std::fs;
use std::path::PathBuf;

use openapiv3::OpenAPI;
use syn::parse_quote;

fn main() {
    let generate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");

    let spec_src = PathBuf::from("openapi.json");

    let file = std::fs::File::open(&spec_src).unwrap();
    let mut spec_json: serde_json::Value =
        serde_json::from_reader(file).expect("Failed to parse openapi spec");

    // Keep only paths under /api/v1/factory/builds — the endpoints the CLI
    // verbs (status, list, logs, cancel) call. All other paths are dropped
    // from codegen (schemas referenced by retained paths are still emitted):
    //   - /api/v1/factory/health, /ready  — probe endpoints with untyped `{}`
    //     schemas that Progenitor 0.11.2 cannot process (assertion failure).
    //   - /api/v1/factory/webhooks/*      — GitHub inbound, never called by CLI.
    //   - /api/v1/factory/callbacks/*     — Build Coordinator inbound, not for CLI.
    //   - /api/v1/factory/tasks/*         — task internals, not exposed by CLI verbs.
    spec_json["paths"]
        .as_object_mut()
        .unwrap()
        .retain(|k, _| k.starts_with("/api/v1/factory/builds"));

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
    settings.with_inner_type(parse_quote! { crate::hooks::RequestHooks });
    progenitor::Generator::new(&settings)
}

fn generate_client(spec: &OpenAPI) -> String {
    let tokens = generator().generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}
