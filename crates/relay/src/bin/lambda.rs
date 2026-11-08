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
    let dynamo = aws_sdk_dynamodb::Client::new(&config);
    let store: Arc<dyn croaker_relay::Store> =
        Arc::new(DynamoStore::new(dynamo.clone(), table.clone()));
    let bucket = std::env::var("CROAKER_MEDIA_BUCKET").expect("CROAKER_MEDIA_BUCKET is required");
    let media = Arc::new(croaker_relay::media::ObjectService {
        relay: store.clone(),
        registry: Arc::new(croaker_relay::media::dynamodb::DynamoRegistry::new(
            dynamo, table,
        )),
        objects: Arc::new(croaker_relay::media::s3::S3Objects::new(
            aws_sdk_s3::Client::new(&config),
            bucket,
        )),
    });
    let verifier = Arc::new(croaker_relay::login::LoginVerifier::new(
        croaker_relay::login::LoginConfig::from_env()?,
    )?);
    lambda_http::run(
        api::router_with_login(store, verifier).merge(croaker_relay::media::router(media)),
    )
    .await
}
