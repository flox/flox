pub mod operations {
    //! [`When`](httpmock::When) and [`Then`](httpmock::Then)
    //! wrappers for each operation. Each can be converted to
    //! its inner type with a call to `into_inner()`. This can
    //! be used to explicitly deviate from permitted values.
    use crate::*;
    pub struct SearchApiV1CatalogSearchGetWhen(httpmock::When);
    impl SearchApiV1CatalogSearchGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(regex::Regex::new("^/api/v1/catalog/search$").unwrap()),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalogs<'a, T>(self, value: T) -> Self
        where
            T: Into<Option<&'a str>>,
        {
            if let Some(value) = value.into() {
                Self(self.0.query_param("catalogs", value.to_string()))
            } else {
                Self(
                    self
                        .0
                        .matches(|req| {
                            req.query_params
                                .as_ref()
                                .and_then(|qs| {
                                    qs.iter().find(|(key, _)| key == "catalogs")
                                })
                                .is_none()
                        }),
                )
            }
        }
        pub fn page<T>(self, value: T) -> Self
        where
            T: Into<Option<i64>>,
        {
            if let Some(value) = value.into() {
                Self(self.0.query_param("page", value.to_string()))
            } else {
                Self(
                    self
                        .0
                        .matches(|req| {
                            req.query_params
                                .as_ref()
                                .and_then(|qs| { qs.iter().find(|(key, _)| key == "page") })
                                .is_none()
                        }),
                )
            }
        }
        pub fn page_size<T>(self, value: T) -> Self
        where
            T: Into<Option<i64>>,
        {
            if let Some(value) = value.into() {
                Self(self.0.query_param("page_size", value.to_string()))
            } else {
                Self(
                    self
                        .0
                        .matches(|req| {
                            req.query_params
                                .as_ref()
                                .and_then(|qs| {
                                    qs.iter().find(|(key, _)| key == "page_size")
                                })
                                .is_none()
                        }),
                )
            }
        }
        pub fn search_term(self, value: &types::SearchTerm) -> Self {
            Self(self.0.query_param("search_term", value.to_string()))
        }
        pub fn system(self, value: types::SystemEnum) -> Self {
            Self(self.0.query_param("system", value.to_string()))
        }
    }
    pub struct SearchApiV1CatalogSearchGetThen(httpmock::Then);
    impl SearchApiV1CatalogSearchGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::PackageSearchResultInput) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn unprocessable_entity(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(422u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct BuildsApiV1CatalogBuildsGetWhen(httpmock::When);
    impl BuildsApiV1CatalogBuildsGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(regex::Regex::new("^/api/v1/catalog/builds$").unwrap()),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn page<T>(self, value: T) -> Self
        where
            T: Into<Option<i64>>,
        {
            if let Some(value) = value.into() {
                Self(self.0.query_param("page", value.to_string()))
            } else {
                Self(
                    self
                        .0
                        .matches(|req| {
                            req.query_params
                                .as_ref()
                                .and_then(|qs| { qs.iter().find(|(key, _)| key == "page") })
                                .is_none()
                        }),
                )
            }
        }
        pub fn page_size<T>(self, value: T) -> Self
        where
            T: Into<Option<i64>>,
        {
            if let Some(value) = value.into() {
                Self(self.0.query_param("page_size", value.to_string()))
            } else {
                Self(
                    self
                        .0
                        .matches(|req| {
                            req.query_params
                                .as_ref()
                                .and_then(|qs| {
                                    qs.iter().find(|(key, _)| key == "page_size")
                                })
                                .is_none()
                        }),
                )
            }
        }
        pub fn pkg_path(self, value: &str) -> Self {
            Self(self.0.query_param("pkg_path", value.to_string()))
        }
    }
    pub struct BuildsApiV1CatalogBuildsGetThen(httpmock::Then);
    impl BuildsApiV1CatalogBuildsGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::PackageBuildsResultInput) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn not_found(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(404u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn unprocessable_entity(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(422u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct ResolveApiV1CatalogResolvePostWhen(httpmock::When);
    impl ResolveApiV1CatalogResolvePostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/resolve$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn body(self, value: &types::PackageGroups) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct ResolveApiV1CatalogResolvePostThen(httpmock::Then);
    impl ResolveApiV1CatalogResolvePostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::ResolvedPackageGroupsInput) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn not_acceptable(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(406u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn unprocessable_entity(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(422u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct GetStatusApiV1MetricsStatusGetWhen(httpmock::When);
    impl GetStatusApiV1MetricsStatusGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(regex::Regex::new("^/api/v1/metrics/status$").unwrap()),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
    }
    pub struct GetStatusApiV1MetricsStatusGetThen(httpmock::Then);
    impl GetStatusApiV1MetricsStatusGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::CatalogStatus) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
}
/// An extension trait for [`MockServer`](httpmock::MockServer) that
/// adds a method for each operation. These are the equivalent of
/// type-checked [`mock()`](httpmock::MockServer::mock) calls.
pub trait MockServerExt {
    fn search_api_v1_catalog_search_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SearchApiV1CatalogSearchGetWhen,
            operations::SearchApiV1CatalogSearchGetThen,
        );
    fn builds_api_v1_catalog_builds_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::BuildsApiV1CatalogBuildsGetWhen,
            operations::BuildsApiV1CatalogBuildsGetThen,
        );
    fn resolve_api_v1_catalog_resolve_post<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::ResolveApiV1CatalogResolvePostWhen,
            operations::ResolveApiV1CatalogResolvePostThen,
        );
    fn get_status_api_v1_metrics_status_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetStatusApiV1MetricsStatusGetWhen,
            operations::GetStatusApiV1MetricsStatusGetThen,
        );
}
impl MockServerExt for httpmock::MockServer {
    fn search_api_v1_catalog_search_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SearchApiV1CatalogSearchGetWhen,
            operations::SearchApiV1CatalogSearchGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::SearchApiV1CatalogSearchGetWhen::new(when),
                operations::SearchApiV1CatalogSearchGetThen::new(then),
            )
        })
    }
    fn builds_api_v1_catalog_builds_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::BuildsApiV1CatalogBuildsGetWhen,
            operations::BuildsApiV1CatalogBuildsGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::BuildsApiV1CatalogBuildsGetWhen::new(when),
                operations::BuildsApiV1CatalogBuildsGetThen::new(then),
            )
        })
    }
    fn resolve_api_v1_catalog_resolve_post<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::ResolveApiV1CatalogResolvePostWhen,
            operations::ResolveApiV1CatalogResolvePostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::ResolveApiV1CatalogResolvePostWhen::new(when),
                operations::ResolveApiV1CatalogResolvePostThen::new(then),
            )
        })
    }
    fn get_status_api_v1_metrics_status_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetStatusApiV1MetricsStatusGetWhen,
            operations::GetStatusApiV1MetricsStatusGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetStatusApiV1MetricsStatusGetWhen::new(when),
                operations::GetStatusApiV1MetricsStatusGetThen::new(then),
            )
        })
    }
}
