//! Provider protocol v1 wire types (spec sections 13.2 and 13.3).
//!
//! A request line is `{"id", "method", "params"}`; a response line is
//! `{"id", "result"}` or `{"id", "error"}`. Both sides ignore object keys
//! they do not know, except inside a selector, which is closed.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use super::{NameVersion, ProviderError, ProviderErrorCode, ProviderInfo, Rendered, Unit};
use crate::model::{Locator, Selector};

/// The protocol version this hibi speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The most inner requests hibi sends in one `batch`.
pub const MAX_BATCH: usize = 1000;

/// The longest response line hibi accepts, in bytes (16 MiB).
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Protocol methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Hello,
    Resolve,
    Units,
    Render,
    Batch,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Hello => "hello",
            Method::Resolve => "resolve",
            Method::Units => "units",
            Method::Render => "render",
            Method::Batch => "batch",
        }
    }

    pub fn from_name(s: &str) -> Option<Method> {
        [
            Method::Hello,
            Method::Resolve,
            Method::Units,
            Method::Render,
            Method::Batch,
        ]
        .into_iter()
        .find(|m| m.as_str() == s)
    }
}

// ---------------------------------------------------------------------------
// Params
// ---------------------------------------------------------------------------

/// `hello` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloParams {
    /// The versions the client speaks, ascending.
    pub protocol: Vec<u32>,
    pub client: NameVersion,
    /// The absolute root.
    pub root: String,
}

/// `resolve` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolveParams {
    pub loc: Locator,
    pub sel: Selector,
}

/// `units` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnitsParams {
    pub loc: Locator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// `render` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RenderParams {
    pub loc: Locator,
    /// A unit key.
    pub unit: String,
}

/// `batch` params. Inner requests have no `id`; `hello` and `batch` inside
/// a batch are `invalid_request` for that entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BatchParams {
    pub requests: Vec<RawCall>,
}

/// A method and its params as they appear on the wire, not yet checked:
/// the `{"method", "params"}` part of a request, and an inner batch request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RawCall {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// A checked call: a known method with params of the right shape.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Hello(HelloParams),
    Resolve(ResolveParams),
    Units(UnitsParams),
    Render(RenderParams),
    Batch(BatchParams),
}

impl Call {
    pub fn method(&self) -> Method {
        match self {
            Call::Hello(_) => Method::Hello,
            Call::Resolve(_) => Method::Resolve,
            Call::Units(_) => Method::Units,
            Call::Render(_) => Method::Render,
            Call::Batch(_) => Method::Batch,
        }
    }

    /// The wire form.
    pub fn to_raw(&self) -> RawCall {
        let params = match self {
            Call::Hello(p) => serde_json::to_value(p),
            Call::Resolve(p) => serde_json::to_value(p),
            Call::Units(p) => serde_json::to_value(p),
            Call::Render(p) => serde_json::to_value(p),
            Call::Batch(p) => serde_json::to_value(p),
        }
        .expect("protocol params always serialize");
        RawCall {
            method: self.method().as_str().to_owned(),
            params,
        }
    }
}

impl RawCall {
    /// Check the method and params. An unknown method is `unknown_method`;
    /// params that are not an object of the method's shape (including an
    /// invalid locator or selector) are `invalid_request`.
    pub fn parse(&self) -> Result<Call, ProviderError> {
        let Some(method) = Method::from_name(&self.method) else {
            return Err(ProviderError::new(
                ProviderErrorCode::UnknownMethod,
                format!("unknown method {:?}", self.method),
            ));
        };
        if !self.params.is_object() {
            return Err(ProviderError::invalid_request("params must be an object"));
        }
        fn p<T: serde::de::DeserializeOwned>(
            v: &Value,
            f: fn(T) -> Call,
        ) -> Result<Call, ProviderError> {
            serde_json::from_value::<T>(v.clone())
                .map(f)
                .map_err(|e| ProviderError::invalid_request(format!("invalid params: {e}")))
        }
        match method {
            Method::Hello => p(&self.params, Call::Hello),
            Method::Resolve => p(&self.params, Call::Resolve),
            Method::Units => p(&self.params, Call::Units),
            Method::Render => p(&self.params, Call::Render),
            Method::Batch => p(&self.params, Call::Batch),
        }
    }

