//! `stack destroy` command: deletes the regional stack while retaining data stores.

use aws_sdk_acm::Client as AcmClient;
use aws_sdk_cloudformation::{types::StackStatus, Client};
use tokio::time::{interval, timeout};
use ulid::Ulid;

use super::destroy_output::render_stack_destroy_output;
pub use super::destroy_output::StackDestroyOutput;
use super::{
    certificate::certificate_config,
    certificate_ownership::{inspect_managed_certificate_stack, ManagedCertificateStack},
    describe_stack_status,
    ownership::{inspect_installation_ownership, InstallationOwnership},
    progress::StackProgress,
    teardown_delete::validate_certificate_ownership,
    POLL_INTERVAL, STACK_TIMEOUT,
};
use crate::cli::{Output, OutputFormat};
use crate::commands::aws_error::{map_aws_error_with_code, sdk_error_code, sdk_error_text};
use crate::commands::support::load_aws_sdk_config;
use crate::config::Config;
use crate::error::{Result, ShimesuError};

/// Opaque token representing confirmed stack destruction intent.
/// Created only by parse(true); parse(false) returns Usage error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackDestroyInput {
    _confirmed: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeleteRequestOutcome {
    Started,
    AlreadyAbsent,
}

impl StackDestroyInput {
    /// Parse destruction confirmation. Returns Usage error if confirm is false.
    pub fn parse(confirm: bool) -> Result<Self> {
        if !confirm {
            return Err(ShimesuError::Usage(
                "stack destroy requires --confirm flag to proceed".to_string(),
            ));
        }
        Ok(Self { _confirmed: () })
    }
}

pub async fn run_stack_destroy(
    config: &Config,
    output: &Output,
    _input: StackDestroyInput,
) -> Result<()> {
    let mut spinner = StackProgress::new(
        output,
        &format!("Deleting stack '{}'...", config.stack_name),
    );
    let aws_config = load_aws_sdk_config(config).await;
    let cfn_client = Client::new(&aws_config);
    let client_request_token = Ulid::generate().to_string();

    let ownership = inspect_installation_ownership(&aws_config, &config.stack_name).await?;
    let expected_domain = match &ownership {
        InstallationOwnership::Owned(outputs) => outputs.base_domain.as_deref(),
        InstallationOwnership::Absent => None,
    };
    let certificate_config = certificate_config(config);
    let certificate_aws_config = load_aws_sdk_config(&certificate_config).await;
    let certificate_stack_inspection = inspect_managed_certificate_stack(
        &Client::new(&certificate_aws_config),
        &config.stack_name,
        expected_domain,
    )
    .await;
    let certificate_inspection = match certificate_stack_inspection {
        Ok(ManagedCertificateStack::Owned { certificate_arn }) => validate_certificate_ownership(
            &AcmClient::new(&certificate_aws_config),
            &certificate_arn,
            &config.stack_name,
        )
        .await
        .map(|exists| (exists, true)),
        Ok(ManagedCertificateStack::Absent) => Ok((false, true)),
        Err(error) => Err(error),
    };
    let (retained_certificate, retained_certificate_verified) = match certificate_inspection {
        Ok(retention) => retention,
        Err(error) => {
            let message = format!(
                "Could not verify managed certificate retention for '{}': {error}",
                config.stack_name
            );
            match output.format() {
                OutputFormat::Human => output.warn(&message),
                OutputFormat::Json => eprintln!("{message}"),
            }
            (false, false)
        }
    };
    let delete_outcome = match ownership {
        InstallationOwnership::Owned(_) => {
            request_stack_delete(&cfn_client, &config.stack_name, &client_request_token).await?
        }
        InstallationOwnership::Absent => DeleteRequestOutcome::AlreadyAbsent,
    };
    if should_wait_for_delete(delete_outcome) {
        spinner.set_message(&format!(
            "Waiting for stack '{}' deletion...",
            config.stack_name
        ));
        wait_for_stack_delete(&cfn_client, &config.stack_name).await?;
    }

    let destroy_output = StackDestroyOutput::from_config(
        config,
        delete_outcome,
        retained_certificate,
        retained_certificate_verified,
    );
    spinner.clear();
    render_stack_destroy_output(output, &destroy_output)
}

