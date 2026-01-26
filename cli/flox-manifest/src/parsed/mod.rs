pub mod common;
pub mod v1;
pub mod v1_9_0;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("no package or group named '{0}' in the manifest")]
    PkgOrGroupNotFound(String),
    #[error("no package named '{0}' in the manifest")]
    PackageNotFound(String),
    #[error(
        "multiple packages match '{0}', please specify an install id from possible matches: {1:?}"
    )]
    MultiplePackagesMatch(String, Vec<String>),
    #[error("not a valid activation mode")]
    ActivateModeInvalid,

    #[error("outputs '{0:?}' don't exists for package {1}")]
    InvalidOutputs(Vec<String>, String),

    #[error("{0}")]
    InvalidServiceConfig(String),
}
/// An interface codifying how to access types that are just semantic wrappers
/// around inner types. This impl may be generated with a macro.
pub trait Inner {
    type Inner;

    fn inner(&self) -> &Self::Inner;
    fn inner_mut(&mut self) -> &mut Self::Inner;
    fn into_inner(self) -> Self::Inner;
}

/// A macro that generates a `Inner` impl.
macro_rules! impl_into_inner {
    ($wrapper:ty, $inner_type:ty) => {
        impl crate::parsed::Inner for $wrapper {
            type Inner = $inner_type;

            fn inner(&self) -> &Self::Inner {
                &self.0
            }

            fn inner_mut(&mut self) -> &mut Self::Inner {
                &mut self.0
            }

            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
}

pub(crate) use impl_into_inner;

/// An interface for the type of function that serde's skip_serializing_if
/// method takes.
pub(crate) trait SkipSerializing {
    fn skip_serializing(&self) -> bool;
}