    /// Check an inner request of a `batch` (section 13.3): as
    /// [`RawCall::parse`], except that `hello` and `batch` are
    /// `invalid_request` whatever their params.
    pub fn parse_inner(&self) -> Result<Call, ProviderError> {
        match Method::from_name(&self.method) {
            Some(m @ (Method::Hello | Method::Batch)) => Err(ProviderError::invalid_request(
                format!("{} is not allowed inside a batch", m.as_str()),
            )),
            _ => self.parse(),
        }
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// A request line: `{"id": <positive integer>, "method", "params"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub call: RawCall,
}

impl Request {
    pub fn new(id: u64, call: &Call) -> Self {
        Request {
            id,
            call: call.to_raw(),
        }
    }

    /// The request as one line, without the terminating newline.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("a request always serializes")
    }
}

/// Why a request line could not be accepted, for a provider's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestError {
    /// The request's id when the line was an object with a valid id, else
    /// `None` (the answer then carries `"id": null`).
    pub id: Option<u64>,
    pub error: ProviderError,
}

/// Read a request line as a provider does (section 13.2). A line that is
/// not a JSON object with a positive integer `id` and a string `method` is
/// `invalid_request` with no id. An unknown method or bad params carry the
/// request's id.
pub fn parse_request_line(line: &str) -> Result<(u64, Call), RequestError> {
    let no_id = |m: &str| RequestError {
        id: None,
        error: ProviderError::invalid_request(m),
    };
    let value: Value = serde_json::from_str(line).map_err(|e| no_id(&format!("not JSON: {e}")))?;
    let Value::Object(map) = value else {
        return Err(no_id("a request is a JSON object"));
    };
    let id = match map.get("id").and_then(Value::as_u64) {
        Some(id) if id > 0 => id,
        _ => return Err(no_id("a request has a positive integer id")),
    };
    let Some(method) = map.get("method").and_then(Value::as_str) else {
        return Err(RequestError {
            id: Some(id),
            error: ProviderError::invalid_request("a request has a string method"),
        });
    };
    let raw = RawCall {
        method: method.to_owned(),
        params: map.get("params").cloned().unwrap_or(Value::Null),
    };
    raw.parse()
        .map(|call| (id, call))
        .map_err(|error| RequestError {
            id: Some(id),
            error,
        })
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// A response line: exactly one of `result` and `error`. `id` is `null`
/// only in the answer to a line that could not be parsed as a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub id: Option<u64>,
    pub outcome: Result<Value, ProviderError>,
}

/// An inner batch response: `{"result": ...}` or `{"error": ...}`.
#[derive(Debug, Clone, PartialEq)]
pub struct InnerResponse(pub Result<Value, ProviderError>);

fn outcome_to_map(outcome: &Result<Value, ProviderError>, map: &mut Map<String, Value>) {
    match outcome {
        Ok(v) => {
            map.insert("result".into(), v.clone());
        }
        Err(e) => {
            map.insert(
                "error".into(),
                serde_json::to_value(e).expect("a provider error always serializes"),
            );
        }
    }
}

fn outcome_from_map<E: serde::de::Error>(
    map: &mut Map<String, Value>,
) -> Result<Result<Value, ProviderError>, E> {
    match (map.remove("result"), map.remove("error")) {
        (Some(r), None) => Ok(Ok(r)),
        (None, Some(e)) => serde_json::from_value::<ProviderError>(e)
            .map(Err)
            .map_err(|e| E::custom(format!("invalid error object: {e}"))),
        (Some(_), Some(_)) => Err(E::custom("a response has both result and error")),
        (None, None) => Err(E::custom("a response has neither result nor error")),
    }
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = Map::new();
        map.insert("id".into(), self.id.map_or(Value::Null, Value::from));
        outcome_to_map(&self.outcome, &mut map);
        map.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = Map::<String, Value>::deserialize(d)?;
        let id = match map.remove("id") {
            Some(Value::Null) => None,
            Some(v) => match v.as_u64() {
                Some(id) if id > 0 => Some(id),
                _ => {
                    return Err(D::Error::custom(
                        "a response id is a positive integer or null",
                    ));
                }
            },
            None => return Err(D::Error::custom("a response has an id")),
        };
        let outcome = outcome_from_map::<D::Error>(&mut map)?;
        // Section 13.2: a null id answers only a line that could not be
        // parsed as a request, and that answer is the error invalid_request.
        if id.is_none()
            && !matches!(&outcome, Err(e) if e.code == ProviderErrorCode::InvalidRequest)
        {
            return Err(D::Error::custom(
                "a response with a null id must carry the error invalid_request",
            ));
        }
        Ok(Response { id, outcome })
    }
}

