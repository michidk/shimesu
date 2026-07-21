use super::*;

fn complete_outputs() -> HashMap<String, String> {
    HashMap::from([
        ("BaseDomain".to_string(), "example.com".to_string()),
        ("BucketName".to_string(), "shimesu-bucket".to_string()),
        ("DistributionId".to_string(), "EDFDVBD632BHDS5".to_string()),
        ("TableName".to_string(), "shimesu-prod-projects".to_string()),
    ])
}

#[test]
fn parse_required_stack_outputs_rejects_any_missing_required_output() {
    for missing_key in ["BaseDomain", "BucketName", "DistributionId", "TableName"] {
        let mut outputs = complete_outputs();
        let removed = outputs.remove(missing_key);
        assert!(
            removed.is_some(),
            "test fixture should include {missing_key}"
        );

        let result = parse_required_stack_outputs("shimesu-prod", &outputs);

        match result {
            Err(ShimesuError::Config(message)) => {
                assert!(message.contains(missing_key));
                assert_eq!(
                    ShimesuError::Config(message).exit_code(),
                    crate::error::ExitCode::Config
                );
            }
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(parsed) => panic!("expected error, got {parsed:?}"),
        }
    }
}

#[test]
fn parse_required_stack_outputs_returns_all_required_values_when_complete() {
    let parsed = parse_required_stack_outputs("shimesu-prod", &complete_outputs())
        .unwrap_or_else(|error| panic!("expected complete outputs to parse: {error}"));

    assert_eq!(
        parsed,
        RequiredStackOutputs {
            base_domain: "example.com".to_string(),
            bucket_name: "shimesu-bucket".to_string(),
            distribution_id: "EDFDVBD632BHDS5".to_string(),
            table_name: "shimesu-prod-projects".to_string(),
        }
    );
}

#[test]
fn build_status_output_preserves_existing_shape_when_outputs_are_complete() {
    let config = Config {
        stack_name: "shimesu-prod".to_string(),
        region: "us-east-1".to_string(),
        profile: Some("team".to_string()),
        json: false,
        yes: false,
    };
    let caller_identity = CallerIdentity {
        account: "123456789012".to_string(),
        arn: "arn:aws:iam::123456789012:user/test".to_string(),
        user_id: "AIDAEXAMPLE".to_string(),
    };
    let required_outputs = RequiredStackOutputs {
        base_domain: "example.com".to_string(),
        bucket_name: "shimesu-bucket".to_string(),
        distribution_id: "EDFDVBD632BHDS5".to_string(),
        table_name: "shimesu-prod-projects".to_string(),
    };

    let status_output = build_status_output(
        &config,
        "CREATE_COMPLETE".to_string(),
        caller_identity,
        required_outputs,
    );

    assert_eq!(status_output.stack_name, "shimesu-prod");
    assert_eq!(status_output.stack_status, "CREATE_COMPLETE");
    assert_eq!(status_output.base_domain.as_deref(), Some("example.com"));
    assert_eq!(status_output.bucket_name.as_deref(), Some("shimesu-bucket"));
    assert_eq!(
        status_output.distribution_id.as_deref(),
        Some("EDFDVBD632BHDS5")
    );
    assert_eq!(
        status_output.table_name.as_deref(),
        Some("shimesu-prod-projects")
    );
    assert_eq!(status_output.caller_identity.account, "123456789012");
    assert_eq!(
        status_output.caller_identity.arn,
        "arn:aws:iam::123456789012:user/test"
    );
    assert_eq!(status_output.caller_identity.user_id, "AIDAEXAMPLE");
}
