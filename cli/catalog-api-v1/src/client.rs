#[allow(unused_imports)]
pub use progenitor_client::{ByteStream, Error, ResponseValue};
#[allow(unused_imports)]
use progenitor_client::{encode_path, RequestBuilderExt};
#[allow(unused_imports)]
use reqwest::header::{HeaderMap, HeaderValue};
/// Types used as operation parameters and responses.
#[allow(clippy::all)]
pub mod types {
    use serde::{Deserialize, Serialize};
    #[allow(unused_imports)]
    use std::convert::TryFrom;
    /// Error types.
    pub mod error {
        /// Error from a TryFrom or FromStr implementation.
        pub struct ConversionError(std::borrow::Cow<'static, str>);
        impl std::error::Error for ConversionError {}
        impl std::fmt::Display for ConversionError {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> Result<(), std::fmt::Error> {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
        impl std::fmt::Debug for ConversionError {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> Result<(), std::fmt::Error> {
                std::fmt::Debug::fmt(&self.0, f)
            }
        }
        impl From<&'static str> for ConversionError {
            fn from(value: &'static str) -> Self {
                Self(value.into())
            }
        }
        impl From<String> for ConversionError {
            fn from(value: String) -> Self {
                Self(value.into())
            }
        }
    }
    ///User
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "User",
    ///  "oneOf": [
    ///    {
    ///      "$ref": "#/components/schemas/Variant1"
    ///    },
    ///    {
    ///      "$ref": "#/components/schemas/Variant2"
    ///    }
    ///  ]
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(untagged)]
    pub enum User {
        Variant1(Variant1),
        Variant2(Variant2),
    }
    impl From<&User> for User {
        fn from(value: &User) -> Self {
            value.clone()
        }
    }
    impl From<Variant1> for User {
        fn from(value: Variant1) -> Self {
            Self::Variant1(value)
        }
    }
    impl From<Variant2> for User {
        fn from(value: Variant2) -> Self {
            Self::Variant2(value)
        }
    }
    ///Variant1
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "variant1",
    ///  "type": "object",
    ///  "properties": {
    ///    "variant": {
    ///      "title": "Variant",
    ///      "default": "variant1"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    pub struct Variant1 {
        #[serde(default = "defaults::variant1_variant")]
        pub variant: serde_json::Value,
    }
    impl From<&Variant1> for Variant1 {
        fn from(value: &Variant1) -> Self {
            value.clone()
        }
    }
    ///Variant2
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Variant2",
    ///  "type": "object",
    ///  "properties": {
    ///    "variant": {
    ///      "title": "Variant",
    ///      "default": "variant2"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    pub struct Variant2 {
        #[serde(default = "defaults::variant2_variant")]
        pub variant: serde_json::Value,
    }
    impl From<&Variant2> for Variant2 {
        fn from(value: &Variant2) -> Self {
            value.clone()
        }
    }
    /// Generation of default values for serde.
    pub mod defaults {
        pub(super) fn variant1_variant() -> serde_json::Value {
            serde_json::from_str::<serde_json::Value>("\"variant1\"").unwrap()
        }
        pub(super) fn variant2_variant() -> serde_json::Value {
            serde_json::from_str::<serde_json::Value>("\"variant2\"").unwrap()
        }
    }
}
#[derive(Clone, Debug)]
/**Client for title

Version: version*/
pub struct Client {
    pub(crate) baseurl: String,
    pub(crate) client: reqwest::Client,
}
impl Client {
    /// Create a new client.
    ///
    /// `baseurl` is the base URL provided to the internal
    /// `reqwest::Client`, and should include a scheme and hostname,
    /// as well as port and a path stem if applicable.
    pub fn new(baseurl: &str) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let client = {
            let dur = std::time::Duration::from_secs(15);
            reqwest::ClientBuilder::new().connect_timeout(dur).timeout(dur)
        };
        #[cfg(target_arch = "wasm32")]
        let client = reqwest::ClientBuilder::new();
        Self::new_with_client(baseurl, client.build().unwrap())
    }
    /// Construct a new client with an existing `reqwest::Client`,
    /// allowing more control over its configuration.
    ///
    /// `baseurl` is the base URL provided to the internal
    /// `reqwest::Client`, and should include a scheme and hostname,
    /// as well as port and a path stem if applicable.
    pub fn new_with_client(baseurl: &str, client: reqwest::Client) -> Self {
        Self {
            baseurl: baseurl.to_string(),
            client,
        }
    }
    /// Get the base URL to which requests are made.
    pub fn baseurl(&self) -> &String {
        &self.baseurl
    }
    /// Get the internal `reqwest::Client` used to make requests.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
    /// Get the version of this API.
    ///
    /// This string is pulled directly from the source OpenAPI
    /// document and may be in any format the API selects.
    pub fn api_version(&self) -> &'static str {
        "version"
    }
}
#[allow(clippy::all)]
impl Client {
    /**Gets a user by ID

A detailed description of the operation. Use markdown for rich text representation, such as **bold**, *italic*, and [links](https://swagger.io).


Sends a `GET` request to `/users/{id}`

Arguments:
- `id`: User ID
*/
    pub async fn get_user_by_id<'a>(
        &'a self,
        id: i64,
    ) -> Result<ResponseValue<types::User>, Error<()>> {
        let url = format!("{}/users/{}", self.baseurl, encode_path(& id.to_string()),);
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .build()?;
        let result = self.client.execute(request).await;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
}
/// Items consumers will typically use such as the Client.
pub mod prelude {
    #[allow(unused_imports)]
    pub use super::Client;
}
