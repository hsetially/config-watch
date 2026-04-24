use sqlx::PgPool;

pub type PgTx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

pub async fn begin(pool: &PgPool) -> anyhow::Result<PgTx<'_>> {
    Ok(pool.begin().await?)
}