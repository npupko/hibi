//! A minimal Rust resolver demonstrating the SDK. It declares a third-party
//! `scip-symbol` kind and returns an `unchanged`/`unchanged` two-axis verdict —
//! illustrating how a new anchor kind plugs in out-of-process with no core
//! change (§7.2).

use hibi_resolver::{
    serve, DescribeResult, Resolver, ResolveParams, ResolveResult, Verdict, VerdictEvidence,
};

struct EchoResolver;

impl Resolver for EchoResolver {
    fn describe(&self) -> DescribeResult {
        DescribeResult {
            name: "rust-echo".to_string(),
            version: "1".to_string(),
            kinds: vec!["scip-symbol".to_string()],
            tier: 2,
            advisory: false,
            verifier_kinds: vec![],
        }
    }

    fn resolve(&self, params: ResolveParams) -> ResolveResult {
        let a = params.assertion;
        ResolveResult {
            verdict: Some(Verdict {
                assertion_id: a.id,
                proposition_id: a.proposition_id,
                document_id: a.document_id,
                doc: "unchanged".to_string(),
                code: "unchanged".to_string(),
                behavior: None,
                expired: false,
                gates: false,
                suppressed: false,
                evidence: VerdictEvidence {
                    doc_region: None,
                    code_regions: vec![],
                    confidence: 1.0,
                    selector_scores: vec![],
                    changed_evidence: vec![],
                    r#ref: Some(a.r#ref),
                },
                notes: vec!["rust-echo: structural symbol present".to_string()],
                advisories: vec![],
            }),
            advisories: vec![],
        }
    }
}

fn main() {
    serve(EchoResolver);
}
