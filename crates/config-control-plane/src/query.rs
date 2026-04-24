use sqlx::PgPool;
use uuid::Uuid;

use config_storage::models::ChangeEventRow;
use config_storage::repositories::change_events::{ChangeEventFilters, ChangeEventsRepo};

pub struct QueryService;

#[derive(Debug, serde::Serialize)]
pub struct PaginatedResponse<T: serde::Serialize> {
    pub items: Vec<T>,
    pub limit: i64,
    pub offset: i64,
}

impl QueryService {
    pub async fn list_changes(
        pool: &PgPool,
        filters: ChangeEventFilters,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<PaginatedResponse<ChangeEventRow>> {
        let items = ChangeEventsRepo::list(pool, &filters, limit, offset).await?;
        Ok(PaginatedResponse { items, limit, offset })
    }

    pub async fn get_change(pool: &PgPool, event_id: Uuid) -> anyhow::Result<Option<ChangeEventRow>> {
        ChangeEventsRepo::get(pool, event_id).await
    }
}