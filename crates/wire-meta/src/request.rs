//! Request side of the request/response machinery: `RequestData<T, P>` wrapping a
//! body (`T`) and params (`P`), plus the `Requestable*` marker traits.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use typeshare::typeshare;

/// Marker + `as_request` helper for request-body types. Blanket-impl'd on any
/// `Serialize + Deserialize`, so every `*Data` struct gets it for free.
///
/// # Example
/// ```rust
/// use halogen_wire_meta::request::{DefaultDataType, RequestableData};
///
/// let store_data = DefaultDataType {};
///
/// // Create a RequestData from the store data
/// let _request = store_data.as_request();
/// ```
pub trait RequestableData: for<'de> Deserialize<'de> + Serialize {
    #[allow(clippy::wrong_self_convention)]
    fn as_request(self) -> RequestData<Self, DefaultParamsType> {
        RequestData::new(Some(self), None)
    }
}

#[typeshare]
/// A placeholder type used when no specific data is needed in requests.
#[derive(Debug, Serialize, Deserialize)]
pub struct DefaultDataType {}

// seems kind of dangerous....
impl<T> RequestableData for T where T: for<'de> serde::Deserialize<'de> + serde::Serialize {}

/// Default params type: arbitrary URL query string key/values.
pub type DefaultParamsType = HashMap<String, String>;

/// Marker + `as_params` helper for query-param types. Blanket-impl'd on any
/// `Serialize + Deserialize`.
pub trait RequestableParams: for<'de> Deserialize<'de> + Serialize {
    #[allow(clippy::wrong_self_convention)]
    fn as_params(self) -> RequestData<DefaultDataType, Self> {
        RequestData {
            data: None,
            params: Some(self),
        }
    }
}

impl<T> RequestableParams for T where T: for<'de> Deserialize<'de> + Serialize {}

#[typeshare]
/// Typed wrapper carrying a request's JSON body (`T`) and URL query params (`P`).
///
/// # Type Parameters
/// - `T`: The type of the request data (typically a `*Data` struct)
/// - `P`: The type of the request parameters (typically a `*Params` struct)
///
/// # Example
/// ```rust
/// use halogen_wire_meta::request::{DefaultDataType, DefaultParamsType, RequestData};
///
/// // Create a request with data only
/// let store_data = DefaultDataType {};
///
/// let _request_with_data: RequestData<DefaultDataType, DefaultParamsType> =
///     RequestData::from_data(store_data);
///
/// // Create a request with parameters only
/// let params: DefaultParamsType = Default::default();
///
/// let _request_with_params: RequestData<DefaultDataType, DefaultParamsType> =
///     RequestData::from_params(params);
/// ```
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "T: RequestableData, P: RequestableParams")]
pub struct RequestData<T, P> {
    // body data
    pub data: Option<T>,
    // query-ish data
    pub params: Option<P>,
}

impl<T, P> RequestData<T, P>
where
    T: RequestableData,
    P: RequestableParams,
{
    /// Create a new RequestData with both data and parameters.
    pub fn new(data: Option<T>, params: Option<P>) -> RequestData<T, P> {
        Self { data, params }
    }

    /// Create a RequestData from just the data (no parameters).
    pub fn from_data(data: T) -> RequestData<T, P> {
        RequestData::new(Some(data), None)
    }

    /// Create a RequestData from just the parameters (no data).
    pub fn from_params(params: P) -> RequestData<T, P> {
        RequestData::new(None, Some(params))
    }
}
