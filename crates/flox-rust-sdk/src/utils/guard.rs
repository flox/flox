use std::future::Future;

/// A guarded value
///
/// A value that can be either initialized or not.
/// Uninitialized values need to be mapped into an initialized value
/// before being accesible
pub enum Guard<I, U> {
    Initialized(I),
    Uninitialized(U),
}

impl<I, U> Guard<I, U> {
    /// try open an initialized value
    ///
    /// returns ok if the value is initialized or reports back [Self]
    /// if not yet initialized
    pub fn open(self) -> Result<I, Self> {
        match self {
            Guard::Initialized(i) => Ok(i),
            Guard::Uninitialized(_) => Err(self),
        }
    }

    /// ensure an initalized value using a fallible async initializer
    ///
    /// akin to [Option<T>::ok_or_else]
    pub async fn ensure_async<Fut: Future<Output = Result<I, E>>, E, F: FnOnce(U) -> Fut>(
        self,
        f: F,
    ) -> Result<I, E> {
        match self {
            Guard::Initialized(i) => Ok(i),
            Guard::Uninitialized(u) => Ok(f(u).await?),
        }
    }

    /// check whether a guard is initialized
    pub fn is_initialized(&self) -> bool {
        matches!(self, Guard::Initialized(_))
    }

    /// check whether a guard is not initialized
    pub fn is_uninitialized(&self) -> bool {
        !self.is_initialized()
    }
}
