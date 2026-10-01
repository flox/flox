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
            // We don't use any info except for base-catalog
            && (*k == "/api/v1/catalog/info/base-catalog" || !k.starts_with("/api/v1/catalog/info"))
            && !k.starts_with("/api/v1/catalog/status")
    });
    // Lookup clients opt into v2. Progenitor's generated `anyOf` wrapper
    // parses v2 as v1 because the v1 group permits unknown fields, losing
    // `not_lockable`. Keep the vendored OAS union and narrow only generation.
    spec_json["paths"]["/api/v1/catalog/build-inputs/lookup"]["post"]["responses"]["200"]["content"]
        ["application/json"]["schema"] =
        serde_json::json!({"$ref": "#/components/schemas/BuildInputsLookupResponseV2"});
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
        "LockedGitSource",
        "crate::types::LockedGitSource",
        vec![].into_iter(),
    );
    settings.with_replacement(
        "LockedInputEntry",
        "crate::types::LockedInputEntry",
        vec![].into_iter(),
    );
    settings.with_inner_type(parse_quote! { crate::hooks::RequestHooks });
    progenitor::Generator::new(&settings)
}

fn generate_client(spec: &OpenAPI) -> String {
    let tokens = generator().generate_tokens(spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    prettyplease::unparse(&ast)
}