pub(super) async fn request_stack_delete(
    cfn_client: &Client,
    stack_name: &str,
    client_request_token: &str,
) -> Result<DeleteRequestOutcome> {
    match cfn_client
        .delete_stack()
        .stack_name(stack_name)
        .client_request_token(client_request_token)
        .send()
        .await
    {
        Ok(_) => Ok(DeleteRequestOutcome::Started),
        Err(error) => {
            map_delete_stack_error(stack_name, sdk_error_code(&error), &sdk_error_text(&error))
        }
    }
}

pub(super) async fn wait_for_stack_delete(cfn_client: &Client, stack_name: &str) -> Result<()> {
    let poll = async {
        let mut ticker = interval(POLL_INTERVAL);

        loop {
            ticker.tick().await;

            match interpret_delete_poll(describe_stack_status(cfn_client, stack_name).await)? {
                true => return Ok(()),
                false => continue,
            }
        }
    };

    timeout(STACK_TIMEOUT, poll).await.map_err(|_| {
        ShimesuError::Timeout(format!(
            "Stack '{stack_name}' deletion timed out after {} seconds",
            STACK_TIMEOUT.as_secs()
        ))
    })?
}

fn map_delete_stack_error(
    stack_name: &str,
    code: Option<&str>,
    error_text: &str,
) -> Result<DeleteRequestOutcome> {
    let lower = error_text.to_ascii_lowercase();

    if lower.contains("does not exist") || lower.contains("not found") {
        Ok(DeleteRequestOutcome::AlreadyAbsent)
    } else {
        Err(map_aws_error_with_code(
            &format!("Failed to delete stack '{}'", stack_name),
            code,
            error_text,
        ))
    }
}

fn should_wait_for_delete(delete_outcome: DeleteRequestOutcome) -> bool {
    matches!(delete_outcome, DeleteRequestOutcome::Started)
}

/// Interpret CloudFormation stack status during deletion polling.
/// Accepts Result<StackStatus> to handle NotFound explicitly.
/// Returns Ok(true) when deletion is complete.
fn interpret_delete_poll(status_result: Result<StackStatus>) -> Result<bool> {
    match status_result {
        Err(ShimesuError::NotFound(_)) => Ok(true),
        Err(e) => Err(e),
        Ok(status) => match &status {
            StackStatus::DeleteComplete => Ok(true),
            StackStatus::DeleteFailed => {
                Err(ShimesuError::Generic("Stack deletion failed".to_string()))
            }
            StackStatus::DeleteInProgress
            | StackStatus::CreateComplete
            | StackStatus::UpdateComplete
            | StackStatus::ImportComplete
            | StackStatus::CreateFailed
            | StackStatus::RollbackComplete
            | StackStatus::RollbackFailed
            | StackStatus::RollbackInProgress
            | StackStatus::UpdateFailed
            | StackStatus::UpdateRollbackComplete
            | StackStatus::UpdateRollbackFailed
            | StackStatus::UpdateCompleteCleanupInProgress
            | StackStatus::UpdateRollbackInProgress
            | StackStatus::UpdateRollbackCompleteCleanupInProgress
            | StackStatus::ImportRollbackComplete
            | StackStatus::ImportRollbackFailed
            | StackStatus::ImportInProgress
            | StackStatus::ImportRollbackInProgress
            | StackStatus::ReviewInProgress
            | StackStatus::CreateInProgress => Ok(false),
            _ => {
                tracing::warn!(
                    stack_status = status.as_str(),
                    "Observed unknown stack status during delete polling; continuing until timeout"
                );
                Ok(false)
            }
        },
    }
}

#[cfg(test)]
#[path = "destroy_tests.rs"]
mod destroy_tests;