impl JsonSchema for Response {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ProviderResponse".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let error = generator.subschema_for::<ProviderError>();
        schemars::json_schema!({
            "type": "object",
            "oneOf": [
                {"required": ["id", "result"], "properties": {"id": {"type": "integer", "minimum": 1}, "result": {"type": "object"}}, "not": {"required": ["error"]}},
                {"required": ["id", "error"], "properties": {"id": {"type": ["integer", "null"], "minimum": 1}, "error": error}, "not": {"required": ["result"]}}
            ]
        })
    }
}

impl Serialize for InnerResponse {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = Map::new();
        outcome_to_map(&self.0, &mut map);
        map.serialize(s)
    }
}

impl<'de> Deserialize<'de> for InnerResponse {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = Map::<String, Value>::deserialize(d)?;
        outcome_from_map::<D::Error>(&mut map).map(InnerResponse)
    }
}

impl JsonSchema for InnerResponse {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ProviderInnerResponse".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let error = generator.subschema_for::<ProviderError>();
        schemars::json_schema!({
            "type": "object",
            "oneOf": [
                {"required": ["result"], "properties": {"result": {"type": "object"}}, "not": {"required": ["error"]}},
                {"required": ["error"], "properties": {"error": error}, "not": {"required": ["result"]}}
            ]
        })
    }
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Response {
            id: Some(id),
            outcome: Ok(result),
        }
    }

    pub fn err(id: Option<u64>, error: ProviderError) -> Self {
        Response {
            id,
            outcome: Err(error),
        }
    }

    /// The response as one line, without the terminating newline.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("a response always serializes")
    }
}

/// The `hello` result is [`ProviderInfo`].
pub type HelloResult = ProviderInfo;

/// The `render` result is [`Rendered`].
pub type RenderResult = Rendered;

/// The `resolve` and `units` result: `{"units": [...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnitsResult {
    pub units: Vec<Unit>,
}

/// The `batch` result: one response per inner request, same order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BatchResult {
    pub responses: Vec<InnerResponse>,
}

