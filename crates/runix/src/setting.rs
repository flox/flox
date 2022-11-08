use std::{fmt::Display, ops::Deref};

use derive_more::{Deref, From};

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
            FlagTypes::Bool => todo!(),
            FlagTypes::List(f) => f(self).to_owned(),
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
#[derive(Clone)]
pub struct OverrideInputs(FlakeRef, FlakeRef);
impl Flag for OverrideInputs {
    const FLAG: &'static str = "--override-inputs";
}
impl ToArgs for OverrideInputs {
    fn args(&self) -> Vec<String> {
        vec![Self::FLAG.to_string(), self.0.clone(), self.1.clone()]
    }
}
impl<T: ToArgs> ToArgs for Option<T> {
    fn args(&self) -> Vec<String> {
        self.iter().map(|t| t.args()).flatten().collect()
    }
}
