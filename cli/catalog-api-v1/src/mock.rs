pub mod operations {
    //! [`When`](httpmock::When) and [`Then`](httpmock::Then)
    //! wrappers for each operation. Each can be converted to
    //! its inner type with a call to `into_inner()`. This can
    //! be used to explicitly deviate from permitted values.
    use crate::*;
    pub struct CreateCatalogApiV1CatalogCatalogsPostWhen(httpmock::When);
    impl CreateCatalogApiV1CatalogCatalogsPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/catalogs/$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn name(self, value: &types::Name) -> Self {
            Self(self.0.query_param("name", value.to_string()))
        }
    }
    pub struct CreateCatalogApiV1CatalogCatalogsPostThen(httpmock::Then);
    impl CreateCatalogApiV1CatalogCatalogsPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn created(self, value: &types::UserCatalog) -> Self {
            Self(
                self
                    .0
                    .status(201u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn conflict(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(409u16)
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
    pub struct GetCatalogApiV1CatalogCatalogsCatalogNameGetWhen(httpmock::When);
    impl GetCatalogApiV1CatalogCatalogsCatalogNameGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/catalogs/[^/]*$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/catalogs/{}$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct GetCatalogApiV1CatalogCatalogsCatalogNameGetThen(httpmock::Then);
    impl GetCatalogApiV1CatalogCatalogsCatalogNameGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::UserCatalog) -> Self {
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
    pub struct DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteWhen(httpmock::When);
    impl DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::DELETE)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/catalogs/[^/]*$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/catalogs/{}$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteThen(httpmock::Then);
    impl DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &serde_json::Value) -> Self {
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
        pub fn not_implemented(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(501u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetWhen(
        httpmock::When,
    );
    impl GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/catalogs/[^/]*/packages$")
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/catalogs/{}/packages$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetThen(
        httpmock::Then,
    );
    impl GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::UserPackageList) -> Self {
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
    pub struct CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostWhen(
        httpmock::When,
    );
    impl CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/catalogs/[^/]*/packages$")
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/catalogs/{}/packages$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn name(self, value: &types::Name) -> Self {
            Self(self.0.query_param("name", value.to_string()))
        }
        pub fn body(self, value: &types::UserPackageCreate) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostThen(
        httpmock::Then,
    );
    impl CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::UserPackage) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn created(self, value: &types::UserPackage) -> Self {
            Self(
                self
                    .0
                    .status(201u16)
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
        pub fn conflict(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(409u16)
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
    pub struct GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetWhen(
        httpmock::When,
    );
    impl GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/packages/[^/]*$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/packages/.*$", value.to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn package_name(self, value: &types::PackageName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/.*/packages/{}$", value.to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetThen(
        httpmock::Then,
    );
    impl GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::UserPackage) -> Self {
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
    pub struct GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetWhen(
        httpmock::When,
    );
    impl GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/packages/[^/]*/builds$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/packages/.*/builds$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn package_name(self, value: &types::PackageName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/.*/packages/{}/builds$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetThen(
        httpmock::Then,
    );
    impl GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::UserBuildList) -> Self {
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
    pub struct CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostWhen(
        httpmock::When,
    );
    impl CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/packages/[^/]*/builds$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/packages/.*/builds$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn package_name(self, value: &types::PackageName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/.*/packages/{}/builds$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn body(self, value: &types::UserBuildPublish) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostThen(
        httpmock::Then,
    );
    impl CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::UserBuildCreationResponse) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn created(self, value: &types::UserBuildCreationResponse) -> Self {
            Self(
                self
                    .0
                    .status(201u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn bad_request(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(400u16)
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
    pub struct PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostWhen(
        httpmock::When,
    );
    impl PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/packages/[^/]*/publish$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/packages/.*/publish$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn package_name(self, value: &types::PackageName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/.*/packages/{}/publish$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn body(self, value: &types::PublishRequest) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostThen(
        httpmock::Then,
    );
    impl PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::PublishResponse) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn bad_request(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(400u16)
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
    pub struct GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetWhen(
        httpmock::When,
    );
    impl GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/catalogs/[^/]*/sharing$")
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/catalogs/{}/sharing$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetThen(
        httpmock::Then,
    );
    impl GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::CatalogShareInfo) -> Self {
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
    pub struct AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostWhen(
        httpmock::When,
    );
    impl AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/sharing/add-read-users$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/sharing/add-read-users$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn body(self, value: &types::CatalogShareInfo) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostThen(
        httpmock::Then,
    );
    impl AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::CatalogShareInfo) -> Self {
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
    pub struct RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostWhen(
        httpmock::When,
    );
    impl RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/sharing/remove-read-users$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/sharing/remove-read-users$", value
                        .to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn body(self, value: &types::CatalogShareInfo) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostThen(
        httpmock::Then,
    );
    impl RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::CatalogShareInfo) -> Self {
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
    pub struct GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetWhen(
        httpmock::When,
    );
    impl GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/store/config$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/store/config$", value.to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
    }
    pub struct GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetThen(
        httpmock::Then,
    );
    impl GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &crate::types::CatalogStoreConfig) -> Self {
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
    pub struct SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutWhen(
        httpmock::When,
    );
    impl SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::PUT)
                    .path_matches(
                        regex::Regex::new(
                                "^/api/v1/catalog/catalogs/[^/]*/store/config$",
                            )
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn catalog_name(self, value: &types::CatalogName) -> Self {
            let re = regex::Regex::new(
                    &format!(
                        "^/api/v1/catalog/catalogs/{}/store/config$", value.to_string()
                    ),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn body(self, value: &crate::types::CatalogStoreConfig) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutThen(
        httpmock::Then,
    );
    impl SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &crate::types::CatalogStoreConfig) -> Self {
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
    pub struct GetPkgPathsApiV1CatalogInfoPkgPathsGetWhen(httpmock::When);
    impl GetPkgPathsApiV1CatalogInfoPkgPathsGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/info/pkg-paths$").unwrap(),
                    ),
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
    }
    pub struct GetPkgPathsApiV1CatalogInfoPkgPathsGetThen(httpmock::Then);
    impl GetPkgPathsApiV1CatalogInfoPkgPathsGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::PkgPathsResult) -> Self {
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
    pub struct PackagesApiV1CatalogPackagesAttrPathGetWhen(httpmock::When);
    impl PackagesApiV1CatalogPackagesAttrPathGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/packages/[^/]*$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn attr_path(self, value: &str) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/packages/{}$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
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
    }
    pub struct PackagesApiV1CatalogPackagesAttrPathGetThen(httpmock::Then);
    impl PackagesApiV1CatalogPackagesAttrPathGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::PackagesResult) -> Self {
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
        pub fn ok(self, value: &types::ResolvedPackageGroups) -> Self {
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
        pub fn ok(self, value: &types::PackageSearchResult) -> Self {
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
    pub struct SettingsApiV1CatalogSettingsKeyPostWhen(httpmock::When);
    impl SettingsApiV1CatalogSettingsKeyPostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/settings/[^/]*$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn key(self, value: &str) -> Self {
            let re = regex::Regex::new(
                    &format!("^/api/v1/catalog/settings/{}$", value.to_string()),
                )
                .unwrap();
            Self(self.0.path_matches(re))
        }
        pub fn value(self, value: &str) -> Self {
            Self(self.0.query_param("value", value.to_string()))
        }
    }
    pub struct SettingsApiV1CatalogSettingsKeyPostThen(httpmock::Then);
    impl SettingsApiV1CatalogSettingsKeyPostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &serde_json::Value) -> Self {
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
    pub struct GetCatalogStatusApiV1CatalogStatusCatalogGetWhen(httpmock::When);
    impl GetCatalogStatusApiV1CatalogStatusCatalogGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/status/catalog$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
    }
    pub struct GetCatalogStatusApiV1CatalogStatusCatalogGetThen(httpmock::Then);
    impl GetCatalogStatusApiV1CatalogStatusCatalogGetThen {
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
        pub fn internal_server_error(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(500u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetWhen(httpmock::When);
    impl GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/status/healthcheck$")
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
    }
    pub struct GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetThen(httpmock::Then);
    impl GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::HealthCheck) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn internal_server_error(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(500u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct TriggerErrorApiV1CatalogStatusSentryDebugGetWhen(httpmock::When);
    impl TriggerErrorApiV1CatalogStatusSentryDebugGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/status/sentry-debug$")
                            .unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
    }
    pub struct TriggerErrorApiV1CatalogStatusSentryDebugGetThen(httpmock::Then);
    impl TriggerErrorApiV1CatalogStatusSentryDebugGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &serde_json::Value) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct GetServiceStatusApiV1CatalogStatusServiceGetWhen(httpmock::When);
    impl GetServiceStatusApiV1CatalogStatusServiceGetWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::GET)
                    .path_matches(
                        regex::Regex::new("^/api/v1/catalog/status/service$").unwrap(),
                    ),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
    }
    pub struct GetServiceStatusApiV1CatalogStatusServiceGetThen(httpmock::Then);
    impl GetServiceStatusApiV1CatalogStatusServiceGetThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::ServiceStatus) -> Self {
            Self(
                self
                    .0
                    .status(200u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
        pub fn internal_server_error(self, value: &types::ErrorResponse) -> Self {
            Self(
                self
                    .0
                    .status(500u16)
                    .header("content-type", "application/json")
                    .json_body_obj(value),
            )
        }
    }
    pub struct GetStoreInfoApiV1CatalogStorePostWhen(httpmock::When);
    impl GetStoreInfoApiV1CatalogStorePostWhen {
        pub fn new(inner: httpmock::When) -> Self {
            Self(
                inner
                    .method(httpmock::Method::POST)
                    .path_matches(regex::Regex::new("^/api/v1/catalog/store$").unwrap()),
            )
        }
        pub fn into_inner(self) -> httpmock::When {
            self.0
        }
        pub fn body(self, value: &types::StoreInfoRequest) -> Self {
            Self(self.0.json_body_obj(value))
        }
    }
    pub struct GetStoreInfoApiV1CatalogStorePostThen(httpmock::Then);
    impl GetStoreInfoApiV1CatalogStorePostThen {
        pub fn new(inner: httpmock::Then) -> Self {
            Self(inner)
        }
        pub fn into_inner(self) -> httpmock::Then {
            self.0
        }
        pub fn ok(self, value: &types::StoreInfoResponse) -> Self {
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
}
/// An extension trait for [`MockServer`](httpmock::MockServer) that
/// adds a method for each operation. These are the equivalent of
/// type-checked [`mock()`](httpmock::MockServer::mock) calls.
pub trait MockServerExt {
    fn create_catalog_api_v1_catalog_catalogs_post<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::CreateCatalogApiV1CatalogCatalogsPostWhen,
            operations::CreateCatalogApiV1CatalogCatalogsPostThen,
        );
    fn get_catalog_api_v1_catalog_catalogs_catalog_name_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogApiV1CatalogCatalogsCatalogNameGetWhen,
            operations::GetCatalogApiV1CatalogCatalogsCatalogNameGetThen,
        );
    fn delete_catalog_api_v1_catalog_catalogs_catalog_name_delete<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteWhen,
            operations::DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteThen,
        );
    fn get_catalog_packages_api_v1_catalog_catalogs_catalog_name_packages_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetWhen,
            operations::GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetThen,
        );
    fn create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostWhen,
            operations::CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostThen,
        );
    fn get_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_package_name_get<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetWhen,
            operations::GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetThen,
        );
    fn get_package_builds_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_get<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetWhen,
            operations::GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetThen,
        );
    fn create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostWhen,
            operations::CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostThen,
        );
    fn publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostWhen,
            operations::PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostThen,
        );
    fn get_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetWhen,
            operations::GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetThen,
        );
    fn add_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_add_read_users_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostWhen,
            operations::AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostThen,
        );
    fn remove_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_remove_read_users_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostWhen,
            operations::RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostThen,
        );
    fn get_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetWhen,
            operations::GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetThen,
        );
    fn set_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_put<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutWhen,
            operations::SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutThen,
        );
    fn get_pkg_paths_api_v1_catalog_info_pkg_paths_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetPkgPathsApiV1CatalogInfoPkgPathsGetWhen,
            operations::GetPkgPathsApiV1CatalogInfoPkgPathsGetThen,
        );
    fn packages_api_v1_catalog_packages_attr_path_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::PackagesApiV1CatalogPackagesAttrPathGetWhen,
            operations::PackagesApiV1CatalogPackagesAttrPathGetThen,
        );
    fn resolve_api_v1_catalog_resolve_post<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::ResolveApiV1CatalogResolvePostWhen,
            operations::ResolveApiV1CatalogResolvePostThen,
        );
    fn search_api_v1_catalog_search_get<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SearchApiV1CatalogSearchGetWhen,
            operations::SearchApiV1CatalogSearchGetThen,
        );
    fn settings_api_v1_catalog_settings_key_post<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SettingsApiV1CatalogSettingsKeyPostWhen,
            operations::SettingsApiV1CatalogSettingsKeyPostThen,
        );
    fn get_catalog_status_api_v1_catalog_status_catalog_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogStatusApiV1CatalogStatusCatalogGetWhen,
            operations::GetCatalogStatusApiV1CatalogStatusCatalogGetThen,
        );
    fn get_catalog_health_check_api_v1_catalog_status_healthcheck_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetWhen,
            operations::GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetThen,
        );
    fn trigger_error_api_v1_catalog_status_sentry_debug_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::TriggerErrorApiV1CatalogStatusSentryDebugGetWhen,
            operations::TriggerErrorApiV1CatalogStatusSentryDebugGetThen,
        );
    fn get_service_status_api_v1_catalog_status_service_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetServiceStatusApiV1CatalogStatusServiceGetWhen,
            operations::GetServiceStatusApiV1CatalogStatusServiceGetThen,
        );
    fn get_store_info_api_v1_catalog_store_post<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetStoreInfoApiV1CatalogStorePostWhen,
            operations::GetStoreInfoApiV1CatalogStorePostThen,
        );
}
impl MockServerExt for httpmock::MockServer {
    fn create_catalog_api_v1_catalog_catalogs_post<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::CreateCatalogApiV1CatalogCatalogsPostWhen,
            operations::CreateCatalogApiV1CatalogCatalogsPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::CreateCatalogApiV1CatalogCatalogsPostWhen::new(when),
                operations::CreateCatalogApiV1CatalogCatalogsPostThen::new(then),
            )
        })
    }
    fn get_catalog_api_v1_catalog_catalogs_catalog_name_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogApiV1CatalogCatalogsCatalogNameGetWhen,
            operations::GetCatalogApiV1CatalogCatalogsCatalogNameGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogApiV1CatalogCatalogsCatalogNameGetWhen::new(when),
                operations::GetCatalogApiV1CatalogCatalogsCatalogNameGetThen::new(then),
            )
        })
    }
    fn delete_catalog_api_v1_catalog_catalogs_catalog_name_delete<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteWhen,
            operations::DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteWhen::new(
                    when,
                ),
                operations::DeleteCatalogApiV1CatalogCatalogsCatalogNameDeleteThen::new(
                    then,
                ),
            )
        })
    }
    fn get_catalog_packages_api_v1_catalog_catalogs_catalog_name_packages_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetWhen,
            operations::GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetWhen::new(
                    when,
                ),
                operations::GetCatalogPackagesApiV1CatalogCatalogsCatalogNamePackagesGetThen::new(
                    then,
                ),
            )
        })
    }
    fn create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostWhen,
            operations::CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostWhen::new(
                    when,
                ),
                operations::CreateCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPostThen::new(
                    then,
                ),
            )
        })
    }
    fn get_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_package_name_get<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetWhen,
            operations::GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetWhen::new(
                    when,
                ),
                operations::GetCatalogPackageApiV1CatalogCatalogsCatalogNamePackagesPackageNameGetThen::new(
                    then,
                ),
            )
        })
    }
    fn get_package_builds_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_get<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetWhen,
            operations::GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetWhen::new(
                    when,
                ),
                operations::GetPackageBuildsApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsGetThen::new(
                    then,
                ),
            )
        })
    }
    fn create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostWhen,
            operations::CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostWhen::new(
                    when,
                ),
                operations::CreatePackageBuildApiV1CatalogCatalogsCatalogNamePackagesPackageNameBuildsPostThen::new(
                    then,
                ),
            )
        })
    }
    fn publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostWhen,
            operations::PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostWhen::new(
                    when,
                ),
                operations::PublishRequestApiV1CatalogCatalogsCatalogNamePackagesPackageNamePublishPostThen::new(
                    then,
                ),
            )
        })
    }
    fn get_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetWhen,
            operations::GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetWhen::new(
                    when,
                ),
                operations::GetCatalogSharingApiV1CatalogCatalogsCatalogNameSharingGetThen::new(
                    then,
                ),
            )
        })
    }
    fn add_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_add_read_users_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostWhen,
            operations::AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostWhen::new(
                    when,
                ),
                operations::AddCatalogSharingApiV1CatalogCatalogsCatalogNameSharingAddReadUsersPostThen::new(
                    then,
                ),
            )
        })
    }
    fn remove_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_remove_read_users_post<
        F,
    >(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostWhen,
            operations::RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostWhen::new(
                    when,
                ),
                operations::RemoveCatalogSharingApiV1CatalogCatalogsCatalogNameSharingRemoveReadUsersPostThen::new(
                    then,
                ),
            )
        })
    }
    fn get_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetWhen,
            operations::GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetWhen::new(
                    when,
                ),
                operations::GetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigGetThen::new(
                    then,
                ),
            )
        })
    }
    fn set_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_put<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutWhen,
            operations::SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutWhen::new(
                    when,
                ),
                operations::SetCatalogStoreConfigApiV1CatalogCatalogsCatalogNameStoreConfigPutThen::new(
                    then,
                ),
            )
        })
    }
    fn get_pkg_paths_api_v1_catalog_info_pkg_paths_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetPkgPathsApiV1CatalogInfoPkgPathsGetWhen,
            operations::GetPkgPathsApiV1CatalogInfoPkgPathsGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetPkgPathsApiV1CatalogInfoPkgPathsGetWhen::new(when),
                operations::GetPkgPathsApiV1CatalogInfoPkgPathsGetThen::new(then),
            )
        })
    }
    fn packages_api_v1_catalog_packages_attr_path_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::PackagesApiV1CatalogPackagesAttrPathGetWhen,
            operations::PackagesApiV1CatalogPackagesAttrPathGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::PackagesApiV1CatalogPackagesAttrPathGetWhen::new(when),
                operations::PackagesApiV1CatalogPackagesAttrPathGetThen::new(then),
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
    fn settings_api_v1_catalog_settings_key_post<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::SettingsApiV1CatalogSettingsKeyPostWhen,
            operations::SettingsApiV1CatalogSettingsKeyPostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::SettingsApiV1CatalogSettingsKeyPostWhen::new(when),
                operations::SettingsApiV1CatalogSettingsKeyPostThen::new(then),
            )
        })
    }
    fn get_catalog_status_api_v1_catalog_status_catalog_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogStatusApiV1CatalogStatusCatalogGetWhen,
            operations::GetCatalogStatusApiV1CatalogStatusCatalogGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogStatusApiV1CatalogStatusCatalogGetWhen::new(when),
                operations::GetCatalogStatusApiV1CatalogStatusCatalogGetThen::new(then),
            )
        })
    }
    fn get_catalog_health_check_api_v1_catalog_status_healthcheck_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetWhen,
            operations::GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetWhen::new(
                    when,
                ),
                operations::GetCatalogHealthCheckApiV1CatalogStatusHealthcheckGetThen::new(
                    then,
                ),
            )
        })
    }
    fn trigger_error_api_v1_catalog_status_sentry_debug_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::TriggerErrorApiV1CatalogStatusSentryDebugGetWhen,
            operations::TriggerErrorApiV1CatalogStatusSentryDebugGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::TriggerErrorApiV1CatalogStatusSentryDebugGetWhen::new(when),
                operations::TriggerErrorApiV1CatalogStatusSentryDebugGetThen::new(then),
            )
        })
    }
    fn get_service_status_api_v1_catalog_status_service_get<F>(
        &self,
        config_fn: F,
    ) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetServiceStatusApiV1CatalogStatusServiceGetWhen,
            operations::GetServiceStatusApiV1CatalogStatusServiceGetThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetServiceStatusApiV1CatalogStatusServiceGetWhen::new(when),
                operations::GetServiceStatusApiV1CatalogStatusServiceGetThen::new(then),
            )
        })
    }
    fn get_store_info_api_v1_catalog_store_post<F>(&self, config_fn: F) -> httpmock::Mock
    where
        F: FnOnce(
            operations::GetStoreInfoApiV1CatalogStorePostWhen,
            operations::GetStoreInfoApiV1CatalogStorePostThen,
        ),
    {
        self.mock(|when, then| {
            config_fn(
                operations::GetStoreInfoApiV1CatalogStorePostWhen::new(when),
                operations::GetStoreInfoApiV1CatalogStorePostThen::new(then),
            )
        })
    }
}
