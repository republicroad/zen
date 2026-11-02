use crate::decision_graph::graph::{DecisionGraph, DecisionGraphConfig, DecisionGraphResponse};
use crate::engine::{EvaluationOptions, EvaluationSerializedOptions, EvaluationTraceKind};
use crate::loader::{DynamicLoader, NoopLoader};
use crate::model::GraphContent;
use crate::nodes::custom::{DynamicCustomNode, NoopCustomNode};
use crate::nodes::function::http_handler::DynamicHttpHandler;
use crate::nodes::NodeHandlerExtensions;
use crate::{DecisionGraphValidationError, EvaluationError};
use serde_json::Value;
use std::cell::OnceCell;
use std::sync::Arc;
use zen_expression::variable::Variable;

/// Represents a JDM decision which can be evaluated
#[derive(Debug, Clone)]
pub struct Decision {
    content: Arc<GraphContent>,
    loader: DynamicLoader,
    adapter: DynamicCustomNode,
    http_handler: DynamicHttpHandler,
}

impl From<GraphContent> for Decision {
    fn from(value: GraphContent) -> Self {
        Self {
            content: value.into(),
            loader: Arc::new(NoopLoader::default()),
            adapter: Arc::new(NoopCustomNode::default()),
            http_handler: None,
        }
    }
}

impl From<Arc<GraphContent>> for Decision {
    fn from(value: Arc<GraphContent>) -> Self {
        Self {
            content: value,
            loader: Arc::new(NoopLoader::default()),
            adapter: Arc::new(NoopCustomNode::default()),
            http_handler: None,
        }
    }
}

impl Decision {
    pub fn with_loader(mut self, loader: DynamicLoader) -> Self {
        self.loader = loader;
        self
    }

    pub fn with_adapter(mut self, adapter: DynamicCustomNode) -> Self {
        self.adapter = adapter;
        self
    }

    pub fn with_http_handler(mut self, http_handler: DynamicHttpHandler) -> Self {
        self.http_handler = http_handler;
        self
    }

    /// Evaluates a decision using an in-memory reference stored in struct
    pub async fn evaluate(
        &self,
        context: Variable,
    ) -> Result<DecisionGraphResponse, Box<EvaluationError>> {
        self.evaluate_with_opts(context, Default::default()).await
    }

    /// Evaluates a decision using in-memory reference with advanced options
    pub async fn evaluate_with_opts(
        &self,
        context: Variable,
        options: EvaluationOptions,
    ) -> Result<DecisionGraphResponse, Box<EvaluationError>> {
        let mut decision_graph = DecisionGraph::try_new(DecisionGraphConfig {
            content: self.content.clone(),
            max_depth: options.max_depth,
            trace: options.trace,
            iteration: 0,
            execution_context: None,
            extensions: NodeHandlerExtensions {
                loader: self.loader.clone(),
                custom_node: self.adapter.clone(),
                http_handler: self.http_handler.clone(),
                compiled_cache: self.content.compiled_cache.clone(),
                dt_indexes: self.content.dt_indexes.clone(),
                stripped_functions: self.content.stripped_functions.clone(),
                validator_cache: Arc::new(OnceCell::from(self.content.validator_cache.clone())),
                ..Default::default()
            },
        })?;

        let response = decision_graph.evaluate(context).await?;

        Ok(response)
    }

    /// Evaluates with an explicit evaluation request (input + options + execution context).
    /// Execution context is injected into every custom node handler request and stays
    /// invisible to graph expressions.
    pub async fn evaluate_with_request(
        &self,
        input: Variable,
        options: EvaluationOptions,
        execution_context: Option<Variable>,
    ) -> Result<DecisionGraphResponse, Box<EvaluationError>> {
        let mut decision_graph = DecisionGraph::try_new(DecisionGraphConfig {
            content: self.content.clone(),
            max_depth: options.max_depth,
            trace: options.trace,
            iteration: 0,
            execution_context,
            extensions: NodeHandlerExtensions {
                loader: self.loader.clone(),
                custom_node: self.adapter.clone(),
                http_handler: self.http_handler.clone(),
                compiled_cache: self.content.compiled_cache.clone(),
                dt_indexes: self.content.dt_indexes.clone(),
                stripped_functions: self.content.stripped_functions.clone(),
                validator_cache: Arc::new(OnceCell::from(self.content.validator_cache.clone())),
                ..Default::default()
            },
        })?;

        decision_graph.evaluate(input).await
    }

