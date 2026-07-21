//! `stack` subcommands and shared CloudFormation polling helpers.

use std::time::Duration;

use aws_sdk_cloudformation::{
    types::{EventFilter, OperationEvent, StackStatus},
    Client as CfnClient,
};
use tokio::time::{interval, timeout};

use super::aws_error::{map_stack_describe_error, map_stack_describe_sdk_error};
use crate::error::{Result, ShimesuError};

mod assets;
mod certificate;
mod certificate_dns;
mod certificate_ownership;
mod certificate_stack;
mod certificate_validation;
mod destroy;
mod destroy_output;
mod init;
mod input;
mod ownership;
mod progress;
mod route53;
mod teardown;
mod teardown_delete;
mod teardown_discovery;
mod teardown_s3;
mod update;

pub use destroy::{run_stack_destroy, StackDestroyInput, StackDestroyOutput};
pub use init::{run_stack_init, DnsRecord, StackInitOutput};
pub use input::StackInitInput;
pub use teardown::{run_stack_teardown, StackTeardownInput, StackTeardownOutput};
pub use update::{run_stack_update, StackUpdateOutput};

pub const STACK_TEMPLATE: &str = include_str!("../../../infra/stack.yaml");
pub const CERTIFICATE_TEMPLATE: &str = include_str!("../../../infra/certificate.yaml");
pub const POLL_INTERVAL: Duration = Duration::from_secs(10);
pub const STACK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

async fn describe_stack_status(cfn_client: &CfnClient, stack_name: &str) -> Result<StackStatus> {
    let response = cfn_client
        .describe_stacks()
        .stack_name(stack_name)
        .send()
        .await
        .map_err(|error| map_stack_describe_sdk_error(stack_name, &error))?;

    let stack = response
        .stacks()
        .first()
        .ok_or_else(|| map_stack_describe_error(stack_name, "stack not found"))?;

    stack
        .stack_status()
        .map(|status| StackStatus::from(status.as_str()))
        .ok_or_else(|| ShimesuError::Generic(format!("Stack '{stack_name}' returned no status")))
}

pub fn interpret_stack_status(status: StackStatus) -> Result<bool> {
    match &status {
        StackStatus::CreateComplete
        | StackStatus::UpdateComplete
        | StackStatus::DeleteComplete
        | StackStatus::ImportComplete => Ok(true),
        StackStatus::CreateInProgress
        | StackStatus::UpdateInProgress
        | StackStatus::DeleteInProgress
        | StackStatus::UpdateCompleteCleanupInProgress
        | StackStatus::UpdateRollbackInProgress
        | StackStatus::UpdateRollbackCompleteCleanupInProgress
        | StackStatus::ImportInProgress
        | StackStatus::ImportRollbackInProgress
        | StackStatus::ReviewInProgress => Ok(false),
        other if other.as_str() == "DELETE_SKIPPED" => Ok(false),
        StackStatus::CreateFailed
        | StackStatus::RollbackComplete
        | StackStatus::RollbackFailed
        | StackStatus::RollbackInProgress
        | StackStatus::DeleteFailed
        | StackStatus::UpdateFailed
        | StackStatus::UpdateRollbackComplete
        | StackStatus::UpdateRollbackFailed
        | StackStatus::ImportRollbackComplete
        | StackStatus::ImportRollbackFailed => Err(ShimesuError::Generic(format!(
            "Stack operation failed: {}",
            status.as_str()
        ))),
        _ => Err(ShimesuError::Generic(format!(
            "Unknown stack status: {:?}",
            status
        ))),
    }
}

pub async fn wait_for_stack(
    cfn_client: &CfnClient,
    stack_name: &str,
    timeout_duration: Duration,
) -> Result<StackStatus> {
    let poll = async {
        let mut ticker = interval(POLL_INTERVAL);

        loop {
            ticker.tick().await;

            let status = describe_stack_status(cfn_client, stack_name).await?;
            match interpret_stack_status(StackStatus::from(status.as_str())) {
                Ok(true) => return Ok(status),
                Ok(false) => continue,
                Err(status_error) => {
                    return Err(
                        stack_failure_with_details(cfn_client, stack_name, status_error).await,
                    )
                }
            }
        }
    };

    timeout(timeout_duration, poll).await.map_err(|_| {
        ShimesuError::Timeout(format!(
            "Stack '{stack_name}' operation timed out after {} seconds",
            timeout_duration.as_secs()
        ))
    })?
}

async fn stack_failure_with_details(
    client: &CfnClient,
    stack_name: &str,
    status_error: ShimesuError,
) -> ShimesuError {
    let status_message = match status_error {
        ShimesuError::Generic(message) => message,
        other => other.to_string(),
    };
    let response = client
        .describe_events()
        .stack_name(stack_name)
        .filters(EventFilter::builder().failed_events(true).build())
        .send()
        .await;

    match response {
        Ok(events) => failure_event_message(events.operation_events())
            .map(|detail| ShimesuError::Generic(format!("{status_message}. {detail}")))
            .unwrap_or_else(|| ShimesuError::Generic(status_message)),
        Err(error) => ShimesuError::Generic(format!(
            "{status_message}. Failed to retrieve CloudFormation failure details: {error}"
        )),
    }
}

fn failure_event_message(events: &[OperationEvent]) -> Option<String> {
    events
        .iter()
        .find(|event| event.resource_type() != Some("AWS::CloudFormation::Stack"))
        .or_else(|| events.first())
        .and_then(|event| {
            let reason = event
                .validation_status_reason()
                .or_else(|| event.resource_status_reason())
                .or_else(|| event.hook_status_reason())?;
            let resource = event.logical_resource_id().unwrap_or("stack");
            Some(format!("{resource}: {reason}"))
        })
}

#[cfg(test)]
#[path = "stack_status_tests.rs"]
mod stack_status_tests;
