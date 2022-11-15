////////////////////////// Remove After Review //////////////////////////////////

// impl TypedFlag for OverrideInputs {
//     const FLAG_TYPE: FlagType<Self> =
//         FlagType::List(|s| vec![Self::FLAG.to_string(), s.0.clone(), s.1.clone()]);
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