/// Read a typed result out of a response value. A result of the wrong shape
/// is the provider's fault and is reported as `internal`; the caller maps it
/// to what its context requires.
pub fn read_result<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ProviderError> {
    serde_json::from_value(value).map_err(|e| {
        ProviderError::new(
            ProviderErrorCode::Internal,
            format!("malformed result: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_lines() {
        let call = Call::Resolve(ResolveParams {
            loc: Locator::new("archctl:model").unwrap(),
            sel: serde_json::from_value(json!({"key": "component/auth"})).unwrap(),
        });
        let line = Request::new(2, &call).to_line();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            v,
            json!({"id":2,"method":"resolve","params":{"loc":"archctl:model","sel":{"key":"component/auth"}}})
        );
        assert_eq!(parse_request_line(&line).unwrap(), (2, call));

        let hello = r#"{"id":1,"method":"hello","params":{"protocol":[1],"client":{"name":"hibi","version":"5.0.0"},"root":"/abs/path"},"extra":true}"#;
        assert!(matches!(
            parse_request_line(hello).unwrap(),
            (1, Call::Hello(_))
        ));

        let e = parse_request_line("not json").unwrap_err();
        assert_eq!(
            (e.id, e.error.code),
            (None, ProviderErrorCode::InvalidRequest)
        );
        let e = parse_request_line("[1]").unwrap_err();
        assert_eq!(e.id, None);
        let e = parse_request_line(r#"{"id":0,"method":"hello","params":{}}"#).unwrap_err();
        assert_eq!(e.id, None);
        let e = parse_request_line(r#"{"id":3,"method":"frobnicate","params":{}}"#).unwrap_err();
        assert_eq!(
            (e.id, e.error.code),
            (Some(3), ProviderErrorCode::UnknownMethod)
        );
        let e = parse_request_line(
            r#"{"id":4,"method":"resolve","params":{"loc":"x:y","sel":{"bogus":1}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            (e.id, e.error.code),
            (Some(4), ProviderErrorCode::InvalidRequest)
        );
        let e = parse_request_line(r#"{"id":5,"method":"units"}"#).unwrap_err();
        assert_eq!(
            (e.id, e.error.code),
            (Some(5), ProviderErrorCode::InvalidRequest)
        );
    }

    #[test]
    fn batch_round_trip() {
        let line = r#"{"id":5,"method":"batch","params":{"requests":[{"method":"resolve","params":{"loc":"archctl:model","sel":{"key":"a"}}},{"method":"hello","params":{}},{"method":"hello","params":{"protocol":[1],"client":{"name":"c","version":"0"},"root":"/r"}},{"method":"batch","params":{"requests":[]}},{"method":"frobnicate","params":{}}]}}"#;
        let (_, call) = parse_request_line(line).unwrap();
        let Call::Batch(b) = call else { panic!() };
        let codes: Vec<Option<ProviderErrorCode>> = b
            .requests
            .iter()
            .map(|r| r.parse_inner().err().map(|e| e.code))
            .collect();
        assert_eq!(
            codes,
            [
                None,
                Some(ProviderErrorCode::InvalidRequest),
                Some(ProviderErrorCode::InvalidRequest),
                Some(ProviderErrorCode::InvalidRequest),
                Some(ProviderErrorCode::UnknownMethod),
            ]
        );
        assert!(matches!(
            b.requests[0].parse_inner().unwrap(),
            Call::Resolve(_)
        ));
        // At the top level the same hello and batch are valid calls.
        assert!(matches!(b.requests[2].parse().unwrap(), Call::Hello(_)));
        assert!(matches!(b.requests[3].parse().unwrap(), Call::Batch(_)));

        let resp = r#"{"id":5,"result":{"responses":[{"result":{"units":[]}},{"error":{"code":"invalid_selector","message":"..."}}]}}"#;
        let r: Response = serde_json::from_str(resp).unwrap();
        assert_eq!(r.id, Some(5));
        let b: BatchResult = read_result(r.outcome.clone().unwrap()).unwrap();
        assert_eq!(b.responses.len(), 2);
        let units: UnitsResult = read_result(b.responses[0].0.clone().unwrap()).unwrap();
        assert!(units.units.is_empty());
        assert_eq!(
            b.responses[1].0.as_ref().unwrap_err().code,
            ProviderErrorCode::InvalidSelector
        );
        let again: Value = serde_json::from_str(&r.to_line()).unwrap();
        assert_eq!(again, serde_json::from_str::<Value>(resp).unwrap());
    }

    #[test]
    fn response_envelope() {
        let r: Response =
            serde_json::from_str(r#"{"id":null,"error":{"code":"invalid_request","message":"m"}}"#)
                .unwrap();
        assert_eq!(r.id, None);
        assert!(
            serde_json::from_str::<Response>(
                r#"{"id":1,"result":{},"error":{"code":"internal","message":""}}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Response>(r#"{"id":1}"#).is_err());
        // Section 13.2: the id is a positive integer; null only with
        // invalid_request.
        for bad in [
            r#"{"id":0,"result":{}}"#,
            r#"{"id":-1,"result":{}}"#,
            r#"{"id":1.5,"result":{}}"#,
            r#"{"id":"1","result":{}}"#,
            r#"{"id":null,"result":{}}"#,
            r#"{"id":null,"error":{"code":"not_found","message":"m"}}"#,
        ] {
            assert!(serde_json::from_str::<Response>(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<Response>(r#"{"result":{}}"#).is_err());
        let r = Response::err(None, ProviderError::invalid_request("bad"));
        assert_eq!(
            r.to_line(),
            r#"{"error":{"code":"invalid_request","message":"bad"},"id":null}"#
        );
        let hello = r#"{"protocol":1,"provider":{"name":"archctl","version":"0.4.0"},"schemes":["archctl"],"selectors":["key"],"kinds":["component","decision"],"capabilities":{"units":true,"render":true,"batch":true}}"#;
        let info: HelloResult = read_result(serde_json::from_str(hello).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::from_str::<Value>(hello).unwrap()
        );
    }
}
