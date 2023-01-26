use std::ffi::OsStr;
use std::ops::Deref;

use super::ToArgs;

/// Setting Flag Container akin to https://cs.github.com/NixOS/nix/blob/499e99d099ec513478a2d3120b2af3a16d9ae49d/src/libutil/config.cc#L199
///
/// Usage:
/// 1. Create a struct for a flag and implement [Flag] for it
/// 2. Define `FLAG_TYPE` as either `FlagType::Bool` if no extra arguments are involved, or as `FlagType::Args` to point at an internal argument list or extra logic on `Self`
pub trait Flag: Sized {
    const FLAG: &'static str;
    const FLAG_TYPE: FlagType<Self>;
}

///
pub enum FlagType<T> {
    /// An explicit two state switch
    ///
    /// If their value is the default value this flag is not printed
    ///
    /// If `true` (default == false) prints the name of the flag
    /// If `false` (default == true) prefix it with `no-`
    ///
    /// Mainly used by nix-config values
    Switch(bool, fn(&T) -> bool),
    /// A boolean flag
    ///
    /// Flags of this kind just print their name as is regardless of the content
    Indicator(fn(&T) -> bool),
    /// A list flag
    ///
    /// list flags consist of a flag and a space delimited list of elements
    /// which is passed as a single arguement.
    ///
    /// ```ignore
    /// --flag "a b c"
    /// ```
    List(fn(&T) -> Vec<String>),
    /// A single arg flag
    ///
    /// single arg flags consist of a flag and corresponding value
    ///
    /// ```ignore
    /// --flag "a b c"
    /// ```
    Arg(fn(&T) -> String),
    /// A flag with variably many arguments
    ///
    /// The implementer of this flag provides the arguements to be passed as is
    ///
    /// ```ignore
    /// --flag a b
    /// ```
    Args(fn(&T) -> Vec<String>),
    /// A custom flag
    ///
    /// The implementer of this flag provides the representation of arguements
    ///
    /// ```ignore
    /// a b c
    /// ```
    Custom(fn(&T) -> Vec<String>),
}

impl<T: Deref<Target = bool>> FlagType<T> {
    pub const fn bool() -> FlagType<T> {
        FlagType::Indicator(|s| *s.deref())
    }

    pub const fn switch(default: bool) -> FlagType<T> {
        FlagType::Switch(default, |s| *s.deref())
    }
}

impl<T: Deref<Target = Vec<String>>> FlagType<T> {
    pub const fn list() -> FlagType<T> {
        FlagType::List(|s| s.deref().to_owned())
    }
}

impl<T: Deref<Target = impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)> + Clone>>
    FlagType<T>
{
    pub const fn map() -> FlagType<T> {
        FlagType::List(|s| {
            s.deref()
                .clone()
                .into_iter()
                .map(|(k, v)| format!("{}={}", k.as_ref(), v.as_ref()))
                .collect::<Vec<_>>()
        })
    }
}

impl<T: Deref<Target = impl ToString>> FlagType<T> {
    pub const fn arg() -> FlagType<T> {
        FlagType::Arg(|s| s.deref().to_string())
    }
}

impl<T: Deref<Target = impl AsRef<OsStr>>> FlagType<T> {
    pub const fn os_str_arg() -> FlagType<T> {
        FlagType::Arg(|s| s.deref().as_ref().to_string_lossy().to_string())
    }
}

impl<T: Deref<Target = u32>> FlagType<T> {
    pub const fn number_arg() -> FlagType<T> {
        FlagType::Arg(|s| s.deref().to_string())
    }
}

impl<T> ToArgs for T
where
    T: Flag,
{
    fn to_args(&self) -> Vec<String> {
        match Self::FLAG_TYPE {
            FlagType::Indicator(f) => match f(self) {
                true => vec![Self::FLAG.to_string()],
                false => Default::default(),
            },
            FlagType::Switch(default, f) => match (f(self), default) {
                (true, false) => vec![Self::FLAG.to_string()],
                (false, true) => vec![format!("--no-{}", &Self::FLAG[2..])],
                _ => Default::default(),
            },
            // Todo: should --listarg "" be allowed?
            FlagType::List(f) => {
                let list = f(self);
                match list.is_empty() {
                    true => Default::default(),
                    false => vec![Self::FLAG.to_string(), f(self).join(" ")],
                }
            },
            FlagType::Arg(f) => vec![Self::FLAG.to_string(), f(self)],
            FlagType::Args(f) => {
                let mut list = f(self);
                match list.is_empty() {
                    true => Default::default(),
                    false => {
                        let mut flags = vec![Self::FLAG.to_string()];
                        flags.append(&mut list);
                        flags
                    },
                }
            },
            FlagType::Custom(f) => f(self),
        }
    }
}
