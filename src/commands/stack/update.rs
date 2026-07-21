use aws_sdk_cloudformation::types::Parameter;
use aws_sdk_cloudformation::Client;
use aws_sdk_s3::Client as S3Client;
use serde::Serialize;

use super::{
    assets::upload_landing_assets,
    ownership::{inspect_installation_ownership, InstallationOwnership},
    progress::StackProgress,
    wait_for_stack, STACK_TEMPLATE, STACK_TIMEOUT,
};
use crate::cli::Output;
use crate::commands::aws_error::{map_aws_error_with_code, sdk_error_code, sdk_error_text};
use crate::commands::support::{load_aws_sdk_config, load_stack_outputs};
use crate::config::Config;
use crate::error::{Result, ShimesuError};

pub(super) const UPDATE_PARAMETER_KEYS: &[&str] = &[
    "BaseDomain",
    "CertificateArn",
    "AttachAliases",
    "HostedZoneId",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateRequestOutcome {
    Started,
    AlreadyUpToDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackUpdateOutput {
    pub stack_name: String,
    pub region: String,
    pub updated: bool,
    pub already_up_to_date: bool,
}

impl StackUpdateOutput {
    fn human_rows(&self) -> [(&'static str, &str); 3] {
        [
            ("Stack Name", &self.stack_name),
            ("Region", &self.region),
            (
                "Template",
                if self.already_up_to_date {
                    "already up to date"
                } else {
                    "updated"
                },
            ),
        ]
    }
}

pub async fn run_stack_update(config: &Config, output: &Output) -> Result<()> {
    let mut spinner = StackProgress::new(
        output,
        &format!("Updating stack '{}'...", config.stack_name),
    );
    let aws_config = load_aws_sdk_config(config).await;
    let cfn_client = Client::new(&aws_config);
    match inspect_installation_ownership(&aws_config, &config.stack_name).await? {
        InstallationOwnership::Owned(_) => {}
        InstallationOwnership::Absent => {
            return Err(ShimesuError::NotFound(format!(
                "Stack '{}' not found",
                config.stack_name
            )))
        }
    }

    let outcome = request_stack_update(&cfn_client, &config.stack_name).await?;
    if outcome == UpdateRequestOutcome::Started {
        spinner.set_message(&format!(
            "Waiting for stack '{}' update...",
            config.stack_name
        ));
        wait_for_stack(&cfn_client, &config.stack_name, STACK_TIMEOUT).await?;
    }

    let stack_outputs = load_stack_outputs(&aws_config, &config.stack_name).await?;
    let bucket_name = stack_outputs.require_bucket_name()?;
    spinner.set_message(&format!("Uploading landing pages to '{bucket_name}'..."));
    upload_landing_assets(&S3Client::new(&aws_config), &bucket_name).await?;

    let update_output = StackUpdateOutput {
        stack_name: config.stack_name.clone(),
        region: config.region.clone(),
        updated: outcome == UpdateRequestOutcome::Started,
        already_up_to_date: outcome == UpdateRequestOutcome::AlreadyUpToDate,
    };
    spinner.clear();
    render_stack_update_output(output, &update_output)
}

fn previous_value_parameters() -> Vec<Parameter> {
    UPDATE_PARAMETER_KEYS
        .iter()
        .map(|key| {
            Parameter::builder()
                .parameter_key(*key)
                .use_previous_value(true)
                .build()
        })
        .collect()
}

async fn request_stack_update(
    cfn_client: &Client,
    stack_name: &str,
) -> Result<UpdateRequestOutcome> {
    match cfn_client
        .update_stack()
        .stack_name(stack_name)
        .template_body(STACK_TEMPLATE)
        .set_parameters(Some(previous_value_parameters()))
        .send()
        .await
    {
        Ok(_) => Ok(UpdateRequestOutcome::Started),
        Err(error) => {
            map_update_stack_error(stack_name, sdk_error_code(&error), &sdk_error_text(&error))
        }
    }
}

fn map_update_stack_error(
    stack_name: &str,
    code: Option<&str>,
    error_text: &str,
) -> Result<UpdateRequestOutcome> {
    let lower = error_text.to_ascii_lowercase();

    if lower.contains("no updates are to be performed") {
        Ok(UpdateRequestOutcome::AlreadyUpToDate)
    } else if lower.contains("does not exist") || lower.contains("not found") {
        Err(ShimesuError::NotFound(format!(
            "Stack '{stack_name}' not found"
        )))
    } else {
        Err(map_aws_error_with_code(
            &format!("Failed to update stack '{stack_name}'"),
            code,
            error_text,
        ))
    }
}

fn render_stack_update_output(output: &Output, update_output: &StackUpdateOutput) -> Result<()> {
    output.render(update_output, |out, update| {
        if update.already_up_to_date {
            out.ok(&format!(
                "Stack '{}' is already up to date",
                update.stack_name
            ));
        } else {
            out.ok(&format!("Updated stack '{}'", update.stack_name));
        }
        for (key, value) in update.human_rows() {
            out.kv(key, value);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_updates_error_reports_already_up_to_date() {
        let outcome = map_update_stack_error(
            "shimesu",
            Some("ValidationError"),
            "ValidationError: No updates are to be performed.",
        );

        assert!(matches!(outcome, Ok(UpdateRequestOutcome::AlreadyUpToDate)));
    }

    #[test]
    fn missing_stack_maps_to_not_found() {
        let outcome = map_update_stack_error(
            "shimesu",
            Some("ValidationError"),
            "ValidationError: Stack [shimesu] does not exist",
        );

        assert!(
            matches!(outcome, Err(ShimesuError::NotFound(message)) if message == "Stack 'shimesu' not found")
        );
    }

    #[test]
    fn other_errors_delegate_to_structured_mapping() {
        let outcome = map_update_stack_error("shimesu", Some("AccessDenied"), "service error");

        assert!(matches!(outcome, Err(ShimesuError::AwsAuth(_))));
    }

    #[test]
    fn update_parameters_reuse_previous_values() {
        let parameters = previous_value_parameters();

        assert_eq!(parameters.len(), UPDATE_PARAMETER_KEYS.len());
        for parameter in &parameters {
            assert_eq!(parameter.use_previous_value(), Some(true));
            assert!(parameter.parameter_value().is_none());
        }
    }
}
