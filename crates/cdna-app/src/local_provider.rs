//! Optional loopback llama.cpp transport. This does not install or launch a model.
use crate::natural_language::{
    AdapterOutput, ExtractionAdapter, ExtractionCall, EXTRACTION_SYSTEM_PROMPT,
    extraction_json_schema,
};
use cdna_domain::{DomainError, ErrorCode, Result};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc, time::{Duration, Instant}};
use tokio::sync::Semaphore;

pub struct LocalLlama {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    permits: Arc<Semaphore>,
}

fn error(code: ErrorCode, message: &str) -> DomainError {
    DomainError { code, message: message.into() }
}

impl LocalLlama {
    /// Host configuration only. Numeric loopback addresses avoid DNS rebinding;
    /// redirects, environment proxies and automatic retries are all disabled.
    pub fn new(address: SocketAddr, model: String) -> Result<Self> {
        if !address.ip().is_loopback() || address.port() == 0
            || model.trim().is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
            return Err(error(ErrorCode::InputInvalid, "invalid local provider configuration"));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(1)
            .no_gzip().no_brotli().no_deflate().no_zstd()
            .build().map_err(|_|error(ErrorCode::ProviderUnavailable, "local provider unavailable"))?;
        Ok(Self { client, endpoint: format!("http://{address}/v1/chat/completions"), model, permits: Arc::new(Semaphore::new(1)) })
    }
}

