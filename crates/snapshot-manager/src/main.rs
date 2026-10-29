use snapshot::Manager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load(&config::Sources::from_process(None, Vec::new()))?;
    let service = &config.snapshot_manager;
    let manager = Manager::from_config(&service.manager, &config.clickhouse, &config.pause).await?;
    snapshot::serve(manager, service.listen).await?;
    Ok(())
}
