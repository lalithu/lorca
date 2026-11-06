//! One model library, with thin Ollama and llama.cpp management adapters.
//! Network discovery uses only the Hugging Face Hub; provider APIs own weights.
use super::*;
use futures_util::{StreamExt, future::join_all};
use serde_json::Value;

#[derive(Clone)]
pub(super) struct DownloadState {
    pub model: String,
    pub host: String,
    pub provider: provider::Engine,
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub bytes_per_second: f64,
    pub error: Option<String>,
    pub updated: Instant,
    pub previous_bytes: u64,
    pub cancel: watch::Sender<bool>,
}
impl DownloadState {
    fn view(&self) -> Value {
        json!({
            "model":self.model, "provider":if self.provider == provider::Engine::Ollama {"ollama"} else {"llama.cpp"},
            "status":self.status, "completed":self.completed,"total":self.total,
            "bytesPerSecond":self.bytes_per_second,"error":self.error
        })
    }
    fn mark(&mut self, completed: u64, total: u64) {
        let dt = self.updated.elapsed().as_secs_f64();
        if dt >= 0.35 {
            let delta = completed.saturating_sub(self.previous_bytes);
            let rate = delta as f64 / dt;
            self.bytes_per_second = if self.bytes_per_second == 0.0 {rate}
                else {0.35*rate + 0.65*self.bytes_per_second};
            self.previous_bytes = completed;
            self.updated = Instant::now();
        }
        self.completed = completed;
        self.total = total;
    }
}
fn http_error(status: reqwest::StatusCode, text: &str) -> String {
    format!("Model runtime returned HTTP {}: {}", status.as_u16(),
        text.chars().take(280).collect::<String>())
}
fn validate_model(model: &str) -> Result<()> {
    if !crate::model_library::valid_runtime_model(model) {
        return Err("Invalid model identifier".into());
    }
    Ok(())
}
impl Core {
    pub(super) async fn model_library(&self, session: &str) -> Result<Value> {
        let host = self.host(session)?;
        let engine = self.engine(session)?;
        let url = if engine == provider::Engine::Ollama {
            format!("{host}/api/tags")
        } else {
            format!("{host}/models")
        };
        let r = self.client.get(url).timeout(Duration::from_secs(8)).send()
            .await.map_err(|e|format!("Cannot list local models: {e}"))?
            .error_for_status().map_err(|e|e.to_string())?;
        let value: Value = r.json().await.map_err(|e|e.to_string())?;
        let installed = if engine == provider::Engine::Ollama {
            let ps: Value = self.client.get(format!("{host}/api/ps"))
                .timeout(Duration::from_secs(3)).send().await.ok()
                .and_then(|_|None).unwrap_or(Value::Null);
            // A fresh /api/ps request is made below; failure is nonfatal.
            let loaded: Value = match self.client.get(format!("{host}/api/ps"))
                .timeout(Duration::from_secs(3)).send().await {
                    Ok(r) => r.json().await.unwrap_or_default(), Err(_) => ps,
                };
            let mut models = Vec::new();
            for m in value["models"].as_array().into_iter().flatten() {
                let Some(name) = m["name"].as_str().or_else(||m["model"].as_str()) else {continue};
                let running = loaded["models"].as_array().into_iter().flatten()
                    .find(|entry|entry["name"].as_str()
                        .or_else(||entry["model"].as_str())
                        .is_some_and(|id|provider::same_model(id,name)));
                models.push(json!({
                    "id":name,"name":name,"provider":"ollama",
                    "sizeBytes":m["size"],"parameters":m["details"]["parameter_size"],
                    "quant":m["details"]["quantization_level"],
                    "vision":Value::Null,"status":if running.is_some() {"loaded"} else {"installed"},
                    "allocatedContext":running.and_then(|item|item["context_length"].as_u64()),
                    "loadedMemoryBytes":running.and_then(|item|item["size"].as_u64()),
                    "canDelete":true
                }));
            }
            models
        } else {
            let mut models = Vec::new();
            for m in value["data"].as_array().into_iter().flatten() {
                let Some(id) = m["id"].as_str() else {continue};
                let status=m["status"]["value"].as_str().unwrap_or("installed");
                let memory = &m["memory"]["effective"];
                models.push(json!({
                    "id":id,"name":id,"provider":"llama.cpp",
                    "sizeBytes":m["size"],"parameters":m["meta"]["n_params"],
                    "quant":m["meta"]["ftype"],
                    "vision":m["architecture"]["input_modalities"].as_array()
                        .map(|xs|xs.iter().any(|v|v == "image")),
                    "status":status,
                    "allocatedContext":memory["n_ctx"].as_u64()
                        .or_else(||m["meta"]["n_ctx"].as_u64()),
                    "loadedMemoryBytes":memory["devices"].as_array()
                        .map(|xs|xs.iter().map(|x| {
                            x["model"].as_u64().unwrap_or(0)
                            + x["context"].as_u64().unwrap_or(0)
                            + x["compute"].as_u64().unwrap_or(0)
                        }).sum::<u64>()),
                    "canDelete":!matches!(status,"loaded"|"loading"|"sleeping"),
                }));
            }
            models
        };
        let metal = self.downloads.lock().unwrap().values()
            .filter(|d|d.host == host && d.provider == engine)
            .map(DownloadState::view).collect::<Vec<_>>();
        Ok(json!({"provider":if engine == provider::Engine::Ollama {"ollama"}else{"llama.cpp"},
            "installed":installed,"downloads":metal,
            "hardware":crate::hardware::HardwareSnapshot::detect(&self.repository.root)}))
    }

