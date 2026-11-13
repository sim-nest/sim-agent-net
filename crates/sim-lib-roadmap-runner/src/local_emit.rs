// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One-line record of a local roadmap command result.

use sim_kernel::{Cx, Result, Symbol, Value};

use crate::local_command::LocalRoadmapReceipt;

pub(crate) fn command_value(cx: &mut Cx, receipt: LocalRoadmapReceipt) -> Result<Value> {
    let line = render_receipt(&receipt);
    let value = cx.factory().table(vec![
        (
            Symbol::new("outcome"),
            cx.factory().string(receipt.outcome)?,
        ),
        (
            Symbol::new("execution"),
            cx.factory().string(receipt.identity.execution)?,
        ),
        (
            Symbol::new("conduct"),
            cx.factory().string(receipt.identity.conduct)?,
        ),
        (
            Symbol::new("model-pick"),
            cx.factory().string(receipt.identity.model_pick)?,
        ),
        (
            Symbol::new("proof-catalog"),
            cx.factory().string(receipt.identity.proof_catalog)?,
        ),
        (
            Symbol::new("runner-generation"),
            cx.factory().string(receipt.identity.runner_generation)?,
        ),
        (
            Symbol::new("journal-acknowledged"),
            cx.factory().bool(receipt.journal_acknowledged)?,
        ),
        (Symbol::new("detail"), cx.factory().string(receipt.detail)?),
    ])?;
    println!("{line}");
    Ok(value)
}

pub(crate) fn render_receipt(receipt: &LocalRoadmapReceipt) -> String {
    format!(
        "#(roadmap/local-result-v1 (outcome {outcome}) (execution {execution}) (conduct {conduct}) (model-pick {model_pick}) (proof-catalog {proof_catalog}) (runner-generation {runner_generation}) (journal-acknowledged {acknowledged}) (detail {detail}))",
        outcome = quote(&receipt.outcome),
        execution = quote(&receipt.identity.execution),
        conduct = quote(&receipt.identity.conduct),
        model_pick = quote(&receipt.identity.model_pick),
        proof_catalog = quote(&receipt.identity.proof_catalog),
        runner_generation = quote(&receipt.identity.runner_generation),
        acknowledged = if receipt.journal_acknowledged {
            "true"
        } else {
            "false"
        },
        detail = quote(&receipt.detail),
    )
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::render_receipt;
    use crate::local_command::{LocalExecutionIdentity, LocalRoadmapReceipt};

    fn receipt(detail: &str, acknowledged: bool) -> LocalRoadmapReceipt {
        LocalRoadmapReceipt {
            outcome: "ok".into(),
            identity: LocalExecutionIdentity {
                execution: "local/no-model".into(),
                conduct: "conduct/local".into(),
                model_pick: "model/local".into(),
                proof_catalog: "proof/local".into(),
                runner_generation: "generation/local".into(),
            },
            journal_acknowledged: acknowledged,
            detail: detail.into(),
        }
    }

    #[test]
    fn status_record_is_one_stable_line() {
        let line = render_receipt(&receipt("execution not started", true));
        assert_eq!(
            line,
            "#(roadmap/local-result-v1 (outcome \"ok\") (execution \"local/no-model\") (conduct \"conduct/local\") (model-pick \"model/local\") (proof-catalog \"proof/local\") (runner-generation \"generation/local\") (journal-acknowledged true) (detail \"execution not started\"))"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn record_text_escapes_and_keeps_a_false_acknowledgement() {
        let line = render_receipt(&receipt("say \"hi\"\\\n", false));
        assert!(line.contains("(journal-acknowledged false)"));
        assert!(line.contains("(detail \"say \\\"hi\\\"\\\\\\n\")"));
        assert!(!line.contains('\n'));
    }
}
