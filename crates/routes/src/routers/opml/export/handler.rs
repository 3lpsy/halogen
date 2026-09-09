use axum::{Json, extract::Extension};
use halogen_orm::podcast::Entity as PodcastEntity;
use halogen_utils::constants::{VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE};
use halogen_wire::{OpmlExportData, ResponseData};
use sea_orm::{DatabaseConnection, EntityTrait};

use crate::routers::errors::ApiError;
use crate::routers::extractors::AdminUser;
use halogen_utils::opml::{OpmlDocument, OpmlOutline, to_opml_xml};

/// GET /opml/export — the current subscriptions as OPML 2.0 XML. **Admin only.**
pub async fn export(
    Extension(dbc): Extension<DatabaseConnection>,
    _admin: AdminUser,
) -> Result<Json<ResponseData<OpmlExportData>>, ApiError> {
    let podcasts = PodcastEntity::find().all(&dbc).await.map_err(|e| {
        ApiError::new(
            VALIDATION_DATABASE_FIELD,
            VALIDATION_PANIC_CODE,
            format!("Failed to list podcasts: {e}"),
        )
    })?;

    let outlines = podcasts
        .into_iter()
        .map(|p| OpmlOutline {
            text: p.title,
            outline_type: Some("rss".to_string()),
            xml_url: Some(p.feed_url),
        })
        .collect();

    let doc = OpmlDocument {
        version: "2.0".to_string(),
        outlines,
    };

    Ok(Json(ResponseData::from_data(OpmlExportData {
        opml: to_opml_xml(&doc),
    })))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
