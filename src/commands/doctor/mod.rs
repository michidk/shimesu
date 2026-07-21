//! `doctor` command: credential, stack, and backend health checks.

mod aws;
mod backend;
mod models;

pub use models::{DoctorCheck, DoctorCheckName, DoctorCheckStatus, DoctorOutput};

use crate::cli::Output;
use crate::commands::support::load_aws_sdk_config;
use crate::config::Config;
use crate::error::{Result, ShimesuError};
use aws::AwsDoctorBackend;
use aws_sdk_cloudformation::types::StackStatus;
use backend::DoctorBackend;

pub async fn run_doctor(config: &Config, output: &Output) -> Result<()> {
    let aws_config = load_aws_sdk_config(config).await;
    let backend = AwsDoctorBackend::new(&aws_config);
    let doctor_output = run_doctor_checks(&backend, &config.stack_name).await;

    render_doctor_output(output, &doctor_output)?;
    validate_doctor_output(&doctor_output)
}

async fn run_doctor_checks<B: DoctorBackend>(backend: &B, stack_name: &str) -> DoctorOutput {
    let mut checks = Vec::new();

    // Check 1: Credentials
    match backend.check_credentials().await {
        Ok(()) => {
            checks.push(DoctorCheck::passed(DoctorCheckName::Credentials));
        }
        Err(e) => {
            checks.push(DoctorCheck::failed(
                DoctorCheckName::Credentials,
                format!("credentials check failed: {}", e),
            ));
        }
    }

    // Check 2: Stack exists
    let snapshot = match backend.describe_stack(stack_name).await {
        Ok(Some(s)) => {
            checks.push(DoctorCheck::passed(DoctorCheckName::StackExists));
            s
        }
        Ok(None) => {
            checks.push(DoctorCheck::failed(
                DoctorCheckName::StackExists,
                "stack not found",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::StackStatus,
                "stack not found",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::StackOutputs,
                "stack not found",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::S3Bucket,
                "stack not found",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::DynamodbTable,
                "stack not found",
            ));
            return DoctorOutput::new(checks);
        }
        Err(e) => {
            checks.push(DoctorCheck::failed(
                DoctorCheckName::StackExists,
                format!("describe stack failed: {}", e),
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::StackStatus,
                "could not describe stack",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::StackOutputs,
                "could not describe stack",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::S3Bucket,
                "could not describe stack",
            ));
            checks.push(DoctorCheck::not_checked(
                DoctorCheckName::DynamodbTable,
                "could not describe stack",
            ));
            return DoctorOutput::new(checks);
        }
    };

    // Check 3: Stack status
    match snapshot.status {
        StackStatus::CreateComplete | StackStatus::UpdateComplete | StackStatus::ImportComplete => {
            checks.push(DoctorCheck::passed(DoctorCheckName::StackStatus));
        }
        _ => {
            checks.push(DoctorCheck::failed(
                DoctorCheckName::StackStatus,
                format!("stack status is unhealthy: {:?}", snapshot.status),
            ));
        }
    }

    // Check 4: Stack outputs
    if snapshot.all_outputs_present() {
        checks.push(DoctorCheck::passed(DoctorCheckName::StackOutputs));
    } else {
        let missing = snapshot.missing_output_names().join(", ");

        checks.push(DoctorCheck::failed(
            DoctorCheckName::StackOutputs,
            format!("missing outputs: {}", missing),
        ));

        checks.push(DoctorCheck::not_checked(
            DoctorCheckName::S3Bucket,
            "outputs not available",
        ));
        checks.push(DoctorCheck::not_checked(
            DoctorCheckName::DynamodbTable,
            "outputs not available",
        ));
        return DoctorOutput::new(checks);
    }

    // Check 5: S3 bucket (only if outputs present)
    if let Some(bucket_name) = &snapshot.bucket_name {
        match backend.head_bucket(bucket_name).await {
            Ok(()) => {
                checks.push(DoctorCheck::passed(DoctorCheckName::S3Bucket));
            }
            Err(e) => {
                checks.push(DoctorCheck::failed(
                    DoctorCheckName::S3Bucket,
                    format!("bucket check failed: {}", e),
                ));
            }
        }
    } else {
        checks.push(DoctorCheck::not_checked(
            DoctorCheckName::S3Bucket,
            "bucket name not available",
        ));
    }

    // Check 6: DynamoDB table (only if outputs present)
    if let Some(table_name) = &snapshot.table_name {
        match backend.describe_table(table_name).await {
            Ok(()) => {
                checks.push(DoctorCheck::passed(DoctorCheckName::DynamodbTable));
            }
            Err(e) => {
                checks.push(DoctorCheck::failed(
                    DoctorCheckName::DynamodbTable,
                    format!("table check failed: {}", e),
                ));
            }
        }
    } else {
        checks.push(DoctorCheck::not_checked(
            DoctorCheckName::DynamodbTable,
            "table name not available",
        ));
    }

    DoctorOutput::new(checks)
}

fn render_doctor_output(output: &Output, doctor_output: &DoctorOutput) -> Result<()> {
    output.render(doctor_output, |out, doctor| {
        out.header("Doctor checks");
        for check in &doctor.checks {
            render_doctor_check(out, check);
        }
        let non_passed = doctor
            .checks
            .iter()
            .filter(|check| check.status != DoctorCheckStatus::Passed)
            .count();
        if non_passed == 0 {
            out.ok("All checks passed");
        } else {
            out.warn(&format!(
                "{non_passed} of {} check(s) did not pass",
                doctor.checks.len()
            ));
        }
    })
}

fn render_doctor_check(output: &Output, check: &DoctorCheck) {
    let message = match &check.message {
        Some(text) => format!("{}: {}", check.name.label(), text),
        None => check.name.label().to_string(),
    };

    match check.status {
        DoctorCheckStatus::Passed => output.ok(&message),
        DoctorCheckStatus::Failed => output.error(&message),
        DoctorCheckStatus::NotChecked => output.warn(&message),
    }
}

fn validate_doctor_output(doctor_output: &DoctorOutput) -> Result<()> {
    let non_passed_check_count = doctor_output
        .checks
        .iter()
        .filter(|check| check.status != DoctorCheckStatus::Passed)
        .count();

    if non_passed_check_count == 0 {
        Ok(())
    } else {
        Err(ShimesuError::Validation(format!(
            "doctor found {non_passed_check_count} non-passed checks"
        )))
    }
}

#[cfg(test)]
mod tests;
