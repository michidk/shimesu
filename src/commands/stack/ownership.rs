//! Verifies regional stack ownership tags before mutating operations.

use aws_sdk_cloudformation::Client as CfnClient;

use crate::commands::aws_error::map_stack_describe_sdk_error;
use crate::commands::support::{stack_outputs_from_description, StackOutputs};
use crate::error::{Result, ShimesuError};

use super::certificate_ownership::cloudformation_tags_belong_to_stack;

pub(super) enum InstallationOwnership {
    Absent,
    Owned(StackOutputs),
}

pub(super) async fn inspect_installation_ownership(
    aws_config: &aws_config::SdkConfig,
    stack_name: &str,
) -> Result<InstallationOwnership> {
    let response = match CfnClient::new(aws_config)
        .describe_stacks()
        .stack_name(stack_name)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return match map_stack_describe_sdk_error(stack_name, &error) {
                ShimesuError::NotFound(_) => Ok(InstallationOwnership::Absent),
                other => Err(other),
            };
        }
    };
    let stack = response
        .stacks()
        .first()
        .ok_or_else(|| ShimesuError::NotFound(format!("Stack '{stack_name}' not found")))?;
    if !cloudformation_tags_belong_to_stack(stack.tags(), stack_name) {
        return Err(ShimesuError::Validation(format!(
            "Refusing to mutate stack '{stack_name}' because it is missing required shimesu ownership tags"
        )));
    }
    let outputs = stack_outputs_from_description(stack);
    validate_installation_outputs(stack_name, &outputs)?;
    Ok(InstallationOwnership::Owned(outputs))
}

pub(super) fn validate_installation_outputs(
    stack_name: &str,
    outputs: &StackOutputs,
) -> Result<()> {
    let expected_table_name = format!("{stack_name}-projects");
    let expected_bucket_prefix = format!("{}-contentbucket-", stack_name.to_ascii_lowercase());
    let is_owned = outputs.table_name.as_deref() == Some(&expected_table_name)
        && outputs
            .bucket_name
            .as_deref()
            .is_some_and(|name| name.starts_with(&expected_bucket_prefix))
        && outputs.base_domain.is_some()
        && outputs.certificate_arn.is_some()
        && outputs.distribution_id.is_some()
        && outputs.distribution_domain_name.is_some();

    if is_owned {
        Ok(())
    } else {
        Err(ShimesuError::Validation(format!(
            "Refusing to mutate stack '{stack_name}' because its outputs do not identify a Shimesu installation"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::validate_installation_outputs;
    use crate::commands::support::StackOutputs;
    use crate::error::ShimesuError;

    fn outputs() -> StackOutputs {
        StackOutputs {
            table_name: Some("shimesu-demo-projects".to_string()),
            bucket_name: Some("shimesu-demo-contentbucket-a1b2c3".to_string()),
            base_domain: Some("pages.example.com".to_string()),
            certificate_arn: Some(
                "arn:aws:acm:us-east-1:123456789012:certificate/example".to_string(),
            ),
            distribution_id: Some("E1234567890".to_string()),
            distribution_domain_name: Some("d111111abcdef8.cloudfront.net".to_string()),
        }
    }

    #[test]
    fn expected_installation_outputs_are_owned() {
        assert!(validate_installation_outputs("shimesu-demo", &outputs()).is_ok());
    }

    #[test]
    fn unrelated_table_output_is_rejected() {
        let mut outputs = outputs();
        outputs.table_name = Some("other-projects".to_string());

        let result = validate_installation_outputs("shimesu-demo", &outputs);

        assert!(
            matches!(result, Err(ShimesuError::Validation(message)) if message.contains("Refusing"))
        );
    }

    #[test]
    fn unrelated_bucket_output_is_rejected() {
        let mut outputs = outputs();
        outputs.bucket_name = Some("other-contentbucket-a1b2c3".to_string());

        assert!(validate_installation_outputs("shimesu-demo", &outputs).is_err());
    }

    #[test]
    fn incomplete_outputs_are_rejected() {
        let mut outputs = outputs();
        outputs.distribution_id = None;

        assert!(validate_installation_outputs("shimesu-demo", &outputs).is_err());
    }
}
