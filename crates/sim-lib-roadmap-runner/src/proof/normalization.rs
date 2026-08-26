fn normalize(
    command: &CommandProof,
    attempt: SandboxAttempt,
    observed_at: String,
) -> TypedProofReceipt {
    let base = |disposition, launcher, detail| TypedProofReceipt {
        proof: command.name.clone(),
        effect_id: Some(command.effect_id.clone()),
        disposition,
        exit_code: None,
        timeout: false,
        signal: None,
        truncated: false,
        launcher_identity: launcher,
        sandbox_identity: None,
        stdout_object: None,
        stderr_object: None,
        observed_at: observed_at.clone(),
        semantic_detail: detail,
    };
    match attempt {
        SandboxAttempt::Completed(result) => {
            let digest = hex_sha256(&result.stdout);
            let controls = result
                .report
                .proves_required(&command.sandbox_request().expect("validated").policy);
            let truncated = !result.report.limit_hits.is_empty();
            let passed = controls
                && !truncated
                && result.exit_code == command.expected.exit_code
                && digest == command.expected.stdout_sha256;
            TypedProofReceipt {
                proof: command.name.clone(),
                effect_id: Some(command.effect_id.clone()),
                disposition: if passed {
                    ProofDisposition::Passed
                } else {
                    ProofDisposition::Failed
                },
                exit_code: Some(result.exit_code),
                timeout: false,
                signal: (result.exit_code < 0).then_some(-result.exit_code),
                truncated,
                launcher_identity: Some(result.report.launcher.clone()),
                sandbox_identity: Some(result.report.cleanup.clone()),
                stdout_object: Some(digest),
                stderr_object: Some(hex_sha256(&result.stderr)),
                observed_at,
                semantic_detail: if passed {
                    "structured expectation matched"
                } else {
                    "operational exit was not semantic proof"
                }
                .into(),
            }
        }
        SandboxAttempt::Stopped(report) => {
            let mut receipt = base(
                ProofDisposition::Failed,
                Some(report.launcher),
                "bounded stop".into(),
            );
            receipt.timeout = true;
            receipt.sandbox_identity = Some(report.cleanup);
            receipt
        }
        SandboxAttempt::Refused(v) => base(ProofDisposition::Failed, Some(v.launcher), v.reason),
        SandboxAttempt::Unknown(v) => base(ProofDisposition::Ambiguous, Some(v.launcher), v.reason),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
