//! What a command hands back for printing, and the output objects of the
//! commands whose result is made in this crate: `init`, `schema`, `version`
//! and `providers` (spec section 15.3).

use std::collections::BTreeMap;

use hibi::engine::{Envelope, ProvidersOutput, Warning};
use hibi::provider::command::ProviderCheck;
use schemars::JsonSchema;
use serde::Serialize;

/// The result of a command that ran.
pub struct Reply {
    pub body: Body,
    /// Printed to standard error in human format. In JSON format they are
    /// already inside the envelope.
    pub warnings: Vec<Warning>,
}

pub enum Body {
    /// The usual case: the JSON envelope as one line, and the same result
    /// as text for a person.
    Result { json: String, human: String },
    /// Text that is the whole standard output in both formats: a schema or
    /// a completion script.
    Raw(String),
}

impl Reply {
    /// The reply of `command` with result `output`. `human` is the text
    /// form of the same result.
    pub fn new<T: Serialize>(
        command: &str,
        output: &T,
        warnings: &[Warning],
        human: String,
    ) -> Result<Reply, hibi::Error> {
        let json = serde_json::to_string(&Envelope::new(command, output))
            .map_err(|e| hibi::Error::internal(format!("the result does not serialize: {e}")))?;
        Ok(Reply {
            body: Body::Result { json, human },
            warnings: warnings.to_vec(),
        })
    }

    pub fn raw(text: String) -> Reply {
        Reply {
            body: Body::Raw(text),
            warnings: Vec::new(),
        }
    }
}

/// The output of `init`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InitOutput {
    /// The root: the git top level that now holds `.hibi/`.
    pub root: String,
    /// The created files, relative to the root.
    pub created: Vec<String>,
    pub warnings: Vec<Warning>,
}

/// The output of `schema` without a name.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SchemaOutput {
    /// Every name `hibi schema <name>` accepts.
    pub schemas: Vec<String>,
    pub warnings: Vec<Warning>,
}

/// The output of `version`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VersionOutput {
    pub version: String,
    /// The provider protocol versions this build speaks.
    pub protocol: Vec<u32>,
    pub store_version: u32,
    /// The fingerprint profiles.
    pub profiles: Vec<String>,
    /// Resource class to the version of its grammar crate.
    pub grammars: BTreeMap<String, String>,
    /// The model id sent to the judge service.
    pub judge_model: String,
    pub warnings: Vec<Warning>,
}

/// The output of `providers`. `check` is present only with `--check`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProvidersReply {
    #[serde(flatten)]
    pub report: ProvidersOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<ProviderCheck>,
}
