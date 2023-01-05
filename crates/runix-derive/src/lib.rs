use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(ToArgs)]
pub fn to_args_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Parse the input tokens into a syntax tree.
    let ast = parse_macro_input!(input as DeriveInput);

    impl_to_args(&ast)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

fn impl_to_args(ast: &syn::DeriveInput) -> Result<TokenStream, syn::Error> {
    let name = &ast.ident;
    let fields = match ast.data {
        syn::Data::Struct(ref s) => &s.fields,
        _ => Err(syn::Error::new(
            ast.ident.span(),
            "`ToArgs` can only be derived from structs",
        ))?,
    };

    let generics = &ast.generics;

    let conversions = fields
        .iter()
        .enumerate()
        .map(|(n, field)| match field.ident {
            Some(ref i) => quote! { self.#i.to_args() },
            // Tuple structs
            None => quote! { self.#n.to_args() },
        })
        .collect::<Vec<_>>();

    let len = conversions.len();

    let gen = quote! {
        impl #generics ToArgs for #name #generics {
            fn to_args(&self) -> ::std::vec::Vec<::std::string::String> {
                let args: [::std::vec::Vec<::std::string::String>; #len] = [
                    #(#conversions),*
                ];

                args
                .into_iter()
                .flatten()
                .collect()
            }
        }
    };
    Ok(gen)
}
