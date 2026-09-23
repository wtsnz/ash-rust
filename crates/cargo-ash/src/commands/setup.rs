use super::migrate::MigrateArgs;

pub async fn run(args: MigrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    super::migrate::run(args).await
}