    pub async fn evaluate_serialized(
        &self,
        context: Variable,
        options: EvaluationSerializedOptions,
    ) -> Result<Value, Value> {
        let response = self
            .evaluate_with_opts(
                context,
                EvaluationOptions {
                    trace: options.trace != EvaluationTraceKind::None,
                    max_depth: options.max_depth,
                },
            )
            .await;

        match response {
            Ok(ok) => Ok(ok
                .serialize_with_mode(serde_json::value::Serializer, options.trace)
                .unwrap_or_default()),
            Err(err) => Err(err
                .serialize_with_mode(serde_json::value::Serializer, options.trace)
                .unwrap_or_default()),
        }
    }

    pub fn validate(&self) -> Result<(), DecisionGraphValidationError> {
        let decision_graph = DecisionGraph::try_new(DecisionGraphConfig {
            content: self.content.clone(),
            max_depth: 1,
            trace: false,
            iteration: 0,
            execution_context: None,
            extensions: Default::default(),
        })?;

        decision_graph.validate()
    }

    pub fn compile(&mut self) -> () {
        let cm = Arc::make_mut(&mut self.content);
        cm.compile();
    }
}

#[cfg(test)]
mod execution_context_tests {
    //! Execution context propagation guard: custom node handlers must receive the
    //! execution_context passed to evaluate; host-owned and single-source per evaluation.
    use super::*;
    use crate::nodes::custom::{CustomNodeAdapter, CustomNodeRequest};
    use crate::nodes::{NodeError, NodeResult};
    use serde_json::{json, Value};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    /// Variable is !Send (Lrc/RefCell) — the capture side stores serde JSON values (Send)
    #[derive(Debug)]
    struct CapturingAdapter {
        seen: Arc<Mutex<Vec<Option<Value>>>>,
    }

    impl CustomNodeAdapter for CapturingAdapter {
        fn handle(
            &self,
            request: CustomNodeRequest,
        ) -> Pin<Box<dyn Future<Output = NodeResult> + '_>> {
            let seen = self.seen.clone();
            Box::pin(async move {
                seen.lock()
                    .unwrap()
                    .push(request.execution_context.map(|v| v.to_value()));
                // Stop evaluation with an error (Noop semantics); assertions focus on captured state
                Err(NodeError {
                    node_id: request.node.id.clone(),
                    trace: None,
                    source: "capture-done".to_string().into(),
                })
            })
        }
    }

    fn graph_content() -> GraphContent {
        serde_json::from_value(json!({
            "nodes": [
                { "id": "in-1", "type": "inputNode", "name": "input", "position": { "x": 0, "y": 0 } },
                { "id": "c-1", "type": "customNode", "name": "handler", "position": { "x": 0, "y": 0 },
                  "content": { "kind": "UDF", "config": {} } },
                { "id": "out-1", "type": "outputNode", "name": "output", "position": { "x": 0, "y": 0 } }
            ],
            "edges": [
                { "id": "e1", "sourceId": "in-1", "targetId": "c-1" },
                { "id": "e2", "sourceId": "c-1", "targetId": "out-1" }
            ]
        }))
        .expect("valid graph")
    }

    #[tokio::test]
    async fn execution_context_flows_to_custom_node_handler() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let decision = Decision::from(graph_content())
            .with_adapter(Arc::new(CapturingAdapter { seen: seen.clone() }));

        let exec_ctx = json!({ "tenantId": "t-1", "traceId": "tr-9" });
        let outcome = decision
            .evaluate_with_request(
                json!({ "ip": "192.0.2.1" }).into(),
                EvaluationOptions::default(),
                Some(exec_ctx.clone().into()),
            )
            .await;

        // The capturing adapter ends evaluation with an error; assertions focus on captured state
        assert!(outcome.is_err());
        let captured = seen.lock().unwrap();
        assert_eq!(captured.len(), 1, "handler must be called exactly once");
        let seen_ctx = captured[0].as_ref().expect("execution_context is Some");
        assert_eq!(
            seen_ctx["tenantId"], json!("t-1"),
            "must reach the handler unchanged"
        );
        assert_eq!(seen_ctx["traceId"], json!("tr-9"));
    }

    #[tokio::test]
    async fn execution_context_defaults_to_none() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let decision = Decision::from(graph_content())
            .with_adapter(Arc::new(CapturingAdapter { seen: seen.clone() }));

        let _ = decision
            .evaluate_with_opts(json!({}).into(), EvaluationOptions::default())
            .await;

        let captured = seen.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert!(
            captured[0].is_none(),
            "handler receives None when no execution context is provided"
        );
    }
}
