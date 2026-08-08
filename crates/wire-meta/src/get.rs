//! Default get parameter types.
//!
//! These are generic parameter structs used by get endpoints across all entities.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use validator::Validate;

use super::includes::{HasIncludes, Includable};

/// Default get parameters: `id` + includes.
///
/// Generic over `T: Includable + Serialize + DeserializeOwned` so any entity can use
/// its own include type.
#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
#[serde(bound = "T: Serialize + for<'a> Deserialize<'a>")]
pub struct DefaultGetParams<T: Includable + Serialize + DeserializeOwned> {
    #[serde(default)]
    pub id: Option<i32>,
    #[serde(default)]
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<T>>,
}

impl<T: Includable + Serialize + DeserializeOwned> HasIncludes<T> for DefaultGetParams<T> {
    fn includes(&mut self) -> &mut Option<Vec<T>> {
        &mut self.includes
    }
}
