use std::{fmt::Display, ops::Deref};

use derive_more::{Constructor, Deref, From};

use crate::{command_line::ToArgs, installable::FlakeRef};

/// Setting Flag Container akin to https://cs.github.com/NixOS/nix/blob/499e99d099ec513478a2d3120b2af3a16d9ae49d/src/libutil/config.cc#L199
///
/// Usage:
/// 1. Create a struct for a flag and implement [Flag] for it
/// 2. Implement [TypedFlag] for the setting or manualy implement [ToArgs]
pub trait Flag {
    const FLAG: &'static str;
}

///
pub enum FlagTypes<T> {
    Bool,
    List(fn(&T) -> Vec<String>),
}

pub trait TypedFlag: Flag
where
    Self: Sized,
{
    const FLAG_TYPE: FlagTypes<Self>;
}

impl TypedFlag for AcceptFlakeConfig {
    const FLAG_TYPE: FlagTypes<Self> = FlagTypes::Bool;
}

impl<D: Deref<Target = Vec<String>> + Flag> TypedFlag for D {
    const FLAG_TYPE: FlagTypes<Self> = FlagTypes::List(|s| s.deref().to_owned());
}

impl<W: TypedFlag> ToArgs for W {
    fn args(&self) -> Vec<String> {
        match Self::FLAG_TYPE {
            FlagTypes::Bool => vec![Self::FLAG.to_string()],
            FlagTypes::List(f) => {
                vec![Self::FLAG.to_string(), f(self).join(" ")]
            }
        }
    }
}

/// flag for warn dirty
#[derive(Clone, From)]
pub struct WarnDirty;
impl Flag for WarnDirty {
    const FLAG: &'static str = "--warn-dirty";
}
impl TypedFlag for WarnDirty {
    const FLAG_TYPE: FlagTypes<Self> = FlagTypes::Bool;
}

/// Flag for accept-flake-config
#[derive(Clone, From)]
pub struct AcceptFlakeConfig;
impl Flag for AcceptFlakeConfig {
    const FLAG: &'static str = "--accept-flake-config";
}

/// Flag for extra experimental features
#[derive(Clone, Deref, From)]
pub struct ExperimentalFeatures(Vec<String>);
impl Flag for ExperimentalFeatures {
    const FLAG: &'static str = "--extra-experimental-features";
}

/// Flag for extra substituters
#[derive(Clone, Deref, From)]
pub struct Substituters(Vec<String>);
impl Flag for Substituters {
    const FLAG: &'static str = "--extra-substituters";
}

/// Tuple like override inputs flag
#[derive(Clone, Constructor)]
pub struct OverrideInputs {
    from: FlakeRef,
    to: FlakeRef,
}

impl Flag for OverrideInputs {
    const FLAG: &'static str = "--override-input";
}
impl ToArgs for OverrideInputs {
    fn args(&self) -> Vec<String> {
        dbg!(vec![
            Self::FLAG.to_string(),
            self.from.clone(),
            self.to.clone()
        ])
    }
}

impl<T: ToArgs> ToArgs for Option<T> {
    fn args(&self) -> Vec<String> {
        self.iter().map(|t| t.args()).flatten().collect()
    }
}

////////////////////////// Remove After Review //////////////////////////////////

// impl TypedFlag for OverrideInputs {
//     const FLAG_TYPE: FlagTypes<Self> =
//         FlagTypes::List(|s| vec![Self::FLAG.to_string(), s.0.clone(), s.1.clone()]);
// }

// /// Setting Container akin to https://cs.github.com/NixOS/nix/blob/499e99d099ec513478a2d3120b2af3a16d9ae49d/src/libutil/config.cc#L199
// #[derive(From, Clone)]
// pub struct Setting<T>(T);

// impl std::fmt::Display for Setting<Vec<String>> {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         write!(f, "{}", self.0.join(" "))
//     }
// }

// impl<T> Setting<T>
// where
//     Setting<T>: Display,
// {
//     pub fn to_args(&self, flag: impl Into<String>) -> Vec<String> {
//         vec![flag.into(), format!("{self}")]
//     }
// }

// impl Setting<bool> {
//     pub fn to_args(&self, flag: impl Into<String>) -> Vec<String> {
//         vec![flag.into()]
//     }
// }

// impl<T, U> Setting<(T, U)>
// where
//     T: Into<String> + Clone,
//     U: Into<String> + Clone,
// {
//     pub fn to_args(&self, flag: impl Into<String>) -> Vec<String> {
//         let (l, r) = self.0.clone();
//         vec![flag.into(), l.into(), r.into()]
//     }
// }

// /// Concrete Container for boolean flags
// #[derive(Clone, From)]
// pub struct BoolFlag<T>(T)
// where
//     T: Flag;

// impl<T> ToArgs for BoolFlag<T>
// where
//     T: Flag,
// {
//     fn args(&self) -> Vec<String> {
//         vec![T::FLAG.to_string()]
//     }
// }

// /// Concrete Container for list flags
// #[derive(Clone, From)]
// pub struct ListFlag<T>(T)
// where
//     T: Flag + std::ops::Deref<Target = Vec<String>>;

// impl<T> ToArgs for ListFlag<T>
// where
//     T: Flag + std::ops::Deref<Target = Vec<String>>,
// {
//     fn args(&self) -> Vec<String> {
//         let arg = self.0.join(" ");
//         vec![T::FLAG.to_string(), arg]
//     }
// }
