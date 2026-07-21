//! Binary entry point: parses arguments, resolves configuration, and dispatches to command handlers.

use clap::Parser;
use shimesu::cli::{Cli, Commands, Output, OutputFormat, SiteCommands, StackCommands};
use shimesu::commands::{
    run_delete, run_doctor, run_inspect, run_list, run_publish, run_stack_destroy, run_stack_init,
    run_stack_teardown, run_stack_update, run_status, PublishRequest, StackDestroyInput,
    StackInitInput, StackTeardownInput,
};
use shimesu::config::Config;
use shimesu::error::ErrorResponse;
use std::process::ExitCode;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> ExitCode {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .try_init()
        .ok();

    let cli = Cli::parse();
    let output_format = OutputFormat::detect(cli.json);
    let output = Output::new(output_format);

    let result = run(&cli, &output).await;

    match result {
        Ok(()) => ExitCode::from(0),
        Err(error) => {
            let exit_code = error.exit_code();
            if output_format == OutputFormat::Json {
                let response = ErrorResponse::from_error(&error);
                if let Ok(json) = serde_json::to_string_pretty(&response) {
                    eprintln!("{json}");
                }
            } else {
                output.error(&error.to_string());
            }
            ExitCode::from(i32::from(exit_code) as u8)
        }
    }
}

async fn run(cli: &Cli, output: &Output) -> shimesu::Result<()> {
    let config = Config::load(cli)?;

    match &cli.command {
        Commands::Status => run_status(&config, output).await,
        Commands::Stack(StackCommands::Init {
            domain,
            certificate_arn,
            hosted_zone_id,
        }) => {
            let domain = domain.to_ascii_lowercase();
            let input = StackInitInput::parse(domain.as_str(), certificate_arn.as_deref())?
                .with_hosted_zone_id(hosted_zone_id.as_deref())?;

            run_stack_init(&config, output, input).await
        }
        Commands::Stack(StackCommands::Status) => run_status(&config, output).await,
        Commands::Stack(StackCommands::Update) => run_stack_update(&config, output).await,
        Commands::Stack(StackCommands::Destroy { confirm }) => {
            let input = StackDestroyInput::parse(*confirm)?;

            run_stack_destroy(&config, output, input).await
        }
        Commands::Stack(StackCommands::Teardown { confirm_data_loss }) => {
            let input = StackTeardownInput::parse(*confirm_data_loss)?;

            run_stack_teardown(&config, output, input).await
        }
        Commands::Doctor => run_doctor(&config, output).await,
        Commands::Site(SiteCommands::List) => run_list(&config, output).await,
        Commands::Site(SiteCommands::Inspect { slug }) => {
            run_inspect(&config, output, slug.as_str()).await
        }
        Commands::Site(SiteCommands::Delete { slug, keep_files }) => {
            run_delete(&config, output, slug.as_str(), *keep_files).await
        }
        Commands::Publish { path, site } => {
            run_publish(&config, output, PublishRequest::new(path, site.as_deref())).await
        }
    }
}
