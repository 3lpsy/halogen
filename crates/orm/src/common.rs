use halogen_utils::{
    constants::{VALIDATION_EXISTS_CODE, VALIDATION_ID_FIELD},
    verrors,
};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, PaginatorTrait, PrimaryKeyTrait,
    QueryFilter, QueryOrder, Select, Value,
};
use sea_orm_migration::async_trait::async_trait;
use validator::ValidationErrors;

use halogen_wire::{DbValidationErrors, Order, Pagination, Paginator};

/// Generic, location-aware DB lookups layered over SeaORM, available on **every**
/// `EntityTrait` via a blanket impl. They wrap `find_by_id` / single-column finds
/// and emit the right error `field`/`code` so producers don't hand-build them:
///
/// - the error `field` is **never** a caller-supplied string. `by_id*` is always
///   keyed `"id"`; `by_column*` derives its field from the column's own `Iden`
///   name (`Column::PodcastId` → `"podcast_id"`), so the reported location can't
///   drift from the column the query actually touched.
/// - a missing row is `code = "exists"` (→ 404); an unexpected `DbErr` routes
///   through [`DbValidationErrors`] (unique → 409, else a non-leaking 500 logged
///   once at the mapping site) — these helpers stay silent.
/// - messages stay generic (`"<table> does not exist"`, derived from the entity);
///   the UI keys off field + code + status, not the prose.
///
/// FK-specific lookups (only meaningful for entities that *have* that column,
/// e.g. `by_podcast_id`) do **not** belong here — give them their own
/// entity-scoped trait. This blanket trait is for what's valid on any entity.
#[async_trait]
pub trait EntityHelpers<E>
where
    E: EntityTrait,
{
    type PrimaryKeyType;

    /// `find_by_id`; a `DbErr` becomes a non-leaking [`DbValidationErrors`].
    async fn by_id(
        dbc: &DatabaseConnection,
        id: Self::PrimaryKeyType,
    ) -> Result<Option<E::Model>, ValidationErrors>;

    /// Like [`by_id`](Self::by_id) but a missing row is an error keyed
    /// `field = "id"`, `code = "exists"` (→ 404). The field is **always** `"id"`.
    async fn by_id_or_err(
        dbc: &DatabaseConnection,
        id: Self::PrimaryKeyType,
    ) -> Result<E::Model, ValidationErrors>;

    /// Whether a row with this primary key exists.
    async fn id_exists(
        dbc: &DatabaseConnection,
        id: Self::PrimaryKeyType,
    ) -> Result<bool, ValidationErrors>;

    /// Single-column lookup. The error `field` (on the `_or_err` variant) is
    /// derived from `col`'s own name via SeaORM's `Iden`, so the reported location
    /// matches the queried column for free — never pass a field string.
    async fn by_column<C, V>(
        dbc: &DatabaseConnection,
        col: C,
        val: V,
    ) -> Result<Option<E::Model>, ValidationErrors>
    where
        C: ColumnTrait + Send,
        V: Into<Value> + Send;

    /// Like [`by_column`](Self::by_column) but a missing row is `code = "exists"`
    /// (→ 404), keyed by the column's derived field.
    async fn by_column_or_err<C, V>(
        dbc: &DatabaseConnection,
        col: C,
        val: V,
    ) -> Result<E::Model, ValidationErrors>
    where
        C: ColumnTrait + Send,
        V: Into<Value> + Send;
}

#[async_trait]
impl<E> EntityHelpers<E> for E
where
    E: EntityTrait,
    E::PrimaryKey: PrimaryKeyTrait,
{
    type PrimaryKeyType = <E::PrimaryKey as PrimaryKeyTrait>::ValueType;

    async fn by_id(
        dbc: &DatabaseConnection,
        id: Self::PrimaryKeyType,
    ) -> Result<Option<E::Model>, ValidationErrors> {
        Ok(E::find_by_id(id)
            .one(dbc)
            .await
            .map_err(DbValidationErrors::from)?)
    }

    async fn by_id_or_err(
        dbc: &DatabaseConnection,
        id: Self::PrimaryKeyType,
    ) -> Result<E::Model, ValidationErrors> {
        match E::by_id(dbc, id).await? {
            Some(model) => Ok(model),
            None => Err(verrors(
                VALIDATION_ID_FIELD,
                VALIDATION_EXISTS_CODE,
                format!("{} does not exist", E::default().table_name()),
            )),
        }
    }

    async fn id_exists(
        dbc: &DatabaseConnection,
        id: Self::PrimaryKeyType,
    ) -> Result<bool, ValidationErrors> {
        Ok(E::by_id(dbc, id).await?.is_some())
    }

    async fn by_column<C, V>(
        dbc: &DatabaseConnection,
        col: C,
        val: V,
    ) -> Result<Option<E::Model>, ValidationErrors>
    where
        C: ColumnTrait + Send,
        V: Into<Value> + Send,
    {
        Ok(E::find()
            .filter(col.eq(val))
            .one(dbc)
            .await
            .map_err(DbValidationErrors::from)?)
    }

    async fn by_column_or_err<C, V>(
        dbc: &DatabaseConnection,
        col: C,
        val: V,
    ) -> Result<E::Model, ValidationErrors>
    where
        C: ColumnTrait + Send,
        V: Into<Value> + Send,
    {
        // `IdenStatic::as_str` is `&'static str`, exactly what `verrors`/`add`
        // need — the location is carried by the column's type, not a string arg.
        let field = col.as_str();
        match E::by_column(dbc, col, val).await? {
            Some(model) => Ok(model),
            None => Err(verrors(
                field,
                VALIDATION_EXISTS_CODE,
                format!("{} does not exist", E::default().table_name()),
            )),
        }
    }
}

/// List endpoints' sortable entities: map a request `order_by` key to a column,
/// falling back to the primary key for unknown keys. The list-side companion to
/// [`EntityHelpers`] — drives [`paginate`] so each entity's sort vocabulary lives
/// with that entity instead of a `match` copy-pasted into every list handler. The
/// per-entity `impl`s live in each entity module (`podcast.rs`, `episode.rs`, …).
pub trait Sortable: EntityTrait {
    fn order_column(order_by: &str) -> Self::Column;
}

/// Apply `order` (via [`Sortable`]) to `query`, then page it: returns the
/// requested page's models plus the response [`Paginator`] metadata. Collapses the
/// `order_by` / `paginate` / `from_db_paginator` / `fetch_page` quartet every list
/// handler repeated; a DB error becomes a non-leaking [`ValidationErrors`].
///
/// The caller passes the fully-FILTERED query (scoping, search, …) WITHOUT a sort;
/// the requested sort is appended here, so a relevance pre-sort the caller already
/// applied stays primary and this becomes the tiebreak.
pub async fn paginate<E>(
    dbc: &DatabaseConnection,
    query: Select<E>,
    pagination: &Pagination,
    order: &Order,
) -> Result<(Vec<E::Model>, Paginator), ValidationErrors>
where
    E: EntityTrait + Sortable,
    E::Model: FromQueryResult + Send + Sync,
{
    let query = query.order_by(
        E::order_column(&order.order_by),
        order.direction.clone().into(),
    );

    let size = pagination.size.max(1) as u64;
    let db_paginator = query.paginate(dbc, size);
    let meta = Paginator::from_db_paginator(
        &db_paginator,
        pagination.page,
        pagination.size,
        order.direction.clone(),
    )
    .await?;
    let items = db_paginator
        .fetch_page(pagination.page.max(0) as u64)
        .await
        .map_err(DbValidationErrors::from)?;
    Ok((items, meta))
}