impl ExtractionAdapter for LocalLlama {
    async fn extract(&self, call: ExtractionCall<'_>) -> Result<AdapterOutput> {
        if call.text.is_empty() || call.text.len() > crate::natural_language::MAX_TEXT_BYTES
            || call.policy.max_output_bytes == 0 || call.policy.max_output_bytes > crate::natural_language::MAX_OUTPUT_BYTES
            || call.policy.max_output_tokens == 0 || call.policy.max_output_tokens > 8192 {
            return Err(error(ErrorCode::BudgetExceeded, "invalid local extraction budget"));
        }
        if call.policy.cloud || call.policy.model != self.model {
            return Err(error(ErrorCode::ScopeDenied, "local provider scope mismatch"));
        }
        let _permit = self.permits.try_acquire().map_err(|_|error(ErrorCode::BudgetExceeded, "local provider busy"))?;
        let remaining = call.policy.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(error(ErrorCode::Cancelled, "extraction deadline elapsed"));
        }
        let body = json!({
            "model":self.model,"stream":false,"n":1,"temperature":0,
            "max_tokens":call.policy.max_output_tokens,
            "chat_template_kwargs":{"enable_thinking":false},
            "response_format":{"type":"json_schema","json_schema":{"name":"extraction","strict":true,"schema":extraction_json_schema()}},
            "messages":[{"role":"system","content":EXTRACTION_SYSTEM_PROMPT},{"role":"user","content":call.text}]
        });
        let mut response = self.client.post(&self.endpoint).timeout(remaining).json(&body).send().await
            .map_err(|_|error(ErrorCode::ProviderUnavailable, "local extraction request failed"))?;
        if !response.status().is_success() {
            return Err(error(ErrorCode::ProviderUnavailable, "local extraction request rejected"));
        }
        // JSON escaping can expand each content byte up to six bytes. Bound the
        // protocol envelope separately before extracting its bounded content.
        let envelope_limit = call.policy.max_output_bytes.saturating_mul(6).saturating_add(16*1024);
        if response.content_length().is_some_and(|n|n > envelope_limit as u64) {
            return Err(error(ErrorCode::InputInvalid, "local extraction response too large"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_|error(ErrorCode::ProviderUnavailable, "local extraction response interrupted"))? {
            if bytes.len().saturating_add(chunk.len()) > envelope_limit {
                return Err(error(ErrorCode::InputInvalid, "local extraction response too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope = cdna_domain::parse_json_with_limit(&bytes, envelope_limit)?;
        let choices = envelope["choices"].as_array().filter(|rows|rows.len()==1)
            .ok_or_else(||error(ErrorCode::InputInvalid, "invalid local extraction response"))?;
        let choice = &choices[0];
        if choice["finish_reason"] != "stop" || choice["message"]["role"] != "assistant"
            || !choice["message"]["tool_calls"].is_null()
            || !choice["message"]["refusal"].is_null() {
            return Err(error(ErrorCode::InputInvalid, "local extraction incomplete"));
        }
        let content = choice["message"]["content"].as_str()
            .filter(|s|s.len() <= call.policy.max_output_bytes)
            .ok_or_else(||error(ErrorCode::InputInvalid, "invalid local extraction content"))?;
        let tokens = envelope["usage"]["completion_tokens"].as_u64()
            .filter(|n|*n <= u64::from(call.policy.max_output_tokens))
            .ok_or_else(||error(ErrorCode::BudgetExceeded, "local output token accounting unavailable"))?;
        Ok(AdapterOutput { json:content.as_bytes().to_vec(), output_tokens:Some(tokens as u32), cost_micros:Some(0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natural_language::{ExtractionPolicy, TrustedHost, extract};
    use axum::{Router, routing::post, response::IntoResponse};
    use cdna_domain::{Domain,Mode};
    use tokio::sync::watch;
    use uuid::Uuid;

    fn policy() -> ExtractionPolicy {
        ExtractionPolicy { model:"synthetic-fixture".into(),allowed_models:vec!["synthetic-fixture".into()],cloud:false,cloud_consent:false,max_output_tokens:768,max_output_bytes:4096,reserved_cost_micros:None,max_cost_micros:0,deadline:Instant::now()+Duration::from_secs(2) }
    }
    fn host() -> TrustedHost {
        TrustedHost { workspace_id:Uuid::new_v4(),request_id:Uuid::new_v4(),as_of:chrono::Utc::now(),mode:Mode::Imitate,domain:Domain::ProductDelivery,can_extract:true,can_rank:true }
    }
    async fn server(router: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=listener.local_addr().unwrap();
        let task=tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
        (address,task)
    }
    #[test]
    fn numeric_loopback_only_and_fixed_host_model() {
        assert!(LocalLlama::new("192.0.2.1:8080".parse().unwrap(),"model".into()).is_err());
        assert!(LocalLlama::new("127.0.0.1:0".parse().unwrap(),"model".into()).is_err());
        assert!(LocalLlama::new("[::1]:8080".parse().unwrap(),"model".into()).is_ok());
    }
    #[tokio::test]
    async fn real_http_transport_preserves_untrusted_interpretation() {
        let router=Router::new().route("/v1/chat/completions",post(|axum::Json(body):axum::Json<serde_json::Value>|async move {
            assert_eq!(body["model"],"synthetic-fixture");
            assert_eq!(body["messages"][1]["content"],"納期を優先する");
            assert_eq!(body["stream"],false);
            assert_eq!(body["response_format"]["json_schema"]["name"],"extraction");
            assert!(body["tools"].is_null());
            let proposal=json!({"facts":[{"key":"deadline_pressure","value":1,"unit":"ratio","source_span":null}],"unknown_fields":[],"candidates":[{"id":"a","text":"外注","attributes":{}},{"id":"b","text":"内製","attributes":{}}]});
            axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":proposal.to_string()}}],"usage":{"completion_tokens":90}}))
        }));
        let (address,task)=server(router).await;
        let adapter=LocalLlama::new(address,"synthetic-fixture".into()).unwrap();
        let (_sender,cancel)=watch::channel(false);
        let result=extract(&adapter,"納期を優先する",&host(),&policy(),cancel).await.unwrap();
        assert!(!result.semantic_accuracy_verified);
        assert!(!result.fact_provenance[0].interpretation_verified);
        assert_eq!(result.cost_micros,Some(0));
        task.abort();let _=task.await;
    }
    #[tokio::test]
    async fn rejects_redirect_oversize_and_truncated_completion() {
        for (status,body) in [
            (302,"redirect".to_owned()),
            (200,"x".repeat(50_000)),
            (200,json!({"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"{}"}}],"usage":{"completion_tokens":768}}).to_string()),
        ] {
            let router=Router::new().route("/v1/chat/completions",post(move || {let body=body.clone();async move{(axum::http::StatusCode::from_u16(status).unwrap(),[("Location","http://192.0.2.1/")],body).into_response()}}));
            let (address,task)=server(router).await;
            let adapter=LocalLlama::new(address,"synthetic-fixture".into()).unwrap();
            let p=policy();
            assert!(adapter.extract(ExtractionCall{text:"test",policy:&p}).await.is_err());
            task.abort();let _=task.await;
        }
    }
    #[tokio::test]
    async fn cancellation_drops_active_transport_and_releases_capacity() {
        let router=Router::new().route("/v1/chat/completions",post(||async {tokio::time::sleep(Duration::from_secs(30)).await; "{}"}));
        let (address,task)=server(router).await;
        let adapter=LocalLlama::new(address,"synthetic-fixture".into()).unwrap();
        let (sender,cancel)=watch::channel(false);
        let p=policy();let h=host();
        let operation=extract(&adapter,"test",&h,&p,cancel);
        let signal=async {tokio::time::sleep(Duration::from_millis(20)).await;sender.send(true).unwrap();};
        let (result,_)=tokio::join!(operation,signal);
        assert!(result.is_err());
        assert_eq!(adapter.permits.available_permits(),1);
        task.abort();let _=task.await;
    }
}
