//! Destroy result model and rendering, including retained-data warnings.

use serde::Serialize;

use crate::cli::{bool_text, Output};
use crate::config::Config;
use crate::error::Result;

use super::destroy::DeleteRequestOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackDestroyOutput {
    pub stack_name: String,
    pub region: String,
    pub deleted: bool,
    pub already_absent: bool,
    pub retained_data: bool,
    pub retained_data_verified: bool,
    pub retained_certificate: bool,
    pub retained_certificate_verified: bool,
    pub certificate_stack_name: String,
}

impl StackDestroyOutput {
    pub(super) fn from_config(
        config: &Config,
        delete_outcome: DeleteRequestOutcome,
        retained_certificate: bool,
        retained_certificate_verified: bool,
    ) -> Self {
        let deleted = matches!(delete_outcome, DeleteRequestOutcome::Started);
        Self {
            stack_name: config.stack_name.clone(),
            region: config.region.as_deref().unwrap_or("auto").to_string(),
            deleted,
            already_absent: !deleted,
            retained_data: true,
            retained_data_verified: deleted,
            retained_certificate,
            retained_certificate_verified,
            certificate_stack_name: format!("{}-certificate", config.stack_name),
        }
    }

    pub(super) fn human_rows(&self) -> [(&'static str, &str); 9] {
        [
            ("Stack Name", &self.stack_name),
            ("Region", &self.region),
            ("Deleted", bool_text(self.deleted)),
            ("Already Absent", bool_text(self.already_absent)),
            ("Retained Data", bool_text(self.retained_data)),
            (
                "Retained Data Verified",
                bool_text(self.retained_data_verified),
            ),
            ("Retained Certificate", bool_text(self.retained_certificate)),
            (
                "Retained Certificate Verified",
                bool_text(self.retained_certificate_verified),
            ),
            ("Certificate Stack", &self.certificate_stack_name),
        ]
    }

    pub(super) fn success_message(&self) -> String {
        if self.deleted {
            format!("Deleted stack '{}'", self.stack_name)
        } else {
            format!("Stack '{}' was already absent", self.stack_name)
        }
    }

    pub(super) fn retained_data_warning(&self) -> Option<String> {
        if !self.retained_data_verified && !self.retained_certificate_verified {
            return Some(
                "The regional stack was already absent, so retained S3 bucket and DynamoDB table data may remain but could not be verified by stack outputs. Managed certificate retention also could not be verified. Use 'shimesu stack teardown --confirm-data-loss' only after resolving certificate-stack ownership."
                    .to_string(),
            );
        }
        if !self.retained_certificate_verified {
            return Some(
                "Managed certificate retention could not be verified. The regional stack operation is unaffected; use 'shimesu stack teardown --confirm-data-loss' only after resolving certificate-stack ownership."
                    .to_string(),
            );
        }
        if !self.retained_data_verified {
            let certificate_note = if self.retained_certificate {
                format!(
                    " The managed certificate stack '{}' and its ACM certificate also remain.",
                    self.certificate_stack_name
                )
            } else {
                String::new()
            };
            return Some(format!(
                "The regional stack was already absent, so retained S3 bucket and DynamoDB table data may remain but could not be verified by stack outputs.{certificate_note} Use 'shimesu stack teardown --confirm-data-loss' for ownership-checked discovery and permanent removal."
            ));
        }
        match (self.retained_data, self.retained_certificate) {
            (true, true) => Some(format!(
                "Retained S3 bucket, DynamoDB table, certificate stack '{}', and ACM certificate remain in your AWS account. Use 'shimesu stack teardown --confirm-data-loss' for permanent removal.",
                self.certificate_stack_name
            )),
            (true, false) => Some(
                "Retained S3 bucket and DynamoDB table remain in your AWS account. Use 'shimesu stack teardown --confirm-data-loss' for permanent removal."
                    .to_string(),
            ),
            (false, true) => Some(format!(
                "Certificate stack '{}' and its ACM certificate remain in your AWS account. Use 'shimesu stack teardown --confirm-data-loss' for permanent removal.",
                self.certificate_stack_name
            )),
            (false, false) => None,
        }
    }
}

pub(super) fn render_stack_destroy_output(
    output: &Output,
    stack_output: &StackDestroyOutput,
) -> Result<()> {
    output.render(stack_output, |out, stack| {
        out.ok(&stack.success_message());
        for (key, value) in stack.human_rows() {
            out.kv(key, value);
        }
        if let Some(warning) = stack.retained_data_warning() {
            out.warn(&warning);
        }
    })
}