    /// Search public Hugging Face GGUF repositories, without downloading.
    pub(super) async fn model_catalog(&self, session: &str, query: &str,
        metal_budget: Option<u64>) -> Result<Value> {
        self.touch(session)?;
        if query.len()>100 || query.chars().any(char::is_control) {
            return Err("Search must be 100 characters or fewer".into());
        }
        let mut url = reqwest::Url::parse("https://huggingface.co/api/models")
            .map_err(|e|e.to_string())?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("filter","gguf").append_pair("sort","downloads")
                .append_pair("direction","-1").append_pair("limit","20");
            if !query.trim().is_empty() {q.append_pair("search",query.trim());}
        }
        let response=self.client.get(url).timeout(Duration::from_secs(12)).send().await
            .map_err(|e|format!("Hugging Face catalog unavailable: {e}"))?;
        let status=response.status();
        if !status.is_success() {return Err(http_error(status,&response.text().await.unwrap_or_default()));}
        let results:Value=response.json().await.map_err(|e|e.to_string())?;
        let items=results.as_array().ok_or("Unexpected Hugging Face response")?;
        let physical=crate::hardware::HardwareSnapshot::detect(&self.repository.root)
            .physical_memory_bytes.unwrap_or(0);
        let budget=crate::model_library::model_memory_budget(physical, metal_budget);
        // Fetch only the leading 10 repository manifests concurrently. Their
        // GGUF siblings provide actual quantizations and size; other results
        // remain browsable without claiming a compatibility recommendation.
        let detail_futures=items.iter().take(10).map(|summary|async move {
            let repo=summary["id"].as_str().unwrap_or("");
            let detail=self.hf_model_details(repo).await.unwrap_or(Value::Null);
            crate::model_library::candidate(summary,&detail,budget)
        });
        let mut output=join_all(detail_futures).await;
        output.extend(items.iter().skip(10).map(crate::model_library::summary_candidate));
        let recommended=output.iter().filter(|m|m["estimatedFit"]==true)
            .cloned().collect::<Vec<_>>();
        Ok(json!({"source":"Hugging Face","items":output,
            "recommended":recommended,"memoryBudgetBytes":budget,
            "fitNote":"Weight and rough KV/compute headroom estimate, not a verified context fit. No models were downloaded."}))
    }

    async fn hf_model_details(&self, repo: &str) -> Result<Value> {
        if !crate::model_library::valid_repo(repo) {return Err("Invalid Hugging Face repository".into());}
        let mut url=reqwest::Url::parse("https://huggingface.co/api/models/")
            .map_err(|e|e.to_string())?;
        url.path_segments_mut().map_err(|_|"Invalid Hugging Face URL")?
            .pop_if_empty().extend(repo.split('/'));
        url.query_pairs_mut().append_pair("blobs","true");
        let response=self.client.get(url).timeout(Duration::from_secs(10)).send()
            .await.map_err(|e|e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Hugging Face repository not found (HTTP {})",response.status()));
        }
        response.json().await.map_err(|e|e.to_string())
    }
    pub(super) async fn model_catalog_details(&self, session: &str,
        repo: &str,metal_budget: Option<u64>) -> Result<Value> {
        self.touch(session)?;
        let details=self.hf_model_details(repo).await?;
        let physical=crate::hardware::HardwareSnapshot::detect(&self.repository.root)
            .physical_memory_bytes.unwrap_or(0);
        let budget=crate::model_library::model_memory_budget(physical,metal_budget);
        Ok(crate::model_library::candidate(&details,&details,budget))
    }

    fn transfer_key(host:&str,model:&str)->String {format!("{host}|{model}")}
    fn update_transfer(&self, host:&str,model:&str,
        progress: Option<(u64,u64)>, status: Option<&str>,error: Option<String>) {
        if let Some(d)=self.downloads.lock().unwrap().get_mut(&Self::transfer_key(host,model)) {
            if let Some((done,total))=progress {d.mark(done,total);}
            if let Some(status)=status {d.status=status.to_owned();}
            if error.is_some() {d.error=error;}
        }
    }
    pub(super) fn model_transfers(&self,session:&str)->Result<Value> {
        let host=self.host(session)?;
        let items=self.downloads.lock().unwrap().values()
            .filter(|d|d.host==host).map(DownloadState::view).collect::<Vec<_>>();
        Ok(json!({"downloads":items}))
    }
    pub(super) async fn model_install(self:&Arc<Self>,session:&str,
        model:&str)->Result<Value> {
        validate_model(model)?;
        let host=self.host(session)?;
        let engine=self.engine(session)?;
        let key=Self::transfer_key(&host,model);
        let (tx,rx)=watch::channel(false);
        {
            let mut downloads=self.downloads.lock().unwrap();
            if downloads.get(&key).is_some_and(|d|
                matches!(d.status.as_str(),"starting"|"downloading")) {
                return Err("This model is already downloading".into());
            }
            downloads.insert(key,DownloadState{
                model:model.into(),host:host.clone(),provider:engine,
                status:"starting".into(),completed:0,total:0,bytes_per_second:0.0,
                error:None,updated:Instant::now(),previous_bytes:0,cancel:tx,
            });
        }
        let core=self.clone();
        let model=model.to_owned();
        tokio::spawn(async move {
            let result=if engine==provider::Engine::Ollama {
                core.download_ollama(&host,&model,rx).await
            }else{
                core.download_llama(&host,&model,rx).await
            };
            match result {
                Ok(())=>core.update_transfer(&host,&model,None,Some("complete"),None),
                Err(e)=>{
                    let cancelled=e=="Cancelled";
                    core.update_transfer(&host,&model,None,
                        Some(if cancelled{"paused"}else{"failed"}),
                        if cancelled{None}else{Some(e)});
                }
            }
        });
        Ok(json!({"accepted":true,"model":model,
            "provider":if engine==provider::Engine::Ollama{"ollama"}else{"llama.cpp"}}))
    }
    async fn download_ollama(&self,host:&str,model:&str,
        mut cancel:watch::Receiver<bool>)->Result<()> {
        let response=self.client.post(format!("{host}/api/pull"))
            .json(&json!({"model":model,"stream":true}))
            .timeout(Duration::from_secs(60*60)).send().await
            .map_err(|e|format!("Ollama download request failed: {e}"))?;
        let status=response.status();
        if !status.is_success() {
            return Err(http_error(status,&response.text().await.unwrap_or_default()));
        }
        self.update_transfer(host,model,None,Some("downloading"),None);
        let mut stream=response.bytes_stream();
        let mut pending=Vec::<u8>::new();
        let mut completed=false;
        loop {
            let next=tokio::select! {
                _=cancel.changed()=>return Err("Cancelled".into()),
                next=stream.next()=>next,
            };
            let Some(next)=next else {break};
            let bytes=next.map_err(|e|format!("Ollama download interrupted: {e}"))?;
            pending.extend_from_slice(&bytes);
            if pending.len()>256*1024 {return Err("Unexpected oversized Ollama progress response".into());}
            while let Some(at)=pending.iter().position(|b|*b==b'\n') {
                let line:Vec<u8>=pending.drain(..=at).collect();
                if let Ok(value)=serde_json::from_slice::<Value>(&line) {
                    if let Some(e)=value["error"].as_str(){return Err(e.to_owned());}
                    if value["status"]=="success" {completed=true;break;}
                    if let (Some(done),Some(total))=(value["completed"].as_u64(),value["total"].as_u64()){
                        self.update_transfer(host,model,Some((done,total)),None,None);
                    }
                }
            }
            if completed{break;}
        }
        if !completed {return Err("Ollama stopped streaming before confirming completion".into());}
        Ok(())
    }
    async fn download_llama(&self,host:&str,model:&str,
        mut cancel:watch::Receiver<bool>)->Result<()> {
        let response=self.client.post(format!("{host}/models"))
            .json(&json!({"model":model,"sidecars":["mmproj"]}))
            .timeout(Duration::from_secs(15)).send().await
            .map_err(|e|format!("llama.cpp download API unavailable: {e}"))?;
        let status=response.status();
        if !status.is_success(){
            return Err(http_error(status,&response.text().await.unwrap_or_default()));
        }
        self.update_transfer(host,model,None,Some("downloading"),None);
        let response=self.client.get(format!("{host}/models/sse"))
            .timeout(Duration::from_secs(60*60)).send().await
            .map_err(|e|format!("llama.cpp progress stream unavailable: {e}"))?;
        if !response.status().is_success(){
            return Err(format!("llama.cpp progress endpoint HTTP {}",response.status()));
        }
        let mut stream=response.bytes_stream();
        let mut buffer=Vec::<u8>::new();
        loop {
            let next=tokio::select! {
                _=cancel.changed()=>return Err("Cancelled".into()),
                next=stream.next()=>next
            };
            let Some(next)=next else{break};
            buffer.extend_from_slice(&next.map_err(|e|e.to_string())?);
            if buffer.len()>512*1024 {return Err("Unexpected oversized llama.cpp progress frame".into());}
            while let Some(at)=buffer.iter().position(|b|*b==b'\n') {
                let line:Vec<u8>=buffer.drain(..=at).collect();
                let line=String::from_utf8_lossy(&line);
                let Some(raw)=line.trim().strip_prefix("data:") else{continue};
                let Ok(ev)=serde_json::from_str::<Value>(raw.trim()) else{continue};
                if ev["model"].as_str()!=Some(model) {continue;}
                match ev["event"].as_str().unwrap_or("") {
                    "download_finished"=>return Ok(()),
                    "download_failed"=>return Err(ev["data"]["error"].as_str()
                        .unwrap_or("llama.cpp download failed").into()),
                    "download_progress"=>{
                        let mut done=0_u64;let mut total=0_u64;
                        if let Some(files)=ev["data"].as_object(){
                            for f in files.values(){
                                done=done.saturating_add(f["done"].as_u64().unwrap_or(0));
                                total=total.saturating_add(f["total"].as_u64().unwrap_or(0));
                            }
                        }
                        self.update_transfer(host,model,Some((done,total)),None,None);
                    }
                    _=>{}
                }
            }
        }
        Err("llama.cpp progress stream disconnected; check model status and retry if needed".into())
    }

    pub(super) async fn model_cancel_download(&self,session:&str,model:&str)->Result<Value>{
        validate_model(model)?;
        let host=self.host(session)?;
        let engine=self.engine(session)?;
        let key=Self::transfer_key(&host,model);
        {
            let downloads=self.downloads.lock().unwrap();
            let d=downloads.get(&key).ok_or("No active transfer for this model")?;
            if !matches!(d.status.as_str(),"starting"|"downloading"){
                return Err("Only an active download can be paused".into());
            }
            d.cancel.send_replace(true);
        }
        if engine==provider::Engine::LlamaCpp {
            // llama.cpp removes partials on DELETE; unlike Ollama, cancellation
            // here is destructive and the UI must state that explicitly.
            let mut url=reqwest::Url::parse(&format!("{host}/models")).map_err(|e|e.to_string())?;
            url.query_pairs_mut().append_pair("model",model);
            let result=self.client.delete(url).timeout(Duration::from_secs(10))
                .send().await.map_err(|e|e.to_string())?;
            if !result.status().is_success(){return Err(format!("llama.cpp cancel HTTP {}",result.status()));}
        }
        self.update_transfer(&host,model,None,Some("paused"),None);
        Ok(json!({"cancelled":true,"model":model}))
    }

    pub(super) async fn model_delete(&self,session:&str,model:&str)->Result<Value>{
        validate_model(model)?;
        let host=self.host(session)?;
        let engine=self.engine(session)?;
        {
            let s=self.state.lock().unwrap();
            if s.view.sessions.iter().any(|v|s.clients.get(&v.id).is_some_and(|c|c.host==host)
                && (v.active_model.as_deref().is_some_and(|a|provider::same_model(a,model))
                    ||v.loading_model.as_deref().is_some_and(|a|provider::same_model(a,model)))) {
                return Err("Model is loaded by a Lorca session. Unload it before deleting.".into());
            }
        }
        if self.downloads.lock().unwrap().values().any(|d|
            d.host==host && provider::same_model(&d.model,model)
                && matches!(d.status.as_str(),"starting"|"downloading")) {
            return Err("Pause the download before deleting the model".into());
        }
        let response=if engine==provider::Engine::Ollama{
            self.client.delete(format!("{host}/api/delete"))
                .json(&json!({"model":model})).timeout(Duration::from_secs(20))
                .send().await.map_err(|e|e.to_string())?
        }else{
            let mut url=reqwest::Url::parse(&format!("{host}/models")).map_err(|e|e.to_string())?;
            url.query_pairs_mut().append_pair("model",model);
            self.client.delete(url).timeout(Duration::from_secs(20))
                .send().await.map_err(|e|e.to_string())?
        };
        let status=response.status();
        if !status.is_success(){return Err(http_error(status,&response.text().await.unwrap_or_default()));}
        let _=self.refresh(session).await?;
        Ok(json!({"deleted":true,"model":model}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_does_not_report_negative_speed() {
        let (cancel,_)=watch::channel(false);
        let mut d=DownloadState{model:"test".into(),host:"localhost".into(),
            provider:provider::Engine::Ollama,status:"downloading".into(),
            completed:0,total:0,bytes_per_second:0.0,error:None,
            updated:Instant::now()-Duration::from_secs(1),
            previous_bytes:0,cancel};
        d.mark(100,200);
        assert!(d.bytes_per_second>=0.0);
        assert_eq!(d.view()["total"],200);
    }
}
