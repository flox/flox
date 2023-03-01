use derive_more::Constructor;

use crate::flox::Flox;
use crate::utils::guard::Guard;

pub mod environment;
pub mod floxmeta;
mod git;
pub mod project;
mod reference;

pub type RootGuard<'flox, I, U> = Guard<Root<'flox, I>, Root<'flox, U>>;

/// Marker for a non finalized [Root]
///
/// intermediate state to model a type driven opening of a [Root] of any type
#[derive(Constructor, Debug)]
pub struct Closed<T> {
    pub inner: T,
}

/// An abstract root representation.
///
/// Wraps a state and a [`Flox`] instance. Generally, Root would not be needed
///
/// # Root?
///
/// As root we understand file system based abstractions in flox,
/// that need to comply with certain guarantees.
///
/// Examples are
/// - flox managed projects
/// - floxmeta repositories
/// - environments
///
/// All of them have certain requirements that shold be provided.
/// Examples include:
/// - must exist, locally
/// - must be a git repo
/// - must be a flake
/// - and more
///
/// Each of these requirements can be modeled as a typestate.
/// By walking those typestates, we can gradually guarantee more requirements.
///
/// A root that is missing some requirements is usually modeled as [`Root<Closed<_>>`].
/// An implementation for [`Root<Closed<_>>`] defines how to "upgrade" the root
/// into a stronger state.
/// This is done by providing methods which map the current state into a [`Guard<New, Old>`].
///
/// ```
/// # use flox_rust_sdk::models::root::{Closed};
/// # use flox_rust_sdk::utils::guard::Guard;
/// # struct Root<T> { state: T };
/// # struct Strong;
/// # struct Weak;
/// # struct InvalidError;
/// # impl Weak { fn test(&self) -> bool { true } }
///
/// impl Root<Closed<Weak>> {
///     fn upgrade(self) -> Result<Guard<Root<Strong>, Self>, InvalidError> {
///         if self.state.inner.test() {
///             Ok(Guard::Initialized(Root { state: Strong }))
///         } else {
///             Ok(Guard::Uninitialized(self))
///         }
///     }
/// }
/// ```
///
/// Using guards we can distinguish invalid state ([`Result::Error(_)`]) from
/// valid existing ([`Result::Ok(Guard::Iniitialized)`])
/// and non existing ([`Result::Ok(Guard::Uninitialized)`]) state.
#[derive(Constructor, Debug)]
pub struct Root<'flox, State> {
    pub flox: &'flox Flox,
    pub state: State,
}

impl<'flox, T> Root<'flox, Closed<T>> {
    /// Create a closed root from any data
    ///
    /// It is not guaranteed that the result implements any method
    /// to upgrade the root into another [Closed] or [Open] state.
    pub fn closed(flox: &'flox Flox, inner: T) -> Self {
        Root {
            flox,
            state: Closed::new(inner),
        }
    }
}
