pub mod cleanup;
pub mod serve;
pub mod setup;
pub mod test;

pub async fn run(cli: crate::cli::Cli) -> anyhow::Result<()> {
    use crate::cli::Commands;
    match cli.command {
        None | Some(Commands::Serve) => serve::run(&cli.args).await,
        Some(Commands::Setup) => setup::run(&cli.args),
        Some(Commands::Cleanup) => cleanup::run(),
        Some(Commands::TestAll) => test::run_all(&cli.args).await,
        Some(Commands::TestSearch { query, engine, count }) => {
            test::run_search(&cli.args, &query, &engine, count).await
        }
        Some(Commands::TestRead { url, max_length }) => {
            test::run_read(&cli.args, &url, max_length).await
        }
        Some(Commands::TestSearchAndRead { query, read_count, max_length }) => {
            test::run_search_and_read(&cli.args, &query, read_count, max_length).await
        }
    }
}
