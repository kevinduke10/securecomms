use croaker_relay::{api, dynamodb::DynamoStore};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter("croaker_relay=info")
        .without_time()
        .init();
    let table = std::env::var("CROAKER_TABLE").expect("CROAKER_TABLE is required");
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let store = DynamoStore::new(aws_sdk_dynamodb::Client::new(&config), table);
    lambda_http::run(api::router(Arc::new(store))).await
}
