use crate::error::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Rel<T> {
    #[default]
    NotLoaded,
    Loaded(T),
}

impl<T> Rel<T> {
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }

    pub fn get(&self) -> Option<&T> {
        match self {
            Self::NotLoaded => None,
            Self::Loaded(value) => Some(value),
        }
    }

    pub fn loaded(&self) -> Result<&T> {
        match self {
            Self::Loaded(value) => Ok(value),
            Self::NotLoaded => Err(Error::Invalid("relationship is not loaded".into())),
        }
    }

    pub fn expect_loaded(&self, msg: &str) -> &T {
        match self {
            Self::Loaded(value) => value,
            Self::NotLoaded => panic!("{msg}"),
        }
    }
}

impl<T> Rel<Option<T>> {
    pub fn as_option(&self) -> Result<Option<&T>> {
        match self {
            Self::Loaded(opt) => Ok(opt.as_ref()),
            Self::NotLoaded => Err(Error::Invalid("relationship is not loaded".into())),
        }
    }
}

impl<T> Rel<Vec<T>> {
    pub fn as_slice(&self) -> Result<&[T]> {
        match self {
            Self::Loaded(vec) => Ok(vec.as_slice()),
            Self::NotLoaded => Err(Error::Invalid("relationship is not loaded".into())),
        }
    }
}
