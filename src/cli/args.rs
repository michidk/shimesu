//! Clap definitions for the `shimesu` command-line surface.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "shimesu")]
#[command(about = "Self-hosted static publishing platform for AWS")]
#[command(version)]
pub struct Cli {
    /// AWS profile to use
    #[arg(long, global = true, env = "AWS_PROFILE")]
    pub profile: Option<String>,

    /// AWS region (AWS_REGION, then AWS_DEFAULT_REGION; default: eu-central-1)
    #[arg(long, global = true, env = "AWS_REGION", hide_env = true)]
    pub region: Option<String>,

    /// Output JSON instead of human-readable text
    #[arg(long, global = true)]
    pub json: bool,

    /// Skip confirmation prompts
    #[arg(long, global = true)]
    pub yes: bool,

    /// CloudFormation stack name
    #[arg(long, global = true, env = "SHIMESU_STACK")]
    pub stack: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    /// Show stack status and connection info
    Status,

    /// Manage the installation CloudFormation stack
    #[command(subcommand)]
    Stack(StackCommands),

    /// Verify AWS credentials, stack health, and backend access
    Doctor,

    /// Manage sites
    #[command(subcommand)]
    Site(SiteCommands),

    /// Publish files to a site
    Publish {
        /// Path to file, directory, or zip to publish
        path: std::path::PathBuf,

        /// Site name (defaults to filename/dirname)
        #[arg(long, short)]
        site: Option<String>,
    },
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum SiteCommands {
    /// List all sites
    List,

    /// Show site details
    Inspect {
        /// Site slug
        slug: String,
    },

    /// Delete a site
    Delete {
        /// Site slug
        slug: String,

        /// Keep files in S3 (only remove metadata)
        #[arg(long)]
        keep_files: bool,
    },
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum StackCommands {
    /// Create the installation stack
    Init {
        /// Domain for the installation, e.g. pages.example.com
        #[arg(long)]
        domain: String,

        /// Existing ACM certificate ARN in us-east-1 (advanced override)
        #[arg(long)]
        certificate_arn: Option<String>,

        /// Route 53 hosted zone ID for automatic DNS records (omit for external DNS)
        #[arg(long)]
        hosted_zone_id: Option<String>,
    },

    /// Show stack status and connection info
    Status,

    /// Apply the embedded template and landing pages to the existing stack
    Update,

    /// Delete the installation stack while retaining managed data stores
    Destroy {
        /// Explicitly confirm stack destruction
        #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
        confirm: bool,
    },

    /// Permanently destroy retained installation data after stack deletion
    Teardown {
        /// Explicitly confirm permanent data loss for retained resources
        #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
        confirm_data_loss: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands, StackCommands};
    use clap::{CommandFactory, Parser};

    #[test]
    fn executable_name_is_shimesu() {
        let command = Cli::command();

        assert_eq!(command.get_name(), "shimesu");
    }

    #[test]
    fn region_help_documents_both_environment_fallbacks() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("AWS_REGION, then AWS_DEFAULT_REGION"));
        assert!(!help.contains("[env: AWS_REGION="));
    }

    #[test]
    fn help_includes_stack_and_doctor_commands() {
        let command = Cli::command();
        let subcommand_names = command
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<Vec<_>>();

        assert!(subcommand_names.iter().any(|name| name == "stack"));
        assert!(subcommand_names.iter().any(|name| name == "doctor"));
        assert!(subcommand_names.iter().any(|name| name == "status"));
    }

    #[test]
    fn stack_init_requires_domain_but_not_certificate_arn() {
        assert!(Cli::try_parse_from(["shimesu", "stack", "init"]).is_err());

        let managed =
            Cli::try_parse_from(["shimesu", "stack", "init", "--domain", "static.example.com"])
                .expect("domain should be enough for managed certificate provisioning");

        assert!(matches!(
            managed.command,
            Commands::Stack(StackCommands::Init {
                ref domain,
                certificate_arn: None,
                ..
            }) if domain == "static.example.com"
        ));

        let override_certificate = Cli::try_parse_from([
            "shimesu",
            "stack",
            "init",
            "--domain",
            "static.example.com",
            "--certificate-arn",
            "arn:aws:acm:us-east-1:123456789012:certificate/12345678-1234-1234-1234-123456789012",
        ])
        .expect("advanced certificate override should parse");

        assert!(matches!(
            override_certificate.command,
            Commands::Stack(StackCommands::Init {
                ref domain,
                certificate_arn: Some(_),
                ..
            }) if domain == "static.example.com"
        ));
    }

    #[test]
    fn stack_destroy_requires_explicit_confirm_flag() {
        let parse_result = Cli::try_parse_from(["shimesu", "stack", "destroy"]);

        assert!(parse_result.is_err());

        let legacy_yes_result = Cli::try_parse_from(["shimesu", "--yes", "stack", "destroy"]);

        assert!(legacy_yes_result.is_err());
    }

    #[test]
    fn stack_teardown_requires_explicit_data_loss_confirmation() {
        assert!(Cli::try_parse_from(["shimesu", "stack", "teardown"]).is_err());
        assert!(Cli::try_parse_from(["shimesu", "--yes", "stack", "teardown"]).is_err());

        let cli = Cli::try_parse_from(["shimesu", "stack", "teardown", "--confirm-data-loss"])
            .expect("explicit data-loss confirmation should parse");

        assert!(matches!(
            cli.command,
            Commands::Stack(StackCommands::Teardown {
                confirm_data_loss: true
            })
        ));
    }

    #[test]
    fn top_level_status_remains_available() {
        let cli = Cli::try_parse_from(["shimesu", "status"]).expect("status should parse");

        assert!(matches!(cli.command, Commands::Status));
    }

    #[test]
    fn stack_status_subcommand_parses() {
        let cli = Cli::try_parse_from(["shimesu", "stack", "status"]).expect("stack status parses");

        assert!(matches!(
            cli.command,
            Commands::Stack(StackCommands::Status)
        ));
    }

    #[test]
    fn stack_update_subcommand_parses_without_arguments() {
        let cli = Cli::try_parse_from(["shimesu", "stack", "update"]).expect("stack update parses");

        assert!(matches!(
            cli.command,
            Commands::Stack(StackCommands::Update)
        ));
    }

    #[test]
    fn doctor_subcommand_parses() {
        let cli = Cli::try_parse_from(["shimesu", "doctor"]).expect("doctor should parse");

        assert!(matches!(cli.command, Commands::Doctor));
    }

    #[test]
    fn publish_subcommand_parses_with_site_flag() {
        let cli = Cli::try_parse_from(["shimesu", "publish", "./dist", "--site", "docs"])
            .expect("publish should parse");

        assert!(matches!(
            cli.command,
            Commands::Publish { ref site, .. } if site.as_deref() == Some("docs")
        ));
    }

    #[test]
    fn deploy_subcommand_is_rejected() {
        let parse_result = Cli::try_parse_from(["shimesu", "deploy", "./dist"]);

        assert!(parse_result.is_err());
    }
}
