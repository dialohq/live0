use compiler::{Access, Params};
use engine::{Engine, Query};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load(&config::Sources::from_process(None, Vec::new()))?;
    let engine = Engine::from_config(&config).await?;
    let mut subscription = engine
        .subscribe(
            &Query {
                sql: "SELECT * FROM events".to_owned(),
                params: Params::new(),
                access: Access::Unrestricted,
            },
            None,
        )
        .await?;
    loop {
        let iteration = subscription.next().await?;
        println!("{:?} at {}", iteration.output, iteration.cursor.encode());
    }
}
