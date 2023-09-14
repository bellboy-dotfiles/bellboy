use std::fmt::{self, Formatter};

#[derive(Clone, Debug)]
pub enum NormalizedEqOutcome<N> {
    ExactMatch,
    MatchAfterNormalization { reason: N },
    NotAMatch,
}

impl<N> NormalizedEqOutcome<N> {
    pub fn matched(&self) -> bool {
        !matches!(self, Self::NotAMatch { .. })
    }
}

pub trait Normalization<T>
where
    Self: Sized,
{
    type Error;

    /// Performs a normalized comparison of `t1` against `t2`.
    fn normalized_eq(t1: &T, t2: &T) -> Result<NormalizedEqOutcome<Self>, Self::Error>;

    /// Writes an explanation of why `T` was matched against as if immediately written after
    /// a noun.
    ///
    /// For example, an implementation of this trait could write "has the matching digest {t:?}",
    /// if the missing noun were "cryptographic hash".
    fn describe(&self, t: &T, f: &mut Formatter<'_>) -> fmt::Result;
}
