use crate::engine::ZenEvaluateOptions;
use crate::mt::spawn_worker;
use crate::safe_result::SafeResult;
use napi::anyhow::anyhow;
use napi_derive::napi;
use serde_json::Value;
use std::sync::Arc;
use zen_engine::{Decision, EvaluationOptions, EvaluationSerializedOptions, EvaluationTraceKind};

#[napi]
pub struct ZenDecision(pub(crate) Arc<Decision>);

impl From<Decision> for ZenDecision {
    fn from(value: Decision) -> Self {
        Self(value.into())
    }
}

#[napi]
impl ZenDecision {
    #[napi(constructor)]
    pub fn new() -> napi::Result<Self> {
        Err(anyhow!("Private constructor").into())
    }

    #[napi(ts_return_type = "Promise<ZenEngineResponse>")]
    pub async fn evaluate(
        &self,
        context: Value,
        opts: Option<ZenEvaluateOptions>,
        execution_context: Option<Value>,
    ) -> napi::Result<Value> {
        let decision = self.0.clone();
        let result = spawn_worker(move || {
            let opts = opts.unwrap_or_default();
            let serialized: EvaluationSerializedOptions = opts.into();
            let trace = serialized.trace;
            let options = EvaluationOptions {
                trace: trace != EvaluationTraceKind::None,
                max_depth: serialized.max_depth,
            };

            async move {
                let context = zen_engine::Variable::try_from_value(context).map_err(
                    |e| serde_json::json!({ "type": "ContextError", "source": e.to_string() }),
                )?;
                let execution_context = execution_context
                    .map(zen_engine::Variable::try_from_value)
                    .transpose()
                    .map_err(|e| {
                        serde_json::json!({
                            "type": "ContextError",
                            "source": format!("executionContext: {}", e)
                        })
                    })?;
                decision
                    .evaluate_with_request(context, options, execution_context)
                    .await
                    .map(|r| {
                        r.serialize_with_mode(serde_json::value::Serializer, trace)
                            .unwrap_or_default()
                    })
                    .map_err(|e| {
                        e.serialize_with_mode(serde_json::value::Serializer, trace)
                            .unwrap_or_default()
                    })
            }
        })
        .await
        .map_err(|_| anyhow!("Hook timed out"))?
        .map_err(|e| anyhow!(e))?;

        Ok(result)
    }

    #[napi(
        ts_return_type = "Promise<{ success: true, data: ZenEngineResponse } | { success: false; error: any; }>"
    )]
    pub async fn safe_evaluate(
        &self,
        context: Value,
        opts: Option<ZenEvaluateOptions>,
        execution_context: Option<Value>,
    ) -> SafeResult<Value> {
        self.evaluate(context, opts, execution_context).await.into()
    }

    #[napi]
    pub fn validate(&self) -> napi::Result<()> {
        let decision = self.0.clone();
        let result = decision
            .validate()
            .map_err(|e| anyhow!(serde_json::to_string(&e).unwrap_or_else(|_| e.to_string())))?;

        Ok(result)
    }
}
