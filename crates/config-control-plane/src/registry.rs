pub struct RegistryService;

impl RegistryService {
    pub async fn derive_statuses(
        pool: &sqlx::PgPool,
        heartbeat_interval_secs: u64,
    ) -> anyhow::Result<()> {
        let hosts =
            config_storage::repositories::hosts::HostsRepo::list(pool, None, 1000, 0).await?;
        for host in hosts {
            let derived = config_storage::repositories::hosts::derive_host_status(
                host.last_heartbeat_at,
                heartbeat_interval_secs,
            );
            if derived != host.status {
                config_storage::repositories::hosts::HostsRepo::update_status(
                    pool,
                    host.host_id,
                    derived,
                )
                .await?;
            }
        }
        Ok(())
    }
}
