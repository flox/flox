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
    // Exclude some endpoints we aren't using
    spec_json["paths"].as_object_mut().unwrap().retain(|k, _| {
        *k != "/metrics/"
            // We don't use any info except for base-catalog and dependencies
            && (*k == "/api/v1/catalog/info/base-catalog"
                || k.starts_with("/api/v1/catalog/info/dependencies")
                || !k.starts_with("/api/v1/catalog/info"))
            && !k.starts_with("/api/v1/catalog/status")
    });
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
    settings.with_pre_hook_async(parse_quote! {
        async |request: &mut ::reqwest::Request| {
            // Propagate the trace ID to catalog-server.
            // This will be a noop when metrics are disabled because Sentry will
            // not have been initialized.
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request.headers_mut().append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        }
    });
    progenitor::Generator::new(&settings)
}

fn generate_client(spec: &OpenAPI) -> String {
    let tokens = generator().generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}
