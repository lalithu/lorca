use super::*;

impl Core {
    /// Read-only model and hardware advice. This never downloads, swaps or
    /// resizes a loaded model. Inference-specific memory measurements from
    /// llama.cpp take precedence over Lorca's rough disk-weight estimate.
    pub(super) async fn hardware_advice(&self, session: &str, online: bool) -> Result<Value> {
        let hardware = crate::hardware::HardwareSnapshot::detect(&self.repository.root);
        let engine = self.engine(session)?;
        let host = self.host(session)?;
        let endpoint = if engine == provider::Engine::Ollama { "/api/tags" } else { "/models" };
        let list: Value = self.client.get(format!("{host}{endpoint}"))
            .timeout(Duration::from_secs(5)).send().await.map_err(|e|e.to_string())?
            .error_for_status().map_err(|e|e.to_string())?
            .json().await.map_err(|e|e.to_string())?;
        let installed = if engine == provider::Engine::Ollama {
            crate::hardware::installed_ollama(&hardware, &list)
        } else {
            crate::hardware::installed_llama_cpp(&list)
        };
        let mut discovery = Vec::new();
        let mut discovery_warning: Option<String> = None;
        if online {
            let url = "https://huggingface.co/api/models?filter=gguf&sort=downloads&direction=-1&limit=12";
            match self.client.get(url).timeout(Duration::from_secs(8)).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.json::<Value>().await {
                        Ok(value) => {
                            for item in value.as_array().into_iter().flatten() {
                                if let Some(id) = item["id"].as_str() {
                                    discovery.push(json!({
                                        "id":id,"downloads":item["downloads"],
                                        "url":format!("https://huggingface.co/{id}"),
                                        "hardwareFit":"unverified: inspect the exact GGUF file size, context and tool capabilities before download"
                                    }));
                                }
                            }
                        }
                        Err(e) => discovery_warning = Some(e.to_string()),
                    }
                }
                Ok(response) => discovery_warning = Some(format!("Hub catalog HTTP {}",response.status())),
                Err(e) => discovery_warning = Some(e.to_string()),
            }
        }
        Ok(json!({
            "hardware":hardware,
            "runtime":if engine == provider::Engine::Ollama {"ollama"} else {"llama.cpp"},
            "installedModels":installed,
            "hubCandidates":discovery,
            "hubWarning":discovery_warning,
            "note":"These are conservative resource hints, not model-quality rankings or verified context allocations. Manual model selection and current context overrides are preserved. No download or reload was performed."
        }))
    }

    pub(super) async fn refresh(&self, session: &str) -> Result<Value> {
        let host = self.host(session)?;
        let engine = self.engine(session)?;
        let outcome = provider::model_list(&self.client, engine, &host).await;
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        match outcome {
            Ok((models, loaded)) => {
                s.view.sessions[i].models = models;
                if let Some(loaded) = loaded {
                    s.view.sessions[i].active_model = Some(loaded.clone());
                    s.view.sessions[i].models_to_release = vec![loaded.clone()];
                }
                let selected_available = s.view.sessions[i]
                    .models
                    .iter()
                    .any(|name| provider::same_model(name, &s.view.sessions[i].model));
                if !selected_available
                    && let Some(model) = s.view.sessions[i]
                        .active_model
                        .clone()
                        .or_else(|| s.view.sessions[i].models.first().cloned())
                {
                    s.view.sessions[i].model = model;
                }
                s.view.sessions[i].is_connected = true;
                s.view.sessions[i].error = None;
            }
            Err(e) => {
                s.view.sessions[i].is_connected = false;
                s.view.sessions[i].error = Some(e);
            }
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }
    pub(super) fn held_by_other(s: &State, session: &str, model: &str, host: &str) -> bool {
        s.view.sessions.iter().any(|v| {
            v.id != session
                && s.clients.get(&v.id).is_some_and(|c| c.host == host)
                && v.models_to_release
                    .iter()
                    .any(|m| provider::same_model(m, model))
        })
    }
    pub(super) async fn prepare(
        &self,
        session: &str,
        model: &str,
        unload_previous: bool,
    ) -> Result<Value> {
        if model.is_empty() || model.len() > 500 || model.chars().any(char::is_control) {
            return Err("Invalid model name".into());
        }
        let _guard = self.lifecycle.lock().await;
        let host = self.host(session)?;
        let engine = self.engine(session)?;
        let (load_cancel, mut load_rx) = watch::channel(false);
        let previous = {
            let mut s = self.state.lock().unwrap();
            let i = Self::session_index(&s, session)?;
            if s.view.sessions[i]
                .active_model
                .as_deref()
                .is_some_and(|m| provider::same_model(m, model))
            {
                s.view.sessions[i].error = None;
                self.publish(&mut s);
                return Ok(json!(s.view));
            }
            s.loads.insert(session.into(), load_cancel);
            s.view.sessions[i].loading_model = Some(model.into());
            s.view.sessions[i].error = None;
            let previous = s.view.sessions[i].active_model.clone();
            if !s.view.sessions[i]
                .models_to_release
                .iter()
                .any(|m| provider::same_model(m, model))
            {
                s.view.sessions[i].models_to_release.push(model.into());
            }
            self.publish(&mut s);
            previous
        };
        let mut released = false;
        let operation = async {
            if unload_previous && let Some(old) = &previous {
                let shared = {
                    let s = self.state.lock().unwrap();
                    Self::held_by_other(&s, session, old, &host)
                };
                if !shared {
                    provider::manage(&self.client, engine, &host, old, false).await?;
                    released = true;
                    let mut s = self.state.lock().unwrap();
                    let i = Self::session_index(&s, session)?;
                    s.view.sessions[i]
                        .models_to_release
                        .retain(|m| !provider::same_model(m, old));
                    s.view.sessions[i].active_model = None;
                }
            }
            // Another lorca client may already hold this exact allocation.
            let shared = {
                let s = self.state.lock().unwrap();
                s.view.sessions.iter().any(|v| {
                    v.id != session
                        && s.clients.get(&v.id).is_some_and(|c| c.host == host)
                        && v.active_model
                            .as_deref()
                            .is_some_and(|m| provider::same_model(m, model))
                })
            };
            if !shared {
                provider::manage(&self.client, engine, &host, model, true).await?;
            }
            Ok::<_, String>(())
        };
        let result = tokio::select! { result=operation=>result, _=load_rx.changed()=>Err("Model loading cancelled.".into()) };
        self.state.lock().unwrap().loads.remove(session);
        if result.is_err()
            && !matches!(&result,Err(message) if message=="Model loading cancelled.")
            && released
            && let Some(old) = &previous
        {
            let _ = provider::manage(&self.client, engine, &host, old, true)
                .await
                .map(|()| {
                    let mut s = self.state.lock().unwrap();
                    if let Ok(i) = Self::session_index(&s, session) {
                        s.view.sessions[i].active_model = Some(old.clone());
                        s.view.sessions[i].models_to_release.push(old.clone());
                    }
                });
        }
        let limit = if result.is_ok() {
            self.context_limit_for(engine, &host, model).await
        } else {
            None
        };
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        s.view.sessions[i].loading_model = None;
        match result {
            Ok(()) => {
                s.view.sessions[i].active_model = Some(model.into());
                s.view.sessions[i].model = model.into();
                s.view.sessions[i].is_connected = true;
                let selected = s.view.sessions[i].selected_id.clone();
                let context_used = s
                    .view
                    .conversations
                    .iter()
                    .find(|c| c.id == selected)
                    .and_then(|c| c.messages.is_empty().then_some(0));
                s.view.sessions[i].allocated_context = limit;
                s.view.sessions[i].usage = Usage {
                    context_used,
                    context_limit: limit,
                    ..Default::default()
                };
            }
            Err(e) => s.view.sessions[i].error = Some(e),
        }
        self.publish(&mut s);
        Ok(json!(s.view))
    }
    pub(super) async fn context_limit(&self, host: &str, model: &str) -> Option<u64> {
        let v: Value = self
            .client
            .get(format!("{host}/api/ps"))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        v["models"].as_array()?.iter().find(|m| {
            m["name"]
                .as_str()
                .or_else(|| m["model"].as_str())
                .is_some_and(|n| provider::same_model(n, model))
        })?["context_length"]
            .as_u64()
            .filter(|v| *v > 0)
    }
    pub(super) async fn context_limit_for(
        &self,
        engine: provider::Engine,
        host: &str,
        model: &str,
    ) -> Option<u64> {
        if engine == provider::Engine::Ollama {
            return self.context_limit(host, model).await;
        }
        let mut url = reqwest::Url::parse(&format!("{host}/props")).ok()?;
        url.query_pairs_mut().append_pair("model", model);
        let value: Value = self
            .client
            .get(url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        if let Some(limit) = value["default_generation_settings"]["n_ctx"]
            .as_u64()
            .filter(|n| *n > 0)
        {
            return Some(limit);
        }
        // Router /props describes the router itself (n_ctx = 0). Read the selected
        // model's actual allocation, never its larger training context_length.
        let models: Value = self
            .client
            .get(format!("{host}/models"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .await
            .ok()?;
        models["data"].as_array()?.iter().find(|m| {
            m["id"]
                .as_str()
                .is_some_and(|id| provider::same_model(id, model))
        })?["meta"]["n_ctx"]
            .as_u64()
            .filter(|n| *n > 0)
    }
    pub(super) async fn release(&self, session: &str, model: &str) -> Result<()> {
        let _guard = self.lifecycle.lock().await;
        let host = self.host(session)?;
        let engine = self.engine(session)?;
        {
            let s = self.state.lock().unwrap();
            if Self::held_by_other(&s, session, model, &host) {
                return Err(format!(
                    "{model} is still used by another lorca session; kept loaded."
                ));
            }
        }
        provider::manage(&self.client, engine, &host, model, false).await?;
        let mut s = self.state.lock().unwrap();
        let i = Self::session_index(&s, session)?;
        s.view.sessions[i]
            .models_to_release
            .retain(|m| !provider::same_model(m, model));
        if s.view.sessions[i]
            .active_model
            .as_deref()
            .is_some_and(|m| provider::same_model(m, model))
        {
            s.view.sessions[i].active_model = None;
        }
        self.publish(&mut s);
        Ok(())
    }
    pub(super) async fn close(&self, session: &str, unload: bool) -> Result<Value> {
        self.agent.cancel_owner(session);
        let names = {
            let mut s = self.state.lock().unwrap();
            let i = Self::session_index(&s, session)?;
            Self::cancel_session(&mut s, session);
            if let Some(cancel) = s.loads.get(session) {
                cancel.send_replace(true);
            }
            s.view.sessions[i].models_to_release.clone()
        };
        for _ in 0..200 {
            if self
                .snapshot()
                .turns
                .iter()
                .all(|t| t.session_id != session || !t.running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut failed = vec![];
        if unload {
            // Another client's model preparation can hold the lifecycle lock.
            // Give all retirements one shared budget, then release this session
            // even when the provider or another allocation is unresponsive.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            for model in names {
                if !matches!(
                    tokio::time::timeout_at(deadline, self.release(session, &model)).await,
                    Ok(Ok(()))
                ) {
                    failed.push(model);
                }
            }
        }
        for _ in 0..200 {
            if !self.state.lock().unwrap().loads.contains_key(session) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.maintenance();
        let mut s = self.state.lock().unwrap();
        s.clients.remove(session);
        s.view.sessions.retain(|v| v.id != session);
        self.publish(&mut s);
        Ok(json!({"failures":failed}))
    }
    pub(super) fn cancel_session(s: &mut State, session: &str) {
        if let Some(cancel) = s.loads.get(session) {
            cancel.send_replace(true);
        }
        for t in s
            .view
            .turns
            .iter()
            .filter(|t| t.session_id == session && t.running)
        {
            if let Some(cancel) = s.cancels.get(&t.id) {
                cancel.send_replace(true);
            }
        }
    }
}
