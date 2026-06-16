//! A minimal Rust resolver demonstrating the SDK. It declares a third-party
//! `scip-symbol` kind and returns a `fresh` verdict — illustrating how a new
//! anchor kind plugs in out-of-process with no core change (§7.2).

use claim_engine_resolver::{serve, DescribeResult, Resolver, ResolveParams, ResolveResult, Verdict};

struct EchoResolver;

impl Resolver for EchoResolver {
    fn describe(&self) -> DescribeResult {
        DescribeResult {
            name: "rust-echo".to_string(),
            version: "1".to_string(),
            kinds: vec!["scip-symbol".to_string()],
            tier: 2,
            advisory: false,
        }
    }

    fn resolve(&self, params: ResolveParams) -> ResolveResult {
        let a = params.assertion;
        ResolveResult {
            verdict: Some(Verdict {
                assertion_id: a.id,
                proposition_id: a.proposition_id,
                document_id: a.document_id,
                state: "fresh".to_string(),
                confidence: 1.0,
                region: None,
                selector_scores: vec![],
                r#ref: Some(a.ref_),
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
