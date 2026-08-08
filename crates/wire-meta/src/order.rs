use std::cmp::Ordering;

use halogen_utils::patterns::ALPHA_DASH;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;
use validator::Validate;

#[typeshare]
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderDirection {
    #[default]
    Asc,
    Desc,
}

impl OrderDirection {
    /// Apply this direction to an ascending ordering (reverses it when `Desc`).
    pub fn apply(&self, ord: Ordering) -> Ordering {
        match self {
            OrderDirection::Asc => ord,
            OrderDirection::Desc => ord.reverse(),
        }
    }
}

/// Compare two optional sort keys for an ordered list: present values compare by
/// `direction`; absent (`None`) always sorts last, regardless of direction. Shared
/// by the client + server smart-reorder so their orderings agree.
pub fn cmp_opt<T: Ord>(a: Option<T>, b: Option<T>, direction: &OrderDirection) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => direction.apply(x.cmp(&y)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(feature = "db")]
impl From<OrderDirection> for sea_orm::Order {
    fn from(direction: OrderDirection) -> Self {
        match direction {
            OrderDirection::Asc => sea_orm::Order::Asc,
            OrderDirection::Desc => sea_orm::Order::Desc,
        }
    }
}
#[derive(Debug, Validate, Serialize, Deserialize, Clone)]
pub struct Order {
    pub direction: OrderDirection,
    #[validate(length(
        min = 1,
        max = 256,
        message = "OrderBy must be between 1 and 256 characters long"
    ))]
    #[validate(regex(path = *ALPHA_DASH, message="Field must only contain alphanumeric or -, ., _ characters"))]
    pub order_by: String,
}
impl Default for Order {
    fn default() -> Self {
        Self {
            direction: OrderDirection::Asc,
            order_by: String::from("id"),
        }
    }
}

pub trait HasOrder {
    fn order(&mut self) -> &mut Option<Order>;

    fn with_order(mut self, order: Order) -> Self
    where
        Self: Sized,
    {
        *self.order() = Some(order);
        self
    }

    fn with_order_direction(mut self, direction: OrderDirection) -> Self
    where
        Self: Sized,
    {
        let order = self.order();
        if order.is_none() {
            *order = Some(Order::default());
        }
        order.as_mut().unwrap().direction = direction;
        self
    }

    fn with_order_by(mut self, order_by: String) -> Self
    where
        Self: Sized,
    {
        let order = self.order();
        if order.is_none() {
            *order = Some(Order::default());
        }
        order.as_mut().unwrap().order_by = order_by;
        self
    }
}
