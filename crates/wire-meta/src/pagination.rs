#[cfg(feature = "db")]
use super::db::DbValidationErrors;
use super::order::OrderDirection;
#[cfg(feature = "db")]
use sea_orm::{ConnectionTrait, SelectorTrait};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;
use validator::Validate;

// Request-side paging input (page is 0-based).
#[derive(Debug, Clone, Validate, Serialize, Deserialize)]
pub struct Pagination {
    #[validate(range(min = 0, max = 65536, message = "Page must be between 0 and 65536"))]
    pub page: i32,
    #[validate(range(min = 1, max = 65536, message = "Size must be between 1 and 65536"))]
    pub size: i32,
}

impl Default for Pagination {
    fn default() -> Self {
        Self { page: 0, size: 10 }
    }
}

// Response-side paging metadata returned to the client (page is 0-based).
#[typeshare]
#[derive(Debug, Serialize, Deserialize)]
pub struct Paginator {
    pub page: i32,
    pub size: i32,
    pub pages: i32,
    pub total: i32,
    pub order: OrderDirection,
}

#[cfg(feature = "db")]
impl Paginator {
    pub async fn from_db_paginator<C, S>(
        paginator: &sea_orm::Paginator<'_, C, S>,
        page: i32,
        size: i32,
        order: OrderDirection,
    ) -> Result<Self, DbValidationErrors>
    where
        C: ConnectionTrait,
        S: SelectorTrait,
    {
        // TODO: usize -> i32 casts below are unchecked
        let total_items_and_pages = paginator
            .num_items_and_pages()
            .await
            .map_err(DbValidationErrors::from)?;
        let pages = total_items_and_pages.number_of_pages as i32;
        let total = total_items_and_pages.number_of_items as i32;
        Ok(Paginator {
            page,
            size,
            pages,
            total,
            order,
        })
    }
}

pub trait HasPagination {
    fn pagination(&mut self) -> &mut Option<Pagination>;
    fn with_page(mut self, page: i32) -> Self
    where
        Self: Sized,
    {
        let pagination = self.pagination();
        *pagination = Some(pagination.take().unwrap_or_default());
        pagination.as_mut().unwrap().page = page;
        self
    }
    fn with_size(mut self, size: i32) -> Self
    where
        Self: Sized,
    {
        let pagination = self.pagination();
        *pagination = Some(pagination.take().unwrap_or_default());
        pagination.as_mut().unwrap().size = size;
        self
    }
}
